import { startFilm, addScenes, addReel, scrollEffects, viewfinder } from "./film.js";
const SCENES = [{"before": "#buildlog", "giant": "SEXOS", "eyebrow": "Single Environment XIPC", "text": "One environment, one address space, on one machine or a whole cluster.", "className": "has-giant"}, {"before": "#concept-pku", "eyebrow": "Hardware keys", "text": "Hardware-enforced PKU isolation.", "side": "right"}, {"before": "#concept-pdx", "eyebrow": "Zero-copy PDX IPC", "text": "3.5\u00d7 faster. Lock-free. Wait-free."}, {"before": "#concept-rust", "eyebrow": "96\u00d7 smaller than Linux", "text": "Every line pure Rust. This is what every OS should have been.", "side": "right"}];

document.documentElement.classList.add("film-on");
const stops = [{"form": "chip", "x": 1.178, "y": 0.05, "s": 0.85, "sel": "#hero"}, {"form": "addressSpace", "x": 0.0, "y": 0, "s": 1.1, "scene": 0}, {"form": "key", "x": -1.054, "y": 0, "s": 1.15, "scene": 1}, {"form": "stream", "x": 0.93, "y": 0, "s": 1, "scene": 2}, {"form": "gear", "x": -1.116, "y": 0, "s": 1, "scene": 3}, {"form": "chip", "x": 1.116, "y": 0, "s": 0.9, "sel": "#videos"}, {"form": "letterX", "x": 0.0, "y": 0.1, "s": 0.6, "sel": "footer"}];
start();
function start() {
  addScenes(SCENES);
  
  stops.forEach((s) => { if (s.scene !== undefined) s.el = SCENES[s.scene].el; });
  scrollEffects(); viewfinder();
  startFilm({ palette: ["#00f5e4", "#9400ff", "#ff0066"], rainbow: false, gain: 1.3, size: 1, flat: [], wings: [],  stops });
}
