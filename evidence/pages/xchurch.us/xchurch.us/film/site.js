import { startFilm, addScenes, addReel, scrollEffects, viewfinder } from "./film.js";
const SCENES = [{"before": "#one", "giant": "xChurch", "eyebrow": "The future has landed", "text": "xChurch isn't your average place of worship.", "className": "has-giant"}, {"before": "#two", "eyebrow": "xChurch believes in time travel", "text": "Time is the degradation of matter, matter is inside spirit.", "side": "right", "small": "Therefore God exists beyond space time."}, {"before": "#three", "eyebrow": "How to repair the timeline", "text": "The universe is evolving towards a state of increasing complexity and unity."}, {"before": "#four", "eyebrow": "The Omega Point", "text": "Christ is the goal towards which all creation is moving.", "side": "right"}, {"before": "#kolob", "eyebrow": "A world coming to the Metaspace", "text": "A place to walk, with a line back to gatherings in the real world."}];

document.documentElement.classList.add("film-on");
const stops = [{"form": "clock", "x": 0.0, "y": 0.1, "s": 0.95, "sel": "#banner"}, {"form": "hourglass", "x": 0.0, "y": 0, "s": 1.15, "scene": 0}, {"form": "clock", "x": -1.116, "y": 0, "s": 1.05, "scene": 1}, {"form": "helix", "x": 0.992, "y": 0, "s": 0.95, "scene": 2}, {"form": "starburst", "x": -1.116, "y": 0, "s": 1.05, "scene": 3}, {"form": "ring", "x": 0.0, "y": -0.05, "s": 1.2, "sel": ".film-reel"}, {"form": "kolob", "x": 1.054, "y": 0, "s": 1.05, "scene": 4}, {"form": "hourglass", "x": 0.0, "y": 0, "s": 0.9, "sel": "#five"}];
start();
function start() {
  addScenes(SCENES);
  addReel({"before": "#kolob", "eyebrow": "Talks from the timeline", "title": "Follow me. Through time.", "note": "New talks land here by themselves. Keep scrolling to spin it.", "fallback": "film/feed.json"});
  stops.forEach((s) => { if (s.scene !== undefined) s.el = SCENES[s.scene].el; });
  scrollEffects(); viewfinder();
  startFilm({ palette: ["#39c5d6", "#7f6be0", "#e44c65"], rainbow: false, gain: 1.3, size: 1, flat: [], wings: [],  stops });
}
