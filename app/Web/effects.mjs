import { clamp, diagonal, mix, noise, rgb, vivid } from './score.mjs';

const tau = Math.PI * 2;

export function field(context, w, h, palette, t, p) {
  const angle = .56 + Math.sin(t / 25) * .3;
  const dx = Math.cos(angle) * w / 2, dy = Math.sin(angle) * h / 2;
  const gradient = context.createLinearGradient(w / 2 - dx, h / 2 - dy, w / 2 + dx, h / 2 + dy);
  const colours = p.albumGradient === false ? [4] : diagonal;
  colours.forEach((index, i) => {
    const colour = vivid(palette[index], p.richness ?? 1);
    gradient.addColorStop(colours.length === 1 ? 0 : i / (colours.length - 1), rgb(colour.map(n => n * .72)));
  });
  context.fillStyle = gradient;
  context.fillRect(0, 0, w, h);
  const glow = context.createRadialGradient(w * (.30 + Math.sin(t / 13) * .09), h * .8, 0, w * .42, h * .71, w * .64);
  glow.addColorStop(0, rgb(palette[6], .30));
  glow.addColorStop(1, rgb(palette[6], 0));
  context.fillStyle = glow;
  context.fillRect(0, 0, w, h);
}

export function lines(context, w, h, palette, t, style) {
  context.lineWidth = Math.max(.5, w / 950);
  for (let line = 0; line < 32; line++) {
    context.beginPath();
    context.strokeStyle = rgb(palette[line % 9], .38);
    for (let i = 0; i <= 64; i++) {
      const x = i / 64;
      const y = line / 32;
      const warp = style === 'grid' ? .018 : .08;
      const curve = y + Math.sin(x * (style === 'contour' ? 13 : 7) + t * .3 + y * 4) * warp
        + Math.sin(x * 19 - t * .19 + y * 10) * warp * .26;
      if (i) context.lineTo(x * w, curve * h);
      else context.moveTo(x * w, curve * h);
    }
    context.stroke();
  }
}

function bands(audio, x) {
  const a = audio?.spectrum;
  if (!a?.length) return 0;
  const index = clamp(x) * (a.length - 1);
  const left = Math.floor(index);
  return a[left] + ((a[left + 1] ?? a[left]) - a[left]) * (index - left);
}

export function ribbons(context, w, h, palette, t, audio, p, style) {
  const count = style === 'fosforaAurora' ? 7 : style === 'curtainAurora' ? 5 : 6;
  const gain = p.sensitivity ?? 1;
  for (let ribbon = 0; ribbon < count; ribbon++) {
    const colour = vivid(palette[(ribbon * 2 + 2) % 9], 1.5);
    const base = .30 + ribbon * .075;
    const path = new Path2D();
    for (let i = 0; i <= 96; i++) {
      const x = i / 96;
      const local = bands(audio, x) * gain;
      const wave = Math.sin(x * 7.2 + t * .33 + ribbon * .53) * (.054 + local * .19)
        + Math.sin(x * 17 - t * .47 + ribbon * 2) * (.011 + local * .07);
      const y = (base + wave) * h;
      if (i) path.lineTo(x * w, y);
      else path.moveTo(0, y);
    }
    context.lineWidth = h * (.025 + clamp((audio?.level ?? 0) * gain) * .012);
    context.strokeStyle = rgb(colour, style === 'silk' ? .12 : .21);
    context.stroke(path);
    context.lineWidth = h * .003;
    context.strokeStyle = rgb(mix(colour, [255, 255, 255], .25), .57);
    context.stroke(path);
  }
}

export function tunnel(context, w, h, palette, t, audio, p) {
  const gain = p.sensitivity ?? 1;
  for (let ring = 17; ring >= 0; ring--) {
    const u = ((ring + t * 1.6) % 18) / 18;
    const radius = (.02 + u * u * .75) * w;
    context.beginPath();
    for (let i = 0; i <= 90; i++) {
      const angle = i / 90 * tau;
      const r = radius * (1 + .16 * Math.sin(5 * angle + t / 3) * bands(audio, i / 90) * gain);
      const x = w / 2 + Math.cos(angle) * r;
      const y = h / 2 + Math.sin(angle) * r * .63;
      if (i) context.lineTo(x, y); else context.moveTo(x, y);
    }
    context.lineWidth = .65 + u * w / 380;
    context.strokeStyle = rgb(vivid(palette[ring % 9], 1.8), .54 * (1 - u * .45));
    context.stroke();
  }
}

export function strata(context, w, h, palette, t, audio, p, style) {
  for (let level = 0; level < 9; level++) {
    context.beginPath();
    context.moveTo(0, h);
    for (let i = 0; i <= 48; i++) {
      const x = i / 48;
      const signal = bands(audio, x) * (p.sensitivity ?? 1);
      const y = .22 + level * .087 + Math.sin(x * 8 + level * .8 + t / 3) * (.033 + signal * .12);
      context.lineTo(x * w, h * y);
    }
    context.lineTo(w, h);
    context.closePath();
    context.fillStyle = rgb(vivid(palette[diagonal[level]], 1.4), style === 'overprint' ? .38 : .8);
    context.fill();
  }
}

const stars = Array.from({ length: 1900 }, (_, i) => [noise(i * 9), noise(i * 9 + 1), noise(i * 9 + 2)]);
export function starTrails(context, w, h, palette, t, audio, p) {
  context.fillStyle = '#03040b';
  context.fillRect(0, 0, w, h);
  for (let i = 0; i < 9; i++) {
    const x = (i % 3) * w * .48 + w * .02, y = Math.floor(i / 3) * h * .49;
    const g = context.createRadialGradient(x, y, 0, x, y, w * .46);
    g.addColorStop(0, rgb(vivid(palette[i], 1.5), .20));
    g.addColorStop(1, rgb(palette[i], 0));
    context.fillStyle = g;
    context.fillRect(0, 0, w, h);
  }
  for (const [x, y, brightness] of stars) {
    const drift = t * (.0009 + brightness * .0016);
    const sx = ((x - drift) % 1 + 1) % 1;
    const sy = (y + drift * .62) % 1;
    context.fillStyle = `rgb(232 225 255 / ${.1 + brightness * .45})`;
    const size = .40 + brightness * Math.max(.75, w / 1400);
    context.fillRect(sx * w, sy * h, size, size);
  }
  for (let j = 0; j < 2; j++) {
    const x = (.50 - j * .08 + Math.sin(t * .23 + j * 1.7) * .09) * w;
    const y = (.43 + j * .14 + Math.sin(t * .19 + j * 1.5) * .08) * h;
    const colour = mix(vivid(palette[j ? 6 : 2], 1.8), j ? [90, 190, 141] : [219, 101, 170], .42);
    const trail = context.createLinearGradient(x, y, 0, h);
    trail.addColorStop(0, rgb(colour, .88));
    trail.addColorStop(1, rgb(colour, .12));
    context.beginPath();
    context.moveTo(-.15 * w, h * (1.17 + j * .1));
    context.quadraticCurveTo(x * .22, y + h * .33, x, y);
    context.lineWidth = w * (.003 + clamp((audio?.bass ?? 0) * (p.sensitivity ?? 1)) * .003);
    context.strokeStyle = trail;
    context.stroke();
    context.save();
    context.translate(x, y);
    context.rotate(-.48);
    const size = Math.max(8, w * .019);
    context.beginPath();
    context.moveTo(size, 0); context.lineTo(-size * .64, size * .4);
    context.lineTo(-size * .21, 0); context.lineTo(-size * .64, -size * .4); context.closePath();
    context.fillStyle = rgb(colour, .96);
    context.fill();
    context.restore();
  }
}
