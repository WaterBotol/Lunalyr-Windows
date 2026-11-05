import { BIN_COUNT, FFT_SIZE, reactive } from "@/lib/audio/reactive";

const HOP = FFT_SIZE / 2;
// One window plus up to seven hops: the host sends every hop since the last frame.
const MAX_SAMPLES = FFT_SIZE * 8;
// The demo's AnalyserNode smoothed by 0.55 on every 60 fps animation frame.
// The same response in seconds keeps Wavecore's look at 15, 30 or 60 fps.
const SMOOTHING_SECONDS = 1 / (60 * Math.log(1 / 0.55));
const bytes = new Uint8Array(BIN_COUNT);
const real = new Float64Array(FFT_SIZE);
const imaginary = new Float64Array(FFT_SIZE);
const magnitudes = new Float64Array(BIN_COUNT);
const smoothed = new Float64Array(BIN_COUNT);

// The Blackman window, magnitude smoothing and byte range mirror Wavecore's
// AnalyserNode. Captured PCM is already here, so no playback graph is needed.
const blackman = Float64Array.from({ length: FFT_SIZE }, (_, i) => {
  const phase = 2 * Math.PI * i / FFT_SIZE;
  return 0.42 - 0.5 * Math.cos(phase) + 0.08 * Math.cos(2 * phase);
});
const bitReversed = Uint16Array.from({ length: FFT_SIZE }, (_, i) => {
  let reversed = 0;
  for (let bit = 1; bit < FFT_SIZE; bit *= 2) {
    reversed = reversed * 2 + (i & 1);
    i >>>= 1;
  }
  return reversed;
});
const cosines = Float64Array.from({ length: BIN_COUNT }, (_, i) => Math.cos(-2 * Math.PI * i / FFT_SIZE));
const sines = Float64Array.from({ length: BIN_COUNT }, (_, i) => Math.sin(-2 * Math.PI * i / FFT_SIZE));

export const engine = {
  analyser: { getByteFrequencyData: (target: Uint8Array) => target.set(bytes) },
  scratch: new Uint8Array(BIN_COUNT),
};

export function updateAudio(encoded: string | undefined, rate: number | undefined, sensitivity: number, dt = 1 / 60) {
  if (!encoded || !rate || !Number.isFinite(rate) || encoded.length > 4 * Math.ceil(MAX_SAMPLES * 4 / 3)) {
    reactive.mode = "idle";
    smoothed.fill(0);
    bytes.fill(0);
    return;
  }

  const decoded = atob(encoded);
  const samples = new Float32Array(Uint8Array.from(decoded, c => c.charCodeAt(0)).buffer);
  let peak = 0;
  for (let i = 0; i < samples.length; i++) peak = Math.max(peak, Math.abs(samples[i]));
  if (samples.length < FFT_SIZE || peak < 0.001) {
    reactive.mode = "idle";
    smoothed.fill(0);
    bytes.fill(0);
    return;
  }
  reactive.mode = "playing";

  // One spectrum per hop the host sent, oldest first, each smoothed for its
  // share of the frame.
  const hops = Math.floor((samples.length - FFT_SIZE) / HOP) + 1;
  const keep = Math.exp(-Math.max(0, dt) / hops / SMOOTHING_SECONDS);
  for (let hop = hops - 1; hop >= 0; hop--) {
    analyseWindow(samples, samples.length - FFT_SIZE - hop * HOP, rate, keep);
  }
  for (let bin = 0; bin < BIN_COUNT; bin++) {
    const db = 20 * Math.log10(smoothed[bin]);
    // Apply user gain after the analyser's logarithmic compression so the
    // bottom of the slider can visibly calm even loud, sustained music.
    const normalized = Math.max(0, Math.min(1, (db + 88) / 74));
    bytes[bin] = 255 * Math.min(1, normalized * Math.max(0.3, Math.min(3, sensitivity)));
  }
}

function analyseWindow(samples: Float32Array, offset: number, rate: number, keep: number) {
  for (let i = 0; i < FFT_SIZE; i++) real[bitReversed[i]] = (samples[i + offset] || 0) * blackman[i];
  imaginary.fill(0);

  for (let span = 2; span <= FFT_SIZE; span *= 2) {
    const half = span / 2;
    const stride = FFT_SIZE / span;
    for (let start = 0; start < FFT_SIZE; start += span) {
      for (let j = 0; j < half; j++) {
        const even = start + j;
        const odd = even + half;
        const twiddle = j * stride;
        const r = cosines[twiddle] * real[odd] - sines[twiddle] * imaginary[odd];
        const im = sines[twiddle] * real[odd] + cosines[twiddle] * imaginary[odd];
        real[odd] = real[even] - r;
        imaginary[odd] = imaginary[even] - im;
        real[even] += r;
        imaginary[even] += im;
      }
    }
  }
  for (let bin = 0; bin < BIN_COUNT; bin++) {
    magnitudes[bin] = Math.hypot(real[bin], imaginary[bin]) / FFT_SIZE;
  }
  for (let bin = 0; bin < BIN_COUNT; bin++) {
    // Wavecore's original bin map assumes about 22 Hz per bin.
    const index = bin * 44_100 / rate;
    const lower = Math.floor(index);
    const first = magnitudes[lower] ?? 0;
    const second = magnitudes[lower + 1] ?? 0;
    const magnitude = first + (second - first) * (index - lower);
    smoothed[bin] = smoothed[bin] * keep + magnitude * (1 - keep);
  }
}
