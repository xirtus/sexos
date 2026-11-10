import { startFilm, addScenes, addReel, scrollEffects, viewfinder } from "./film.js";
const SCENES = [{"before": "#today", "giant": "Xirtus", "eyebrow": "this star is my orchard of thoughts", "text": "Engines, operating systems, games and a connected world, always changing.", "className": "has-giant"}, {"before": "#work", "eyebrow": "The work", "text": "Foundations you can use, read and walk through.", "side": "right"}, {"before": "#universe", "eyebrow": "The Xirtus universe", "text": "Everything that started it, still here."}, {"before": "#arcade", "eyebrow": "The arcade", "text": "Original games. A new pick every day.", "side": "right", "small": "Now playable right in your browser on Pigaloo.", "href": "https://pigaloo.com", "cta": "Play on Pigaloo"}, {"before": "#metaspace", "eyebrow": "The Metaspace", "text": "Walk between the projects instead of clicking between them."}, {"before": "#support", "eyebrow": "Support the Xirtus", "text": "Thank you for collaborating with us.", "side": "right", "small": "We share all our information, and with your help we have more information to share."}];

document.documentElement.classList.add("film-on");
const stops = [{"form": "network", "x": 1.178, "y": 0.05, "s": 0.92, "sel": ".hero"}, {"form": "network", "x": 0.0, "y": -0.02, "s": 1.05, "scene": 0}, {"form": "ring", "x": 0.0, "y": -0.05, "s": 1.2, "sel": ".film-reel"}, {"form": "cube", "x": -1.178, "y": 0, "s": 1, "scene": 1}, {"form": "pig", "x": 1.141, "y": 0, "s": 0.92, "scene": 2}, {"form": "gamepad", "x": -1.116, "y": 0, "s": 1.1, "scene": 3}, {"form": "city", "x": 0.992, "y": -0.05, "s": 1.05, "scene": 4}, {"form": "heart", "x": -1.178, "y": 0, "s": 0.95, "scene": 5}, {"form": "letterX", "x": 1.116, "y": 0, "s": 0.7, "sel": "#contact"}];
start();
function start() {
  addScenes(SCENES);
  addReel({"before": "#work", "eyebrow": "Today at Xirtus", "title": "On the reel.", "note": "New videos land here by themselves. Keep scrolling to spin it.", "fallback": "/film/feed.json", "articles": "/synctank-index.json"});
  stops.forEach((s) => { if (s.scene !== undefined) s.el = SCENES[s.scene].el; });
  scrollEffects(); viewfinder();
  startFilm({ palette: ["#6fc3df", "#a79bea", "#f09c90"], rainbow: false, gain: 1.5, size: 1, flat: ["letterX"], wings: ["pig"], images: {pig: { url: "/film/pig-src.png", test: (r, g, b, a) => (a > 150 ? Math.min(1, Math.max(.1, (238 - (r + g + b) / 3) / 80)) : 0) }}, stops });
}
