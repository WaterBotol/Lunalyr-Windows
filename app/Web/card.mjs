import { field } from './effects.mjs';
import { fallback, cueAt } from './score.mjs';
import { font, paintLyrics } from './stage.mjs';

function lzw(pixels) {
  const bytes = [];
  let current = 0, bits = 0, width = 9, next = 258;
  const dictionary = new Map();
  function emit(code) {
    current |= code << bits;
    bits += width;
    while (bits >= 8) {
      bytes.push(current & 255);
      current >>>= 8;
      bits -= 8;
    }
  }
  emit(256);
  let prefix = pixels[0];
  for (let i = 1; i < pixels.length; i++) {
    const key = prefix * 256 + pixels[i];
    const found = dictionary.get(key);
    if (found !== undefined) { prefix = found; continue; }
    emit(prefix);
    if (next === 4096) {
      emit(256);
      dictionary.clear();
      next = 258;
      width = 9;
    } else {
      if (next === 1 << width) width++;
      dictionary.set(key, next++);
    }
    prefix = pixels[i];
  }
  emit(prefix);
  if (next === 1 << width && width < 12) width++;
  emit(257);
  if (bits) bytes.push(current & 255);
  return bytes;
}

export function createGif(width, height, delay) {
  const output = [...'GIF89a'].map(letter => letter.charCodeAt(0));
  const word = value => { output.push(value & 255, value >>> 8 & 255); };
  word(width); word(height);
  output.push(0xf7, 0, 0);
  for (let i = 0; i < 256; i++)
    output.push(Math.floor(i / 36) % 6 * 51, Math.floor(i / 6) % 6 * 51, i % 6 * 51);
  output.push(0x21, 0xff, 0x0b, ...[...'NETSCAPE2.0'].map(letter => letter.charCodeAt(0)), 3, 1, 0, 0, 0);
  return {
    frame(rgba) {
      const indexed = new Uint8Array(width * height);
      for (let i = 0, j = 0; i < indexed.length; i++, j += 4)
        indexed[i] = Math.round(rgba[j] / 51) * 36 + Math.round(rgba[j + 1] / 51) * 6 + Math.round(rgba[j + 2] / 51);
      output.push(0x21, 0xf9, 4, 4);
      word(delay);
      output.push(0, 0, 0x2c);
      word(0); word(0); word(width); word(height);
      output.push(0, 8);
      const compressed = lzw(indexed);
      for (let start = 0; start < compressed.length; start += 255) {
        const block = compressed.slice(start, start + 255);
        output.push(block.length, ...block);
      }
      output.push(0);
    },
    finish() { return Uint8Array.from([...output, 0x3b]); }
  };
}

export async function lyricCard(state, time, palette = fallback) {
  const selected = state.lyrics?.selected;
  const { track, artwork } = state.playback;
  const { line } = cueAt(selected?.lines ?? [], time, track?.duration);
  if (!line) throw new Error('Play a song with lyrics to copy a lyric card.');
  const w = 1080, h = 1350;
  const duration = Math.max(.8, Math.min(12, (line.end ?? time + 2) - (line.start ?? time)));
  const frames = Math.min(42, Math.ceil(duration / .12));
  const delay = Math.round(duration * 100 / frames);
  const encoder = createGif(w, h, delay);
  const canvas = document.createElement('canvas');
  canvas.width = w; canvas.height = h;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  const image = artwork ? new Image() : null;
  if (image) { image.src = artwork; await image.decode().catch(() => {}); }
  for (let frame = 0; frame < frames; frame++) {
    if (frame % 3 === 0) await new Promise(resolve => setTimeout(resolve, 0));
    const t = frame * duration / frames;
    field(context, w, h, palette, t, state.preferences);
    if (image?.naturalWidth) {
      context.save();
      context.beginPath();
      context.roundRect(80, h - 235, 150, 150, 20);
      context.clip();
      context.drawImage(image, 80, h - 235, 150, 150);
      context.restore();
    }
    context.fillStyle = '#fff';
    context.font = '700 36px ' + font(state.preferences);
    context.fillText(track.title.slice(0, 40), image ? 264 : 80, h - 168, 730);
    context.fillStyle = 'rgb(255 255 255 / .70)';
    context.font = '500 29px ' + font(state.preferences);
    context.fillText(track.artist.slice(0, 45), image ? 264 : 80, h - 116, 730);
    context.save();
    context.translate(64, 80);
    paintLyrics(context, w - 128, h - 380, { lines: [line] }, (line.start ?? time) + t,
      track, { ...state.preferences, lyricsVisible: true }, palette, t);
    context.restore();
    encoder.frame(context.getImageData(0, 0, w, h).data);
  }
  const bytes = encoder.finish();
  let binary = '';
  for (let i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
  return btoa(binary);
}
