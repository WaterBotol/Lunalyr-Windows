import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cueAt, albumGrid, lineOpacity, wordState, tileColours } from '../app/Web/score.mjs';
import { position } from '../app/Web/stage.mjs';
import { createGif } from '../app/Web/card.mjs';

test('mixed timing transitions keep plain lines and handle actual gaps', () => {
  const first = { start: 1, end: 2, words: [{}] };
  const second = { start: 4, end: 6, words: [] };
  assert.equal(cueAt([first, second], 1.5).line, first);
  assert.deepEqual(cueAt([first, second], 2.5), { next: second, gap: 1.5 });
  assert.equal(cueAt([first, second], 4).line, second);
  assert.equal(cueAt([first, second], 6).line, undefined);
});

test('plain lyrics advance with playback and seek to the corresponding line', () => {
  const lines = ['first', 'second', 'third'].map(text => ({ text, words: [] }));
  const second = cueAt(lines, 25, 60);
  assert.equal(second.line.text, 'second');
  assert.equal(second.line.start, 20);
  assert.equal(second.line.end, 40);
  assert.equal(second.plain, true);
  assert.equal(cueAt(lines, 100).line.text, 'third');
  assert.equal(cueAt(lines, 1, 60).line.text, 'first');
});

test('line changes fade at cue boundaries and respect reduced motion', () => {
  assert.equal(lineOpacity(4, 4, 8, false), 0);
  assert.ok(lineOpacity(4.12, 4, 8, false) > 0 && lineOpacity(4.12, 4, 8, false) < 1);
  assert.equal(lineOpacity(5, 4, 8, false), 1);
  assert.ok(lineOpacity(7.88, 4, 8, false) < 1);
  assert.equal(lineOpacity(4, 4, 8, true), 1);
  assert.equal(lineOpacity(10, 4, 8, false, true), 1);
});

test('preview and held words have different highlight states', () => {
  const word = { start: 5, end: 6 };
  assert.equal(wordState(word, 4, true, true, .45).alpha, 0);
  assert.equal(wordState(word, 4.75, true, true, .45).phase, 'preview');
  assert.equal(wordState(word, 5.5, true, true, .45).amount, .5);
  assert.equal(wordState(word, 6.1, true, true, .45).amount, 1);
});

test('all nine album regions survive sampling', () => {
  const pixels = Uint8Array.from(Array.from({ length: 9 }, (_, i) => [i, 3 * i, 7 * i, 255]).flat());
  assert.deepEqual(albumGrid(pixels, 3), Array.from({ length: 9 }, (_, i) => [i, 3 * i, 7 * i]));
  assert.deepEqual(tileColours([0, 0, 0], true).foreground, [255, 255, 255]);
});

test('clock honours pause and duration', () => {
  const state = { playback: { track: { duration: 2 }, position: .9, playing: true }, syncedAt: 100 };
  assert.equal(position(state, 400), 1.2);
  assert.equal(position(state, 4000), 2);
  state.playback.rate = 2;
  assert.equal(position(state, 400), 1.5);
  state.playback.playing = false;
  assert.equal(position(state, 4000), .9);
});

test('animated card uses GIF89a and contains an end marker', () => {
  const gif = createGif(2, 2, 12);
  gif.frame(new Uint8Array(16));
  gif.frame(new Uint8Array(16).fill(255));
  const bytes = gif.finish();
  assert.equal(Buffer.from(bytes.subarray(0, 6)).toString(), 'GIF89a');
  assert.equal(bytes.at(-1), 0x3b);
});
