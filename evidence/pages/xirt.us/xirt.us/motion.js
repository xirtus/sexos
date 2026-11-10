// xirt.us: the motion from the xirtus.com design, in the plasma's colors.
(function () {
  var reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  var COLORS = ['#ff5ea8', '#40b8ff', '#ffd166', '#ff6a3d', '#7fe3b5', '#c792ff'];

  // ---- A pixel XIRTUS that scatters from the cursor, wired to every site.
  function network() {
    var canvas = document.getElementById('pixels'), g = canvas.getContext('2d');
    if (!g) return;
    var host = canvas.parentElement, w = 0, h = 0, dpr = 1, cell = 8, dots = [], wordBottom = 0;
    var pointer = { x: -1e4, y: -1e4 };
    function layout() {
      dpr = Math.min(devicePixelRatio || 1, 2);
      w = canvas.clientWidth; h = canvas.clientHeight;
      canvas.width = w * dpr; canvas.height = h * dpr;
      var small = w < 720;
      cell = small ? 5 : Math.max(7, Math.round(w / 150));
      var cols = Math.floor(w / cell), rows = Math.ceil(cols * .3);
      var off = document.createElement('canvas'); off.width = cols; off.height = rows;
      var o = off.getContext('2d', { willReadFrequently: true });
      o.fillStyle = '#fff'; o.textBaseline = 'middle'; o.textAlign = 'center';
      o.font = '700 ' + rows + 'px "Geist Serif", Georgia, serif';
      var size = Math.floor(rows * Math.min(1, cols / o.measureText('XIRTUS').width));
      o.font = '700 ' + size + 'px "Geist Serif", Georgia, serif';
      o.fillText('XIRTUS', cols / 2, rows / 2);
      var data = o.getImageData(0, 0, cols, rows).data;
      dots = [];
      for (var y = 0; y < rows; y++) for (var x = 0; x < cols; x++) {
        if (data[(y * cols + x) * 4 + 3] < 130) continue;
        var hx = x * cell, hy = y * cell;
        dots.push({ hx: hx, hy: hy, x: hx + (Math.random() - .5) * 90, y: hy + (Math.random() - .5) * 90, vx: 0, vy: 0, c: COLORS[Math.floor((x / cols) * COLORS.length * 1.5 + y * .05) % COLORS.length], s: cell - 1 });
      }
      wordBottom = dots.reduce(function (m, d) { return Math.max(m, d.hy); }, 0) + cell;
      if (reduced) { dots.forEach(function (d) { d.x = d.hx; d.y = d.hy; }); draw(0); }
    }
    function draw(t) {
      g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
      var box = canvas.getBoundingClientRect();
      var nodes = [].map.call(document.querySelectorAll('.netmap .stars li'), function (li) {
        var m = li.querySelector('a').getBoundingClientRect(), big = li.classList.contains('big');
        return { x: m.left - box.left + (big ? 11 : 7), y: m.top - box.top + m.height / 2, big: big };
      });
      var hub = nodes.filter(function (n) { return n.big; })[0];
      if (hub) {
        var links = nodes.filter(function (n) { return n !== hub; });
        links.push({ x: hub.x, y: wordBottom });
        links.forEach(function (n, i) {
          g.strokeStyle = 'rgba(255,255,255,.6)'; g.lineWidth = 1.5;
          g.beginPath(); g.moveTo(hub.x, hub.y); g.lineTo(n.x, n.y); g.stroke();
          if (!reduced) {
            var f = (t / 2400 + i * .17) % 1, s = Math.max(5, cell * .7);
            g.fillStyle = COLORS[i % COLORS.length];
            g.fillRect(hub.x + (n.x - hub.x) * f - s / 2, hub.y + (n.y - hub.y) * f - s / 2, s, s);
          }
        });
      }
      dots.forEach(function (d) {
        var dx = d.x - pointer.x, dy = d.y - pointer.y, r2 = dx * dx + dy * dy;
        if (r2 < 22000) { var f = (22000 - r2) / 22000 * 2.4, r = Math.sqrt(r2) || 1; d.vx += dx / r * f; d.vy += dy / r * f; }
        var wave = reduced ? 0 : Math.sin(t * .0016 + d.hx * .012) * 3;
        d.vx += (d.hx - d.x) * .045; d.vy += (d.hy + wave - d.y) * .045;
        d.vx *= .82; d.vy *= .82; d.x += d.vx; d.y += d.vy;
        g.fillStyle = d.c; g.fillRect(d.x, d.y, d.s, d.s);
      });
    }
    host.addEventListener('pointermove', function (e) { var b = canvas.getBoundingClientRect(); pointer.x = e.clientX - b.left; pointer.y = e.clientY - b.top; });
    host.addEventListener('pointerleave', function () { pointer.x = pointer.y = -1e4; });
    addEventListener('resize', layout);
    (document.fonts && document.fonts.ready || Promise.resolve()).then(layout);
    if (!reduced) {
      var visible = false;
      new IntersectionObserver(function (es) { visible = es[0].isIntersecting; }).observe(canvas);
      (function loop(t) { if (visible && !document.hidden) draw(t || 0); requestAnimationFrame(loop); })();
    }
  }

  // ---- Panels slide into place as they arrive.
  function arrive() {
    if (reduced || !('IntersectionObserver' in window)) return;
    var io = new IntersectionObserver(function (es) { es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }); }, { rootMargin: '0px 0px -6% 0px' });
    [].forEach.call(document.querySelectorAll('.band .glass, .band .way, .band .fund, .games li'), function (el, i) {
      el.classList.add('pop'); el.style.transitionDelay = (i % 5) * 45 + 'ms'; io.observe(el);
    });
  }

  // ---- Number keys and a launcher, as on xirtus.com. The terminal keeps its own typing.
  function keys() {
    var ORDER = ['#network', '#today', '#work', '#universe', '#arcade', '#support', '#contact'];
    var TARGETS = [['The Network', '#network'], ['Today at Xirtus', '#today'], ['The Work', '#work'], ['The Xirtus Universe', '#universe'], ['The Arcade', '#arcade'], ['Support the Xirtus', '#support'], ['Get in Touch', '#contact'], ['Enter the Metaspace', 'https://metaspace.xirtus.com'], ['xirtus.com', 'https://xirtus.com'], ['SexOS', 'https://sexos.dev'], ['3JSE', 'https://www.3jse.com'], ['Pigaloo', 'https://pigaloo.com'], ['andreas.me', 'https://andreas.me'], ['xChurch', 'https://xchurch.us'], ['Narwallagram', 'https://narwallagram.com'], ['Tartary Nova', 'https://tartarynova.com'], ['Merch', 'https://www.teepublic.com/xirtus'], ['YouTube', 'https://youtube.com/@andreasxirtus'], ['X', 'https://x.com/andreasxirtus'], ['GitHub', 'https://github.com/xirtus']];
    var dialog = document.getElementById('launcher'), q = document.getElementById('launch-q'), list = document.getElementById('launch-list'), at = 0;
    function render() {
      var term = q.value.trim().toLowerCase();
      var hits = TARGETS.filter(function (t) { return t[0].toLowerCase().indexOf(term) > -1; });
      at = Math.min(at, Math.max(0, hits.length - 1));
      list.replaceChildren.apply(list, hits.map(function (t, i) {
        var li = document.createElement('li'), a = document.createElement('a');
        a.href = t[1]; a.textContent = t[0]; a.className = i === at ? 'on' : '';
        a.addEventListener('click', function () { dialog.close(); });
        li.appendChild(a); return li;
      }));
    }
    function open() { q.value = ''; at = 0; render(); dialog.showModal(); q.focus(); }
    document.getElementById('open-launcher').addEventListener('click', open);
    q.addEventListener('input', function () { at = 0; render(); });
    q.addEventListener('keydown', function (e) {
      var n = list.children.length;
      if (e.key === 'ArrowDown' && n) { e.preventDefault(); at = (at + 1) % n; render(); }
      if (e.key === 'ArrowUp' && n) { e.preventDefault(); at = (at - 1 + n) % n; render(); }
      if (e.key === 'Enter') { e.preventDefault(); var a = list.querySelector('a.on'); if (a) a.click(); }
    });
    addEventListener('keydown', function (e) {
      if (e.target.matches('input, textarea') || document.querySelector('dialog[open]')) return;
      if (e.key === '/' || (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey))) { e.preventDefault(); open(); return; }
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      var n = Number(e.key);
      if (n >= 1 && n <= ORDER.length) { e.preventDefault(); document.querySelector(ORDER[n - 1]).scrollIntoView({ behavior: reduced ? 'auto' : 'smooth' }); }
    });
  }

  network();
  arrive();
  keys();
})();
