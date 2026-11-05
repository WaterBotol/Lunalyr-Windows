export const clamp = (n, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, n));
export const smooth = n => { n = clamp(n); return n * n * (3 - 2 * n); };
export function hash(text) {
  let seed = 2166136261;
  for (const character of text) seed = Math.imul(seed ^ character.codePointAt(0), 16777619);
  return seed >>> 0;
}
export function noise(seed) {
  seed = Math.imul(seed ^ seed >>> 16, 0x21f0aaad);
  seed = Math.imul(seed ^ seed >>> 15, 0x735a2d97);
  return ((seed ^ seed >>> 15) >>> 0) / 4294967296;
}
export const arrivalFor = {
  scattered: 'pop', constellation: 'orbit', constellationEditorial: 'lift',
  editorial: 'fade', echo: 'lift', spoken: 'glide', cascade: 'lift', cutout: 'cutoutFold'
};

export function cueAt(lines, time) {
  if (!lines.length) return {};
  if (lines[0].start == null) return { line: lines[0], next: lines[1], index: 0, plain: true };
  let left = 0;
  let right = lines.length;
  while (left < right) {
    const mid = (left + right) >>> 1;
    if ((lines[mid].start ?? Infinity) <= time) left = mid + 1;
    else right = mid;
  }
  const prior = lines[left - 1];
  const next = lines[left];
  if (!prior || time >= prior.end) return { next, gap: next ? next.start - time : undefined };
  return { line: prior, next, index: left - 1 };
}

export function wordState(word, time, reveal, preview, lead) {
  if (time >= word.end) return { alpha: 1, amount: 1, age: time - word.start, phase: 'held' };
  if (time >= word.start)
    return { alpha: 1, amount: clamp((time - word.start) / Math.max(.05, word.end - word.start)), age: time - word.start, phase: 'sung' };
  const until = word.start - time;
  if (preview && until <= lead) return { alpha: .09 + .19 * smooth(1 - until / lead), amount: 0, age: 0, phase: 'preview' };
  return { alpha: reveal ? 0 : .31, amount: 0, age: 0, phase: 'future' };
}

export function albumGrid(pixels, size) {
  const result = [];
  for (let row = 0; row < 3; row++) {
    for (let column = 0; column < 3; column++) {
      const values = [0, 0, 0];
      let count = 0;
      for (let y = Math.floor(row * size / 3); y < Math.floor((row + 1) * size / 3); y++) {
        for (let x = Math.floor(column * size / 3); x < Math.floor((column + 1) * size / 3); x++) {
          const at = (y * size + x) * 4;
          const weight = pixels[at + 3] / 255;
          for (let c = 0; c < 3; c++) values[c] += pixels[at + c] * weight;
          count += weight;
        }
      }
      result.push(count ? values.map(value => value / count) : [25, 29, 45]);
    }
  }
  return result;
}

export const fallback = [[37, 39, 74], [71, 54, 117], [116, 54, 99],
  [29, 60, 96], [65, 65, 121], [134, 63, 99], [32, 81, 100], [59, 67, 115], [83, 54, 107]];
export const diagonal = [0, 1, 3, 2, 4, 6, 5, 7, 8];
export function mix(a, b, t) { return a.map((n, i) => n + (b[i] - n) * t); }
export function rgb(values, alpha = 1) { return `rgb(${values.map(n => Math.round(clamp(n, 0, 255))).join(' ')} / ${clamp(alpha)})`; }
export const hex = text => /^#[\da-f]{6}$/i.test(text) ? text.slice(1).match(/../g).map(value => parseInt(value, 16)) : [255, 255, 255];

function luminance(colour) {
  const linear = colour.map(n => n / 255 <= .04045 ? n / 3294.6 : Math.pow((n / 255 + .055) / 1.055, 2.4));
  return .2126 * linear[0] + .7152 * linear[1] + .0722 * linear[2];
}
const white = [255, 255, 255];
const ink = [19, 19, 26];
export function tileColours(colour, calm) {
  const threshold = calm ? 7 : 4.5;
  const contrast = (a, b) => (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
  let tile = colour;
  const light = luminance(tile) > .27;
  const foreground = light ? ink : white;
  for (let i = 0; i < 15 && contrast(luminance(tile), luminance(foreground)) < threshold; i++)
    tile = mix(tile, light ? white : [0, 0, 0], .14);
  return { tile, foreground };
}

export function vivid(colour, amount) {
  const gray = .2126 * colour[0] + .7152 * colour[1] + .0722 * colour[2];
  return colour.map(n => clamp(gray + (n - gray) * amount, 0, 255));
}

export function trackHue(palette) {
  return palette.reduce((a, b) => Math.max(...b) - Math.min(...b) > Math.max(...a) - Math.min(...a) ? b : a);
}

export function typography(p, palette, clock, audio) {
  const colour = trackHue(palette);
  const accent = mix(vivid(colour, 1.9), white, .45);
  const strength = p.colourStrength ?? .75;
  let end = white;
  if (p.textColour === 'albumDrift') {
    const position = ((clock / 6) % palette.length + palette.length) % palette.length;
    end = mix(mix(palette[Math.floor(position)], palette[(Math.floor(position) + 1) % palette.length], position % 1), white, .50);
  } else if (p.textColour === 'musicReactive') {
    end = mix(accent, [240, 182, 244], clamp((audio?.level ?? 0) * 1.8));
  } else if (p.textColour === 'customGradient') end = hex(p.customEnd);
  else if (p.textColour === 'sungSweep') end = accent;
  end = mix(white, end, strength);
  const start = p.textColour === 'customGradient' ? mix(white, hex(p.customText), strength)
    : p.textColour === 'sungSweep' ? white : end;
  const complement = colour.map(n => 255 - n);
  const tile = p.highlight === 'custom' ? hex(p.customHighlight)
    : p.highlight === 'album' || p.textColour === 'white' ? vivid(colour, 1.55) : vivid(complement, 1.2);
  return { start, end, ...tileColours(tile, p.calm) };
}
