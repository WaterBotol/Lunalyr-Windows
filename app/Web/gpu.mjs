import { clamp, rgb } from './score.mjs';

export const effectNames = {
  plasma: 'Plasma', kaleido: 'Kaleidoscope', aurora: 'Aurora', fractal: 'Fractal', liquid: 'Liquid',
  warp: 'Marble', nebula: 'Nebula', voronoi: 'Voronoi', orbs: 'Orbs', silk: 'Silk', waves: 'Waves',
  helix: 'Helix', ripple: 'Ripple', mirror: 'Mirror', wormhole: 'Wormhole', rings: 'Rings',
  vortex: 'Vortex', orbit: 'Orbit', bubbles: 'Bubbles', galaxy: 'Galaxy', strata: 'Strata',
  clouds: 'Clouds', curtains: 'Curtains', ribbons: 'Ribbons', prism: 'Prism', tidal: 'Tidal',
  lightTrails: 'Light Trails'
};
const vertex = `#version 300 es
void main() {
    vec2 point = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(point * 2.0 - 1.0, 0.0, 1.0);
}`;
const musicEffects = {
  silk: 'auroraFragment', aurora: 'auroraFragment', tunnel: 'tunnelFragment',
  curtainAurora: 'curtainAuroraFragment', fosforaAurora: 'fosforaAuroraFragment',
  colourStrata: 'colourStrataFragment', overprint: 'overprintFragment'
};
let source;
let musicSource;
if (typeof document !== 'undefined') {
  fetch('ambient.frag').then(response => response.ok ? response.text() : null)
    .then(text => { source = text; }).catch(() => {});
  fetch('music.frag').then(response => response.ok ? response.text() : null)
    .then(text => { musicSource = text; }).catch(() => {});
}
function oklab(values) {
  const [r, g, b] = values.map(value => {
    const c = value / 255;
    return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4;
  });
  const l = Math.cbrt(.4122214708 * r + .5363325363 * g + .0514459929 * b);
  const m = Math.cbrt(.2119034982 * r + .6806995451 * g + .1073969566 * b);
  const s = Math.cbrt(.0883024619 * r + .2817188376 * g + .6299787005 * b);
  return [.2104542553 * l + .7936177850 * m - .0040720468 * s,
    1.9779984951 * l - 2.4285922050 * m + .4505937099 * s,
    .0259040371 * l + .7827717662 * m - .8086757660 * s];
}

export class GpuScene {
  constructor(element, above, preview) {
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'scene';
    this.canvas.hidden = true;
    this.preview = preview;
    this.gl = this.canvas.getContext('webgl2', { alpha: false, antialias: false, depth: false });
    this.programs = new Map();
    element.insertBefore(this.canvas, above);
    this.canvas.addEventListener('webglcontextlost', event => { event.preventDefault(); this.hide(); });
    this.canvas.addEventListener('webglcontextrestored', () => this.programs.clear());
  }
  program(effect, music) {
    const key = (music ? 'music:' : 'ambient:') + effect;
    if (this.programs.has(key)) return this.programs.get(key);
    const gl = this.gl;
    const name = 'wl' + effect[0].toUpperCase() + effect.slice(1);
    const fragment = music ? musicSource + `
void main() { outColor = ${musicEffects[effect]}(gl_FragCoord.xy / u.resolution); }
` : source + `
void main() {
    vec2 position = vec2(gl_FragCoord.x, uSize.y - gl_FragCoord.y);
    vec4 colour = ${name}(position, vec4(0.0, 0.0, 0.0, 1.0), uSize, uTime, uFlow, uTintA, uTintB${effect === 'aurora' ? ', vec3(0.0)' : ''});
    outColor = wlGrade(colour, position, uSize, uVividness, uStops);
}`;
    const shaders = [[gl.VERTEX_SHADER, vertex], [gl.FRAGMENT_SHADER, fragment]].map(([type, code]) => {
      const shader = gl.createShader(type);
      gl.shaderSource(shader, code);
      gl.compileShader(shader);
      return shader;
    });
    const program = gl.createProgram();
    shaders.forEach(shader => gl.attachShader(program, shader));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      console.warn('Lunalyr GPU effect unavailable:', gl.getShaderInfoLog(shaders[1]));
      shaders.forEach(shader => gl.deleteShader(shader));
      gl.deleteProgram(program);
      this.programs.set(key, null);
      return null;
    }
    shaders.forEach(shader => gl.deleteShader(shader));
    const keys = music
      ? ['u.resolution', 'u.time', 'u.bass', 'u.mid', 'u.treble', 'u.level', 'u.beat', 'u.sensitivity', 'u.speed',
        'u.palette', 'u.p[0]', 'u.subBass', 'u.lowMid', 'u.highMid', 'u.presence', 'u.air', 'u.trebleBeat',
        'u.flux', 'u.spectralCentre', 'u.reduceMotion', 'album[0]', 'bands[0]', 'wave[0]', 'exposure']
      : ['uSize', 'uTime', 'uFlow', 'uVividness', 'uTintA', 'uTintB', 'uStops[0]'];
    const uniforms = Object.fromEntries(keys.map(name => [name, gl.getUniformLocation(program, name)]));
    const result = { program, uniforms };
    this.programs.set(key, result);
    return result;
  }
  draw(effect, width, height, elapsed, p, palette, signal = null) {
    const music = signal !== null;
    const names = music ? musicEffects : effectNames;
    if (!(music ? musicSource : source) || !this.gl || this.gl.isContextLost() || !Object.hasOwn(names, effect)) return this.hide();
    const result = this.program(effect, music);
    if (!result) return this.hide();
    const { gl, canvas } = this;
    const budget = effect === 'tunnel' && music ? (this.preview ? 85_000 : 240_000) : (this.preview ? 220_000 : 650_000);
    const scale = Math.min(devicePixelRatio, Math.sqrt(budget / (width * height)));
    const w = Math.max(1, Math.round(width * scale)), h = Math.max(1, Math.round(height * scale));
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    const { program, uniforms: u } = result;
    gl.viewport(0, 0, w, h);
    gl.useProgram(program);
    if (music) this.audioUniforms(u, w, h, elapsed, p, palette, effect, signal);
    else {
      gl.uniform2f(u.uSize, w, h);
      gl.uniform1f(u.uTime, elapsed);
      gl.uniform1f(u.uFlow, elapsed * (p.speed ?? 1));
      gl.uniform1f(u.uVividness, p.richness ?? 1);
      gl.uniform3fv(u.uTintA, palette[0].map(n => n / 255));
      gl.uniform3fv(u.uTintB, palette[8].map(n => n / 255));
      gl.uniform3fv(u['uStops[0]'], [0, 2, 4, 6, 8].flatMap(i => oklab(palette[i])));
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    canvas.style.backgroundColor = rgb(palette[4]);
    canvas.hidden = false;
    canvas.dataset.effect = effect;
    return true;
  }
  audioUniforms(u, w, h, time, p, palette, effect, { audio = {}, calm, artwork }) {
    const { gl } = this;
    const bins = audio.spectrum ?? [];
    const readBand = index => bins[Math.min(bins.length - 1, Math.floor(index * bins.length))] ?? 0;
    const transient = calm ? 0 : (p.punch ?? 1);
    const values = {
      time, bass: audio.bass, mid: audio.mid, treble: audio.treble, level: audio.level,
      sensitivity: p.sensitivity, speed: p.speed * (effect === 'tunnel' ? .22 : .34),
      palette: effect === 'silk' ? 3 : 0, beat: (audio.beat ?? 0) * .20 * transient,
      trebleBeat: (audio.treble ?? 0) * .12 * transient, flux: (audio.flux ?? 0) * .14 * transient,
      subBass: readBand(.10), lowMid: readBand(.38), highMid: readBand(.64),
      presence: readBand(.75), air: readBand(.96), reduceMotion: calm ? 1 : 0,
      spectralCentre: bins.length ? bins.reduce((sum, value, i) => sum + value * i / bins.length, 0) / Math.max(.01, bins.reduce((a, b) => a + b, 0)) : 0
    };
    for (const [name, value] of Object.entries(values)) gl.uniform1f(u['u.' + name], value ?? 0);
    gl.uniform2f(u['u.resolution'], w, h);
    const params = new Float32Array(16);
    if (effect === 'tunnel') params.set([.38, .78, .50, 0, .42, .72]);
    else if (effect === 'fosforaAurora') params.set([readBand(.27), readBand(.51)], 3);
    else params.set(effect === 'silk' ? [4, .16, 0] : [6, .20, 0]);
    params[15] = clamp(p.richness, .4, 1.8);
    gl.uniform1fv(u['u.p[0]'], params);
    gl.uniform4fv(u['album[0]'], [0, 2, 4, 6, 8].flatMap(i => [...palette[i].map(n => n / 255), artwork ? 1 : 0]));
    gl.uniform1fv(u['bands[0]'], Array.from({ length: 64 }, (_, i) => readBand(i / 64)));
    gl.uniform1fv(u['wave[0]'], audio.wave ?? new Float32Array(256));
    gl.uniform1f(u.exposure, 1);
  }
  hide() { this.canvas.hidden = true; return false; }
  dispose() {
    for (const result of this.programs.values()) if (result) this.gl.deleteProgram(result.program);
    this.canvas.remove();
  }
}
