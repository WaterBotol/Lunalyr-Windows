// Synthetic `lunalyr://audio` frames for the browser demo: a 120 BPM kick,
// a slow chord change and soft hats, as log-spaced bands plus a 2048-sample
// f32 PCM window (the shape the Rust side sends from WASAPI loopback).

import type { AudioFrame } from "../contract";

const BANDS = 32;
const WINDOW = 2048;
const CHORDS = [[220, 277.18, 329.63], [196, 246.94, 293.66], [174.61, 220, 261.63], [196, 246.94, 329.63]];

function kickEnvelope(t: number): number {
  const phase = ((t * 2) % 1 + 1) % 1;
  return Math.exp(-phase * 9);
}

function hatEnvelope(t: number): number {
  const phase = ((t * 4) % 1 + 1) % 1;
  return Math.exp(-phase * 28);
}

function noise(i: number): number {
  const x = Math.sin(i * 12.9898) * 43758.5453;
  return (x - Math.floor(x)) * 2 - 1;
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary);
}

/** One frame of synthetic audio at time `t` (seconds). */
export function syntheticAudio(t: number, sampleRate = 48000): AudioFrame {
  const chord = CHORDS[Math.floor(t / 2) % CHORDS.length];
  const samples = new Float32Array(WINDOW);
  const start = Math.floor(t * sampleRate) - WINDOW;
  for (let i = 0; i < WINDOW; i++) {
    const n = start + i;
    const time = n / sampleRate;
    let v = 0.55 * kickEnvelope(time) * Math.sin(2 * Math.PI * 55 * time);
    for (const f of chord) v += 0.07 * Math.sin(2 * Math.PI * f * time);
    v += 0.05 * hatEnvelope(time) * noise(n);
    samples[i] = v;
  }
  const kick = kickEnvelope(t);
  const hat = hatEnvelope(t);
  const bands: number[] = [];
  for (let b = 0; b < BANDS; b++) {
    const freq = 30 * Math.pow(16000 / 30, b / (BANDS - 1));
    const octave = (f: number) => Math.log2(freq / f);
    let v = 0.85 * kick * Math.exp(-(octave(60) ** 2) * 1.5);
    for (const f of chord) v += 0.32 * Math.exp(-(octave(f) ** 2) * 6);
    v += 0.4 * hat * Math.exp(-(octave(8000) ** 2) * 0.8);
    v += 0.04 * (1 + Math.sin(t * 1.3 + b));
    bands.push(Math.min(1, v));
  }
  const bass = bands.slice(0, 8).reduce((a, x) => a + x, 0) / 8;
  const mid = bands.slice(8, 20).reduce((a, x) => a + x, 0) / 12;
  const treble = bands.slice(20).reduce((a, x) => a + x, 0) / (BANDS - 20);
  return {
    level: Math.min(1, 0.3 + 0.5 * kick + 0.2 * mid),
    bass,
    mid,
    treble,
    beat: kick > 0.8 ? 1 : 0,
    flux: kick * 0.6 + hat * 0.3,
    centroid: 0.3 + 0.3 * treble,
    bands,
    pcm: base64(new Uint8Array(samples.buffer)),
    sampleRate,
  };
}
