import { startFilm, addScenes, addReel, scrollEffects, viewfinder } from "./film.js";
const SCENES = [{"before": "#network", "eyebrow": "this star is my orchard of thoughts", "text": "Kindly feel welcomed to the place of Xirtus."}, {"before": "#work", "eyebrow": "Today at Xirtus", "text": "Light seven stars. Then keep going.", "side": "right"}, {"before": "#arcade", "eyebrow": "The arcade", "text": "Original games. A new pick every day."}, {"before": "#support", "eyebrow": "Support the Xirtus", "text": "Thank you for collaborating with us.", "side": "right"}];

document.documentElement.classList.add("film-on");
const stops = [{"form": "starburst", "x": 1.178, "y": 0.05, "s": 0.85, "sel": "body"}, {"form": "starburst", "x": 1.054, "y": 0, "s": 1.15, "scene": 0}, {"form": "terminal", "x": -1.054, "y": 0, "s": 1, "scene": 1}, {"form": "ring", "x": 0.0, "y": -0.05, "s": 1.2, "sel": ".film-reel"}, {"form": "gamepad", "x": 1.054, "y": 0, "s": 1.1, "scene": 2}, {"form": "heart", "x": -1.116, "y": 0, "s": 0.95, "scene": 3}, {"form": "letterX", "x": 1.054, "y": 0, "s": 0.75, "sel": "#contact"}];
start();
function start() {
  addScenes(SCENES);
  addReel({"before": "#universe", "eyebrow": "Latest", "title": "On the reel.", "note": "New videos land here by themselves. Keep scrolling to spin it.", "fallback": "film/feed.json"});
  stops.forEach((s) => { if (s.scene !== undefined) s.el = SCENES[s.scene].el; });
  scrollEffects(); viewfinder();
  startFilm({ palette: ["#ff0066", "#00f5e4", "#ffd700"], rainbow: true, gain: 1.7, size: 1.25, flat: ["letterX"], wings: [],  stops });
}
