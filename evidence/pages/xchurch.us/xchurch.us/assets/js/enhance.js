// xChurch enhancement layer. Adds motion and daily change on top of the original page.
(function () {
  var reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  var saved = function (k, v) { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch (e) { return null; } };
  var day = Math.floor(Date.now() / 864e5);

  // Daily reflection, in the words already on this page.
  var lines = [
    "The universe is evolving towards a state of increasing complexity and unity.",
    "We should be aware of the presence of God in all of creation.",
    "We should be open to the new and the unexpected.",
    "We are called to collaborate with God in the work of creation and evolution.",
    "Every action taken within the timeline has far-reaching consequences.",
    "We should work to build a more united and fraternal world.",
    "Christ is the Omega Point of evolution, the goal towards which all creation is moving."
  ];
  document.getElementById("reflection").textContent = "Today: " + lines[day % lines.length];

  // Hero art changes on its own, starting from a different piece each day.
  var art = [].slice.call(document.querySelectorAll(".orbit img"));
  var at = day % art.length;
  art[at].classList.add("on");
  if (!reduced) setInterval(function () {
    art[at].classList.remove("on");
    at = (at + 1) % art.length;
    art[at].classList.add("on");
  }, 6000);

  // Stars drift toward a point behind the art.
  var c = document.getElementById("stars"), g = c.getContext("2d");
  var stars = [];
  for (var i = 0; i < 220; i++) stars.push({ a: i * 2.39996, r: .08 + ((i * .618034) % 1) * .95, s: .5 + ((i * .37) % 1) * 1.5 });
  var W, H, D;
  function size() { D = Math.min(devicePixelRatio || 1, 2); W = c.clientWidth; H = c.clientHeight; c.width = W * D; c.height = H * D; }
  function stars_draw(t) {
    g.setTransform(D, 0, 0, D, 0, 0); g.clearRect(0, 0, W, H);
    var cx = W * .62, cy = H * .48, R = Math.max(W, H) * .72;
    stars.forEach(function (d) {
      var r = reduced ? d.r : (d.r - t * .000012 * d.s + 2) % 1;
      var a = d.a + t * .00002;
      g.globalAlpha = Math.min(1, r * 2.2) * .85;
      g.fillStyle = d.s > 1.5 ? "#e44c65" : "#fff";
      g.beginPath(); g.arc(cx + Math.cos(a) * R * r, cy + Math.sin(a) * R * r, d.s * (.35 + r * .8), 0, 7); g.fill();
    });
    g.globalAlpha = 1;
    if (!reduced) requestAnimationFrame(stars_draw);
  }
  addEventListener("resize", size); size(); requestAnimationFrame(stars_draw);

  // Page rail and scene focus: the page in view is bright, the others wait.
  var pages = [["banner", "The future has landed"], ["one", "Believe"], ["two", "The Timeline"], ["three", "Repair"], ["four", "The Temporal Path"], ["kolob", "Kolob"], ["five", "Join"]];
  var rail = document.createElement("nav"); rail.id = "rail"; rail.setAttribute("aria-label", "Pages");
  pages.forEach(function (p) {
    var a = document.createElement("a"); a.href = "#" + p[0]; a.setAttribute("aria-label", p[1]);
    var s = document.createElement("span"); s.textContent = p[1]; a.appendChild(s); rail.appendChild(a); p.push(a);
    a.addEventListener("click", function (e) { e.preventDefault(); document.getElementById(p[0]).scrollIntoView({ behavior: reduced ? "auto" : "smooth" }); });
  });
  document.body.appendChild(rail);
  var seen = new IntersectionObserver(function (es) {
    es.forEach(function (e) {
      e.target.classList.toggle("scene-off", !e.isIntersecting && e.target.classList.contains("spotlight"));
      if (e.isIntersecting) pages.forEach(function (p) { p[2].classList.toggle("on", p[0] === e.target.id); });
    });
  }, { rootMargin: "-45% 0px -45% 0px" });
  pages.forEach(function (p) { var el = document.getElementById(p[0]); if (el) seen.observe(el); });

  // The six practices can be walked; progress returns with the visitor.
  var walked; try { walked = JSON.parse(saved("xchurch.walked") || "[]"); } catch (e) { walked = []; }
  var steps = [].slice.call(document.querySelectorAll("#four section.col-4"));
  var note = document.createElement("p"); note.id = "walk";
  if (steps.length) steps[0].parentNode.parentNode.appendChild(note);
  function paint() {
    steps.forEach(function (s, n) { s.classList.toggle("walked", walked.indexOf(n) > -1); s.setAttribute("aria-pressed", walked.indexOf(n) > -1); });
    note.textContent = walked.length === steps.length ? "You have walked the whole path. Kolob is next." : walked.length ? "You have walked " + walked.length + " of " + steps.length + " practices. Your path is saved." : "Select a practice to mark it walked. Your path is saved for your return.";
  }
  steps.forEach(function (s, n) {
    s.tabIndex = 0; s.setAttribute("role", "button");
    var toggle = function () { var i = walked.indexOf(n); if (i > -1) walked.splice(i, 1); else walked.push(n); saved("xchurch.walked", JSON.stringify(walked)); paint(); };
    s.addEventListener("click", toggle);
    s.addEventListener("keydown", function (e) { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggle(); } });
  });
  paint();

  // Latest related talks, from a feed file rebuilt on the Mac.
  fetch("https://data.narwallagram.com/network.json", { cache: "no-cache" }).then(function (r) { if (!r.ok) throw 0; return r.json(); }).catch(function () { return fetch("feed.json").then(function (r) { return r.json(); }); }).then(function (feed) {
    var talks = feed.videos.filter(function (v) { return /mormon|church|god|bible|christ|latter|ancient|prophet|faith/i.test(v.title); }).slice(0, 3);
    if (!talks.length) return;
    var list = document.getElementById("talk-list");
    talks.forEach(function (v) {
      var a = document.createElement("a"); a.href = "https://www.youtube.com/watch?v=" + encodeURIComponent(v.id);
      var img = document.createElement("img"); img.src = "https://i.ytimg.com/vi/" + encodeURIComponent(v.id) + "/hqdefault.jpg"; img.alt = ""; img.loading = "lazy";
      var t = document.createElement("span"); t.textContent = v.title;
      a.appendChild(img); a.appendChild(t); list.appendChild(a);
    });
    document.getElementById("talks").hidden = false;
  }).catch(function () {});
})();
