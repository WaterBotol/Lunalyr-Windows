//! Audio analysis for the visualisers: mono PCM in, [`AudioFrame`] out.
//!
//! A port of the macOS analyser (Sources/Visualizers/AudioAnalyzer.swift,
//! itself adapted from Synesthia by Jon Jaques, MIT licence): a 2048-sample
//! sliding window analysed every 1024 new samples — Hann window → FFT →
//! log-spaced bands → auto-gain dB mapping → attack/release smoothing →
//! bass/mid/treble, spectral flux, beat envelope and spectral centroid. The
//! renderer's frame has 32 bands (macOS: 64), so band ranges and the
//! flux/centroid scales are converted to keep the same frequencies and
//! sensitivity. The raw 2048-sample window is shipped as `pcm` for the
//! Wavefield engine (its FFT size).
//!
//! Also decodes the PCM formats WASAPI hands out and downmixes them to mono.
//! Platform independent and unit-tested on every host; only the Windows
//! audio capture uses it at run time.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::VecDeque;

use lunalyr_core::contract::AudioFrame;

/// FFT length and the PCM window length (~43 ms at 48 kHz).
pub const FFT_SIZE: usize = 2048;
/// New samples per analysis pass (50% overlap, ~47 passes/s at 48 kHz).
pub const HOP: usize = 1024;
/// Log-spaced bands in [`AudioFrame::bands`].
pub const BAND_COUNT: usize = 32;

const MIN_HZ: f64 = 30.0;
const MAX_HZ: f64 = 16_000.0;
// Same frequency spans as macOS's 64-band ranges 0..10, 20..42, 46..64:
// ~30-80 Hz, ~210 Hz-1.8 kHz, ~2.7-16 kHz.
const BASS: std::ops::Range<usize> = 0..5;
const MID: std::ops::Range<usize> = 10..21;
const TREBLE: std::ops::Range<usize> = 23..32;
/// The macOS constants are tuned for 64 bands.
const BAND_RATIO: f32 = 64.0 / BAND_COUNT as f32;

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

/// Decodes interleaved frames (`block_align` bytes each) and appends their
/// mono downmix to `out`; returns the number of frames decoded. Stereo is
/// (L+R)/2; more channels are summed and scaled by 1/sqrt(n), which keeps
/// the level of both stereo-in-5.1 and real surround content. Non-finite
/// samples count as silence; the result is clamped to -1..1.
pub fn downmix(data: &[u8], format: SampleFormat, channels: usize, block_align: usize, out: &mut Vec<f32>) -> usize {
    let width = format.bytes();
    if channels == 0 || block_align == 0 || block_align < channels * width {
        return 0;
    }
    let scale = match channels {
        1 => 1.0,
        2 => 0.5,
        n => 1.0 / (n as f32).sqrt(),
    };
    let frames = data.len() / block_align;
    out.reserve(frames);
    for frame in data.chunks_exact(block_align) {
        let mut sum = 0.0f32;
        for sample in frame[..channels * width].chunks_exact(width) {
            let value = format.decode(sample);
            if value.is_finite() {
                sum += value;
            }
        }
        out.push((sum * scale).clamp(-1.0, 1.0));
    }
    frames
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

/// Streaming analyser. Feed it mono samples with [`Analyzer::push`]; read
/// the newest result with [`Analyzer::frame`].
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
    /// `BAND_COUNT + 1` FFT-bin indices delimiting the bands.
    edges: Vec<usize>,
    raw: [f32; BAND_COUNT],
    previous_raw: [f32; BAND_COUNT],
    smoothed: [f32; BAND_COUNT],
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
        let mut analyzer = Analyzer {
            sample_rate: sanitize_rate(sample_rate),
            fft: Fft::new(FFT_SIZE),
            window,
            recent: VecDeque::from(vec![0.0; FFT_SIZE]),
            pending: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            power: vec![0.0; FFT_SIZE / 2],
            edges: Vec::new(),
            raw: [0.0; BAND_COUNT],
            previous_raw: [0.0; BAND_COUNT],
            smoothed: [0.0; BAND_COUNT],
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
        };
        analyzer.rebuild_edges();
        analyzer
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Clears all state for a new source (device or sample-rate change), so
    /// the old source's tail does not bleed into the new one.
    pub fn reset(&mut self, sample_rate: u32) {
        *self = Analyzer::new(sample_rate);
    }

    /// Appends mono samples (-1..1). Runs one analysis pass per `HOP` new
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
            && self.smoothed.iter().all(|band| *band < 0.01)
            && self.recent.iter().all(|sample| sample.abs() < 1e-6)
    }

    /// The newest analysis, with the PCM window for Wavefield.
    pub fn frame(&self) -> AudioFrame {
        let mut bytes = Vec::with_capacity(FFT_SIZE * 4);
        for sample in &self.recent {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        AudioFrame {
            level: self.level,
            bass: self.bass,
            mid: self.mid,
            treble: self.treble,
            beat: self.beat_envelope,
            flux: self.flux_envelope.min(1.0),
            centroid: self.centroid,
            bands: self.smoothed.to_vec(),
            pcm: Some(base64(&bytes)),
            sample_rate: self.sample_rate,
        }
    }

    /// FFT-bin edges of the log-spaced bands between 30 Hz and
    /// min(16 kHz, 0.45 × sample rate); each band owns at least one bin.
    fn rebuild_edges(&mut self) {
        let bins = FFT_SIZE / 2;
        let rate = f64::from(self.sample_rate);
        let hz = |bin: usize| bin as f64 * rate / FFT_SIZE as f64;
        let max_hz = MAX_HZ.min(rate * 0.45);
        let mut edges: Vec<usize> = Vec::with_capacity(BAND_COUNT + 1);
        for band in 0..=BAND_COUNT {
            let frequency = MIN_HZ * (max_hz / MIN_HZ).powf(band as f64 / BAND_COUNT as f64);
            let mut bin = 0;
            while bin < bins - 1 && hz(bin) < frequency {
                bin += 1;
            }
            if let Some(&last) = edges.last() {
                bin = bin.max(last + 1);
            }
            edges.push(bin.min(bins - 1));
        }
        self.edges = edges;
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

        // 3. Bands: RMS magnitude over each band's bins, mapped through dB
        //    (-72 dB → 0, -6 dB → 1, shifted by the auto-gain). The 2/N scale
        //    reproduces vDSP's real-FFT magnitudes.
        let norm = 2.0 / FFT_SIZE as f32;
        for band in 0..BAND_COUNT {
            let low = self.edges[band];
            let high = self.edges[band + 1].max(low + 1).min(self.power.len());
            let low = low.min(high.saturating_sub(1));
            let sum: f32 = self.power[low..high].iter().sum();
            let magnitude = (sum / (high - low) as f32).sqrt() * norm;
            let db = 20.0 * magnitude.max(1e-9).log10();
            self.raw[band] = ((db + self.agc_gain_db + 72.0) / 66.0).clamp(0.0, 1.0);
        }

        // 4. Attack/release smoothing: rise 65% per pass, fall 12%.
        for (smoothed, &raw) in self.smoothed.iter_mut().zip(&self.raw) {
            *smoothed = if raw > *smoothed { *smoothed * 0.35 + raw * 0.65 } else { *smoothed * 0.88 + raw * 0.12 };
        }
        self.bass = mean(&self.smoothed[BASS]);
        self.mid = mean(&self.smoothed[MID]);
        self.treble = mean(&self.smoothed[TREBLE]);

        // 5. Spectral flux: new energy since the last pass, as an envelope.
        let flux_now: f32 =
            self.raw.iter().zip(&self.previous_raw).map(|(now, before)| (now - before).max(0.0)).sum::<f32>()
                * BAND_RATIO;
        self.flux_envelope = (flux_now * 0.9).min(1.0).max(self.flux_envelope * 0.86);

        // 6. Spectral centroid (0 dark … 1 bright), only with enough energy.
        let total: f32 = self.raw.iter().sum();
        if total > 0.02 / BAND_RATIO {
            let weighted: f32 = self.raw.iter().enumerate().map(|(band, value)| value * band as f32).sum();
            let instant = weighted / (total * (BAND_COUNT - 1) as f32);
            self.centroid = self.centroid * 0.9 + instant * 0.1;
        }

        // 7. Beat: bass energy 30% over its running average fires the
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

    #[test]
    fn downmix_decodes_wasapi_formats() {
        let mut out = Vec::new();
        // Stereo float: (L + R) / 2.
        let stereo: Vec<u8> = [0.5f32, 0.25, -1.0, -1.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(downmix(&stereo, SampleFormat::F32, 2, 8, &mut out), 2);
        assert_eq!(out, vec![0.375, -1.0]);

        out.clear();
        let pcm16: Vec<u8> = [16_384i16, -32_768].iter().flat_map(|s| s.to_le_bytes()).collect();
        downmix(&pcm16, SampleFormat::I16, 1, 2, &mut out);
        assert_eq!(out, vec![0.5, -1.0]);

        out.clear();
        // Packed 24-bit: 0x400000 = +0.5, 0xC00000 = -0.5.
        downmix(&[0x00, 0x00, 0x40, 0x00, 0x00, 0xC0], SampleFormat::I24, 1, 3, &mut out);
        assert_eq!(out, vec![0.5, -0.5]);

        out.clear();
        // 24 valid bits in a 32-bit container are MSB-aligned.
        let pcm32: Vec<u8> = (0x4000_0000i32).to_le_bytes().to_vec();
        downmix(&pcm32, SampleFormat::I32, 1, 4, &mut out);
        assert_eq!(out, vec![0.5]);

        out.clear();
        // 5.1 with only front left/right playing: (L + R) / sqrt(6).
        let surround: Vec<u8> = [0.6f32, 0.6, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        downmix(&surround, SampleFormat::F32, 6, 24, &mut out);
        assert!((out[0] - 1.2 / 6f32.sqrt()).abs() < 1e-6);

        out.clear();
        // NaN counts as silence; block_align too small is rejected.
        let bad: Vec<u8> = [f32::NAN, 0.5].iter().flat_map(|s| s.to_le_bytes()).collect();
        downmix(&bad, SampleFormat::F32, 2, 8, &mut out);
        assert_eq!(out, vec![0.25]);
        assert_eq!(downmix(&bad, SampleFormat::F32, 2, 4, &mut out), 0);
        assert_eq!(SampleFormat::from_wave(true, 32), Some(SampleFormat::F32));
        assert_eq!(SampleFormat::from_wave(false, 8), None);
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
            assert_eq!(analyzer.edges.len(), BAND_COUNT + 1);
            assert!(analyzer.edges.windows(2).all(|pair| pair[0] < pair[1]), "{rate}: {:?}", analyzer.edges);
            assert!(analyzer.edges[BAND_COUNT] < FFT_SIZE / 2);
        }
        assert_eq!(Analyzer::new(0).sample_rate(), 48_000);
    }
}
