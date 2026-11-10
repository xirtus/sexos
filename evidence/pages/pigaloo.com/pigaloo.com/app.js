// Pigaloo: a daily flight, an arcade that remembers you, a TV channel on a shared schedule, and live feeds.
const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
const dayNumber = Math.floor(Date.now() / 864e5);
const saved = (k, v) => { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch { return null; } };
const shortDate = (iso) => new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
const games = window.__games;
let videos = window.__videos || [];

// ---- When Pigs Fly: one sky a day, the same for everyone.
function seeded(seed) {
  return () => { seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
const RANKS = [[0, 'Grounded'], [10, 'Piglet'], [50, 'Hopeful Hog'], [150, 'Barnstormer'], [400, 'Sky Swine'], [1000, 'Aviator of the Sty'], [2500, 'Legend of 1999']];
const stats = { best: 0, bestDay: -1, streak: 0, lastDay: -1, truffles: 0 };
try { Object.assign(stats, JSON.parse(saved('pigaloo.sky') || '{}')); } catch { /* start fresh */ }
if (stats.bestDay !== dayNumber) stats.best = 0;
function paintStats() {
  document.getElementById('s-best').textContent = stats.best.toLocaleString();
  document.getElementById('s-streak').textContent = stats.lastDay >= dayNumber - 1 ? stats.streak : 0;
  document.getElementById('s-truffles').textContent = stats.truffles.toLocaleString();
  document.getElementById('s-rank').textContent = `Rank: ${[...RANKS].reverse().find(([n]) => stats.truffles >= n)[1]}`;
}
paintStats();

function sky() {
  const hero = document.getElementById('top'), canvas = document.getElementById('sky'), ctx = canvas.getContext('2d'), pig = document.getElementById('pig');
  const hud = document.getElementById('hud'), result = document.getElementById('result');
  let W = 0, H = 0, S = 1;
  const resize = () => { const r = hero.getBoundingClientRect(); S = Math.min(2, devicePixelRatio || 1); W = r.width; H = r.height; canvas.width = W * S; canvas.height = H * S; };
  addEventListener('resize', resize); resize();

  const scenery = seeded(19990101);
  const clouds = Array.from({ length: 9 }, () => ({ x: scenery(), y: scenery() * .62, r: 26 + scenery() * 54, v: .25 + scenery() * .5 }));
  const isles = Array.from({ length: 6 }, (_, i) => ({ x: i * 340 + scenery() * 120, w: 190 + scenery() * 210, h: 34 + scenery() * 58, c: i }));
  const QUILT = ['#ed3c89', '#d6f354', '#1b6f78', '#eee3c7', '#a81d5e', '#f7a13a'];

  let state = 'idle', t = 0, last = performance.now(), scroll = 0, py = 0, vy = 0, hold = false, dist = 0, truf = 0;
  let items = [], rnd = seeded(dayNumber), nextAt = 0, tx = .68, ty = .32;

  function start() {
    state = 'flying'; hero.classList.add('flying'); hud.hidden = false;
    rnd = seeded(dayNumber); items = []; nextAt = W * .9; py = H * .42; vy = 0; dist = 0; truf = 0; scroll = 0; hold = false;
    document.getElementById('hud-best').textContent = stats.best;
    hero.scrollIntoView({ block: 'start' });
  }
  function finish() {
    state = 'idle'; hero.classList.remove('flying'); hud.hidden = true; hold = false;
    const score = Math.floor(dist) + truf * 10, fresh = score > stats.best;
    if (fresh) { stats.best = score; stats.bestDay = dayNumber; }
    if (stats.lastDay !== dayNumber) { stats.streak = stats.lastDay === dayNumber - 1 ? stats.streak + 1 : 1; stats.lastDay = dayNumber; }
    stats.truffles += truf; saved('pigaloo.sky', JSON.stringify(stats)); paintStats();
    result.textContent = `${fresh ? 'New best! ' : ''}${score.toLocaleString()} points, ${truf} truffle${truf === 1 ? '' : 's'}.`;
    const share = document.createElement('button');
    share.type = 'button'; share.textContent = 'Copy score';
    share.addEventListener('click', async () => { try { await navigator.clipboard.writeText(`When Pigs Fly, sky ${dayNumber}: ${score} points. https://pigaloo.com`); share.textContent = 'Copied'; } catch { share.textContent = 'Could not copy'; } });
    result.append(share);
    document.getElementById('fly').textContent = 'Fly again';
  }
  function spawn() {
    const y = H * (.1 + rnd() * .68);
    if (rnd() < .46) items.push({ k: 'storm', x: W + 110, y, r: 42 + rnd() * 36 + Math.min(32, dist / 90) });
    else { const n = 1 + Math.floor(rnd() * 3); for (let i = 0; i < n; i++) items.push({ k: 'truffle', x: W + 90 + i * 58, y: y + Math.sin(i) * 26, r: 15 }); }
    nextAt = 190 + rnd() * 170 - Math.min(90, dist / 45);
  }

  function drawCloud(x, y, r, color) {
    ctx.fillStyle = color;
    [[-.8, .15, .62], [-.25, -.2, .82], [.42, -.05, .72], [.95, .22, .5], [0, .28, .78]].forEach(([a, b, c]) => { ctx.beginPath(); ctx.arc(x + a * r, y + b * r, c * r, 0, 7); ctx.fill(); });
  }
  function drawIsle(x, base, w, h, c) {
    ctx.save();
    ctx.beginPath(); ctx.moveTo(x, base); ctx.quadraticCurveTo(x + w * .3, base - h * 1.5, x + w * .5, base - h); ctx.quadraticCurveTo(x + w * .78, base - h * .7, x + w, base); ctx.closePath(); ctx.clip();
    const size = 26;
    for (let i = 0; i * size < w; i++) for (let j = 0; j * size < h * 1.5; j++) { ctx.fillStyle = QUILT[(i * 3 + j * 2 + c) % QUILT.length]; ctx.fillRect(x + i * size, base - h * 1.5 + j * size, size - 1, size - 1); }
    ctx.restore();
  }
  function frame(now) {
    const dt = Math.min(.05, (now - last) / 1000); last = now; t += dt;
    const flying = state === 'flying', speed = flying ? 250 + Math.min(230, dist / 8) : reduced ? 0 : 22;
    scroll += speed * dt;

    if (hero.getBoundingClientRect().bottom > 0 || flying) {
      ctx.setTransform(S, 0, 0, S, 0, 0); ctx.clearRect(0, 0, W, H);
      // A low sun over the patchwork sea.
      const sx = W * .77, sy = H * .68;
      const glow = ctx.createRadialGradient(sx, sy, 8, sx, sy, Math.max(W, H) * .42);
      glow.addColorStop(0, '#fff3c4d9'); glow.addColorStop(1, '#fff3c400');
      ctx.fillStyle = glow; ctx.fillRect(0, 0, W, H);
      clouds.forEach((c) => { const span = W + 320, x = ((c.x * span - scroll * c.v * .35) % span + span) % span - 160; drawCloud(x, c.y * H, c.r, '#ffffffa6'); });
      const seaY = H * .86;
      const period = 2040;
      isles.forEach((s) => { const x = ((s.x - scroll * .55) % period + period) % period - 300; if (x < W + 20) drawIsle(x, seaY + 2, s.w, s.h, s.c); });
      const sea = ctx.createLinearGradient(0, seaY, 0, H); sea.addColorStop(0, '#1b8f95'); sea.addColorStop(1, '#0e4d63');
      ctx.fillStyle = sea; ctx.fillRect(0, seaY, W, H - seaY);
      ctx.strokeStyle = '#ffffff4d'; ctx.lineWidth = 1.5; ctx.setLineDash([9, 8]); ctx.lineDashOffset = scroll * .9;
      [.02, .06, .1].forEach((f) => { ctx.beginPath(); ctx.moveTo(0, seaY + H * f); ctx.lineTo(W, seaY + H * f); ctx.stroke(); });
      ctx.setLineDash([]);

      if (flying) {
        vy += (hold ? -1120 : 720) * dt; vy = Math.max(-430, Math.min(520, vy)); py += vy * dt; dist += speed * dt / 12;
        const pw = pig.offsetWidth, ph = pw * .76, px = W * .2;
        if (py < ph * .3) { py = ph * .3; vy = Math.max(0, vy); }
        nextAt -= speed * dt; if (nextAt <= 0) spawn();
        const cx = px + pw * .05, cy = py, hit = pw * .28;
        items = items.filter((o) => {
          o.x -= speed * dt;
          const near = Math.hypot(o.x - cx, o.y - cy);
          if (o.k === 'truffle') {
            if (near < o.r + hit) { truf++; return false; }
            ctx.fillStyle = '#4a2b1b'; ctx.beginPath(); ctx.arc(o.x, o.y, o.r, 0, 7); ctx.fill();
            ctx.fillStyle = '#d6f354'; ctx.beginPath(); ctx.arc(o.x - 4, o.y - 5, 3.5, 0, 7); ctx.arc(o.x + 5, o.y + 3, 2.5, 0, 7); ctx.fill();
            ctx.strokeStyle = '#fff6dd'; ctx.lineWidth = 2; ctx.beginPath(); ctx.arc(o.x, o.y, o.r + 3 + Math.sin(t * 7) * 1.5, 0, 7); ctx.stroke();
          } else {
            drawCloud(o.x, o.y, o.r, '#33233a');
            if (near < o.r * .95 + hit * .7) finish();
            ctx.strokeStyle = '#d6f354'; ctx.lineWidth = 3; ctx.beginPath(); ctx.moveTo(o.x + 4, o.y + o.r * .55); ctx.lineTo(o.x - 8, o.y + o.r * 1.05); ctx.lineTo(o.x + 5, o.y + o.r * 1.05); ctx.lineTo(o.x - 6, o.y + o.r * 1.55); ctx.stroke();
          }
          return o.x > -160;
        });
        if (py + ph * .28 > seaY) finish();
        if (state === 'flying') {
          pig.style.transform = `translate(${px - pw / 2}px, ${py - ph / 2}px) rotate(${Math.max(-18, Math.min(22, vy / 22))}deg)`;
          document.getElementById('hud-dist').textContent = Math.floor(dist); document.getElementById('hud-truf').textContent = truf;
        }
      } else {
        // At rest the pig hovers on the right and leans toward the pointer.
        const pw = pig.offsetWidth, ph = pw * .76, narrow = W < 1040;
        const hx = narrow ? W * .5 : W * (.72 + (tx - .5) * .05), hy = (narrow ? Math.min(H * .24, pw * .48) : H * (.33 + (ty - .5) * .07)) + (reduced ? 0 : Math.sin(t * 1.5) * 12);
        pig.style.transform = `translate(${hx - pw / 2}px, ${hy - ph / 2}px) rotate(${reduced ? 0 : Math.sin(t * .8) * 2.5 - 2}deg)`;
      }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  document.getElementById('fly').addEventListener('click', (e) => { e.stopPropagation(); start(); });
  hero.addEventListener('pointerdown', () => { if (state === 'flying') hold = true; });
  addEventListener('pointerup', () => { hold = false; });
  addEventListener('pointercancel', () => { hold = false; });
  hero.addEventListener('pointermove', (e) => { tx = e.clientX / W; ty = e.clientY / H; });
  addEventListener('keydown', (e) => {
    if (state !== 'flying') return;
    if (e.code === 'Space' || e.code === 'ArrowUp') { e.preventDefault(); hold = true; }
    if (e.code === 'Escape') finish();
  });
  addEventListener('keyup', (e) => { if (e.code === 'Space' || e.code === 'ArrowUp') hold = false; });
  return () => state === 'flying';
}
const isFlying = sky();

// ---- The arcade: plays in place, remembers what you have tried.
const items = [...document.querySelectorAll('.game')];
const buttons = [...document.querySelectorAll('[data-play]')];
let played = new Set();
try { played = new Set(JSON.parse(saved('pigaloo.played') || '[]')); } catch { /* start fresh */ }
function paintPlayed() {
  items.forEach((li) => li.classList.toggle('done', played.has(li.dataset.slug)));
  const n = [...played].filter((s) => games.some((g) => g.slug === s)).length;
  if (n) document.getElementById('played-note').textContent = n === games.length ? `You have played all ${n}. You may be a pig.` : `You have tried ${n} of ${games.length}. ${games.length - n} to go.`;
}
const dialog = document.getElementById('player'), frameHost = document.getElementById('p-frame');
function play(slug) {
  const g = games.find((x) => x.slug === slug);
  if (!g) return;
  const iframe = document.createElement('iframe');
  iframe.src = `/play/${g.slug}/`; iframe.title = g.name; iframe.allow = 'autoplay; fullscreen; gamepad; pointer-lock; xr-spatial-tracking; accelerometer; gyroscope';
  document.getElementById('p-title').textContent = g.name;
  document.getElementById('p-open').href = `/play/${g.slug}/`;
  document.getElementById('p-src').href = `https://github.com/xirtus/${g.repo}`;
  frameHost.replaceChildren(iframe); dialog.showModal(); iframe.focus();
  played.add(slug); saved('pigaloo.played', JSON.stringify([...played])); paintPlayed();
}
buttons.forEach((b) => b.addEventListener('click', () => play(b.dataset.play)));
document.getElementById('p-close').addEventListener('click', () => dialog.close());
dialog.addEventListener('close', () => frameHost.replaceChildren());

const today = games[dayNumber % games.length], todayLink = document.getElementById('today');
document.getElementById('today-img').src = `/img/${today.img}.webp`;
document.getElementById('today-name').textContent = today.name;
document.getElementById('today-blurb').textContent = today.blurb;
todayLink.href = `/play/${today.slug}/`;
todayLink.addEventListener('click', (e) => { e.preventDefault(); play(today.slug); });

let kind = 'All';
function filter() {
  const q = document.getElementById('q').value.trim().toLowerCase();
  let shown = 0;
  items.forEach((li) => { const ok = (kind === 'All' || li.dataset.kind === kind) && li.dataset.name.includes(q); li.hidden = !ok; if (ok) shown++; });
  document.getElementById('none').hidden = shown > 0;
}
document.getElementById('q').addEventListener('input', filter);
document.querySelectorAll('.chips button').forEach((b) => b.addEventListener('click', () => {
  document.querySelectorAll('.chips button').forEach((x) => x.classList.toggle('on', x === b));
  kind = b.dataset.kind; filter();
}));
document.getElementById('surprise').addEventListener('click', () => {
  const fresh = games.filter((g) => !played.has(g.slug)), pool = fresh.length ? fresh : games;
  play(pool[Math.floor(Math.random() * pool.length)].slug);
});
paintPlayed();

// ---- The four islands
const tabs = [...document.querySelectorAll('[data-isle]')];
tabs.forEach((t, i) => t.addEventListener('click', () => {
  tabs.forEach((x) => x.setAttribute('aria-selected', String(x === t)));
  document.querySelectorAll('[data-isle-panel]').forEach((p) => { p.hidden = p.dataset.islePanel !== String(i); });
}));

// ---- Pigaloo TV: half-hour slots counted from the clock, so everyone is on the same programme.
const SLOT = 30 * 60 * 1000;
let channel = 0, watching = false;
const screen = document.getElementById('screen'), tvPlay = document.getElementById('tv-play');
const pick = (slotOffset) => videos[(((Math.floor(Date.now() / SLOT) + channel + slotOffset) % videos.length) + videos.length) % videos.length];
function paintTv() {
  if (!videos.length) return;
  const now = pick(0);
  document.getElementById('tv-img').src = `https://i.ytimg.com/vi/${encodeURIComponent(now.id)}/hqdefault.jpg`;
  document.getElementById('tv-title').textContent = now.title;
  document.getElementById('tv-bar').style.width = `${(Date.now() % SLOT) / SLOT * 100}%`;
  document.getElementById('ch').textContent = `Ch ${channel + 1}`;
  const start = Math.floor(Date.now() / SLOT) * SLOT;
  const list = document.getElementById('guide');
  list.replaceChildren(...[1, 2, 3, 4].map((n) => {
    const v = pick(n), li = document.createElement('li'), img = document.createElement('img'), box = document.createElement('div'), time = document.createElement('time'), b = document.createElement('b');
    img.src = `https://i.ytimg.com/vi/${encodeURIComponent(v.id)}/default.jpg`; img.alt = ''; img.loading = 'lazy';
    time.textContent = new Date(start + n * SLOT).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }); b.textContent = v.title;
    box.append(time, b); li.append(img, box); return li;
  }));
}
function watch() {
  const iframe = document.createElement('iframe');
  iframe.src = `https://www.youtube-nocookie.com/embed/${encodeURIComponent(pick(0).id)}?autoplay=1`;
  iframe.allow = 'autoplay; encrypted-media; picture-in-picture'; iframe.allowFullscreen = true; iframe.title = 'Pigaloo TV';
  screen.replaceChildren(iframe); screen.classList.add('live'); watching = true;
}
function zap(d) {
  channel = (channel + d + 99) % 99; paintTv();
  if (watching) watch();
}
tvPlay.addEventListener('click', watch);
document.getElementById('ch-up').addEventListener('click', () => zap(1));
document.getElementById('ch-down').addEventListener('click', () => zap(-1));
setInterval(() => { if (!watching) paintTv(); else document.getElementById('tv-bar').style.width = `${(Date.now() % SLOT) / SLOT * 100}%`; }, 30000);
paintTv();

// ---- Live feed for the channel, the X post and the bench. A slow or failed feed leaves the built page.
function ago(iso) {
  const h = Math.floor((Date.now() - Date.parse(iso)) / 36e5);
  return h < 1 ? 'in the last hour' : h < 24 ? `${h} hour${h === 1 ? '' : 's'} ago` : `${Math.floor(h / 24)} day${h < 48 ? '' : 's'} ago`;
}
(async () => {
  try {
    const timer = new AbortController();
    setTimeout(() => timer.abort(), 4000);
    const res = await fetch('https://data.narwallagram.com/network.json', { cache: 'no-cache', signal: timer.signal });
    if (!res.ok) return;
    const data = await res.json();
    if (data.videos?.length) { videos = data.videos; if (!watching) paintTv(); }
    if (data.post?.text) {
      document.getElementById('xpost').textContent = data.post.text.replace(/\s*https:\/\/t\.co\/\S+\s*$/, '');
      if (/^https:\/\/x\.com\//.test(data.post.url)) document.getElementById('xlink').href = data.post.url;
    }
    if (data.commits?.length) {
      document.getElementById('last-commit').textContent = `Last change to the code: ${ago(data.commits[0].date)}.`;
      document.getElementById('commits').replaceChildren(...data.commits.slice(0, 6).map((c) => {
        const li = document.createElement('li'), repo = document.createElement('span'), time = document.createElement('time'), a = document.createElement('a');
        repo.textContent = c.repo; time.textContent = shortDate(c.date); a.textContent = c.message; a.title = c.message;
        if (/^https:\/\/github\.com\//.test(c.url)) a.href = c.url;
        li.append(repo, time, a); return li;
      }));
    }
    const engine = data.repos?.['3JSE'];
    if (engine) document.getElementById('engine-meta').textContent = `${engine.stars} stars · ${engine.license} · updated ${shortDate(engine.pushed)}`;
  } catch { document.getElementById('commits').closest('section').querySelector('#commits').replaceChildren(); }
})();

// ---- Dock, keys and small motion
const dockLinks = [...document.querySelectorAll('.dock a[href^="#"]')].filter((a) => a.getAttribute('href') !== '#top');
const spy = new IntersectionObserver((es) => es.forEach((e) => {
  if (e.isIntersecting) dockLinks.forEach((a) => a.classList.toggle('on', a.getAttribute('href') === `#${e.target.id}`));
}), { rootMargin: '-45% 0px -50% 0px' });
['top', 'arcade', 'archipelago', 'tv', 'story', 'studio', 'feed'].forEach((id) => spy.observe(document.getElementById(id)));
addEventListener('keydown', (e) => {
  if (isFlying() || dialog.open || e.metaKey || e.ctrlKey || e.altKey || /INPUT|TEXTAREA/.test(document.activeElement?.tagName)) return;
  const n = Number(e.key);
  if (n >= 1 && n <= dockLinks.length) dockLinks[n - 1].click();
});

const btc = document.getElementById('btc');
btc.addEventListener('click', async () => {
  try { await navigator.clipboard.writeText(btc.dataset.address); btc.textContent = 'Copied. Thank you.'; }
  catch { btc.textContent = 'Select the address above'; }
});

if (!reduced) {
  const bg = document.querySelector('.arch-bg'), arch = document.getElementById('archipelago');
  const parallax = () => { const r = arch.getBoundingClientRect(); if (r.bottom > 0 && r.top < innerHeight) bg.style.transform = `translateY(${-r.top * .08}px)`; };
  addEventListener('scroll', parallax, { passive: true }); parallax();
  const io = new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }), { rootMargin: '0px 0px -6% 0px' });
  document.querySelectorAll('.head, .game, .stu, .time li, .strip li, .way').forEach((el, i) => { el.classList.add('pop'); el.style.transitionDelay = `${(i % 4) * 50}ms`; io.observe(el); });
}
