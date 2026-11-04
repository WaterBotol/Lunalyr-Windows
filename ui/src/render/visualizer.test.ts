import { describe, expect, it } from "vitest";
import { syntheticAudio } from "../demo/audio";
import { smoothBands, WAVEFIELD_PCM_BASE64_LENGTH, wavefieldFrame } from "./visualizer";

const settings = { audioSensitivity: 1.4, effectSpeed: 0.8, audioReactive: true };

describe("visualiser", () => {
  it("builds the Wavefield frame with the macOS host's keys", () => {
    const audio = syntheticAudio(3);
    expect(audio.pcm?.length).toBe(WAVEFIELD_PCM_BASE64_LENGTH);
    const frame = wavefieldFrame({ clock: 12.5, width: 1920.4, height: 1080, settings, reduced: false, albumGradient: "linear-gradient(red, blue)", audio });
    expect(frame).toMatchObject({
      clock: 12.5, width: 1920, height: 1080, preset: "default", projection: "preset",
      sensitivity: 1.4, motion: 0.8, reduced: false, paused: false, preview: false,
      albumGradient: "linear-gradient(red, blue)", sampleRate: 48000,
    });
    expect(frame.pcm).toBe(audio.pcm);
  });

  it("drops PCM the engine would reject, audio while not reactive, and the gradient without artwork", () => {
    const audio = { ...syntheticAudio(1), pcm: "AAAA" };
    const frame = wavefieldFrame({ clock: 0, width: 800, height: 600, settings, reduced: true, albumGradient: null, audio });
    expect(frame.pcm).toBeUndefined();
    expect(frame.sampleRate).toBeUndefined();
    expect("albumGradient" in frame).toBe(false);
    const quiet = wavefieldFrame({ clock: 0, width: 800, height: 600, settings: { ...settings, audioReactive: false }, reduced: false, albumGradient: null, audio: syntheticAudio(1) });
    expect(quiet.pcm).toBeUndefined();
  });

  it("bands rise quickly and fall slowly, frame-rate independent", () => {
    const up = smoothBands(new Float32Array([0]), [1], 1 / 30);
    const down = smoothBands(new Float32Array([1]), [0], 1 / 30);
    expect(up[0]).toBeGreaterThan(1 - down[0]);
    const fine = new Float32Array([0]);
    for (let i = 0; i < 4; i++) smoothBands(fine, [1], 1 / 120);
    const coarse = smoothBands(new Float32Array([0]), [1], 1 / 30);
    expect(fine[0]).toBeCloseTo(coarse[0], 6);
  });

  it("synthetic demo audio has the contract shape", () => {
    const frame = syntheticAudio(0.25);
    expect(frame.bands.length).toBe(32);
    expect(frame.bands.every((b) => b >= 0 && b <= 1)).toBe(true);
    const bytes = Uint8Array.from(atob(frame.pcm!), (c) => c.charCodeAt(0));
    expect(bytes.length).toBe(2048 * 4);
  });
});
