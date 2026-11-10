import { startFilm, addScenes, scrollEffects } from "./film.js";

document.documentElement.classList.add("film-on");
const scenes = [
  { before: "#arcade", eyebrow: "Since 1999", text: "If pigs could fly, video could travel down a phone line.", side: "right" },
  { before: "#archipelago", eyebrow: "Four living islands", text: "A sword, a stitch, and somewhere to go.", side: "right" },
  { before: "#tv", eyebrow: "Pigaloo TV", text: "The channel never switches off." },
  { before: "#story", eyebrow: "The story", text: "We were streaming in 1999.", side: "right" },
  { before: "#feed", eyebrow: "Feed the pig", text: "Truffles keep it flying." },
];
addScenes(scenes);
scrollEffects();

const TAU = Math.PI * 2;
const forms = {
  // A patchwork island over a stitched sea.
  island(r, o) {
    const a = r() * TAU, q = Math.sqrt(r()) * 1.25, h = q > 1.05 ? 0 : Math.cos(q / 1.05 * Math.PI / 2) ** 2;
    if (q > 1.05) { o[0] = Math.cos(a) * q * 1.2; o[1] = -.42; o[2] = Math.sin(a) * q * .8; return; }
    const x = Math.cos(a) * q, z = Math.sin(a) * q * .7, patch = (Math.floor(x * 5) + Math.floor(z * 5)) % 2 ? 0 : .012;
    o[0] = x; o[1] = -.42 + h * .72 + Math.sin(x * 6) * Math.cos(z * 7) * .06 * h + patch * 4; o[2] = z;
  },
  sword(r, o) {
    const p = r();
    if (p < .62) { const t = r(), w = (1 - t) * .075 + .01; o[0] = (r() - .5) * w * 2; o[1] = -.28 + t * 1.42; o[2] = (r() - .5) * .025; return; }
    if (p < .78) { o[0] = (r() - .5) * .78; o[1] = -.3 + Math.abs(o[0]) * .13 + (r() - .5) * .04; o[2] = (r() - .5) * .05; return; }
    if (p < .95) { const a = r() * TAU; o[0] = Math.cos(a) * .045; o[1] = -.34 - r() * .46; o[2] = Math.sin(a) * .045; return; }
    const a = r() * TAU, q = Math.sqrt(r()) * .085; o[0] = Math.cos(a) * q; o[1] = -.86 + Math.sin(a) * q; o[2] = 0;
  },
  television(r, o) {
    const p = r();
    if (p < .4) { const f = Math.floor(r() * 4), u = r() * 2 - 1, q = [[u * 1.1, .72], [u * 1.1, -.72], [1.1, u * .72], [-1.1, u * .72]][f]; o[0] = q[0]; o[1] = q[1] - .14; o[2] = (r() - .5) * .1; return; }
    if (p < .72) { o[0] = (r() * 2 - 1) * .82; o[1] = -.14 + Math.round((r() * 2 - 1) * 12) / 12 * .55; o[2] = 0; return; }
    if (p < .9) { const s = r() < .5 ? 1 : -1, t = r(); o[0] = s * t * .42; o[1] = .58 + t * .52; o[2] = 0; return; }
    const k = r() < .5 ? .93 : .93, a = r() * TAU, q = Math.sqrt(r()) * .06; o[0] = k + Math.cos(a) * q; o[1] = -.3 + (r() < .5 ? .2 : -.1) + Math.sin(a) * q; o[2] = .03;
  },
  truffle(r, o) {
    const y = r() * 2 - 1, a = r() * TAU, q = Math.sqrt(1 - y * y), bump = 1 + Math.sin(a * 5 + y * 4) * .09 + Math.sin(a * 11 - y * 7) * .05;
    o[0] = Math.cos(a) * q * bump * .9; o[1] = y * bump * .8; o[2] = Math.sin(a) * q * bump * .9;
  },
};
startFilm({
  palette: ["#ff9dc4", "#ed3c89", "#d6f354"], gain: 1.45, wings: ["pig"], forms,
  images: { pig: { url: "/film/pig-src.png", test: (r, g, b, a) => (a > 150 ? Math.min(1, Math.max(.1, (238 - (r + g + b) / 3) / 80)) : 0) } },
  covers: ["#arcade"],
  stops: [
    { sel: "#top", form: "pig", x: 1.2, y: .5, s: .9 },
    { el: scenes[0].el, form: "pig", x: -1.3, y: 0, s: 1 },
    { sel: "#arcade", form: "pig", x: 0, y: 0, s: 1 },
    { el: scenes[1].el, form: "island", x: -1.15, y: 0, s: 1.05 },
    { sel: "#archipelago", form: "sword", x: 1, y: 0, s: 1.1 },
    { el: scenes[2].el, form: "television", x: 1.05, y: 0, s: .95 },
    { el: scenes[3].el, form: "pig", x: -1.3, y: 0, s: 1 },
    { el: scenes[4].el, form: "truffle", x: 1, y: 0, s: 1 },
  ],
});
