// Motion and daily change for xirtus.com. The page reads fine if any of this fails.
const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
const saved = (k, v) => { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch { return null; } };
const dayNumber = Math.floor(Date.now() / 864e5);
const COLORS = ['#6fc3df', '#8d82c4', '#ec8d81', '#e7b788', '#8ea9e8', '#87c5a4'];

// ---- Color: changes daily, and a visitor can pick their own.
function colorway() {
  const root = document.documentElement;
  const pick = saved('xirtus.accent');
  let i = pick === null || pick === '' ? dayNumber % COLORS.length : Number(pick);
  root.dataset.accent = i;
  const next = () => { i = (i + 1) % COLORS.length; root.dataset.accent = i; saved('xirtus.accent', String(i)); };
  document.getElementById('themes').addEventListener('click', next);
  return next;
}

// ---- The hero: a field of colored pixels spells XIRTUS and scatters from the cursor.
function pixels() {
  const canvas = document.getElementById('pixels');
  const g = canvas.getContext('2d');
  if (!g) return;
  const hero = canvas.parentElement;
  let w = 0, h = 0, dpr = 1, cell = 12, dots = [], ambient = [], wordBottom = 0;
  const pointer = { x: -1e4, y: -1e4 };

  function layout() {
    dpr = Math.min(devicePixelRatio || 1, 2);
    w = canvas.clientWidth; h = canvas.clientHeight;
    canvas.width = w * dpr; canvas.height = h * dpr;
    const small = w < 720;
    cell = small ? 5 : Math.max(7, Math.round(w / 150));
    // Draw the word off-screen once, then turn each lit cell into a pixel.
    const gutter = Math.max(12, Math.min(40, w * .03));
    const wordW = small ? w - 2 * gutter : Math.min(w - 2 * gutter, h * 1.5);
    const off = document.createElement('canvas');
    const cols = Math.floor(wordW / cell), rows = Math.ceil(cols * .3);
    off.width = cols; off.height = rows;
    const o = off.getContext('2d', { willReadFrequently: true });
    o.fillStyle = '#fff'; o.textBaseline = 'middle'; o.textAlign = 'center';
    let size = rows;
    o.font = `900 ${size}px Lato, Helvetica, sans-serif`;
    size = Math.floor(size * Math.min(1, cols / o.measureText('XIRTUS').width));
    o.font = `900 ${size}px Lato, Helvetica, sans-serif`;
    o.fillText('XIRTUS', cols / 2, rows / 2);
    const data = o.getImageData(0, 0, cols, rows).data;
    const ox = gutter;
    const oy = small ? 52 : Math.max(40, h * .05);
    dots = [];
    for (let y = 0; y < rows; y++) {
      for (let x = 0; x < cols; x++) {
        if (data[(y * cols + x) * 4 + 3] < 130) continue;
        const hx = ox + x * cell, hy = oy + y * cell;
        dots.push({ hx, hy, x: Math.random() * w, y: Math.random() * h, vx: 0, vy: 0, c: COLORS[Math.floor((x / cols) * COLORS.length * 1.5 + y * .05) % COLORS.length], s: cell - 1 });
      }
    }
    wordBottom = dots.reduce((m, d) => Math.max(m, d.hy), 0) + cell;
    ambient = Array.from({ length: small ? 70 : 170 }, (_, i) => ({ x: ((i * .618034) % 1) * w, y: ((i * .754877) % 1) * h, s: 2 + ((i * 3) % 5), c: COLORS[i % COLORS.length], p: i * 1.7 }));
    if (reduced) { dots.forEach((d) => { d.x = d.hx; d.y = d.hy; }); draw(0); }
  }

  function draw(t) {
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);
    const snap = Math.round;
    for (const a of ambient) {
      g.globalAlpha = .1 + .22 * (.5 + .5 * Math.sin(t * .0009 + a.p));
      g.fillStyle = a.c;
      g.fillRect(snap(a.x / cell) * cell, snap((a.y + t * .006 * (a.s - 1)) % h / cell) * cell, a.s, a.s);
    }
    g.globalAlpha = 1;
    // Every site is wired to the Metaspace, and the Metaspace to the word.
    const box = canvas.getBoundingClientRect();
    const nodes = [...document.querySelectorAll('.stars li')].map((li) => {
      const m = li.querySelector('a').getBoundingClientRect();
      const big = li.classList.contains('big');
      return { x: m.left - box.left + (big ? 11 : 7), y: m.top - box.top + m.height / 2, big };
    });
    const hub = nodes.find((n) => n.big);
    if (hub) {
      const links = nodes.filter((n) => n !== hub);
      if (wordBottom) links.push({ x: hub.x, y: wordBottom });
      links.forEach((n, i) => {
        g.strokeStyle = 'rgba(255,255,255,.36)'; g.lineWidth = 1.5;
        g.beginPath(); g.moveTo(hub.x, hub.y); g.lineTo(n.x, n.y); g.stroke();
        if (!reduced) {
          const f = (t / 2400 + i * .17) % 1, s = Math.max(5, cell * .7);
          g.fillStyle = COLORS[i % COLORS.length];
          g.fillRect(hub.x + (n.x - hub.x) * f - s / 2, hub.y + (n.y - hub.y) * f - s / 2, s, s);
        }
      });
    }
    for (const d of dots) {
      const dx = d.x - pointer.x, dy = d.y - pointer.y, r2 = dx * dx + dy * dy;
      if (r2 < 22000) { const f = (22000 - r2) / 22000 * 2.4; const r = Math.sqrt(r2) || 1; d.vx += dx / r * f; d.vy += dy / r * f; }
      // A slow wave passes through the letters so they are never still.
      const wave = reduced ? 0 : Math.sin(t * .0016 + d.hx * .012) * 3;
      d.vx += (d.hx - d.x) * .045; d.vy += (d.hy + wave - d.y) * .045;
      d.vx *= .82; d.vy *= .82;
      d.x += d.vx; d.y += d.vy;
      g.fillStyle = d.c;
      g.fillRect(d.x, d.y, d.s, d.s);
    }
  }
  hero.addEventListener('pointermove', (e) => { const b = canvas.getBoundingClientRect(); pointer.x = e.clientX - b.left; pointer.y = e.clientY - b.top; });
  hero.addEventListener('pointerleave', () => { pointer.x = pointer.y = -1e4; });
  addEventListener('resize', layout);
  (document.fonts?.ready ?? Promise.resolve()).then(layout);
  if (!reduced) {
    let visible = true;
    new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(canvas);
    const loop = (t) => { if (visible && !document.hidden) draw(t); requestAnimationFrame(loop); };
    requestAnimationFrame(loop);
  }
}

// ---- Windows slide into place as they arrive.
function arrive() {
  const items = document.querySelectorAll('.win, .games li, .tools li');
  if (reduced || !('IntersectionObserver' in window)) return;
  const io = new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }), { rootMargin: '0px 0px -6% 0px' });
  items.forEach((el, i) => { el.classList.add('pop'); el.style.transitionDelay = `${(i % 5) * 45}ms`; io.observe(el); });
}

// ---- The bar: current section, number keys, a clock.
function bar(nextColor) {
  const links = [...document.querySelectorAll('.spaces a')];
  const io = new IntersectionObserver((es) => es.forEach((e) => {
    if (e.isIntersecting) links.forEach((a) => a.classList.toggle('on', a.getAttribute('href') === `#${e.target.id}`));
  }), { rootMargin: '-45% 0px -50% 0px' });
  links.forEach((a) => { const s = document.querySelector(a.getAttribute('href')); if (s) io.observe(s); });
  const clock = document.getElementById('clock');
  const tick = () => { clock.textContent = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); };
  tick(); setInterval(tick, 20000);
  addEventListener('keydown', (e) => {
    if (e.target.matches('input, textarea') || e.metaKey || e.ctrlKey || e.altKey || document.querySelector('dialog[open]')) return;
    const a = links.find((l) => l.dataset.key === e.key);
    if (a) { e.preventDefault(); document.querySelector(a.getAttribute('href')).scrollIntoView({ behavior: reduced ? 'auto' : 'smooth' }); }
    if (e.key === 't' || e.key === 'T') nextColor();
  });
}

// ---- The launcher: go anywhere from the keyboard.
function launcher() {
  const TARGETS = [
    ['Today at Xirtus', '#today'], ['Synctank episodes', '/synctank/'], ['Articles · Synctank', '/tags/articles/'], ['The Work', '#work'], ['The Xirtus Universe', '#universe'], ['The Arcade', '#arcade'], ['Get in Touch', '#contact'], ['Donate', '#support'],
    ['Enter the Metaspace', 'https://metaspace.xirtus.com'], ['SexOS', 'https://sexos.dev'], ['3JSE', 'https://www.3jse.com'], ['Pigaloo', 'https://pigaloo.com'], ['xirt.us', 'https://xirt.us'], ['andreas.me', 'https://andreas.me'], ['xChurch', 'https://xchurch.us'], ['Narwallagram', 'https://narwallagram.com'], ['Tartary Nova', 'https://tartarynova.com'], ['Merch', 'https://www.teepublic.com/xirtus'], ['YouTube', 'https://youtube.com/@andreasxirtus'], ['X', 'https://x.com/andreasxirtus'], ['GitHub', 'https://github.com/xirtus'],
  ];
  const dialog = document.getElementById('launcher');
  const q = document.getElementById('launch-q');
  const list = document.getElementById('launch-list');
  let at = 0;
  const render = () => {
    const term = q.value.trim().toLowerCase();
    const hits = TARGETS.filter(([name]) => name.toLowerCase().includes(term));
    at = Math.min(at, Math.max(0, hits.length - 1));
    list.replaceChildren(...hits.map(([name, href], i) => {
      const li = document.createElement('li'), a = document.createElement('a');
      a.href = href; a.textContent = name; a.className = i === at ? 'on' : '';
      a.addEventListener('click', () => dialog.close());
      li.append(a); return li;
    }));
  };
  const open = () => { q.value = ''; at = 0; render(); dialog.showModal(); q.focus(); };
  document.getElementById('open-launcher').addEventListener('click', open);
  q.addEventListener('input', () => { at = 0; render(); });
  q.addEventListener('keydown', (e) => {
    const n = list.children.length;
    if (e.key === 'ArrowDown' && n) { e.preventDefault(); at = (at + 1) % n; render(); }
    if (e.key === 'ArrowUp' && n) { e.preventDefault(); at = (at - 1 + n) % n; render(); }
    if (e.key === 'Enter') { e.preventDefault(); list.querySelector('a.on')?.click(); }
  });
  addEventListener('keydown', (e) => {
    if (e.target.matches('input, textarea') || document.querySelector('dialog[open]')) return;
    if (e.key === '/' || (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey))) { e.preventDefault(); open(); }
  });
}

// ---- Return visits and daily picks.
function freshness() {
  const last = Number(saved('xirtus.lastVisit')) || 0;
  if (last) {
    let fresh = 0;
    document.querySelectorAll('[data-date]').forEach((el) => {
      if (Date.parse(el.dataset.date) > last) { fresh++; (el.querySelector('.msg') || el.parentElement.querySelector('.vtitle'))?.classList.add('new'); }
    });
    const days = Math.floor((Date.now() - last) / 864e5);
    const when = days === 0 ? 'earlier today' : days === 1 ? 'yesterday' : `${days} days ago`;
    document.getElementById('since').textContent = fresh ? `Welcome back. ${fresh} new since ${when}.` : `Welcome back. You last came ${when}.`;
  }
  saved('xirtus.lastVisit', String(Date.now()));
  const games = window.__games;
  if (games?.length) {
    const g = games[dayNumber % games.length];
    document.getElementById('daily').href = `https://github.com/xirtus/${g.repo}`;
    document.getElementById('daily-img').src = `/img/${g.img}.webp`;
    document.getElementById('daily-name').textContent = g.name;
    document.getElementById('daily-blurb').textContent = g.blurb;
  }
  const signals = ['love is only one syllable', 'i is an imaginary number', 'time travel is real', 'this star is my orchard of thoughts', 'solvents are sometimes vile', 'drink mineral water', 'fnord'];
  document.getElementById('signal').textContent = signals[dayNumber % signals.length];
}

async function latestPost() {
  try {
    const res = await fetch('https://data.narwallagram.com/latest-tweet.json', { cache: 'no-cache' });
    if (!res.ok) return;
    const post = await res.json();
    if (!post.text) return;
    document.getElementById('xpost').textContent = post.text.replace(/\s*https:\/\/t\.co\/\S+\s*$/, '');
    if (/^https:\/\/x\.com\//.test(post.url)) document.getElementById('xlink').href = post.url;
  } catch { /* fallback stays */ }
}

function extras() {
  const dialog = document.getElementById('player');
  const frame = document.getElementById('player-frame');
  document.querySelectorAll('[data-video]').forEach((b) => b.addEventListener('click', () => {
    const iframe = document.createElement('iframe');
    iframe.src = `https://www.youtube-nocookie.com/embed/${encodeURIComponent(b.dataset.video)}?autoplay=1`;
    iframe.allow = 'autoplay; encrypted-media; picture-in-picture';
    iframe.allowFullscreen = true;
    iframe.title = 'Video player';
    frame.replaceChildren(iframe);
    dialog.showModal();
  }));
  document.getElementById('player-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => frame.replaceChildren());
  const btc = document.getElementById('btc');
  btc.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(btc.dataset.address); btc.textContent = 'Copied. Thank you.'; }
    catch { btc.textContent = btc.dataset.address; }
  });
}

// ---- New work and videos appear without a rebuild. The built page is the fallback.
async function liveFeed() {
  try {
    const res = await fetch('https://data.narwallagram.com/network.json', { cache: 'no-cache' });
    if (!res.ok) return;
    const feed = await res.json();
    const short = (iso) => new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
    const list = document.querySelector('.commits ol');
    if (list && feed.commits?.length) {
      list.replaceChildren(...feed.commits.slice(0, 6).map((c) => {
        const li = document.createElement('li'), a = document.createElement('a'), b = document.createElement('b'), m = document.createElement('span'), t = document.createElement('time');
        li.dataset.date = c.date; a.href = c.url; b.textContent = c.repo; m.className = 'msg'; m.textContent = c.message; t.dateTime = c.date; t.textContent = short(c.date);
        a.append(b, ' ', m); li.append(a, t); return li;
      }));
    }
    const v = feed.videos?.[0], thumb = document.querySelector('.video .thumb');
    if (v && thumb) {
      thumb.dataset.video = v.id; thumb.dataset.date = v.published; thumb.setAttribute('aria-label', `Play ${v.title}`);
      thumb.querySelector('img').src = `https://i.ytimg.com/vi/${encodeURIComponent(v.id)}/hqdefault.jpg`;
      document.querySelector('.vtitle').textContent = v.title;
    }
  } catch { /* the built page stays */ }
}

const nextColor = colorway();
pixels();
arrive();
bar(nextColor);
launcher();
liveFeed().finally(freshness);
latestPost();
extras();
