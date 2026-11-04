// The album colour field (AnimatedGradientView.swift): five very soft colour
// masses drifting on long Lissajous paths over a darkened base, saturated by
// `vividness`, under the readability scrim.
//
// One fragment shader evaluates the five blobs analytically on a half-
// resolution canvas (the field has no detail to lose) with a 1/255 dither
// against banding, so a 4K wallpaper costs a single cheap quad per frame and
// no CPU pixel work. Without WebGL it falls back to a static CSS gradient.

import { el } from "./dom";
import { averageStop, cssGradient, type RGB } from "./palette";

const PERIODS_X = [37, 43, 53, 61, 71];
const PERIODS_Y = [47, 59, 41, 67, 79];
const PERIODS_R = [23, 29, 31, 19, 26];
const ORBITS_X = [0.26, 0.3, 0.2, 0.32, 0.16];
const ORBITS_Y = [0.22, 0.16, 0.28, 0.2, 0.3];
const RADII = [0.72, 0.62, 0.8, 0.58, 0.68];
const TURN = Math.PI * 2;

export interface Blob {
  x: number;
  y: number;
  radius: number;
  /** Blur sigma relative to the radius. */
  softness: number;
}

/** Blob centres and radii in CSS pixels at colour-field time `time`, after the 1.2x overscan. */
export function blobLayout(time: number, width: number, height: number, out: Blob[] = []): Blob[] {
  const shortest = Math.min(width, height);
  const cx = width / 2;
  const cy = height / 2;
  for (let slot = 0; slot < 5; slot++) {
    const phase = slot * 1.27;
    const angleX = (time / PERIODS_X[slot]) * TURN + phase;
    const angleY = (time / PERIODS_Y[slot]) * TURN + phase * 1.3;
    const x = cx + Math.cos(angleX) * ORBITS_X[slot] * width;
    const y = cy + Math.sin(angleY) * ORBITS_Y[slot] * height;
    const breathe = 1 + 0.05 * Math.sin((time / PERIODS_R[slot]) * TURN + phase);
    const radius = shortest * RADII[slot] * breathe;
    // Blurred by 7.5% of the shortest side, then scaled 1.2x about the centre.
    const blob = out[slot] ?? (out[slot] = { x: 0, y: 0, radius: 0, softness: 0 });
    blob.x = cx + 1.2 * (x - cx);
    blob.y = cy + 1.2 * (y - cy);
    blob.radius = 1.2 * radius;
    blob.softness = (shortest * 0.075) / radius;
  }
  return out;
}

const VERTEX = `
attribute vec2 aPosition;
varying vec2 vUv;
void main() {
  vUv = aPosition * 0.5 + 0.5;
  gl_Position = vec4(aPosition, 0.0, 1.0);
}`;

const FRAGMENT = `
precision highp float;
varying vec2 vUv;
uniform vec2 uSize;
uniform vec3 uBase;
uniform vec3 uColor[5];
uniform vec4 uBlob[5];
uniform float uSaturation;

float smax(float a, float b, float k) {
  float d = a - b;
  return 0.5 * (a + b + sqrt(d * d + k * k));
}

// RadialGradient stops 0.85 @ 0, 0.42 @ 0.42, 0 @ 1, with the kinks rounded
// the way the Gaussian blur rounds them.
float profile(float d, float soft) {
  float r = sqrt(d * d + 0.5 * soft * soft);
  float inner = 0.85 - (0.43 / 0.42) * r;
  float outer = 0.42 * (1.0 - r) / 0.58;
  return clamp(smax(smax(inner, outer, 0.3 * soft), 0.0, 1.4 * soft), 0.0, 1.0);
}

void main() {
  vec2 p = vec2(vUv.x, 1.0 - vUv.y) * uSize;
  vec3 c = uBase;
  for (int i = 0; i < 5; i++) {
    vec4 b = uBlob[i];
    float a = profile(length(p - b.xy) / b.z, b.w);
    c = mix(c, uColor[i], a);
  }
  float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
  c = clamp(vec3(l) + (c - vec3(l)) * uSaturation, 0.0, 1.0);
  float n = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
  gl_FragColor = vec4(c + (n - 0.5) / 255.0, 1.0);
}`;

interface Program {
  gl: WebGLRenderingContext;
  program: WebGLProgram;
  size: WebGLUniformLocation | null;
  base: WebGLUniformLocation | null;
  colors: WebGLUniformLocation | null;
  blobs: WebGLUniformLocation | null;
  saturation: WebGLUniformLocation | null;
}

export class GradientBackground {
  readonly root: HTMLDivElement;
  private readonly canvas: HTMLCanvasElement;
  private program: Program | null = null;
  private readonly blobs: Blob[] = [];
  private readonly colorData = new Float32Array(15);
  private readonly blobData = new Float32Array(20);
  private lost = false;
  private fallbackKey = "";
  private width = 0;
  private height = 0;
  /** Canvas pixels per CSS pixel. The field is smooth, so half resolution is plenty. */
  static readonly renderScale = 0.5;

  constructor(parent: HTMLElement) {
    this.root = el("div", "ll-bg", parent);
    this.canvas = el("canvas", "ll-bg-canvas", this.root);
    this.canvas.addEventListener("webglcontextlost", (event) => {
      event.preventDefault();
      this.lost = true;
      this.program = null;
    });
    this.canvas.addEventListener("webglcontextrestored", () => {
      this.lost = false;
      this.program = this.createProgram();
    });
    this.program = this.createProgram();
    if (!this.program) this.canvas.style.display = "none";
  }

  private createProgram(): Program | null {
    const gl = this.canvas.getContext("webgl", { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: true, powerPreference: "low-power" });
    if (!gl) return null;
    const compile = (type: number, source: string) => {
      const shader = gl.createShader(type);
      if (!shader) return null;
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        console.warn("Gradient shader:", gl.getShaderInfoLog(shader));
        return null;
      }
      return shader;
    };
    const vs = compile(gl.VERTEX_SHADER, VERTEX);
    const fs = compile(gl.FRAGMENT_SHADER, FRAGMENT);
    const program = gl.createProgram();
    if (!vs || !fs || !program) return null;
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      console.warn("Gradient program:", gl.getProgramInfoLog(program));
      return null;
    }
    gl.useProgram(program);
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    // One oversized triangle covers the viewport.
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const position = gl.getAttribLocation(program, "aPosition");
    gl.enableVertexAttribArray(position);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    return {
      gl,
      program,
      size: gl.getUniformLocation(program, "uSize"),
      base: gl.getUniformLocation(program, "uBase"),
      colors: gl.getUniformLocation(program, "uColor"),
      blobs: gl.getUniformLocation(program, "uBlob"),
      saturation: gl.getUniformLocation(program, "uSaturation"),
    };
  }

  resize(width: number, height: number): void {
    this.width = width;
    this.height = height;
    const w = Math.max(1, Math.round(width * GradientBackground.renderScale));
    const h = Math.max(1, Math.round(height * GradientBackground.renderScale));
    if (this.canvas.width !== w) this.canvas.width = w;
    if (this.canvas.height !== h) this.canvas.height = h;
  }

  get available(): boolean {
    return this.program !== null && !this.lost;
  }

  /** `time` is the colour-field phase (VisualClock flow × 0.34). */
  render(time: number, stops: readonly RGB[], vividness: number): void {
    const p = this.program;
    if (!p || this.lost) {
      this.renderFallback(stops);
      return;
    }
    const gl = p.gl;
    blobLayout(time, this.width, this.height, this.blobs);
    const base = averageStop(stops);
    for (let i = 0; i < 5; i++) {
      const color = stops[i] ?? base;
      this.colorData[i * 3] = color[0];
      this.colorData[i * 3 + 1] = color[1];
      this.colorData[i * 3 + 2] = color[2];
      const b = this.blobs[i];
      this.blobData[i * 4] = b.x;
      this.blobData[i * 4 + 1] = b.y;
      this.blobData[i * 4 + 2] = b.radius;
      this.blobData[i * 4 + 3] = b.softness;
    }
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.uniform2f(p.size, this.width, this.height);
    gl.uniform3f(p.base, base[0] * 0.45, base[1] * 0.45, base[2] * 0.45);
    gl.uniform3fv(p.colors, this.colorData);
    gl.uniform4fv(p.blobs, this.blobData);
    gl.uniform1f(p.saturation, vividness);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  private renderFallback(stops: readonly RGB[]): void {
    const key = JSON.stringify(stops);
    if (key === this.fallbackKey) return;
    this.fallbackKey = key;
    this.root.style.background = cssGradient(stops);
  }

  setVisible(visible: boolean): void {
    this.root.style.display = visible ? "" : "none";
  }
}

/** The readability scrim: darker at the top and bottom (NowPlayingView.scrim). */
export function scrimGradient(strength: number): string {
  const a = (v: number) => Math.round(Math.min(1, Math.max(0, v * strength)) * 1000) / 1000;
  return `linear-gradient(to bottom, rgba(0,0,0,${a(0.34)}), rgba(0,0,0,${a(0.12)}), rgba(0,0,0,${a(0.5)}))`;
}
