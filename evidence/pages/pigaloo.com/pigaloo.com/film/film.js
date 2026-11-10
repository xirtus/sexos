// The film: one cloud of points on a fixed canvas, flying between forms as the page scrolls.
// Shared by the network. Each site passes its forms, colours, scenes and reel in a small config file.
const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
const TAU = Math.PI * 2;
const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));

export function random(seed) {
  return () => { seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
const line = (r, o, a, b, j = .012) => { const t = r(); o[0] = a[0] + (b[0] - a[0]) * t + (r() - .5) * j; o[1] = a[1] + (b[1] - a[1]) * t + (r() - .5) * j; o[2] = a[2] + (b[2] - a[2]) * t + (r() - .5) * j; };
const box = (r, o, cx, cy, cz, w, h, d) => {
  const f = Math.floor(r() * 3), s = r() < .5 ? -1 : 1, u = r() * 2 - 1, v = r() * 2 - 1;
  const p = f === 0 ? [s, u, v] : f === 1 ? [u, s, v] : [u, v, s];
  o[0] = cx + p[0] * w; o[1] = cy + p[1] * h; o[2] = cz + p[2] * d;
};
const polygon = (r, o, points, z = 0, j = .012) => { const i = Math.floor(r() * points.length), a = points[i], b = points[(i + 1) % points.length]; line(r, o, [a[0], a[1], z], [b[0], b[1], z], j); };
const starPoints = (n, R, q, rot = -Math.PI / 2) => Array.from({ length: n * 2 }, (_, i) => { const m = i % 2 ? q : R, a = rot + i * Math.PI / n; return [Math.cos(a) * m, Math.sin(a) * m]; });

export const FORMS = {
  sphere(r, o) { const y = r() * 2 - 1, a = r() * TAU, q = Math.sqrt(1 - y * y); o[0] = Math.cos(a) * q; o[1] = y; o[2] = Math.sin(a) * q; },
  network(r, o) {
    // Seven bright hubs on a sphere, joined by threads through the middle.
    const hubs = [[.9, .2, .3], [-.8, .4, .4], [.1, .95, -.2], [-.3, -.85, .4], [.6, -.6, -.5], [-.7, -.2, -.7], [.2, .1, .98]];
    const p = r();
    if (p < .34) { const h = hubs[Math.floor(r() * 7)], a = r() * TAU, y = r() * 2 - 1, q = Math.sqrt(1 - y * y), s = .1; o[0] = h[0] + Math.cos(a) * q * s; o[1] = h[1] + y * s; o[2] = h[2] + Math.sin(a) * q * s; }
    else if (p < .76) { const a = hubs[Math.floor(r() * 7)], b = hubs[Math.floor(r() * 7)]; line(r, o, a, b, .018); }
    else { FORMS.sphere(r, o); o[0] *= 1.02; o[1] *= 1.02; o[2] *= 1.02; }
  },
  aperture(r, o) {
    if (r() < .28) { const a = r() * TAU, R = 1.02 + r() * .1; o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = (r() - .5) * .45; return; }
    const b = Math.floor(r() * 7), t = r(), sweep = b / 7 * TAU + t * 1.25, rad = .3 + t * .74, spread = (r() - .5) * .34 * (1 - t * .35);
    o[0] = Math.cos(sweep + spread) * rad; o[1] = Math.sin(sweep + spread) * rad; o[2] = (b % 2 ? .05 : -.05) + (t - .5) * .22;
  },
  wave(r, o) { const x = r() * 3.2 - 1.6, z = r() * 2.2 - 1.1; o[0] = x; o[2] = z; o[1] = .26 * Math.sin(x * 2.6 + z * 2.1) + .15 * Math.sin(x * 5.3 - z * 3.2) - .05; },
  ring(r, o) { const a = r() * TAU, band = r() < .5 ? 1.42 : 1.5, tube = (r() - .5) * .2; o[0] = Math.cos(a) * (band + tube * .3); o[1] = tube * .6 + Math.sin(a * 5) * .03; o[2] = Math.sin(a) * (band + tube * .3); },
  cube(r, o) {
    // A lattice: an engine's worth of little cells.
    const n = 4, g = () => (Math.floor(r() * (n + 1)) / n - .5) * 1.8, t = r() * 1.8 - .9, axis = Math.floor(r() * 3), a = g(), b = g();
    const p = axis === 0 ? [t, a, b] : axis === 1 ? [a, t, b] : [a, b, t], j = r() < .3 ? .03 : .006;
    o[0] = p[0] + (r() - .5) * j; o[1] = p[1] + (r() - .5) * j; o[2] = p[2] + (r() - .5) * j;
  },
  chip(r, o) {
    const p = r();
    if (p < .34) { box(r, o, 0, 0, 0, .78, .78, .08); return; }
    if (p < .6) { const s = Math.floor(r() * 4), k = Math.floor(r() * 8), u = (k / 7 - .5) * 1.34, t = .78 + r() * .34; const q = [[u, t], [u, -t], [t, u], [-t, u]][s]; o[0] = q[0] + (r() - .5) * .03; o[1] = q[1] + (r() - .5) * .03; o[2] = (r() - .5) * .02; return; }
    if (p < .86) { const x = Math.floor(r() * 7), y = Math.floor(r() * 7), horizontal = r() < .5, t = r() * .16; o[0] = (x / 6 - .5) * 1.1 + (horizontal ? t : 0); o[1] = (y / 6 - .5) * 1.1 + (horizontal ? 0 : t); o[2] = .09; return; }
    box(r, o, 0, 0, .12, .24, .24, .03);
  },
  city(r, o) {
    const i = Math.floor(r() * 30), a = random(i * 7 + 3), x = a() * 2.8 - 1.4, z = a() * 1.6 - .8, h = .12 + a() * a() * 1.15, w = .05 + a() * .09;
    if (r() < .08) { o[0] = r() * 3 - 1.5; o[1] = -.72; o[2] = r() * 1.8 - .9; return; }
    box(r, o, x, -.72 + h / 2, z, w, h / 2, w); o[1] = Math.round(o[1] * 18) / 18;
  },
  gamepad(r, o) {
    const p = r();
    if (p < .58) { const a = r() * TAU, s = r() < .5 ? -1 : 1, q = .42 + (r() - .5) * .018; const x = Math.cos(a) * q, y = Math.sin(a) * q; if (r() < .55) { o[0] = s * .55 + x; o[1] = y - (s * x > 0 ? .18 * s * x : 0); } else { o[0] = (r() - .5) * 1.1; o[1] = r() < .5 ? q : -.12; } o[2] = (r() - .5) * .22; return; }
    if (p < .78) { const v = r() < .5, t = (r() - .5) * .34; o[0] = -.56 + (v ? (r() - .5) * .09 : t); o[1] = .03 + (v ? t : (r() - .5) * .09); o[2] = .12; return; }
    const k = [[.45, .03], [.65, .03], [.55, .14], [.55, -.08]][Math.floor(r() * 4)], a = r() * TAU, q = Math.sqrt(r()) * .07; o[0] = k[0] + Math.cos(a) * q; o[1] = k[1] + Math.sin(a) * q; o[2] = .12;
  },
  heart(r, o) {
    const t = r() * TAU, k = r() < .55 ? 1 : Math.sqrt(r()), x = 16 * Math.sin(t) ** 3, y = 13 * Math.cos(t) - 5 * Math.cos(2 * t) - 2 * Math.cos(3 * t) - Math.cos(4 * t);
    o[0] = x / 16 * k; o[1] = y / 16 * k + .05; o[2] = (r() - .5) * .28 * (1 - k * .6);
  },
  letterX(r, o) {
    // A pixel X, built of little square cells.
    const s = 9, c = Math.floor(r() * s), d = r() < .5 ? c : s - 1 - c, cx = (c / (s - 1) - .5) * 1.9, cy = (d / (s - 1) - .5) * 1.9, w = .1;
    box(r, o, cx, cy, 0, w, w, w);
  },
  starburst(r, o) { polygon(r, o, starPoints(7, 1.12, .48), (r() - .5) * .1, .03); if (r() < .3) { const a = r() * TAU, q = r() * .46; o[0] = Math.cos(a) * q; o[1] = Math.sin(a) * q; } },
  terminal(r, o) {
    const p = r();
    if (p < .34) { const f = Math.floor(r() * 4), u = r() * 2 - 1; const q = [[u * 1.2, .78], [u * 1.2, -.78], [1.2, u * .78], [-1.2, u * .78]][f]; o[0] = q[0]; o[1] = q[1]; o[2] = (r() - .5) * .03; return; }
    if (p < .62) { line(r, o, [-.9, .4, 0], r() < .5 ? [-.5, .13, 0] : [-.5, .13, 0], .05); if (r() < .5) line(r, o, [-.5, .13, 0], [-.9, -.14, 0], .05); return; }
    if (p < .8) { o[0] = -.25 + (r() - .5) * .18; o[1] = -.2 + (r() - .5) * .34; o[2] = 0; return; }
    const row = Math.floor(r() * 3); o[0] = .1 + r() * (.9 - row * .2); o[1] = .45 - row * .24 + (r() - .5) * .03; o[2] = 0;
  },
  // SexOS: the address space, the keys, the wire, the gear.
  addressSpace(r, o) {
    const p = r(), a = r() * TAU;
    if (p < .5) { const R = 1.05 + (r() - .5) * .05; o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = (r() - .5) * .34; return; }
    const seg = Math.floor(a / TAU * 16) / 16 * TAU, R = .3 + Math.floor(r() * 4) * .18; if (Math.floor(a / TAU * 16) % 2) { FORMS.sphere(r, o); o[0] *= .14; o[1] *= .14; o[2] *= .14; return; }
    o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = Math.sin(seg * 3) * .12;
  },
  key(r, o) {
    const p = r();
    if (p < .5) { const a = r() * TAU, q = .34 + r() * .16; o[0] = -.6 + Math.cos(a) * q; o[1] = Math.sin(a) * q; o[2] = (r() - .5) * .14; return; }
    if (p < .84) { box(r, o, .32, 0, 0, .55, .07, .05); return; }
    const k = Math.floor(r() * 3); box(r, o, .35 + k * .22, -.15, 0, .05, .1, .05);
  },
  stream(r, o) {
    const lane = Math.floor(r() * 5), c = Math.floor(r() * 7), y = (lane / 4 - .5) * 1.5, x = ((c + lane * .4) % 7) / 6 * 3 - 1.5;
    if (r() < .28) { line(r, o, [-1.7, y, 0], [1.7, y, 0], .012); return; }
    box(r, o, x, y, 0, .15, .08, .08);
  },
  gear(r, o) {
    const teeth = 14, a = r() * TAU, tooth = Math.cos(a * teeth) > 0 ? 1.08 : .88, p = r(), s = r() < .5 ? 1 : -1;
    if (p < .5) { o[0] = Math.cos(a) * tooth; o[1] = Math.sin(a) * tooth; o[2] = s * .11 * r(); return; }
    if (p < .8) { const q = .42 + r() * (.46); o[0] = Math.cos(a) * q; o[1] = Math.sin(a) * q; o[2] = s * .11; return; }
    const q = .28 + (r() - .5) * .04; o[0] = Math.cos(a) * q; o[1] = Math.sin(a) * q; o[2] = (r() - .5) * .22;
  },
  // xChurch: time.
  hourglass(r, o) {
    const y = r() * 2 - 1, w = Math.abs(y) ** 1.25 * .62 + .03, a = r() * TAU, p = r();
    if (p < .1) { const s = r() < .5 ? 1 : -1, q = Math.sqrt(r()) * .72; o[0] = Math.cos(a) * q; o[1] = s * 1.04; o[2] = Math.sin(a) * q; return; }
    if (p < .6) { o[0] = Math.cos(a) * w; o[1] = y; o[2] = Math.sin(a) * w; return; }
    if (p < .84) { const yy = -.98 + r() * .6, q = Math.abs(yy) ** 1.25 * .62 * Math.sqrt(r()) * (1 - (yy + .98) * .55); o[0] = Math.cos(a) * q; o[1] = yy; o[2] = Math.sin(a) * q; return; }
    o[0] = (r() - .5) * .02; o[1] = r() * .8 - .4; o[2] = (r() - .5) * .02;
  },
  helix(r, o) {
    const t = r() * 2 - 1, a = t * 7, R = .42, p = r();
    if (p < .78) { const s = r() < .5 ? 0 : Math.PI; o[0] = t * 1.5; o[1] = Math.cos(a + s) * R + (r() - .5) * .02; o[2] = Math.sin(a + s) * R; return; }
    const k = Math.round((t + 1) * 13) / 13 - 1, q = r() * 2 - 1; o[0] = k * 1.5; o[1] = Math.cos(k * 7) * R * q; o[2] = Math.sin(k * 7) * R * q;
  },
  clock(r, o) {
    const p = r(), a = r() * TAU;
    if (p < .4) { const R = 1.05 + (r() - .5) * .04; o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = (r() - .5) * .06; return; }
    if (p < .62) { const h = Math.floor(r() * 12) / 12 * TAU, q = .84 + r() * .14; o[0] = Math.cos(h) * q; o[1] = Math.sin(h) * q; o[2] = 0; return; }
    if (p < .8) { const t = r() * .78; o[0] = t * Math.cos(.95); o[1] = t * Math.sin(.95); o[2] = .02; return; }
    if (p < .94) { const t = r() * .5; o[0] = t * Math.cos(3.7); o[1] = t * Math.sin(3.7); o[2] = .02; return; }
    const q = Math.sqrt(r()) * .06; o[0] = Math.cos(a) * q; o[1] = Math.sin(a) * q; o[2] = .03;
  },
  kolob(r, o) {
    if (r() < .56) { FORMS.sphere(r, o); o[0] *= .72; o[1] *= .72; o[2] *= .72; return; }
    const ring = Math.floor(r() * 3), a = r() * TAU, R = 1 + ring * .18 + (r() - .5) * .03, x = Math.cos(a) * R, z = Math.sin(a) * R; o[0] = x; o[1] = z * .28 + x * .12; o[2] = z;
  },
  // Tartary: forts, globes and the old domes.
  globe(r, o) {
    const p = r(), a = r() * TAU, t = r() * Math.PI;
    if (p < .4) { const m = Math.floor(r() * 12) / 12 * TAU; o[0] = Math.sin(t) * Math.cos(m); o[1] = Math.cos(t); o[2] = Math.sin(t) * Math.sin(m); return; }
    if (p < .8) { const lat = (Math.floor(r() * 9) / 8 - .5) * Math.PI * .9, q = Math.cos(lat); o[0] = Math.cos(a) * q; o[1] = Math.sin(lat); o[2] = Math.sin(a) * q; return; }
    const R = 1.22 + r() * .02; o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = 0;
  },
  dome(r, o) {
    const p = r(), a = r() * TAU;
    if (p < .42) { const t = r() * Math.PI / 2, q = Math.cos(t) * .62; o[0] = Math.cos(a) * q; o[1] = .08 + Math.sin(t) * .7; o[2] = Math.sin(a) * q; return; }
    if (p < .55) { o[0] = (r() - .5) * .02; o[1] = .78 + r() * .34; o[2] = (r() - .5) * .02; return; }
    if (p < .84) { const c = Math.floor(r() * 12) / 12 * TAU; o[0] = Math.cos(c) * .66 + (r() - .5) * .025; o[1] = -.52 + r() * .6; o[2] = Math.sin(c) * .66 + (r() - .5) * .025; return; }
    const q = .7 + r() * .44; o[0] = Math.cos(a) * q; o[1] = -.54 - Math.floor((q - .7) / .15) * .1; o[2] = Math.sin(a) * q;
  },
  compass(r, o) {
    const p = r(), a = r() * TAU;
    if (p < .3) { const R = 1.08 + (r() - .5) * .03; o[0] = Math.cos(a) * R; o[1] = Math.sin(a) * R; o[2] = 0; return; }
    polygon(r, o, starPoints(p < .7 ? 4 : 8, p < .7 ? 1 : .68, p < .7 ? .13 : .1, p < .7 ? -Math.PI / 2 : -Math.PI / 2 + Math.PI / 8), (r() - .5) * .04, .02);
  },
  // Andreas: the deep.
  whale(r, o) {
    const p = r();
    if (p < .78) {
      const t = r(), x = 1.5 - t * 3, body = Math.sin(Math.PI * t ** .62) * .46 + .02, a = r() * TAU, bend = Math.sin(t * 2.6) * .16;
      o[0] = x; o[1] = Math.sin(a) * body * (Math.sin(a) < 0 ? 1.12 : .88) + bend; o[2] = Math.cos(a) * body * .78; return;
    }
    if (p < .9) { const s = r() < .5 ? 1 : -1, t = r(), w = r(); o[0] = -1.45 - t * .38; o[1] = .29 + t * .16 + (w - .5) * .05; o[2] = s * t * .55; return; }
    const t = r(); o[0] = .55 - t * .44 + r() * .18 * (1 - t); o[1] = -.3 - t * .42; o[2] = .4 + t * .36;
  },
  jelly(r, o) {
    const p = r(), a = r() * TAU;
    if (p < .56) { const t = r() * Math.PI / 2, q = Math.cos(t) * .82 * (1 + Math.sin(a * 8) * .04); o[0] = Math.cos(a) * q; o[1] = .3 + Math.sin(t) * .62; o[2] = Math.sin(a) * q; return; }
    // Tentacles hang and sway on their own; they never cross into a net.
    const k = Math.floor(r() * 11), b = k / 11 * TAU, t = r(), R = .58 * (1 - t * .45), sway = Math.sin(t * 7 + k * 1.9) * .07 * t;
    o[0] = Math.cos(b) * R + sway + (r() - .5) * .012; o[1] = .28 - t * (1.15 + (k % 3) * .22); o[2] = Math.sin(b) * R + (r() - .5) * .012;
  },
  nautilus(r, o) {
    const t = r() ** .7 * 3.4, a = t * TAU * .86, R = .07 * Math.exp(t * .78), tube = R * .36, b = r() * TAU;
    o[0] = Math.cos(a) * (R + Math.cos(b) * tube) - .1; o[1] = Math.sin(a) * (R + Math.cos(b) * tube); o[2] = Math.sin(b) * tube;
  },
  trident(r, o) {
    const p = r(), head = (x, y, w, h) => { const t = r(), q = (1 - t) * w * (t < .5 ? t * 2 : 1); o[0] = x + (r() - .5) * q * 2; o[1] = y + t * h; o[2] = (r() - .5) * q; };
    if (p < .34) { box(r, o, 0, -.42, 0, .032, .84, .032); return; }
    if (p < .6) {
      // The two outer tines sweep out of the shaft in one curve.
      const s = r() < .5 ? 1 : -1, t = r(), c = t * Math.PI / 2; o[0] = s * .52 * Math.sin(c) + (r() - .5) * .03; o[1] = .36 + (1 - Math.cos(c)) * .5 + (r() - .5) * .03; o[2] = (r() - .5) * .03; return;
    }
    if (p < .72) { o[0] = (r() - .5) * .05; o[1] = .36 + r() * .62; o[2] = (r() - .5) * .03; return; }
    const k = r();
    if (k < .34) head(0, .92, .13, .32); else if (k < .67) head(.52, .84, .11, .27); else head(-.52, .84, .11, .27);
  },
};

export function sampleImage(url, N, test) {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement('canvas'); c.width = img.naturalWidth; c.height = img.naturalHeight;
      const g = c.getContext('2d', { willReadFrequently: true }); g.drawImage(img, 0, 0);
      const { data, width: w, height: h } = g.getImageData(0, 0, c.width, c.height);
      const r = random(1999), hits = [];
      let tries = 0, x0 = w, x1 = 0, y0 = h, y1 = 0;
      while (hits.length < N * 2 && tries++ < N * 90) {
        const x = Math.floor(r() * w), y = Math.floor(r() * h), i = (y * w + x) * 4;
        if (r() < test(data[i], data[i + 1], data[i + 2], data[i + 3])) {
          hits.push(x + r(), y + r());
          if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
        }
      }
      if (!hits.length) { resolve(null); return; }
      const out = new Float32Array(N * 3), span = Math.max(x1 - x0, y1 - y0), cx = (x0 + x1) / 2, cy = (y0 + y1) / 2, k = hits.length / 2;
      for (let n = 0; n < N; n++) {
        const m = Math.floor(r() * k);
        out[n * 3] = (hits[m * 2] - cx) / span * 2.5;
        out[n * 3 + 1] = -(hits[m * 2 + 1] - cy) / span * 2.5;
        out[n * 3 + 2] = (r() - .5) * .1;
      }
      resolve(out);
    };
    img.onerror = () => resolve(null);
    img.src = url;
  });
}

const VERT = `#version 300 es
  in vec3 aA; in vec3 aB; in vec4 aS;
  uniform float uT, uTime, uScatter, uIntro, uScale, uAspect, uPx, uWing, uRainbow;
  uniform vec2 uRot, uOff; uniform vec3 uC0, uC1, uC2;
  out vec3 vColor; out float vA;
  vec3 rotY(vec3 p, float a){ float c=cos(a), s=sin(a); return vec3(c*p.x+s*p.z,p.y,-s*p.x+c*p.z); }
  vec3 rotX(vec3 p, float a){ float c=cos(a), s=sin(a); return vec3(p.x,c*p.y-s*p.z,s*p.y+c*p.z); }
  vec3 hue(float h){ return clamp(abs(mod(h*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.); }
  void main(){
    float stagger=clamp((uT-aS.w*.25)/.75,0.,1.);
    float t=stagger*stagger*(3.-2.*stagger);
    vec3 p=mix(aA,aB,t);
    vec3 s=aS.xyz*2.-1.;
    p+=s*4.*t*(1.-t)*uScatter*(.35+aS.w*.8);
    p+=.014*vec3(sin(uTime*.7+aS.x*40.),cos(uTime*.6+aS.y*40.),sin(uTime*.5+aS.z*40.));
    float wing=smoothstep(-.1,.75,p.y-p.x*.32)*uWing*(1.-t);
    p.y+=wing*.075*sin(uTime*1.7-p.x*1.5);
    p.z+=wing*.13*cos(uTime*1.7-p.x*1.5);
    float e=1.-pow(1.-uIntro,3.);
    p=mix(s*vec3(3.4,2.4,2.),p,clamp(e*1.15-aS.w*.15,0.,1.));
    vec3 q=rotX(rotY(p*uScale,uRot.y),uRot.x);
    q.xy+=uOff;
    float d=3.6-q.z, f=2.75/max(d,.2);
    gl_Position=vec4(q.x*f/uAspect,q.y*f,0.,1.);
    gl_PointSize=(1.05+aS.x*1.5+(aS.w>.97?1.5:0.))*uPx*f*.56;
    float g=clamp((p.x+p.y*.45)*.33+.5,0.,1.);
    vColor=g<.5?mix(uC0,uC1,g*2.):mix(uC1,uC2,(g-.5)*2.);
    if(uRainbow>.5) vColor=mix(hue(g*.9+uTime*.05+aS.w*.06),vec3(1.),.08);
    if(aS.w>.97) vColor=mix(vColor,vec3(1.),.75);
    vA=clamp(e*1.3,0.,1.);
  }`;
const FRAG = `#version 300 es
  precision mediump float; in vec3 vColor; in float vA; uniform float uGain; out vec4 c;
  void main(){ float d=length(gl_PointCoord-.5); float a=smoothstep(.5,.2,d)*vA; c=vec4(vColor*a*uGain,a*.9); }`;

const rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);

// config: { palette: [a, b, c], rainbow, forms: {extra}, images: {name: {url, test}}, flat: [names], stops: [{sel, form, x, y, s}], gain, size }
export async function startFilm(config) {
  const canvas = document.createElement('canvas');
  canvas.id = 'film'; canvas.setAttribute('aria-hidden', 'true');
  document.body.append(canvas);
  const gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: true, antialias: false, powerPreference: 'high-performance' });
  if (!gl) { canvas.remove(); document.documentElement.classList.add('no-film'); return null; }
  const compile = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; };
  const prog = gl.createProgram();
  gl.attachShader(prog, compile(gl.VERTEX_SHADER, VERT)); gl.attachShader(prog, compile(gl.FRAGMENT_SHADER, FRAG));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) { canvas.remove(); document.documentElement.classList.add('no-film'); return null; }
  gl.useProgram(prog);
  gl.enable(gl.BLEND); gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

  const small = innerWidth < 760 || (navigator.hardwareConcurrency || 8) < 5;
  const N = small ? 32000 : (config.count || 76000);
  const seeds = new Float32Array(N * 4), rs = random(731);
  for (let i = 0; i < N * 4; i++) seeds[i] = rs();
  const loc = (n) => gl.getAttribLocation(prog, n);
  const aA = loc('aA'), aB = loc('aB'), aS = loc('aS');
  gl.enableVertexAttribArray(aA); gl.enableVertexAttribArray(aB); gl.enableVertexAttribArray(aS);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer()); gl.bufferData(gl.ARRAY_BUFFER, seeds, gl.STATIC_DRAW);
  gl.vertexAttribPointer(aS, 4, gl.FLOAT, false, 0, 0);

  const buffers = {}, places = { ...FORMS, ...(config.forms || {}) };
  const store = (name, data) => { const b = buffers[name] || gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, data, gl.DYNAMIC_DRAW); buffers[name] = b; };
  const build = (place, seed = 100) => { const r = random(seed), d = new Float32Array(N * 3), o = [0, 0, 0]; for (let i = 0; i < N; i++) { place(r, o); d[i * 3] = o[0]; d[i * 3 + 1] = o[1]; d[i * 3 + 2] = o[2]; } return d; };
  const needed = new Set(config.stops.map((s) => s.form));
  needed.forEach((name) => { if (places[name]) store(name, build(places[name], 100 + name.length)); });
  await Promise.all(Object.entries(config.images || {}).map(async ([name, im]) => {
    const data = await sampleImage(im.url, N, im.test);
    store(name, data || build(FORMS.sphere));
  }));

  const u = Object.fromEntries(['uT', 'uTime', 'uScatter', 'uIntro', 'uScale', 'uAspect', 'uPx', 'uWing', 'uRainbow', 'uRot', 'uOff', 'uC0', 'uC1', 'uC2', 'uGain'].map((n) => [n, gl.getUniformLocation(prog, n)]));
  const [c0, c1, c2] = (config.palette || ['#2190d4', '#7a4be0', '#e5278c']).map(rgb);
  gl.uniform3f(u.uC0, ...c0); gl.uniform3f(u.uC1, ...c1); gl.uniform3f(u.uC2, ...c2);
  gl.uniform1f(u.uGain, config.gain || 1.3); gl.uniform1f(u.uRainbow, config.rainbow ? 1 : 0);

  let W = 1, H = 1, px = 1, stops = [];
  const measure = () => {
    px = Math.min(1.6, devicePixelRatio || 1); W = innerWidth; H = innerHeight;
    canvas.width = Math.round(W * px); canvas.height = Math.round(H * px); gl.viewport(0, 0, canvas.width, canvas.height);
    stops = config.stops.flatMap((s, i) => {
      const el = s.el || document.querySelector(s.sel); if (!el) return [];
      const b = el.getBoundingClientRect(), y = i === 0 ? 0 : b.top + scrollY + b.height / 2 - H / 2;
      return [{ y, form: s.form, x: s.x || 0, my: s.y || 0, s: s.s || 1 }];
    }).sort((a, b) => a.y - b.y);
  };
  addEventListener('resize', measure); measure();
  document.fonts?.ready.then(measure); addEventListener('load', measure); setTimeout(measure, 1600); setTimeout(measure, 4000);

  let sy = scrollY, last = performance.now(), mx = 0, my = 0, tx = 0, ty = 0, velocity = 0;
  const born = reduced ? -1e5 : last;
  addEventListener('pointermove', (e) => { tx = e.clientX / W - .5; ty = e.clientY / H - .5; }, { passive: true });
  const flats = new Set([...(config.flat || []), ...Object.keys(config.images || {})]);
  const wings = new Set(config.wings || []);
  const size = config.size || 1;
  const covers = (config.covers || []).map((s) => document.querySelector(s)).filter(Boolean);

  function frame(now) {
    const dt = Math.min(.05, (now - last) / 1000); last = now;
    const before = sy; sy += (scrollY - sy) * (1 - Math.exp(-dt * (reduced ? 40 : 6.5)));
    velocity += ((sy - before) / Math.max(dt, .001) / 2200 - velocity) * .1;
    mx += (tx - mx) * .05; my += (ty - my) * .05;
    const hidden = covers.some((el) => { const b = el.getBoundingClientRect(); return b.top <= 0 && b.bottom >= H; });
    if (!hidden && stops.length) {
      let a = stops[0], b = stops[0], k = 0;
      for (let i = 0; i < stops.length - 1; i++) if (sy >= stops[i].y) { a = stops[i]; b = stops[i + 1]; k = Math.min(1, (sy - a.y) / Math.max(1, b.y - a.y)); }
      const t = clamp((k - .18) / .64), aspect = W / H, narrow = aspect < 1, ease = t * t * (3 - 2 * t), off = (p, q) => p + (q - p) * ease;
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffers[a.form]); gl.vertexAttribPointer(aA, 3, gl.FLOAT, false, 0, 0);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffers[b.form]); gl.vertexAttribPointer(aB, 3, gl.FLOAT, false, 0, 0);
      gl.uniform1f(u.uT, t); gl.uniform1f(u.uTime, reduced ? 0 : now / 1000); gl.uniform1f(u.uScatter, reduced ? 0 : 1.1);
      gl.uniform1f(u.uIntro, Math.min(1, (now - born) / 2600));
      gl.uniform1f(u.uScale, off(a.s, b.s) * (narrow ? .54 : .93)); gl.uniform1f(u.uAspect, aspect);
      gl.uniform1f(u.uPx, size * px * (narrow ? 1.25 : 1.7 * Math.min(1.5, Math.max(.9, W / 1440))));
      gl.uniform1f(u.uWing, wings.has(a.form) && !reduced ? 1 : 0);
      const flat = flats.has(a.form) ? 1 - ease : flats.has(b.form) ? ease : 0;
      const spin = reduced ? 0 : Math.sin(now / 3800 + sy * .0014) * .52 * (1 - flat);
      gl.uniform2f(u.uRot, my * .22 + velocity * .3 + .17 * (1 - flat), mx * .38 + spin + (flat ? Math.sin(now / 2400) * .07 * flat : 0));
      gl.uniform2f(u.uOff, narrow ? 0 : off(a.x, b.x), narrow ? off(a.my, b.my) * .5 + .38 : off(a.my, b.my));
      gl.drawArrays(gl.POINTS, 0, N);
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  return { measure, setForm(name, place, seed) { store(name, build(place, seed)); } };
}

// ---- Scenes: full-screen moments between a site's own sections.
export function addScenes(list) {
  return list.map((sc) => {
    const el = document.createElement('section');
    el.className = `scene ${sc.side === 'right' ? 'right' : 'left'} ${sc.className || ''}`;
    if (sc.id) el.id = sc.id;
    if (sc.giant) { const g = document.createElement('p'); g.className = 'scene-giant'; g.setAttribute('aria-hidden', 'true'); g.textContent = sc.giant; el.append(g); }
    const copy = document.createElement('div'); copy.className = 'scene-copy';
    if (sc.eyebrow) { const p = document.createElement('p'); p.className = 'scene-eyebrow'; p.textContent = sc.eyebrow; copy.append(p); }
    const p = document.createElement('p'); p.className = 'scene-line'; p.dataset.reveal = ''; p.textContent = sc.text; copy.append(p);
    if (sc.small) { const q = document.createElement('p'); q.className = 'scene-small'; q.textContent = sc.small; copy.append(q); }
    if (sc.href) { const a = document.createElement('a'); a.className = 'scene-link'; a.href = sc.href; a.textContent = sc.cta; copy.append(a); }
    el.append(copy);
    const anchor = document.querySelector(sc.before);
    anchor.parentElement.insertBefore(el, anchor);
    sc.el = el;
    return el;
  });
}

// ---- Words develop as they scroll past; giant words grow and fade.
export function scrollEffects() {
  const reveals = [...document.querySelectorAll('[data-reveal]')].map((el) => {
    const text = el.textContent.trim(), words = text.split(/\s+/);
    el.setAttribute('aria-label', text);
    el.replaceChildren(...words.flatMap((w) => { const s = document.createElement('span'); s.className = 'w'; s.textContent = w; s.setAttribute('aria-hidden', 'true'); return [s, ' ']; }));
    return { el, words: [...el.querySelectorAll('.w')] };
  });
  const giants = [...document.querySelectorAll('.scene-giant')];
  if (reduced) { reveals.forEach(({ words }) => words.forEach((w) => w.classList.add('lit'))); return; }
  (function tick() {
    const H = innerHeight;
    reveals.forEach(({ el, words }) => {
      const b = el.getBoundingClientRect();
      if (b.top > H * 1.1 || b.bottom < -H * .3) return;
      const narrow = innerWidth < 860, lit = Math.ceil(clamp(((narrow ? H * 1.04 : H * .9) - b.top) / (b.height + H * (narrow ? .18 : .34))) * words.length);
      words.forEach((w, i) => w.classList.toggle('lit', i < lit));
    });
    giants.forEach((g) => {
      const b = g.parentElement.getBoundingClientRect();
      if (b.top > H || b.bottom < 0) return;
      const p = (H / 2 - (b.top + b.height / 2)) / H;
      g.style.transform = `translateX(-50%) scale(${1 + Math.max(0, p) * .3})`;
      g.style.opacity = clamp(1.25 - Math.abs(p) * 1.7);
    });
    requestAnimationFrame(tick);
  })();
}

// ---- A soft ring that follows the pointer and says what a thing does.
export function viewfinder() {
  if (!matchMedia('(hover: hover) and (pointer: fine)').matches || reduced) return;
  const cursor = document.createElement('div'), label = document.createElement('span');
  cursor.className = 'finder'; cursor.setAttribute('aria-hidden', 'true'); cursor.append(label); document.body.append(cursor);
  let x = -99, y = -99, cx = -99, cy = -99;
  addEventListener('pointermove', (e) => {
    x = e.clientX; y = e.clientY; cursor.classList.add('on');
    const hit = e.target.closest?.('[data-cursor]');
    cursor.classList.toggle('big', !!hit);
    if (hit) label.textContent = hit.dataset.cursor;
  }, { passive: true });
  document.documentElement.addEventListener('pointerleave', () => cursor.classList.remove('on'));
  (function move() { cx += (x - cx) * .24; cy += (y - cy) * .24; cursor.style.transform = `translate(${cx}px, ${cy}px)`; requestAnimationFrame(move); })();
}

// ---- The reel: the newest videos around a cylinder, turned by the scrollbar.
export function addReel({ before, eyebrow, title, note, fallback, link }) {
  const section = document.createElement('section');
  section.className = 'film-reel';
  section.innerHTML = `<div class="film-reel-pin"><header><p class="scene-eyebrow"></p><h2></h2><p class="film-reel-note"></p></header><div class="film-stage"><div class="film-spin"></div></div><p class="film-reel-foot"><a class="scene-link" href="https://www.youtube.com/xirtus">Everything on YouTube</a></p></div>`;
  section.querySelector('.scene-eyebrow').textContent = eyebrow;
  section.querySelector('h2').textContent = title;
  section.querySelector('.film-reel-note').textContent = note;
  if (link) section.querySelector('.scene-link').href = link;
  const anchor = document.querySelector(before);
  anchor.parentElement.insertBefore(section, anchor);

  const dialog = document.createElement('dialog'); dialog.className = 'film-player'; dialog.setAttribute('aria-label', 'Video player');
  const close = document.createElement('button'); close.type = 'button'; close.textContent = 'Close';
  const frame = document.createElement('div'); dialog.append(close, frame); document.body.append(dialog);
  close.addEventListener('click', () => dialog.close()); dialog.addEventListener('close', () => frame.replaceChildren());
  const play = (id) => {
    const f = document.createElement('iframe'); f.src = `https://www.youtube-nocookie.com/embed/${encodeURIComponent(id)}?autoplay=1`;
    f.allow = 'autoplay; encrypted-media; picture-in-picture'; f.allowFullscreen = true; f.title = 'Video player';
    frame.replaceChildren(f); dialog.showModal();
  };

  const spin = section.querySelector('.film-spin');
  let cards = [], radius = 600;
  const place = () => { const n = cards.length || 1, w = spin.offsetWidth; radius = (w / 2) / Math.tan(Math.PI / n) * 1.12; cards.forEach((c, i) => { c.style.transform = `rotateY(${i * 360 / n}deg) translateZ(${radius}px)`; }); };
  const fill = (videos) => {
    cards = videos.slice(0, 15).map((v) => {
      const b = document.createElement('button'), img = document.createElement('img'), t = document.createElement('span');
      b.type = 'button'; b.className = 'film-card'; b.dataset.cursor = 'Play'; b.setAttribute('aria-label', `Play ${v.title}`);
      img.src = `https://i.ytimg.com/vi/${encodeURIComponent(v.id)}/hqdefault.jpg`; img.alt = ''; img.loading = 'lazy'; t.textContent = v.title;
      b.append(img, t); b.addEventListener('click', () => play(v.id));
      return b;
    });
    spin.replaceChildren(...cards); place();
  };
  addEventListener('resize', place);
  (async () => {
    let vids = [];
    try { const a = new AbortController(); setTimeout(() => a.abort(), 4000); const r = await fetch('https://data.narwallagram.com/network.json', { cache: 'no-cache', signal: a.signal }); if (r.ok) vids = (await r.json()).videos || []; } catch { /* use the built list */ }
    if (!vids.length && fallback) { try { vids = (await (await fetch(fallback)).json()).videos || []; } catch { /* no reel today */ } }
    if (vids.length) fill(vids); else section.hidden = true;
  })();
  let sy = scrollY;
  (function tick(now) {
    sy += (scrollY - sy) * .12;
    const b = section.getBoundingClientRect();
    if (b.top < innerHeight && b.bottom > 0) {
      const p = clamp(-b.top / Math.max(1, b.height - innerHeight));
      spin.style.transform = `translateZ(${-radius * .92}px) rotateX(-5deg) rotateY(${-((reduced ? 0 : (now || 0) * .0012) + p * 300)}deg)`;
    }
    requestAnimationFrame(tick);
  })();
  return section;
}
