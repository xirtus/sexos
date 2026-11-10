// xirt.us: a terminal game living inside the original plasma. Solving a clue lights a star and stirs the plasma there.
(function () {
  var reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  var saved = function (k, v) { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch (e) { return null; } };

  // Answers are lightly obscured so "view source" is a puzzle of its own, not a spoiler.
  var CLUES = [
    { q: 'A number that does not exist, yet has its own letter.', a: 'aQ==' },
    { q: 'Five letters you were trained not to see.', a: 'Zm5vcmQ=' },
    { q: 'Violence never solves anything, but solvents are sometimes ____.', a: 'dmlsZQ==' },
    { q: 'How many syllables is love?', a: 'b25l' },
    { q: 'This star is my ______ of thoughts.', a: 'b3JjaGFyZA==' },
    { q: 'Teilhard de ______ waits to be read.', a: 'Y2hhcmRpbg==' },
    { q: 'Behind every door, one world. Name it.', a: 'bWV0YXNwYWNl' }
  ];
  var NOTES = {
    'notes.txt': '* i is an imaginary number\n* fnord\n* violence never solves anything but solvents are sometimes vile\n* love is only one syllable.',
    'welcome.txt': 'kindly feel welcomed to the place of xirtus\nthis star is my orchard of thoughts',
    'choice.txt': 'You may choose whether to read philosophy from Teilhard Chardin now,\nor pursue other clues and interests.'
  };
  var DOORS = { xirtus: 'https://xirtus.com', andreas: 'https://andreas.me', xchurch: 'https://xchurch.us', sexos: 'https://sexos.dev', '3jse': 'https://www.3jse.com', pigaloo: 'https://pigaloo.com', metaspace: 'https://metaspace.xirtus.com' };
  var out = document.getElementById('out'), input = document.getElementById('cmd');
  var solved = Math.min(CLUES.length, Number(saved('xirt.solved')) || 0);

  function print(text, cls) {
    var line = document.createElement('div');
    if (cls) line.className = cls;
    line.textContent = text;
    out.appendChild(line);
    out.scrollTop = out.scrollHeight;
  }
  function printLink(label, href) {
    var line = document.createElement('div'), a = document.createElement('a');
    a.href = href; a.textContent = label;
    line.appendChild(a); out.appendChild(line); out.scrollTop = out.scrollHeight;
  }
  function score() { document.getElementById('score').textContent = solved + '/' + CLUES.length + ' stars lit'; }

  // ---- Stirring: drive the plasma's own pointer handler, so nothing about the effect changes.
  var realMove = null, lastReal = 0, stirring = 0;
  function feed(x, y) { if (realMove) realMove({ pageX: x, pageY: y, clientX: x, clientY: y }); }
  function burst(cx, cy) {
    if (reduced) return;
    var n = 0; stirring = Date.now() + 1400;
    var id = setInterval(function () {
      n++; var r = 20 + n * 3.2, a = n * .55;
      feed(cx + Math.cos(a) * r, cy + Math.sin(a) * r);
      if (n > 44) clearInterval(id);
    }, 30);
  }
  function starXY(i) {
    var box = document.querySelector('.orchard').getBoundingClientRect();
    var a = -Math.PI / 2 + i * (2 * Math.PI / CLUES.length);
    return [box.left + box.width / 2 + Math.cos(a) * box.width * .47, box.top + box.height / 2 + Math.sin(a) * box.height * .47];
  }

  function signal() {
    print('tuning...', 'sys');
    fetch('https://data.narwallagram.com/latest-tweet.json', { cache: 'no-cache' }).then(function (r) { return r.json(); }).then(function (p) {
      print('[x] ' + p.text.replace(/\s*https:\/\/t\.co\/\S+\s*$/, ''));
    }).catch(function () { print('[x] no carrier. try x.com/andreasxirtus', 'warn'); });
    fetch('https://data.narwallagram.com/network.json', { cache: 'no-cache' }).then(function (r) { if (!r.ok) throw 0; return r.json(); }).catch(function () { return fetch('feed.json').then(function (r) { return r.json(); }); }).then(function (f) {
      (f.videos || []).slice(0, 3).forEach(function (v) { printLink('[tube] ' + v.title, 'https://www.youtube.com/watch?v=' + encodeURIComponent(v.id)); });
    }).catch(function () {});
  }

  var COMMANDS = {
    help: function () { print('help      this list\nls        files in the orchard\ncat F     read a file\nclue      your current clue\nanswer X  answer the clue\nsignal    latest transmissions\ndonate    support the xirtus\nstir      disturb the plasma\ndoors     exits to other worlds\nopen D    go through a door\nwhoami    who are you\nreset     forget everything\nclear     clean the screen'); },
    ls: function () { print(Object.keys(NOTES).join('   ')); },
    cat: function (a) { if (Object.hasOwn(NOTES, a)) print(NOTES[a]); else print('cat: ' + (a || '(nothing)') + ': no such thought', 'warn'); },
    clue: function () {
      if (solved >= CLUES.length) { print('All stars are lit. The orchard is yours.', 'win'); return; }
      print('clue ' + (solved + 1) + '/' + CLUES.length + ': ' + CLUES[solved].q);
      print('type: answer <word>', 'sys');
    },
    answer: function (a) {
      if (solved >= CLUES.length) { print('Nothing left to answer. Yet.', 'sys'); return; }
      if ((a || '').toLowerCase() === atob(CLUES[solved].a)) {
        var at = starXY(solved);
        solved++; saved('xirt.solved', String(solved)); score();
        burst(at[0], at[1]);
        print('correct. star ' + solved + ' lit.', 'win');
        if (solved === CLUES.length) { print('the orchard is complete. a door opens:', 'win'); printLink('metaspace.xirtus.com', DOORS.metaspace); }
        else COMMANDS.clue();
      } else print('no. the star stays dark.', 'warn');
    },
    signal: signal,
    donate: function () { print('thank you. opening the support panels...', 'win'); document.getElementById('support').scrollIntoView({ behavior: reduced ? 'auto' : 'smooth' }); },
    stir: function () { burst(innerWidth * .62, innerHeight * .5); print('the plasma remembers.', 'sys'); },
    doors: function () { print(Object.keys(DOORS).join('   ')); print('type: open <door>', 'sys'); },
    open: function (a) {
      if (!Object.hasOwn(DOORS, a)) { print('open: ' + (a || '(nothing)') + ': no such door', 'warn'); return; }
      print('opening ' + a + '...', 'sys'); location.href = DOORS[a];
    },
    whoami: function () { print('guest, carrying ' + solved + ' of ' + CLUES.length + ' stars'); },
    fnord: function () { print('you did not see this line.', 'sys'); },
    sudo: function () { print('nice try. violence never solves anything.', 'warn'); },
    reset: function () { solved = 0; saved('xirt.solved', '0'); score(); print('forgotten.', 'sys'); },
    clear: function () { out.replaceChildren(); }
  };
  function run(raw) {
    var line = raw.trim(); if (!line) return;
    print('guest@orchard:~$ ' + line, 'you');
    var parts = line.split(/\s+/), cmd = parts.shift().toLowerCase();
    if (Object.hasOwn(COMMANDS, cmd)) COMMANDS[cmd](parts.join(' '));
    else print(cmd + ': not found. try help', 'warn');
  }
  document.getElementById('form').addEventListener('submit', function (e) { e.preventDefault(); run(input.value); input.value = ''; });
  document.querySelectorAll('[data-run]').forEach(function (b) { b.addEventListener('click', function () { run(b.dataset.run); input.focus({ preventScroll: true }); }); });
  // Double-click clears the words, as it always has, except while using the terminal.
  document.querySelector('.term').addEventListener('dblclick', function (e) { e.stopPropagation(); });

  // ---- The seven stars ring the planet, and connect as they are lit.
  function sky() {
    var c = document.getElementById('sky'), g = c.getContext('2d'), w, h, dpr;
    function size() { dpr = Math.min(devicePixelRatio || 1, 2); w = innerWidth; h = innerHeight; c.width = w * dpr; c.height = h * dpr; if (reduced) draw(0); }
    function draw(t) {
      g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
      if (document.getElementById('desc').style.visibility === 'hidden') { if (!reduced) requestAnimationFrame(draw); return; }
      var pts = CLUES.map(function (_, i) { return starXY(i); });
      g.strokeStyle = '#ffe66daa'; g.lineWidth = 1.5; g.beginPath();
      pts.slice(0, solved).forEach(function (p, i) { if (i) g.lineTo(p[0], p[1]); else g.moveTo(p[0], p[1]); });
      if (solved === pts.length) g.closePath();
      g.stroke();
      pts.forEach(function (p, i) {
        var lit = i < solved, pulse = reduced ? .5 : .5 + .5 * Math.sin(t * .003 + i);
        g.fillStyle = lit ? 'rgba(255,230,109,' + (.14 + pulse * .2) + ')' : 'rgba(0,0,0,.35)';
        g.beginPath(); g.arc(p[0], p[1], lit ? 20 + pulse * 7 : 9, 0, 7); g.fill();
        g.fillStyle = lit ? '#ffe66d' : '#ffffff99';
        g.beginPath(); g.arc(p[0], p[1], lit ? 6 : 3, 0, 7); g.fill();
      });
      if (!reduced) requestAnimationFrame(draw);
    }
    addEventListener('resize', size); size();
    requestAnimationFrame(draw);
  }

  addEventListener('load', function () {
    // The plasma sets its pointer handler during load; this script's listener runs after it.
    realMove = document.onmousemove;
    document.onmousemove = function (e) { lastReal = Date.now(); if (realMove) realMove(e); };
    addEventListener('touchmove', function (e) { var p = e.touches[0]; if (p) { lastReal = Date.now(); feed(p.clientX, p.clientY); } }, { passive: true });
    // It keeps breathing when nobody touches it.
    var t0 = Date.now();
    if (!reduced) setInterval(function () {
      if (Date.now() - lastReal < 4000 || Date.now() < stirring || document.hidden) return;
      var s = (Date.now() - t0) / 1000;
      feed(innerWidth * (.5 + .38 * Math.sin(s * .37) * Math.cos(s * .11)), innerHeight * (.5 + .38 * Math.sin(s * .23 + 1)));
    }, 50);
    sky();
  });

  // ---- The sections under the terminal: daily picks, the live post, return visits.
  var day = Math.floor(Date.now() / 864e5);
  var GAMES = [['ZENDRIVE', 'Procedural vaporwave racer', 'zendrive'], ['MANDELPOD', 'Psychedelic octopus platformer', 'mandalpod'], ['MACHINE ELF', 'WebGPU fractal rail shooter', 'machine-elf'], ['RABBITVERSE', 'Procedural mascot platformer', 'rabbitverse'], ['TSUNAMI', 'Endless giant-wave surf game', 'tsunami-surf'], ['Rubber Duckie River Race', 'A 100-duck river race', 'rubber-duckie-river-race'], ['KRIKET', 'Cricket sim with a bat-cam', 'kriket-3'], ['MANDELHOP', 'Infinite fractal-garden frogger', 'mandelhop'], ['Damn Beavers', 'Systemic beaver-civilization sim', 'dambeavers'], ['NARAYANA', 'Procedural underwater adventure', 'narayana-the-hadal-cycle']];
  var g = GAMES[day % GAMES.length];
  document.getElementById('daily').href = 'https://github.com/xirtus/' + g[2];
  document.getElementById('daily-img').src = 'img/' + g[2] + '.webp';
  document.getElementById('daily-name').textContent = g[0];
  document.getElementById('daily-blurb').textContent = g[1];
  var SIGNALS = ['love is only one syllable', 'i is an imaginary number', 'time travel is real', 'this star is my orchard of thoughts', 'solvents are sometimes vile', 'drink mineral water', 'fnord'];
  document.getElementById('signal').textContent = SIGNALS[day % SIGNALS.length];
  fetch('https://data.narwallagram.com/latest-tweet.json', { cache: 'no-cache' }).then(function (r) { return r.json(); }).then(function (p) {
    if (!p || !p.text) return;
    document.getElementById('xpost').textContent = p.text.replace(/\s*https:\/\/t\.co\/\S+\s*$/, '');
    if (/^https:\/\/x\.com\//.test(p.url)) document.getElementById('xlink').href = p.url;
  }).catch(function () {});
  function liveFeed(f) {
    var list = document.querySelector('.commits ol');
    var short = function (iso) { return new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' }); };
    if (list && f.commits && f.commits.length) {
      list.replaceChildren();
      f.commits.slice(0, 6).forEach(function (c) {
        var li = document.createElement('li'), a = document.createElement('a'), b = document.createElement('b'), m = document.createElement('span'), t = document.createElement('time');
        li.dataset.date = c.date; a.href = c.url; b.textContent = c.repo; m.className = 'msg'; m.textContent = c.message; t.textContent = short(c.date);
        a.appendChild(b); a.appendChild(document.createTextNode(' ')); a.appendChild(m); li.appendChild(a); li.appendChild(t); list.appendChild(li);
      });
    }
    var v = f.videos && f.videos[0], thumb = document.querySelector('.thumb');
    if (v && thumb) {
      thumb.href = 'https://www.youtube.com/watch?v=' + encodeURIComponent(v.id);
      thumb.querySelector('img').src = 'https://i.ytimg.com/vi/' + encodeURIComponent(v.id) + '/hqdefault.jpg';
      document.querySelector('.vtitle').textContent = v.title;
    }
  }
  function welcome() {
  var last = Number(saved('xirt.lastVisit')) || 0, fresh = 0;
  if (last) {
    document.querySelectorAll('[data-date]').forEach(function (el) { if (Date.parse(el.dataset.date) > last) { fresh++; var m = el.querySelector('.msg'); if (m) m.classList.add('new'); } });
    document.getElementById('since').textContent = fresh ? 'Welcome back. ' + fresh + ' new since your last visit.' : 'Welcome back.';
  }
  saved('xirt.lastVisit', String(Date.now()));
  }
  fetch('https://data.narwallagram.com/network.json', { cache: 'no-cache' }).then(function (r) { if (!r.ok) throw 0; return r.json(); }).then(liveFeed).catch(function () {}).then(welcome);
  var btc = document.getElementById('btc');
  btc.addEventListener('click', function () {
    navigator.clipboard.writeText(btc.dataset.address).then(function () { btc.textContent = 'Copied. Thank you.'; }).catch(function () { btc.textContent = btc.dataset.address; });
  });

  print('XIRT U.S.', 'win');
  print('seven stars circle the orchard. ' + (solved ? solved + ' still burn.' : 'all of them dark.'), 'sys');
  print("type 'clue' to begin, or 'help'.\n", 'sys');
  score();
})();
