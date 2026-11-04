// Exact ports of Sources/Support/ConstellationGeometry.c: word arrival and
// visibility, the instrumental-gap finder, the fixed ink envelope and the
// constellation link builder (minimum spanning tree with clipping).

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Segment {
  ax: number;
  ay: number;
  bx: number;
  by: number;
  from: number;
  to: number;
}

export interface VocalWindow {
  start: number;
  end: number;
}

export interface VocalGap {
  start: number;
  /** -1 when the gap runs to an unknown track end. */
  end: number;
  /** Index of the vocal window that ends the gap, or -1 at the end of the track. */
  nextVocal: number;
}

/** `vc_vocal_gap`: the instrumental gap containing `time`, if it is long enough. */
export function vocalGap(
  windows: readonly VocalWindow[],
  time: number,
  trackDuration: number,
  minimumGap: number,
): VocalGap | null {
  const count = windows.length;
  if (count > 20000 || !Number.isFinite(time) || time < 0) return null;
  let minimum = Number.isFinite(minimumGap) ? minimumGap : 2.4;
  minimum = Math.max(0.5, Math.min(10, minimum));
  const knownDuration = Number.isFinite(trackDuration) && trackDuration > 0;
  if (knownDuration && time >= trackDuration) return null;
  for (let i = 0; i < count; i++) {
    const w = windows[i];
    if (!Number.isFinite(w.start) || !Number.isFinite(w.end) || w.start < 0 || w.end < w.start ||
        (i > 0 && w.start < windows[i - 1].start)) return null;
  }
  let previous = 0;
  for (let i = 0; i < count; i++) {
    const start = windows[i].start;
    let end = windows[i].end;
    if (time >= previous && time < start && start - previous >= minimum) {
      return { start: previous, end: start, nextVocal: i };
    }
    while (i + 1 < count && windows[i + 1].start <= end) {
      i++;
      end = Math.max(end, windows[i].end);
    }
    if (time >= start && time < end) return null;
    previous = Math.max(previous, end);
  }
  if (time >= previous && (!knownDuration || trackDuration - previous >= minimum)) {
    return { start: previous, end: knownDuration ? trackDuration : -1, nextVocal: -1 };
  }
  return null;
}

/** `vc_word_envelope`: fixed ink box covering every rotation up to `degrees` and growth to `peak`. */
export function wordEnvelope(width: number, height: number, peakIn: number, degreesIn: number): { width: number; height: number } {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width < 0 || height < 0) return { width: 0, height: 0 };
  const peakFinite = Number.isFinite(peakIn) ? peakIn : 1.2;
  const degrees = Number.isFinite(degreesIn) ? degreesIn : 0;
  const peak = Math.max(1, Math.min(peakFinite, 2));
  const angle = Math.min(90, Math.abs(degrees)) * 0.017453292519943295;
  const s = Math.sin(angle);
  return { width: (width + height * s) * peak, height: (height + width * s) * peak };
}

/** `vc_word_reveal`: 0 before the word's timestamp, 1 after its entrance. Seek-safe. */
export function wordReveal(time: number, start: number, end: number): number {
  if (!Number.isFinite(time) || !Number.isFinite(start) || !Number.isFinite(end) || time < start) return 0;
  const duration = Math.min(0.46, Math.max(0.16, (end - start) * 0.65));
  const t = Math.min(1, Math.max(0, (time - start) / duration));
  return t * t * (3 - 2 * t);
}

/** `vc_word_visibility`: reveal, plus the faint preview of the next word. */
export function wordVisibility(
  time: number,
  start: number,
  end: number,
  previewEnabled: boolean,
  nextWord: boolean,
  previewLead: number,
): number {
  if (!Number.isFinite(time) || !Number.isFinite(start) || !Number.isFinite(end)) return 0;
  if (time < start) {
    if (!previewEnabled || !nextWord || !Number.isFinite(previewLead) || previewLead <= 0) return 0;
    const t = Math.min(1, Math.max(0, 1 - (start - time) / previewLead));
    return 0.25 * t * t * (3 - 2 * t);
  }
  const progress = wordReveal(time, start, end);
  return previewEnabled ? 0.25 + 0.75 * progress : progress;
}

/** `vc_word_settle`: a settle curve with a small overshoot (<= 1.05). */
export function wordSettle(progress: number): number {
  if (!Number.isFinite(progress) || progress <= 0) return 0;
  if (progress >= 1) return 1;
  const t = progress - 1;
  return 1 + 1.85 * t * t * t + 0.85 * t * t;
}

/** `vc_estimated_line_end`: conservative end for a line-only lyric before a long break. */
export function estimatedLineEnd(start: number, next: number, words: number, perWordIn: number): number {
  if (!Number.isFinite(start) || !Number.isFinite(next) || next <= start || words < 1 || words > 10000) return next;
  const perWord = Math.max(0.18, Math.min(1.2, Number.isFinite(perWordIn) ? perWordIn : 0.45));
  const sung = Math.max(4, Math.min(22, words * perWord * 1.35 + 1.25));
  return next - start > Math.max(12, sung + 6) ? start + sung : next;
}

// MARK: constellation links

function finiteBox(b: Box): boolean {
  return Number.isFinite(b.x) && Number.isFinite(b.y) && Number.isFinite(b.width) && Number.isFinite(b.height) &&
    b.width > 0 && b.height > 0 && Math.abs(b.x) < 1e8 && Math.abs(b.y) < 1e8 && b.width < 1e8 && b.height < 1e8;
}

function padded(b: Box, p: number): Box {
  return { x: b.x - p, y: b.y - p, width: b.width + 2 * p, height: b.height + 2 * p };
}

function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
}

function interior(b: Box, x: number, y: number): boolean {
  const e = 1e-7;
  return x > b.x + e && x < b.x + b.width - e && y > b.y + e && y < b.y + b.height - e;
}

const clipP = [0, 0, 0, 0];
const clipQ = [0, 0, 0, 0];

/** Liang–Barsky clipping in place; also used to reject links through another word. */
function clip(s: Segment, r: Box): boolean {
  const dx = s.bx - s.ax;
  const dy = s.by - s.ay;
  let t0 = 0;
  let t1 = 1;
  clipP[0] = -dx; clipP[1] = dx; clipP[2] = -dy; clipP[3] = dy;
  clipQ[0] = s.ax - r.x; clipQ[1] = r.x + r.width - s.ax; clipQ[2] = s.ay - r.y; clipQ[3] = r.y + r.height - s.ay;
  for (let i = 0; i < 4; i++) {
    const p = clipP[i];
    const q = clipQ[i];
    if (Math.abs(p) < 1e-12) {
      if (q < 0) return false;
      continue;
    }
    const t = q / p;
    if (p < 0) {
      if (t > t1) return false;
      if (t > t0) t0 = t;
    } else {
      if (t < t0) return false;
      if (t < t1) t1 = t;
    }
  }
  const x = s.ax;
  const y = s.ay;
  s.ax = x + t0 * dx;
  s.ay = y + t0 * dy;
  s.bx = x + t1 * dx;
  s.by = y + t1 * dy;
  return Math.hypot(s.bx - s.ax, s.by - s.ay) > 1;
}

function connect(a: Box, b: Box, view: Box, gap: number, s: Segment): boolean {
  const pa = padded(a, gap);
  const pb = padded(b, gap);
  if (overlaps(pa, pb)) return false;
  const ax = a.x + a.width / 2;
  const ay = a.y + a.height / 2;
  const bx = b.x + b.width / 2;
  const by = b.y + b.height / 2;
  const dx = bx - ax;
  const dy = by - ay;
  if (Math.hypot(dx, dy) < 1e-8) return false;
  const at = Math.min(Math.abs(dx) > 1e-12 ? pa.width / 2 / Math.abs(dx) : Infinity,
    Math.abs(dy) > 1e-12 ? pa.height / 2 / Math.abs(dy) : Infinity);
  const bt = Math.min(Math.abs(dx) > 1e-12 ? pb.width / 2 / Math.abs(dx) : Infinity,
    Math.abs(dy) > 1e-12 ? pb.height / 2 / Math.abs(dy) : Infinity);
  s.ax = ax + dx * at;
  s.ay = ay + dy * at;
  s.bx = bx - dx * bt;
  s.by = by - dy * bt;
  if (!clip(s, view)) return false;
  return !interior(pa, s.ax, s.ay) && !interior(pa, s.bx, s.by) && !interior(pb, s.ax, s.ay) && !interior(pb, s.bx, s.by);
}

function root(parent: number[], start: number): number {
  let x = start;
  while (parent[x] !== x) {
    parent[x] = parent[parent[x]];
    x = parent[x];
  }
  return x;
}

/**
 * `vc_build_connections`: text-to-text links as a minimum spanning tree over
 * word centres, clipped outside padded glyph bounds and inside the canvas.
 * Invalid, overlapping and off-screen nodes never produce a link.
 */
export function buildConnections(boxes: readonly Box[], viewport: Box, clearance: number, capacity = 64): Segment[] {
  const count = boxes.length;
  if (count < 2 || count > 64 || capacity < 1 || !finiteBox(viewport)) return [];
  const gap = Number.isFinite(clearance) ? Math.max(0, Math.min(clearance, 40)) : 6;
  const edges: { segment: Segment; cost: number }[] = [];
  const parent: number[] = [];
  for (let i = 0; i < count; i++) parent.push(i);
  for (let i = 0; i < count; i++) {
    if (!finiteBox(boxes[i]) || !overlaps(boxes[i], viewport)) continue;
    for (let j = i + 1; j < count; j++) {
      if (!finiteBox(boxes[j]) || !overlaps(boxes[j], viewport)) continue;
      const line: Segment = { ax: 0, ay: 0, bx: 0, by: 0, from: i, to: j };
      if (!connect(boxes[i], boxes[j], viewport, gap, line)) continue;
      let blocked = false;
      for (let k = 0; k < count; k++) {
        if (k === i || k === j || !finiteBox(boxes[k])) continue;
        const test: Segment = { ...line };
        if (clip(test, padded(boxes[k], gap * 0.6))) {
          blocked = true;
          break;
        }
      }
      if (blocked) continue;
      const dx = (boxes[j].x + boxes[j].width / 2) - (boxes[i].x + boxes[i].width / 2);
      const dy = (boxes[j].y + boxes[j].height / 2) - (boxes[i].y + boxes[i].height / 2);
      edges.push({ segment: line, cost: dx * dx + dy * dy });
    }
  }
  edges.sort((a, b) => {
    if (a.cost < b.cost) return -1;
    if (a.cost > b.cost) return 1;
    if (a.segment.from !== b.segment.from) return a.segment.from - b.segment.from;
    return a.segment.to - b.segment.to;
  });
  const output: Segment[] = [];
  for (let i = 0; i < edges.length && output.length < capacity; i++) {
    const a = root(parent, edges[i].segment.from);
    const b = root(parent, edges[i].segment.to);
    if (a === b) continue;
    parent[a] = b;
    output.push(edges[i].segment);
  }
  return output;
}

function sameBox(a: Box, b: Box): boolean {
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/**
 * `vc_cached_connections`: one bounded memo per canvas. Lighting and arrival
 * redraws reuse geometry; only an actual layout change rebuilds it.
 */
export class ConnectionCache {
  private boxes: Box[] = [];
  private viewport: Box = { x: 0, y: 0, width: 0, height: 0 };
  private clearance = Number.NaN;
  private segments: Segment[] = [];
  private valid = false;
  builds = 0;

  get(boxes: readonly Box[], viewport: Box, clearance: number, capacity = 64): readonly Segment[] {
    const count = boxes.length;
    if (count < 2 || count > 64 || capacity < 1 || !finiteBox(viewport)) return [];
    const gap = Number.isFinite(clearance) ? Math.max(0, Math.min(clearance, 40)) : 6;
    let hit = this.valid && this.boxes.length === count && this.clearance === gap && sameBox(this.viewport, viewport);
    if (hit) {
      for (let i = 0; i < count; i++) {
        if (!sameBox(this.boxes[i], boxes[i])) {
          hit = false;
          break;
        }
      }
    }
    if (!hit) {
      this.segments = buildConnections(boxes, viewport, gap, 63);
      this.boxes = boxes.map((b) => ({ x: b.x, y: b.y, width: b.width, height: b.height }));
      this.viewport = { ...viewport };
      this.clearance = gap;
      this.valid = true;
      this.builds++;
    }
    return this.segments.length <= capacity ? this.segments : this.segments.slice(0, capacity);
  }
}
