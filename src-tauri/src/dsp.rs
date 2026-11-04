//! Audio analysis for the visualisers: PCM in, [`AudioFrame`] out.
//!
//! A port of the macOS analyser (Sources/Visualizers/AudioAnalyzer.swift,
//! itself adapted from Synesthia by Jon Jaques, MIT licence): a 2048-sample
//! sliding window analysed every 1024 new samples — Hann window → FFT →
//! log-spaced bands → auto-gain dB mapping → attack/release smoothing →
//! bass/mid/treble, spectral flux, beat envelope and spectral centroid. The
//! analysis runs on the macOS layout of 64 bands, so every feature covers
//! the same FFT bins with the same scale as on macOS (each band owns at
//! least one bin, so below ~850 Hz a 32-band layout cannot reproduce those
//! spans). The renderer's spectrum, [`AudioFrame::bands`], is a separate
//! 32-band reduction of the same FFT. The raw 2048-sample window is shipped
//! as `pcm` for the Wavefield engine (its FFT size).
//!
//! Like macOS (VisualizerAudio.swift), left and right are analysed
//! separately and merged — the maximum of every band and feature, the
//! centroid of the louder channel, the PCM window of the stronger one — so
//! opposite-phase content cannot cancel as it would in a mono downmix.
//! Also decodes the PCM formats WASAPI hands out and folds multichannel
//! layouts into that stereo pair. Platform independent and unit-tested on
//! every host; only the Windows audio capture uses it at run time.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::VecDeque;
use std::ops::Range;

use lunalyr_core::contract::AudioFrame;

/// FFT length and the PCM window length (~43 ms at 48 kHz).
pub const FFT_SIZE: usize = 2048;
/// New samples per analysis pass (50% overlap, ~47 passes/s at 48 kHz).
pub const HOP: usize = 1024;
/// Log-spaced bands in [`AudioFrame::bands`] (the renderer's spectrum).
pub const BAND_COUNT: usize = 32;
/// Log-spaced bands the features are computed from: the macOS layout
/// (`AudioSnapshot.bandCount`), so the ranges below and the flux and
/// centroid scales are exactly the macOS ones.
const ANALYSIS_BANDS: usize = 64;

const MIN_HZ: f64 = 30.0;
const MAX_HZ: f64 = 16_000.0;
// macOS ranges over the 64 analysis bands. At 48 kHz: FFT bins 2-11
// (~47-258 Hz), 22-78 (~0.5-1.8 kHz) and 117-682 (~2.7-16 kHz).
const BASS: Range<usize> = 0..10;
const MID: Range<usize> = 20..42;
const TREBLE: Range<usize> = 46..ANALYSIS_BANDS;

// Loudness normalisation (slow AGC), as on macOS.
const AGC_REFERENCE_DB: f32 = -8.0;
const AGC_GATE_DB: f32 = -55.0;
const AGC_MAX_BOOST_DB: f32 = 24.0;
const AGC_MAX_CUT_DB: f32 = 12.0;
const AGC_PEAK_DECAY_DB: f32 = 0.053;

// ---- PCM decoding -------------------------------------------------------------------

/// Sample encodings found in WASAPI mix formats (little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    F64,
    I16,
    /// Packed 3-byte samples.
    I24,
    /// 32-bit integer container (also 24 valid bits, MSB-aligned).
    I32,
}

impl SampleFormat {
    /// From a WAVEFORMATEX(TENSIBLE): IEEE float or integer PCM, and the
    /// container size `wBitsPerSample`.
    pub fn from_wave(is_float: bool, container_bits: u16) -> Option<Self> {
        match (is_float, container_bits) {
            (true, 32) => Some(SampleFormat::F32),
            (true, 64) => Some(SampleFormat::F64),
            (false, 16) => Some(SampleFormat::I16),
            (false, 24) => Some(SampleFormat::I24),
            (false, 32) => Some(SampleFormat::I32),
            _ => None,
        }
    }

    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::F32 | SampleFormat::I32 => 4,
            SampleFormat::F64 => 8,
            SampleFormat::I16 => 2,
            SampleFormat::I24 => 3,
        }
    }

    /// One sample as -1..1 (`bytes` is exactly `self.bytes()` long).
    fn decode(self, bytes: &[u8]) -> f32 {
        fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
            bytes.try_into().unwrap_or([0; N])
        }
        match self {
            SampleFormat::F32 => f32::from_le_bytes(array(bytes)),
            SampleFormat::F64 => f64::from_le_bytes(array(bytes)) as f32,
            SampleFormat::I16 => f32::from(i16::from_le_bytes(array(bytes))) / 32_768.0,
            SampleFormat::I24 => {
                let [a, b, c] = array::<3>(bytes);
                (i32::from_le_bytes([0, a, b, c]) >> 8) as f32 / 8_388_608.0
            }
            SampleFormat::I32 => i32::from_le_bytes(array(bytes)) as f32 / 2_147_483_648.0,
        }
    }
}

/// Speaker positions: the bits of `WAVEFORMATEXTENSIBLE::dwChannelMask`
/// (ksmedia.h `SPEAKER_*`). Interleaved channels follow the set bits in
/// ascending order; channels beyond them have no position.
pub mod speaker {
    pub const FRONT_LEFT: u32 = 0x1;
    pub const FRONT_RIGHT: u32 = 0x2;
    pub const LOW_FREQUENCY: u32 = 0x8;
    pub const BACK_LEFT: u32 = 0x10;
    pub const BACK_RIGHT: u32 = 0x20;
    pub const FRONT_LEFT_OF_CENTER: u32 = 0x40;
    pub const FRONT_RIGHT_OF_CENTER: u32 = 0x80;
    pub const SIDE_LEFT: u32 = 0x200;
    pub const SIDE_RIGHT: u32 = 0x400;
    pub const TOP_FRONT_LEFT: u32 = 0x1000;
    pub const TOP_FRONT_RIGHT: u32 = 0x4000;
    pub const TOP_BACK_LEFT: u32 = 0x8000;
    pub const TOP_BACK_RIGHT: u32 = 0x2_0000;
    /// Every defined position (bits 0-17); anything else is reserved or
    /// SPEAKER_ALL, i.e. no usable layout.
    pub const DEFINED: u32 = 0x3_FFFF;
}

/// Centre channels reach both sides at -3 dB.
const CENTRE_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// How the interleaved channels of a capture format feed the left and right
/// analysers. Stereo is kept as it is. More channels fold into the pair the
/// way a stereo mixdown does (macOS analyses the system's stereo tap):
/// left = FL + FLC + SL + BL (+ top left) + 0.707·(centres), right likewise;
/// the LFE channel is left out. Without a usable channel mask, channels 0
/// and 1 are left and right and the rest go to both at -3 dB.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelFold {
    /// (left, right) gain per interleaved channel.
    gains: Vec<(f32, f32)>,
}

impl ChannelFold {
    /// `channel_mask`: `WAVEFORMATEXTENSIBLE::dwChannelMask`, 0 when the
    /// format has none.
    pub fn new(channels: usize, channel_mask: u32) -> Self {
        let gains = match channels {
            0 => Vec::new(),
            1 => vec![(1.0, 0.0)],
            2 => vec![(1.0, 0.0), (0.0, 1.0)],
            _ if channel_mask == 0 || channel_mask & !speaker::DEFINED != 0 => (0..channels)
                .map(|channel| match channel {
                    0 => (1.0, 0.0),
                    1 => (0.0, 1.0),
                    _ => (CENTRE_GAIN, CENTRE_GAIN),
                })
                .collect(),
            _ => {
                let mut positions = (0..32).map(|bit| 1u32 << bit).filter(|position| channel_mask & position != 0);
                (0..channels).map(|_| positions.next().map_or((CENTRE_GAIN, CENTRE_GAIN), fold_gains)).collect()
            }
        };
        ChannelFold { gains }
    }

    /// Two analysis channels (the source has two or more).
    pub fn is_stereo(&self) -> bool {
        self.gains.len() >= 2
    }

    /// Appends `frames` silent frames (a silent WASAPI packet, or silence
    /// synthesised while nothing plays).
    pub fn silence(&self, frames: usize, left: &mut Vec<f32>, right: &mut Vec<f32>) {
        left.resize(left.len() + frames, 0.0);
        if self.is_stereo() {
            right.resize(right.len() + frames, 0.0);
        }
    }

    /// Decodes interleaved frames (`block_align` bytes each) and appends
    /// them to `left` and (stereo only) `right`; returns the number of
    /// frames decoded. Non-finite samples count as silence; the results are
    /// clamped to -1..1.
    pub fn decode(
        &self,
        data: &[u8],
        format: SampleFormat,
        block_align: usize,
        left: &mut Vec<f32>,
        right: &mut Vec<f32>,
    ) -> usize {
        let width = format.bytes();
        let channels = self.gains.len();
        if channels == 0 || block_align == 0 || block_align < channels * width {
            return 0;
        }
        let stereo = self.is_stereo();
        let frames = data.len() / block_align;
        left.reserve(frames);
        if stereo {
            right.reserve(frames);
        }
        for frame in data.chunks_exact(block_align) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (sample, &(to_left, to_right)) in frame[..channels * width].chunks_exact(width).zip(&self.gains) {
                let value = format.decode(sample);
                if value.is_finite() {
                    l += value * to_left;
                    r += value * to_right;
                }
            }
            left.push(l.clamp(-1.0, 1.0));
            if stereo {
                right.push(r.clamp(-1.0, 1.0));
            }
        }
        frames
    }
}

/// (left, right) gains of the channel at speaker `position`.
fn fold_gains(position: u32) -> (f32, f32) {
    use speaker::*;
    match position {
        FRONT_LEFT | FRONT_LEFT_OF_CENTER | SIDE_LEFT | BACK_LEFT | TOP_FRONT_LEFT | TOP_BACK_LEFT => (1.0, 0.0),
        FRONT_RIGHT | FRONT_RIGHT_OF_CENTER | SIDE_RIGHT | BACK_RIGHT | TOP_FRONT_RIGHT | TOP_BACK_RIGHT => (0.0, 1.0),
        LOW_FREQUENCY => (0.0, 0.0),
        // Front (0x4), back (0x100) and top (0x800, 0x2000, 0x1_0000) centre.
        _ => (CENTRE_GAIN, CENTRE_GAIN),
    }
}

// ---- base64 --------------------------------------------------------------------------

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (what the renderer's `atob` expects).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(char::from(BASE64[(n >> 18) as usize & 63]));
        out.push(char::from(BASE64[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 { char::from(BASE64[(n >> 6) as usize & 63]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(BASE64[n as usize & 63]) } else { '=' });
    }
    out
}

// ---- FFT -----------------------------------------------------------------------------

/// In-place iterative radix-2 complex FFT (forward, unscaled).
struct Fft {
    n: usize,
    /// e^(-2πik/n) for k in 0..n/2.
    twiddle_re: Vec<f32>,
    twiddle_im: Vec<f32>,
    reversed: Vec<usize>,
}

impl Fft {
    /// `n` must be a power of two ≥ 2.
    fn new(n: usize) -> Self {
        debug_assert!(n >= 2 && n.is_power_of_two());
        let bits = n.trailing_zeros();
        let reversed = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let (twiddle_re, twiddle_im) = (0..n / 2)
            .map(|k| {
                let angle = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
                (angle.cos() as f32, angle.sin() as f32)
            })
            .unzip();
        Fft { n, twiddle_re, twiddle_im, reversed }
    }

    fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        if re.len() != n || im.len() != n {
            return;
        }
        for (i, &j) in self.reversed.iter().enumerate() {
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.twiddle_re[k * step], self.twiddle_im[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

// ---- Analyser ---------------------------------------------------------------------

/// Streaming analyser for one channel. Feed it samples with
/// [`Analyzer::push`]; read the newest result with [`Analyzer::frame`].
pub struct Analyzer {
    sample_rate: u32,
    fft: Fft,
    window: Vec<f32>,
    /// The newest `FFT_SIZE` samples, oldest first (zero-filled at start).
    recent: VecDeque<f32>,
    pending: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    power: Vec<f32>,
    /// `BAND_COUNT + 1` FFT-bin indices delimiting the spectrum bands.
    edges: Vec<usize>,
    /// `ANALYSIS_BANDS + 1` FFT-bin indices delimiting the analysis bands.
    analysis_edges: Vec<usize>,
    /// The smoothed spectrum shipped in `AudioFrame::bands`.
    bands: [f32; BAND_COUNT],
    /// This pass's analysis bands, swapped with `previous_raw` afterwards.
    raw: [f32; ANALYSIS_BANDS],
    previous_raw: [f32; ANALYSIS_BANDS],
    smoothed: [f32; ANALYSIS_BANDS],
    level: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    beat_average: f32,
    beat_envelope: f32,
    beat_latched: bool,
    flux_envelope: f32,
    centroid: f32,
    agc_peak_db: f32,
    agc_primed: bool,
    agc_gain_db: f32,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        // vDSP_HANN_NORM: 0.8165 * (1 - cos(2πn/N)), i.e. an energy-normalised
        // (×sqrt(8/3)) periodic Hann window — matches the macOS levels.
        let window = (0..FFT_SIZE)
            .map(|n| (0.8165 * (1.0 - (2.0 * std::f64::consts::PI * n as f64 / FFT_SIZE as f64).cos())) as f32)
            .collect();
        let sample_rate = sanitize_rate(sample_rate);
        Analyzer {
            sample_rate,
            fft: Fft::new(FFT_SIZE),
            window,
            recent: VecDeque::from(vec![0.0; FFT_SIZE]),
            pending: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            power: vec![0.0; FFT_SIZE / 2],
            edges: band_edges(sample_rate, BAND_COUNT),
            analysis_edges: band_edges(sample_rate, ANALYSIS_BANDS),
            bands: [0.0; BAND_COUNT],
            raw: [0.0; ANALYSIS_BANDS],
            previous_raw: [0.0; ANALYSIS_BANDS],
            smoothed: [0.0; ANALYSIS_BANDS],
            level: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            beat_average: 0.0,
            beat_envelope: 0.0,
            beat_latched: false,
            flux_envelope: 0.0,
            centroid: 0.4,
            agc_peak_db: AGC_REFERENCE_DB,
            agc_primed: false,
            agc_gain_db: 0.0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Clears all state for a new source (device or sample-rate change), so
    /// the old source's tail does not bleed into the new one.
    pub fn reset(&mut self, sample_rate: u32) {
        *self = Analyzer::new(sample_rate);
    }

    /// Appends samples (-1..1). Runs one analysis pass per `HOP` new
    /// samples; returns true if at least one ran.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        let mut analysed = false;
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (HOP - self.pending).min(rest.len());
            let (now, later) = rest.split_at(take);
            for &sample in now {
                let sample = if sample.is_finite() { sample.clamp(-1.0, 1.0) } else { 0.0 };
                self.recent.pop_front();
                self.recent.push_back(sample);
            }
            self.pending += take;
            rest = later;
            if self.pending >= HOP {
                self.pending = 0;
                self.analyse();
                analysed = true;
            }
        }
        analysed
    }

    /// True once the input window is silent and every output has decayed.
    pub fn is_silent(&self) -> bool {
        self.level < 1e-3
            && self.beat_envelope < 0.01
            && self.flux_envelope < 0.01
            && self.bands.iter().chain(&self.smoothed).all(|band| *band < 0.01)
            && self.recent.iter().all(|sample| sample.abs() < 1e-6)
    }

    /// The newest analysis, with the PCM window for Wavefield.
    pub fn frame(&self) -> AudioFrame {
        AudioFrame {
            level: self.level,
            bass: self.bass,
            mid: self.mid,
            treble: self.treble,
            beat: self.beat_envelope,
            flux: self.flux_envelope.min(1.0),
            centroid: self.centroid,
            bands: self.bands.to_vec(),
            pcm: Some(self.pcm()),
            sample_rate: self.sample_rate,
        }
    }

    /// The newest `FFT_SIZE` samples as base64 of little-endian f32.
    fn pcm(&self) -> String {
        let mut bytes = Vec::with_capacity(FFT_SIZE * 4);
        for sample in &self.recent {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        base64(&bytes)
    }

    /// RMS of the PCM window (macOS `WavefieldPCMWindow.level`).
    fn window_rms(&self) -> f32 {
        (self.recent.iter().map(|sample| sample * sample).sum::<f32>() / FFT_SIZE as f32).sqrt()
    }

    fn analyse(&mut self) {
        // 1. Hann window → FFT → power per bin.
        for (index, (sample, weight)) in self.recent.iter().zip(&self.window).enumerate() {
            self.re[index] = sample * weight;
            self.im[index] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        for (bin, power) in self.power.iter_mut().enumerate() {
            *power = self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin];
        }

        // 2. Loudness (RMS in dB) and the auto-gain it drives.
        let mean_square = self.recent.iter().map(|sample| sample * sample).sum::<f32>() / FFT_SIZE as f32;
        let level_db = 20.0 * mean_square.sqrt().max(1e-9).log10();
        self.update_auto_gain(level_db);
        self.level = ((level_db + self.agc_gain_db + 60.0) / 60.0).clamp(0.0, 1.0);

        // 3. Bands on the macOS layout, and the renderer's spectrum, from the
        //    same FFT; each smoothed with a fast attack and a slow release.
        band_levels(&self.power, &self.analysis_edges, self.agc_gain_db, &mut self.raw);
        smooth(&mut self.smoothed, &self.raw);
        let mut spectrum = [0.0f32; BAND_COUNT];
        band_levels(&self.power, &self.edges, self.agc_gain_db, &mut spectrum);
        smooth(&mut self.bands, &spectrum);
        self.bass = mean(&self.smoothed[BASS]);
        self.mid = mean(&self.smoothed[MID]);
        self.treble = mean(&self.smoothed[TREBLE]);

        // 4. Spectral flux: new energy since the last pass, as an envelope.
        let flux_now: f32 = self.raw.iter().zip(&self.previous_raw).map(|(now, before)| (now - before).max(0.0)).sum();
        self.flux_envelope = (flux_now * 0.9).min(1.0).max(self.flux_envelope * 0.86);

        // 5. Spectral centroid (0 dark … 1 bright), only with enough energy.
        let total: f32 = self.raw.iter().sum();
        if total > 0.02 {
            let weighted: f32 = self.raw.iter().enumerate().map(|(band, value)| value * band as f32).sum();
            let instant = weighted / (total * (ANALYSIS_BANDS - 1) as f32);
            self.centroid = self.centroid * 0.9 + instant * 0.1;
        }

        // 6. Beat: bass energy 30% over its running average fires the
        //    envelope once; it re-arms below 1.1× (hysteresis).
        let instant = mean(&self.raw[BASS]);
        self.beat_average = self.beat_average * 0.975 + instant * 0.025;
        if !self.beat_latched && instant > self.beat_average * 1.3 && instant > 0.15 {
            self.beat_latched = true;
            self.beat_envelope = 1.0;
        } else {
            if instant < self.beat_average * 1.1 {
                self.beat_latched = false;
            }
            self.beat_envelope *= 0.9;
        }

        std::mem::swap(&mut self.raw, &mut self.previous_raw);
    }

    /// Slow AGC: track the program's recent peak level and steer the gain
    /// toward `reference - peak`; silence (below the gate) holds the gain.
    fn update_auto_gain(&mut self, level_db: f32) {
        if level_db > AGC_GATE_DB {
            if self.agc_primed {
                self.agc_peak_db = level_db.max(self.agc_peak_db - AGC_PEAK_DECAY_DB);
            } else {
                self.agc_peak_db = level_db;
                self.agc_primed = true;
            }
        }
        let target = (AGC_REFERENCE_DB - self.agc_peak_db).clamp(-AGC_MAX_CUT_DB, AGC_MAX_BOOST_DB);
        let speed = if target < self.agc_gain_db { 0.15 } else { 0.02 };
        self.agc_gain_db += (target - self.agc_gain_db) * speed;
    }
}

/// FFT-bin edges of `count` log-spaced bands between 30 Hz and
/// min(16 kHz, 0.45 × sample rate); each band owns at least one bin, so at
/// the low end the edges follow the band index rather than the frequency.
fn band_edges(sample_rate: u32, count: usize) -> Vec<usize> {
    let bins = FFT_SIZE / 2;
    let rate = f64::from(sample_rate);
    let hz = |bin: usize| bin as f64 * rate / FFT_SIZE as f64;
    let max_hz = MAX_HZ.min(rate * 0.45);
    let mut edges: Vec<usize> = Vec::with_capacity(count + 1);
    for band in 0..=count {
        let frequency = MIN_HZ * (max_hz / MIN_HZ).powf(band as f64 / count as f64);
        let mut bin = 0;
        while bin < bins - 1 && hz(bin) < frequency {
            bin += 1;
        }
        if let Some(&last) = edges.last() {
            bin = bin.max(last + 1);
        }
        edges.push(bin.min(bins - 1));
    }
    edges
}

/// One value per band: the RMS magnitude over the band's bins, mapped
/// through dB (-72 dB → 0, -6 dB → 1, shifted by the auto-gain). The 2/N
/// scale reproduces vDSP's real-FFT magnitudes.
fn band_levels(power: &[f32], edges: &[usize], gain_db: f32, out: &mut [f32]) {
    let norm = 2.0 / FFT_SIZE as f32;
    for (band, value) in out.iter_mut().enumerate() {
        let low = edges[band];
        let high = edges[band + 1].max(low + 1).min(power.len());
        let low = low.min(high.saturating_sub(1));
        let sum: f32 = power[low..high].iter().sum();
        let magnitude = (sum / (high - low) as f32).sqrt() * norm;
        let db = 20.0 * magnitude.max(1e-9).log10();
        *value = ((db + gain_db + 72.0) / 66.0).clamp(0.0, 1.0);
    }
}

/// Attack/release smoothing: rise 65% of the way per pass, fall 12%.
fn smooth(smoothed: &mut [f32], raw: &[f32]) {
    for (smoothed, &raw) in smoothed.iter_mut().zip(raw) {
        *smoothed = if raw > *smoothed { *smoothed * 0.35 + raw * 0.65 } else { *smoothed * 0.88 + raw * 0.12 };
    }
}

// ---- Stereo --------------------------------------------------------------------------

/// One [`Analyzer`] per channel of a stereo source, merged like macOS's
/// VisualizerAudio. A mono source uses the left one only.
pub struct StereoAnalyzer {
    left: Analyzer,
    right: Analyzer,
    /// The source has a right channel (see [`ChannelFold::is_stereo`]).
    stereo: bool,
}

impl StereoAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        StereoAnalyzer { left: Analyzer::new(sample_rate), right: Analyzer::new(sample_rate), stereo: false }
    }

    /// Prepares for a source with this rate and channel layout. A new rate
    /// clears everything (the old source's tail must not bleed into the new
    /// one); gaining or losing the right channel clears that one.
    pub fn configure(&mut self, sample_rate: u32, stereo: bool) {
        if sanitize_rate(sample_rate) != self.left.sample_rate() {
            self.left.reset(sample_rate);
            self.right.reset(sample_rate);
        } else if stereo != self.stereo {
            self.right.reset(sample_rate);
        }
        self.stereo = stereo;
    }

    /// Appends one block per channel (the same number of frames each;
    /// `right` is ignored for a mono source). True if an analysis pass ran.
    pub fn push(&mut self, left: &[f32], right: &[f32]) -> bool {
        let analysed = self.left.push(left);
        if self.stereo {
            self.right.push(right) || analysed
        } else {
            analysed
        }
    }

    /// True once every channel is silent.
    pub fn is_silent(&self) -> bool {
        self.left.is_silent() && (!self.stereo || self.right.is_silent())
    }

    /// The merged analysis (VisualizerAudio.latest / wavefieldPCM): the
    /// maximum of each band and feature, the centroid of the louder channel
    /// and the PCM window of the stronger one (left on a tie). Wavefield
    /// wants one channel, and picking one cannot cancel as a downmix can.
    pub fn frame(&self) -> AudioFrame {
        let (a, b) = (&self.left, &self.right);
        if !self.stereo {
            return a.frame();
        }
        let louder = if b.level > a.level { b } else { a };
        let stronger = if b.window_rms() > a.window_rms() { b } else { a };
        AudioFrame {
            level: a.level.max(b.level),
            bass: a.bass.max(b.bass),
            mid: a.mid.max(b.mid),
            treble: a.treble.max(b.treble),
            beat: a.beat_envelope.max(b.beat_envelope),
            flux: a.flux_envelope.max(b.flux_envelope).min(1.0),
            centroid: louder.centroid,
            bands: a.bands.iter().zip(&b.bands).map(|(l, r)| l.max(*r)).collect(),
            pcm: Some(stronger.pcm()),
            sample_rate: a.sample_rate,
        }
    }
}

fn sanitize_rate(rate: u32) -> u32 {
    if (8_000..=384_000).contains(&rate) {
        rate
    } else {
        48_000
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(frequency: f64, amplitude: f64, count: usize, offset: usize) -> Vec<f32> {
        (offset..offset + count)
            .map(|i| (amplitude * (2.0 * std::f64::consts::PI * frequency * i as f64 / f64::from(RATE)).sin()) as f32)
            .collect()
    }

    /// Deterministic white noise in -amplitude..amplitude.
    fn noise(amplitude: f32, count: usize, seed: &mut u64) -> Vec<f32> {
        (0..count)
            .map(|_| {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((*seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    fn decode_base64(text: &str) -> Vec<u8> {
        let value = |c: u8| BASE64.iter().position(|&b| b == c).map(|p| p as u32);
        let mut out = Vec::new();
        for chunk in text.as_bytes().chunks(4) {
            let n = chunk.iter().fold(0u32, |acc, &c| (acc << 6) | value(c).unwrap_or(0));
            let pad = chunk.iter().filter(|&&c| c == b'=').count();
            let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            out.extend_from_slice(&bytes[..3 - pad]);
        }
        out
    }

    #[test]
    fn fft_matches_a_naive_dft() {
        let n = 64;
        let fft = Fft::new(n);
        let mut seed = 7;
        let input = noise(1.0, n, &mut seed);
        let mut re = input.clone();
        let mut im = vec![0.0; n];
        fft.forward(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0f64, 0.0f64);
            for (t, &x) in input.iter().enumerate() {
                let angle = -2.0 * std::f64::consts::PI * (k * t) as f64 / n as f64;
                sr += f64::from(x) * angle.cos();
                si += f64::from(x) * angle.sin();
            }
            assert!((f64::from(re[k]) - sr).abs() < 1e-3, "re[{k}]");
            assert!((f64::from(im[k]) - si).abs() < 1e-3, "im[{k}]");
        }
    }

    #[test]
    fn fft_puts_a_bin_centred_sine_in_its_bin() {
        let fft = Fft::new(FFT_SIZE);
        let bin = 100;
        let mut re: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (2.0 * std::f64::consts::PI * bin as f64 * i as f64 / FFT_SIZE as f64).sin() as f32)
            .collect();
        let mut im = vec![0.0; FFT_SIZE];
        fft.forward(&mut re, &mut im);
        let magnitude = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        assert!((magnitude(bin) - FFT_SIZE as f32 / 2.0).abs() < 0.5);
        assert!(magnitude(bin + 3) < 1e-2);
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    fn f32_bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// Decodes with `fold`, returning (left, right).
    fn decode(fold: &ChannelFold, data: &[u8], format: SampleFormat, block_align: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        fold.decode(data, format, block_align, &mut left, &mut right);
        (left, right)
    }

    /// 5.1 (FL FR FC LFE BL BR) and 7.1 (+ SL SR) channel masks.
    const MASK_5_1: u32 = 0x3F;
    const MASK_7_1: u32 = 0x63F;
    const FRONT_CENTER: u32 = 0x4;

    #[test]
    fn decode_reads_wasapi_formats() {
        // Stereo float: left and right stay separate (no (L + R) / 2).
        let stereo = ChannelFold::new(2, 0);
        assert!(stereo.is_stereo());
        let (mut left, mut right) = (Vec::new(), Vec::new());
        assert_eq!(stereo.decode(&f32_bytes(&[0.5, 0.25, -1.0, -1.0]), SampleFormat::F32, 8, &mut left, &mut right), 2);
        assert_eq!((left, right), (vec![0.5, -1.0], vec![0.25, -1.0]));

        // Mono: the left channel only.
        let mono = ChannelFold::new(1, 0);
        assert!(!mono.is_stereo());
        let pcm16: Vec<u8> = [16_384i16, -32_768].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(decode(&mono, &pcm16, SampleFormat::I16, 2), (vec![0.5, -1.0], vec![]));
        // Packed 24-bit: 0x400000 = +0.5, 0xC00000 = -0.5.
        let (left, _) = decode(&mono, &[0x00, 0x00, 0x40, 0x00, 0x00, 0xC0], SampleFormat::I24, 3);
        assert_eq!(left, vec![0.5, -0.5]);
        // 24 valid bits in a 32-bit container are MSB-aligned.
        let (left, _) = decode(&mono, &0x4000_0000i32.to_le_bytes(), SampleFormat::I32, 4);
        assert_eq!(left, vec![0.5]);

        // NaN counts as silence; block_align too small is rejected.
        let bad = f32_bytes(&[f32::NAN, 0.5]);
        assert_eq!(decode(&stereo, &bad, SampleFormat::F32, 8), (vec![0.0], vec![0.5]));
        let (mut left, mut right) = (Vec::new(), Vec::new());
        assert_eq!(stereo.decode(&bad, SampleFormat::F32, 4, &mut left, &mut right), 0);
        assert!(left.is_empty() && right.is_empty());
        assert_eq!(SampleFormat::from_wave(true, 32), Some(SampleFormat::F32));
        assert_eq!(SampleFormat::from_wave(false, 8), None);

        // Silent packets go to every analysed channel.
        let (mut left, mut right) = (vec![0.5], vec![0.5]);
        stereo.silence(2, &mut left, &mut right);
        assert_eq!((left, right), (vec![0.5, 0.0, 0.0], vec![0.5, 0.0, 0.0]));
        let (mut left, mut right) = (Vec::new(), Vec::new());
        mono.silence(2, &mut left, &mut right);
        assert_eq!((left.len(), right.len()), (2, 0));
    }

    #[test]
    fn multichannel_folds_into_the_stereo_pair() {
        let c = std::f32::consts::FRAC_1_SQRT_2;
        let close = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);

        // Stereo content in a 5.1 mix format comes through exactly.
        let five_one = ChannelFold::new(6, MASK_5_1);
        let (left, right) = decode(&five_one, &f32_bytes(&[0.6, -0.3, 0.0, 0.0, 0.0, 0.0]), SampleFormat::F32, 24);
        assert!(close(&left, &[0.6]) && close(&right, &[-0.3]), "{left:?} {right:?}");
        // Only the centre speaks (dialogue): both sides at -3 dB.
        let (left, right) = decode(&five_one, &f32_bytes(&[0.0, 0.0, 0.5, 0.0, 0.0, 0.0]), SampleFormat::F32, 24);
        assert!(close(&left, &[0.5 * c]) && close(&right, &[0.5 * c]), "{left:?} {right:?}");
        // Surrounds join their side; the LFE is left out.
        let (left, right) = decode(&five_one, &f32_bytes(&[0.0, 0.0, 0.0, 0.9, 0.25, -0.5]), SampleFormat::F32, 24);
        assert!(close(&left, &[0.25]) && close(&right, &[-0.5]), "{left:?} {right:?}");

        // 7.1: side channels too; the sum is clamped.
        let seven_one = ChannelFold::new(8, MASK_7_1);
        let (left, right) =
            decode(&seven_one, &f32_bytes(&[0.8, 0.1, 0.0, 0.0, 0.0, 0.0, 0.7, 0.2]), SampleFormat::F32, 32);
        assert!(close(&left, &[1.0]) && close(&right, &[0.3]), "{left:?} {right:?}");

        // A mask with fewer positions than channels: the extra ones go to both.
        let partial = ChannelFold::new(3, 0x3);
        let (left, right) = decode(&partial, &f32_bytes(&[0.1, 0.2, 0.4]), SampleFormat::F32, 12);
        assert!(close(&left, &[0.1 + 0.4 * c]) && close(&right, &[0.2 + 0.4 * c]), "{left:?} {right:?}");
        // A centre-only mono layout in a three-channel format.
        let centre = ChannelFold::new(3, FRONT_CENTER | 0x1 | 0x2);
        let (left, right) = decode(&centre, &f32_bytes(&[0.0, 0.0, 0.4]), SampleFormat::F32, 12);
        assert!(close(&left, &[0.4 * c]) && close(&right, &[0.4 * c]), "{left:?} {right:?}");

        // No usable mask (none, or SPEAKER_ALL): channels 0 and 1 are left and
        // right, the rest go to both.
        for mask in [0, 0x8000_0000] {
            let unknown = ChannelFold::new(4, mask);
            let (left, right) = decode(&unknown, &f32_bytes(&[0.1, 0.2, 0.4, 0.0]), SampleFormat::F32, 16);
            assert!(close(&left, &[0.1 + 0.4 * c]) && close(&right, &[0.2 + 0.4 * c]), "{mask:#x}: {left:?} {right:?}");
        }
    }

    /// Interleaves two channels as a stereo F32 WASAPI buffer.
    fn interleave(left: &[f32], right: &[f32]) -> Vec<u8> {
        left.iter().zip(right).flat_map(|(l, r)| l.to_le_bytes().into_iter().chain(r.to_le_bytes())).collect()
    }

    fn pcm_samples(frame: &AudioFrame) -> Vec<f32> {
        let pcm = decode_base64(frame.pcm.as_deref().unwrap_or_default());
        pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
    }

    #[test]
    fn opposite_phase_stereo_does_not_cancel() {
        // macOS testStereoFFTDoesNotCancelOppositePhaseAudio: a 1 kHz sine at
        // 0.25 in the left channel and its negation in the right.
        let left = sine(1_000.0, 0.25, 18 * FFT_SIZE, 0);
        let right: Vec<f32> = left.iter().map(|s| -s).collect();
        let fold = ChannelFold::new(2, 0);
        let (l, r) = decode(&fold, &interleave(&left, &right), SampleFormat::F32, 8);
        // A (L + R) / 2 downmix of this is exactly silent.
        assert!(l.iter().zip(&r).all(|(a, b)| (a + b) * 0.5 == 0.0));

        let mut analyzer = StereoAnalyzer::new(RATE);
        analyzer.configure(RATE, fold.is_stereo());
        for (l, r) in l.chunks(FFT_SIZE).zip(r.chunks(FFT_SIZE)) {
            analyzer.push(l, r);
        }
        let frame = analyzer.frame();
        let loudest = frame.bands.iter().copied().fold(0.0f32, f32::max);
        assert!(loudest > 0.4, "bands {:?}", frame.bands);
        assert!(frame.level > 0.5, "level {}", frame.level);
        assert!(!analyzer.is_silent());
        let pcm = pcm_samples(&frame);
        assert_eq!(pcm.len(), FFT_SIZE, "Wavefield expects exactly 2048 f32 samples");
        assert!(pcm.iter().any(|s| s.abs() > 0.2));
        // Equal strength: the left window, as on macOS.
        assert_eq!(pcm, left[left.len() - FFT_SIZE..]);
        // Exactly what either channel shows on its own.
        let mut alone = Analyzer::new(RATE);
        alone.push(&left);
        assert_eq!(frame, alone.frame());
    }

    #[test]
    fn stereo_frames_merge_like_macos() {
        // Left: a loud bass line. Right: quiet treble.
        let left = sine(100.0, 0.5, RATE as usize, 0);
        let right = sine(8_000.0, 0.05, RATE as usize, 0);
        let mut stereo = StereoAnalyzer::new(RATE);
        stereo.configure(RATE, true);
        stereo.push(&left, &right);
        let (mut alone_left, mut alone_right) = (Analyzer::new(RATE), Analyzer::new(RATE));
        alone_left.push(&left);
        alone_right.push(&right);
        let (merged, l, r) = (stereo.frame(), alone_left.frame(), alone_right.frame());
        for (index, band) in merged.bands.iter().enumerate() {
            assert_eq!(*band, l.bands[index].max(r.bands[index]), "band {index}");
        }
        assert_eq!(merged.level, l.level.max(r.level));
        assert_eq!(
            (merged.bass, merged.mid, merged.treble),
            (l.bass.max(r.bass), l.mid.max(r.mid), l.treble.max(r.treble))
        );
        assert_eq!((merged.beat, merged.flux), (l.beat.max(r.beat), l.flux.max(r.flux)));
        assert!(r.treble > l.treble, "the quiet channel still contributes its treble");
        // The louder (left) channel's centroid and PCM window.
        assert!(l.level > r.level);
        assert_eq!(merged.centroid, l.centroid);
        assert_eq!(merged.pcm, l.pcm);

        // Swapped: the right channel is now the stronger one.
        let mut swapped = StereoAnalyzer::new(RATE);
        swapped.configure(RATE, true);
        swapped.push(&right, &left);
        let frame = swapped.frame();
        assert_eq!(frame.pcm, l.pcm);
        assert_eq!(frame.centroid, l.centroid);
    }

    #[test]
    fn mono_sources_and_idle_detection() {
        let tone = sine(440.0, 0.3, RATE as usize, 0);
        // A mono source: the left analyser alone, whatever `right` holds.
        let mut mono = StereoAnalyzer::new(RATE);
        mono.configure(RATE, false);
        mono.push(&tone, &[]);
        let mut alone = Analyzer::new(RATE);
        alone.push(&tone);
        assert_eq!(mono.frame(), alone.frame());

        // Stereo is idle only once both channels are silent.
        let mut stereo = StereoAnalyzer::new(RATE);
        stereo.configure(RATE, true);
        let silence = vec![0.0; RATE as usize];
        stereo.push(&silence, &tone);
        assert!(!stereo.is_silent(), "the right channel is still playing");
        stereo.push(&vec![0.0; 3 * RATE as usize], &vec![0.0; 3 * RATE as usize]);
        assert!(stereo.is_silent());

        // Losing the right channel clears it; a new rate clears everything.
        let mut source = StereoAnalyzer::new(RATE);
        source.configure(RATE, true);
        source.push(&silence, &tone);
        source.configure(RATE, false);
        source.configure(RATE, true);
        assert!(source.right.is_silent() && source.right.recent.iter().all(|s| *s == 0.0));
        source.push(&tone, &tone);
        source.configure(44_100, true);
        assert!(source.is_silent());
        assert_eq!(source.frame().sample_rate, 44_100);
    }

    #[test]
    fn features_use_the_macos_band_layout() {
        // A 200 Hz tone (a bass line, a low voice) is bass on macOS: its
        // 64-band ranges put 200 Hz in "bass" (FFT bins 2-11) and none of it
        // in "mid" (from bin 22, ~516 Hz). A 32-band layout cannot: there the
        // one-bin low bands push "mid" down to ~280 Hz.
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&sine(200.0, 0.5, RATE as usize * 2, 0));
        let frame = analyzer.frame();
        assert!(frame.mid < 0.02, "mid {}", frame.mid);
        assert!(frame.bass > 0.45, "bass {}", frame.bass);
        assert!((frame.centroid - 0.105).abs() < 0.02, "centroid {}", frame.centroid);
        // Same edges as AudioAnalyzer.swift at 48 kHz.
        assert_eq!(&analyzer.analysis_edges[..12], &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
        assert_eq!(analyzer.analysis_edges[BASS.end], 12);
        assert_eq!(analyzer.analysis_edges[MID.start], 22);
        assert_eq!(analyzer.analysis_edges[TREBLE.start], 117);
        assert_eq!(analyzer.analysis_edges[ANALYSIS_BANDS], 683);
    }

    #[test]
    fn silence_stays_silent() {
        let mut analyzer = Analyzer::new(RATE);
        assert!(analyzer.push(&vec![0.0; 48_000]));
        let frame = analyzer.frame();
        assert_eq!(frame.bands.len(), BAND_COUNT);
        assert!(frame.level == 0.0 && frame.beat == 0.0 && frame.flux == 0.0);
        assert!(frame.bands.iter().all(|band| *band == 0.0));
        assert!(analyzer.is_silent());
        assert_eq!(frame.sample_rate, RATE);
        let pcm = decode_base64(frame.pcm.as_deref().unwrap_or_default());
        assert_eq!(pcm.len(), FFT_SIZE * 4, "Wavefield expects exactly 2048 f32 samples");
        assert!(pcm.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn pcm_window_is_the_latest_2048_samples() {
        let mut analyzer = Analyzer::new(RATE);
        let ramp: Vec<f32> = (0..5_000).map(|i| i as f32 / 5_000.0).collect();
        analyzer.push(&ramp);
        let pcm = decode_base64(analyzer.frame().pcm.as_deref().unwrap_or_default());
        let samples: Vec<f32> = pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        assert_eq!(samples.len(), FFT_SIZE);
        assert_eq!(samples[FFT_SIZE - 1], ramp[4_999]);
        assert_eq!(samples[0], ramp[5_000 - FFT_SIZE]);
    }

    fn loudest_band(frame: &AudioFrame) -> usize {
        frame.bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(index, _)| index).unwrap_or(0)
    }

    fn band_of(analyzer: &Analyzer, frequency: f64) -> usize {
        let bin = (frequency * FFT_SIZE as f64 / f64::from(RATE)).round() as usize;
        (0..BAND_COUNT)
            .find(|&b| analyzer.edges[b] <= bin && bin < analyzer.edges[b + 1].max(analyzer.edges[b] + 1))
            .unwrap_or(0)
    }

    #[test]
    fn a_tone_lights_its_band() {
        for (frequency, region) in [(60.0, "bass"), (1_000.0, "mid"), (6_000.0, "treble")] {
            let mut analyzer = Analyzer::new(RATE);
            analyzer.push(&sine(frequency, 0.5, RATE as usize, 0));
            let frame = analyzer.frame();
            let expected = band_of(&analyzer, frequency);
            let loudest = loudest_band(&frame);
            assert!(loudest.abs_diff(expected) <= 1, "{frequency} Hz: loudest band {loudest}, expected {expected}");
            assert!(frame.level > 0.6, "{frequency} Hz level {}", frame.level);
            let (bass, mid, treble) = (frame.bass, frame.mid, frame.treble);
            match region {
                "bass" => assert!(bass > mid && bass > treble, "{bass} {mid} {treble}"),
                "mid" => assert!(mid > bass && mid > treble, "{bass} {mid} {treble}"),
                _ => assert!(treble > bass && treble > mid, "{bass} {mid} {treble}"),
            }
        }
    }

    #[test]
    fn centroid_follows_brightness() {
        let mut dark = Analyzer::new(RATE);
        let mut bright = Analyzer::new(RATE);
        dark.push(&sine(120.0, 0.5, RATE as usize * 2, 0));
        bright.push(&sine(8_000.0, 0.5, RATE as usize * 2, 0));
        assert!(dark.frame().centroid < 0.4 && bright.frame().centroid > 0.4);
        assert!(bright.frame().centroid > dark.frame().centroid + 0.2);
    }

    #[test]
    fn noise_is_broadband() {
        let mut analyzer = Analyzer::new(RATE);
        let mut seed = 42;
        analyzer.push(&noise(0.5, RATE as usize, &mut seed));
        let frame = analyzer.frame();
        assert!(frame.bands.iter().filter(|band| **band > 0.2).count() > BAND_COUNT / 2, "{:?}", frame.bands);
        assert!(frame.level > 0.5 && frame.treble > 0.2 && frame.bass > 0.1);
        assert!(frame.bands.iter().all(|band| (0.0..=1.0).contains(band)));
    }

    #[test]
    fn kicks_fire_the_beat_and_flux() {
        let mut analyzer = Analyzer::new(RATE);
        // Two seconds of 55 Hz kicks: 100 ms on, 400 ms off, read once per hop.
        let mut signal = Vec::new();
        for kick in 0..4 {
            signal.extend(sine(55.0, 0.8, 4_800, kick * 24_000));
            signal.extend_from_slice(&[0.0; 19_200]);
        }
        let mut fired = 0;
        let mut previous_beat = 0.0f32;
        let mut peak_flux = 0.0f32;
        for hop in signal.chunks(HOP) {
            analyzer.push(hop);
            let frame = analyzer.frame();
            if frame.beat > 0.99 && previous_beat < 0.99 {
                fired += 1;
            }
            previous_beat = frame.beat;
            peak_flux = peak_flux.max(frame.flux);
        }
        assert_eq!(fired, 4, "one beat per kick");
        assert!(peak_flux > 0.3, "flux {peak_flux}");
        // After a long silence everything decays and the window is empty.
        analyzer.push(&vec![0.0; RATE as usize * 3]);
        assert!(analyzer.is_silent());
    }

    #[test]
    fn auto_gain_lifts_quiet_sources() {
        let mut quiet = Analyzer::new(RATE);
        quiet.push(&sine(1_000.0, 0.02, RATE as usize * 4, 0));
        let mut loud = Analyzer::new(RATE);
        loud.push(&sine(1_000.0, 0.5, RATE as usize * 4, 0));
        // -34 dBFS vs -6 dBFS: the AGC closes most of the 28 dB gap.
        assert!(quiet.frame().level > 0.6, "quiet level {}", quiet.frame().level);
        assert!((loud.frame().level - quiet.frame().level).abs() < 0.25);
    }

    #[test]
    fn odd_sample_rates_keep_valid_bands() {
        for rate in [8_000, 22_050, 44_100, 96_000, 192_000, 0] {
            let analyzer = Analyzer::new(rate);
            // The spectrum (`frame.bands`) and the analysis layout.
            for (edges, count) in [(&analyzer.edges, BAND_COUNT), (&analyzer.analysis_edges, ANALYSIS_BANDS)] {
                assert_eq!(edges.len(), count + 1);
                assert!(edges.windows(2).all(|pair| pair[0] < pair[1]), "{rate}: {edges:?}");
                assert!(edges[count] < FFT_SIZE / 2);
            }
        }
        assert_eq!(Analyzer::new(0).sample_rate(), 48_000);
    }
}
