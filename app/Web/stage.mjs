import { albumGrid, clamp, cueAt, fallback, hash, mix, noise, rgb, smooth, typography, wordState, arrivalFor } from './score.mjs';
import { field, lines, ribbons, starTrails, strata, tunnel } from './effects.mjs';
import { GpuScene, effectNames } from './gpu.mjs';

export const demo = {
  track: { title: 'Golden Hour Static', artist: 'Lunar Tides', key: 'fixture', duration: 16 },
  lyrics: { lines: [
    { text: 'Every word, in time', start: 0, end: 8, words: [
      { text: 'Every', start: 0, end: 2 }, { text: 'word,', start: 2, end: 4 },
      { text: 'in', start: 4, end: 5 }, { text: 'time', start: 5, end: 8 }
    ] },
    { text: 'with the music', start: 8, end: 16, words: [] }
  ] }
};

const fonts = {
  manrope: 'Manrope, "Segoe UI", sans-serif', sans: '"Segoe UI", sans-serif',
  serif: 'Georgia, serif', slab: 'Rockwell, "Roboto Slab", Georgia, serif',
  soft: '"Segoe UI Rounded", "Segoe UI", sans-serif', auto: '"Segoe UI", sans-serif'
};
const alternate = 'Georgia, serif';
const mediaOrigin = 'https://media.lunalyr.local/';
const engines = {
  wavefield: ['Wavefield', 'preset'], wavefieldSphere: ['Wavefield', 'sphere'],
  wavefieldOriginal: ['Wavefield', 'preset'], wavecore: ['Wavecore', 'preset'], light3D: ['Light3D', 'preset']
};

export function font(p) { return fonts[p.font] ?? fonts.sans; }
export function position(state, now) {
  const play = state.playback;
  if (!play) return 0;
  const value = (play.position ?? 0) + (play.playing ? Math.max(0, now - state.syncedAt) / 1000 : 0);
  return play.track?.duration > 0 ? clamp(value, 0, play.track.duration) : Math.max(0, value);
}

export class Stage {
  constructor(element, state, preview) {
    this.element = element;
    this.state = state;
    this.preview = preview;
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'scene';
    this.canvas.setAttribute('aria-hidden', 'true');
    this.ink = document.createElement('canvas');
    this.ink.className = 'ink';
    this.ink.setAttribute('aria-hidden', 'true');
    this.canvasContext = this.canvas.getContext('2d');
    this.inkContext = this.ink.getContext('2d');
    this.from = this.to = fallback;
    this.paletteStart = 0;
    this.artwork = undefined;
    this.last = -Infinity;
    this.elapsed = 0;
    this.render = this.render.bind(this);
    element.append(this.canvas, this.ink);
    this.animation = requestAnimationFrame(this.render);
  }
  dispose() {
    cancelAnimationFrame(this.animation);
    this.image?.remove();
    this.engine?.remove();
    this.gpu?.dispose();
  }
  updateArtwork(artwork, now) {
    if (artwork === this.artwork) return;
    this.artwork = artwork;
    const set = palette => {
      this.from = this.palette(performance.now());
      this.to = palette;
      this.paletteStart = performance.now();
    };
    if (!artwork) { set(fallback); return; }
    const image = new Image();
    image.onload = () => {
      if (this.artwork !== artwork) return;
      const sample = document.createElement('canvas');
      sample.width = sample.height = 30;
      const context = sample.getContext('2d', { willReadFrequently: true });
      context.drawImage(image, 0, 0, 30, 30);
      set(albumGrid(context.getImageData(0, 0, 30, 30).data, 30));
    };
    image.src = artwork;
  }
  palette(now) { return this.from.map((colour, i) => mix(colour, this.to[i], smooth((now - this.paletteStart) / 1450))); }
  render(now) {
    this.animation = requestAnimationFrame(this.render);
    if (document.hidden || !this.element.isConnected || this.element.closest('[hidden]')
        || (!this.preview && this.state.covered)) return;
    const p = this.state.preferences;
    const calm = p.calm || matchMedia('(prefers-reduced-motion: reduce)').matches;
    const low = p.lowPower || this.state.status?.lowPower;
    const rate = low ? Math.min(p.fps, 30) : p.fps;
    if (now - this.last < 1000 / rate - 2) return;
    if (p.wallpaperEnabled || this.preview) this.elapsed += Math.min(.1, (now - this.last) / 1000) * (calm ? .13 : 1);
    this.last = now;
    const w = this.element.clientWidth, h = this.element.clientHeight;
    if (!w || !h) return;
    const ratio = Math.min(devicePixelRatio, Math.sqrt((this.preview ? 520_000 : 2_400_000) / (w * h)));
    const dw = Math.max(1, Math.round(w * ratio)), dh = Math.max(1, Math.round(h * ratio));
    for (const canvas of [this.canvas, this.ink]) {
      if (canvas.width !== dw || canvas.height !== dh) {
        canvas.width = dw;
        canvas.height = dh;
      }
    }
    this.updateArtwork(this.state.playback?.artwork, now);
    const palette = this.palette(now);
    const t = this.elapsed * p.speed;
    const audio = this.state.audio;
    let background = p.background;
    if (low && !['custom', 'desktop', 'colour'].includes(background)) background = 'patterns';
    this.drawMedia(background, p);
    const trackSeed = hash(this.state.playback?.track?.key ?? '');
    if (background === 'match' && this.state.playback?.track) background = trackSeed % 3 ? 'effects' : 'patterns';
    const engine = background === 'visualiser' ? engines[p.visualiser] : null;
    this.drawEngine(engine, p, w, h, palette, audio, calm);
    const context = this.canvasContext;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    field(context, w, h, palette, t, p);
    if (background === 'patterns') lines(context, w, h, palette, t, p.pattern);
    if (background === 'effects') {
      this.gpu ??= new GpuScene(this.element, this.ink, this.preview);
      const styles = Object.keys(effectNames);
      const effect = p.effect === 'auto' ? styles[trackSeed % styles.length] : p.effect;
      if (!this.gpu.draw(effect, w, h, this.elapsed, p, palette)) ribbons(context, w, h, palette, t, null, p, 'silk');
    } else if (background === 'visualiser' && !engine) {
      this.gpu ??= new GpuScene(this.element, this.ink, this.preview);
      if (p.visualiser === 'starTrails') {
        this.gpu.hide();
        starTrails(context, w, h, palette, t, audio, p);
      } else if (!this.gpu.draw(p.visualiser, w, h, this.elapsed, p, palette, { audio, calm, artwork: !!this.artwork })) {
        if (p.visualiser === 'tunnel') tunnel(context, w, h, palette, t, audio, p);
        else if (p.visualiser === 'colourStrata' || p.visualiser === 'overprint') strata(context, w, h, palette, t, audio, p, p.visualiser);
        else ribbons(context, w, h, palette, t, audio, p, p.visualiser);
      }
    } else this.gpu?.hide();

    const draw = this.inkContext;
    draw.setTransform(ratio, 0, 0, ratio, 0, 0);
    draw.clearRect(0, 0, w, h);
    const track = this.state.playback?.track ?? (this.preview ? demo.track : null);
    const selected = this.state.lyrics?.selected ?? (this.preview && !this.state.playback?.track ? demo.lyrics : null);
    const time = (track === demo.track ? now / 1000 % 16 : position(this.state, now)) - (this.state.lyrics?.offset ?? 0);
    if (p.lyricsVisible) paintLyrics(draw, w, h, selected, time, track, {
      ...p, calm, searching: this.state.lyrics?.lookup === 'Looking for lyrics…'
    }, palette, this.elapsed, audio, this.preview);
    if (p.darkening !== 1) {
      draw.globalCompositeOperation = 'destination-over';
      draw.fillStyle = p.darkening > 1 ? `rgb(0 0 0 / ${clamp((p.darkening - 1) * .45)})` : `rgb(255 255 255 / ${clamp((1 - p.darkening) * .1)})`;
      draw.fillRect(0, 0, w, h);
      draw.globalCompositeOperation = 'source-over';
    }
  }
  drawMedia(mode, p) {
    const file = mode === 'custom' && /^[a-f\d]{32}\.(?:png|jpg|jpeg|webp|gif|mp4|webm)$/i.test(p.wallpaper ?? '') ? p.wallpaper : null;
    if (file !== this.file) {
      this.file = file;
      this.image?.remove();
      this.image = null;
      if (file) {
        const video = /\.(mp4|webm)$/i.test(file);
        const image = document.createElement(video ? 'video' : 'img');
        image.className = 'custom';
        if (video) { image.loop = image.muted = image.autoplay = image.playsInline = true; }
        else image.alt = '';
        image.src = mediaOrigin + file;
        this.element.insertBefore(image, this.ink);
        this.image = image;
      }
    }
    if (this.image) {
      this.image.style.objectFit = p.placement === 'fit' ? 'contain' : 'cover';
      this.image.style.objectPosition = p.placement === 'top' ? 'center top' : p.placement === 'bottom' ? 'center bottom' : 'center';
      if (this.image instanceof HTMLVideoElement && this.image.paused) this.image.play().catch(() => {});
    }
  }
  drawEngine(engine, p, width, height, palette, audio, calm) {
    const name = engine?.[0];
    if (name !== this.engineName) {
      this.engineName = name;
      this.engine?.remove();
      this.engine = null;
      if (name) {
        this.engine = document.createElement('iframe');
        this.engine.title = name + ' visualiser';
        this.engine.className = 'engine';
        this.engine.src = 'engines/' + name + 'Engine.html';
        this.engine.tabIndex = -1;
        this.element.insertBefore(this.engine, this.ink);
      }
    }
    const api = this.engine?.contentWindow?.['lunalyr' + name];
    if (!api || this.drawing) return;
    const signal = audio ? { ...audio, bands: audio.spectrum ?? Array(48).fill(0), centroid: 0 } : undefined;
    const frame = {
      width, height, clock: this.elapsed, preset: p.preset,
      projection: p.visualiser === 'wavefield' ? p.projection : engine[1], sensitivity: p.sensitivity,
      motion: p.speed, reduced: !!p.lowPower, calm, preview: this.preview, paused: false, punch: p.punch,
      audio: signal, pcm: audio?.pcm, sampleRate: audio?.sampleRate || 48000
    };
    this.drawing = true;
    Promise.resolve().then(() => api.draw(frame)).catch(() => {
      this.engine?.setAttribute('data-unavailable', '');
    }).finally(() => { this.drawing = false; });
  }
}

function fontAt(p, size, editorial = false) {
  return `${editorial ? 'italic 600' : '800'} ${size}px ${editorial ? alternate : font(p)}`;
}
function measure(context, text, size, p, editorial) {
  context.font = fontAt(p, size, editorial);
  return context.measureText(text).width;
}
function text(context, content, x, y, size, p, colour, italic = false, shadow = false) {
  context.font = fontAt(p, size, italic);
  context.textAlign = 'center';
  context.textBaseline = 'middle';
  context.fillStyle = colour;
  if (shadow) { context.shadowColor = 'rgb(0 0 0 / .76)'; context.shadowBlur = Math.max(5, size * .17); }
  context.fillText(content, x, y);
  context.shadowBlur = 0;
}

function enter(style, age, size, seed, calm) {
  if (calm) return [0, 0, 1, 0];
  const t = 1 - smooth(age / .37);
  switch (style) {
    case 'lift': return [0, t * size * .35, 1, 0];
    case 'pop': return [0, 0, 1 - t * .27, 0];
    case 'orbit': return [t * size * -.32, t * size * .26, 1 - .14 * t, -.2 * t];
    case 'glide': return [t * size * -.44, 0, 1, 0];
    case 'unfold': return [0, 0, 1 - t * .42, 0];
    case 'cutoutFold': return [0, size * t * .16, 1 - t * .32, -.18 * t];
    case 'cutoutSlide': return [size * .48 * t, 0, 1, .08 * t];
    case 'satinRibbon': return [size * t * .23, size * -.33 * t, 1 - t * .17, .12 * t];
    case 'splitInk': return [size * t * (seed % 2 ? .22 : -.22), 0, 1 - t * .16, 0];
    default: return [0, 0, 1, 0];
  }
}

function wrap(context, value, size, p, width) {
  let rows = [''];
  const segments = value.split(/\s+/).flatMap(word => measure(context, word, size, p) > width ? [...word] : [word]);
  for (const word of segments) {
    const current = rows.at(-1);
    if (current && measure(context, current + ' ' + word, size, p) > width) rows.push(word);
    else rows[rows.length - 1] = current ? current + ' ' + word : word;
  }
  return rows;
}

export function paintLyrics(context, w, h, selected, time, track, p, palette, clock, audio, preview = false) {
  const scale = Math.min(w / 860, h / 560);
  const base = clamp(66 * scale * p.lyricSize, 12, Math.min(w / 6, h / 3.2));
  const maxWidth = w * .88;
  const theme = typography(p, palette, clock, audio);
  const { line, next, gap, plain } = cueAt(selected?.lines ?? [], time);
  if (!line) {
    if (p.interludes && p.background !== 'desktop' && gap > 1.5) {
      const small = Math.max(11, base * .38);
      text(context, '♪', w / 2, h * .45, base, p, rgb(theme.end, .58), false, p.scrim);
      if (gap < 25) text(context, `Lyrics return in ${Math.ceil(gap)} s`, w / 2, h * .56, small, p, rgb(theme.end, .58), false, p.scrim);
    } else if (!selected && track && p.missingLyrics !== 'nothing') {
      const label = p.searching ? 'Looking for lyrics…' : p.missingLyrics === 'track' ? track.title : 'Lyrics unavailable';
      const size = Math.max(12, Math.min(base * .64, base * maxWidth / Math.max(1, measure(context, label, base, p))));
      text(context, label, w / 2, h / 2, size, p, rgb(theme.end, .65), false, p.scrim);
    }
    return;
  }
  const style = p.composition === 'auto'
    ? Object.keys(arrivalFor)[hash(track?.key ?? line.text) % 8] : p.composition;
  const key = p.composition === 'auto' ? 'auto' : style;
  let arrival = p.arrivals?.[key] ?? 'auto';
  if (arrival === 'auto') arrival = arrivalFor[style] ?? 'fade';
  if (arrival === 'mix') arrival = ['fade', 'lift', 'cutoutFold'][hash(line.text) % 3];
  const reveal = ['spoken', 'cascade', 'cutout'].includes(style) || arrival.startsWith('cutout') || (p.reveals?.[key] ?? true);
  const previewNext = p.nextWords?.[key] ?? true;
  const displayed = p.reading === 'replace' && line.reading ? line.reading : line.text;
  const words = displayed === line.text ? line.words : [];
  if (!words.length) {
    const size = Math.min(base, Math.max(base * .63, base * maxWidth / Math.max(1, measure(context, displayed, base, p))));
    const rows = wrap(context, displayed, size, p, maxWidth).slice(0, 6);
    rows.forEach((row, index) => text(context, row, w / 2, h * .48 + (index - (rows.length - 1) / 2) * size * 1.16, size, p,
      rgb(theme.end, plain ? .83 : 1), style === 'editorial', p.scrim));
  } else {
    let visible = words.map((word, i) => ({ ...word, i, ...wordState(word, time, reveal, previewNext, p.previewLead) }));
    let sung = visible.findLastIndex(word => word.phase === 'held' || word.phase === 'sung');
    let rolling = style === 'spoken' || style === 'cascade';
    if (rolling) visible = visible.slice(Math.max(0, sung - 3), sung + 3);
    const spread = ['scattered', 'constellation', 'constellationEditorial'].includes(style);
    const useEditorial = style === 'editorial' || style === 'constellationEditorial' || p.editorial;
    const seed = hash(line.text);
    const fullWidth = words.reduce((total, word) => total + measure(context, word.text, base, p) + base * .22, 0);
    const size = rolling ? base : base * Math.min(1, maxWidth * (spread ? 1.55 : 1.08) / Math.max(1, fullWidth));
    const layouts = visible.map(word => {
      const emphasis = word.i === sung;
      const italic = useEditorial && (word.i % 3 === 1 || word.text.length > 7);
      const wordSize = Math.min(rolling ? size * (emphasis ? 1.22 : .76) : size * (spread ? .9 + noise(seed + word.i) * .26 : 1),
        size * maxWidth / Math.max(1, measure(context, word.text, size, p, italic)));
      return { ...word, italic, size: wordSize, width: measure(context, word.text, wordSize, p, italic) };
    });
    const width = layouts.reduce((total, word) => total + word.width + size * .23, -size * .23);
    let x = (w - width) / 2;
    const points = [];
    for (const word of layouts) {
      let xAt = x + word.width / 2;
      let yAt = h * .48;
      x += word.width + size * .23;
      if (rolling) {
        const relative = word.i - Math.max(0, sung);
        xAt = w * (.5 + (style === 'spoken' ? relative * .088 : Math.sin(word.i * 3) * .071));
        yAt = h * .47 + relative * size * 1.13;
      } else if (spread) {
        const baseX = .12 + .76 * (word.i + .5) / words.length;
        xAt = clamp((baseX + (noise(seed ^ (word.i + 65)) - .5) * .18) * w, word.width / 2 + w * .04, w - word.width / 2 - w * .04);
        yAt += (noise(seed + word.i * 7) - .5) * h * .49 * clamp(p.spread, .2, 1.5);
      }
      points.push([xAt, yAt, word]);
    }
    if (style.startsWith('constellation')) {
      for (let i = 1; i < points.length; i++) {
        const [x0, y0, prior] = points[i - 1], [x1, y1, word] = points[i];
        context.strokeStyle = rgb(theme.end, (p.links ?? .2) * Math.min(prior.alpha, word.alpha));
        context.lineWidth = Math.max(.6, scale * 1.3);
        context.beginPath(); context.moveTo(x0, y0); context.lineTo(x1, y1); context.stroke();
      }
    }
    if (style === 'echo') {
      for (let copy = 0; copy < 7; copy++) {
        context.save();
        context.translate(w * (.12 + noise(seed + copy) * .76), h * (.14 + noise(seed + copy * 31) * .67));
        context.rotate(copy % 2 ? Math.PI : 0);
        const echoSize = Math.min(base * (.34 + noise(seed + copy * 89) * .26), base * maxWidth / Math.max(1, measure(context, line.text, base, p)));
        text(context, line.text, 0, 0, echoSize, p, rgb(theme.end, .16), copy % 3 === 0);
        context.restore();
      }
    }
    for (const [xAt, yAt, word] of points) {
      if (!word.alpha) continue;
      const [dx, dy, zoom, rotation] = word.phase === 'sung' ? enter(arrival, word.age, size, seed + word.i, p.calm) : [0, 0, 1, 0];
      context.save();
      context.translate(xAt + dx, yAt + dy);
      context.rotate(rotation);
      context.scale(zoom, zoom);
      const tile = (style === 'cutout' && word.i % 2 === 0 || arrival === 'cutoutHighlight' && word.phase === 'sung')
        && (word.phase === 'sung' || word.phase === 'held');
      if (tile) {
        context.fillStyle = rgb(theme.tile, word.alpha);
        context.beginPath();
        context.roundRect(-word.width / 2 - word.size * .12, -word.size * .56, word.width + word.size * .24, word.size * 1.04, word.size * .10);
        context.fill();
      }
      const ink = tile ? theme.foreground : theme.start;
      text(context, word.text, 0, 0, word.size, p, rgb(ink, word.alpha), word.italic, p.scrim && !tile);
      if (!tile && (p.textColour === 'sungSweep' || p.textColour === 'customGradient') && word.amount > 0) {
        context.beginPath();
        context.rect(-word.width / 2 - 1, -word.size, word.width * word.amount + 2, word.size * 2);
        context.clip();
        text(context, word.text, 0, 0, word.size, p, rgb(theme.end, word.alpha), word.italic, false);
      }
      context.restore();
    }
  }
  if (p.reading === 'below' && line.reading) {
    const size = Math.max(11, Math.min(base * .38, base * maxWidth / Math.max(1, measure(context, line.reading, base, p))));
    text(context, line.reading, w / 2, h * .72, size, p, rgb(theme.end, .76), false, p.scrim);
  } else if (next && (!words.length || preview)) {
    const size = Math.min(base * .68, base * maxWidth / Math.max(1, measure(context, next.text, base, p)));
    text(context, next.text, w / 2, h * .65, size, p, rgb(theme.end, .37), false, p.scrim);
  }
}
