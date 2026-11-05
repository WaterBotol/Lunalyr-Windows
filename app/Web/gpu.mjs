import { rgb } from './score.mjs';

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
let source;
if (typeof document !== 'undefined') {
  fetch('ambient.frag').then(response => response.ok ? response.text() : null)
    .then(text => { source = text; }).catch(() => {});
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

export class Ambient {
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
  program(effect) {
    if (this.programs.has(effect)) return this.programs.get(effect);
    const gl = this.gl;
    const name = 'wl' + effect[0].toUpperCase() + effect.slice(1);
    const fragment = source + `
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
      console.warn('Lunalyr ambient effect unavailable:', gl.getShaderInfoLog(shaders[1]));
      shaders.forEach(shader => gl.deleteShader(shader));
      gl.deleteProgram(program);
      this.programs.set(effect, null);
      return null;
    }
    shaders.forEach(shader => gl.deleteShader(shader));
    const uniforms = Object.fromEntries(['uSize', 'uTime', 'uFlow', 'uVividness', 'uTintA', 'uTintB', 'uStops[0]']
      .map(key => [key, gl.getUniformLocation(program, key)]));
    const result = { program, uniforms };
    this.programs.set(effect, result);
    return result;
  }
  draw(effect, width, height, elapsed, p, palette) {
    if (!source || !this.gl || this.gl.isContextLost() || !Object.hasOwn(effectNames, effect)) return this.hide();
    const result = this.program(effect);
    if (!result) return this.hide();
    const { gl, canvas } = this;
    const budget = this.preview ? 220_000 : 650_000;
    const scale = Math.min(devicePixelRatio, Math.sqrt(budget / (width * height)));
    const w = Math.max(1, Math.round(width * scale)), h = Math.max(1, Math.round(height * scale));
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    const { program, uniforms: u } = result;
    gl.viewport(0, 0, w, h);
    gl.useProgram(program);
    gl.uniform2f(u.uSize, w, h);
    gl.uniform1f(u.uTime, elapsed);
    gl.uniform1f(u.uFlow, elapsed * (p.speed ?? 1));
    gl.uniform1f(u.uVividness, p.richness ?? 1);
    gl.uniform3fv(u.uTintA, palette[0].map(n => n / 255));
    gl.uniform3fv(u.uTintB, palette[8].map(n => n / 255));
    gl.uniform3fv(u['uStops[0]'], [0, 2, 4, 6, 8].flatMap(i => oklab(palette[i])));
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    canvas.style.backgroundColor = rgb(palette[4]);
    canvas.hidden = false;
    canvas.dataset.effect = effect;
    return true;
  }
  hide() { this.canvas.hidden = true; return false; }
  dispose() {
    for (const result of this.programs.values()) if (result) this.gl.deleteProgram(result.program);
    this.canvas.remove();
  }
}
