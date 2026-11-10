use sex_js::{Document, Limits, Script};

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(html: &str) -> Script<Document> {
    let dom = sex_html::parse::parse(html);
    Script::new(Document::new(dom, "http://example.test/dir/page.html", (410, 330), clock), Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> {
    s.page_mut().log.drain(..).map(|(l, t)| format!("{l}:{t}")).collect()
}

#[test]
fn evaluates_and_logs() {
    let mut s = page("<p>hi</p>");
    s.eval("console.log('sum', 1 + 2, {a: [1, 2]}, 0.1 + 0.2)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:sum 3 {\"a\":[1,2]} 0.30000000000000004"]);
}

#[test]
fn exceptions_are_reported() {
    let mut s = page("");
    assert_eq!(s.eval("null.x", "bad.js"), Err(sex_js::Error::Thrown));
    let l = logs(&mut s);
    assert!(l[0].starts_with("2:TypeError"), "{l:?}");
    // The engine still works afterwards.
    s.eval("console.log('ok')", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:ok"]);
}

#[test]
fn reads_and_changes_the_tree() {
    let mut s = page("<div id=a class='x y'><p>one</p><p>two</p></div>");
    s.eval(r#"
        const a = document.getElementById('a');
        console.log(a.tagName, a.children.length, a.querySelectorAll('p').length, a.classList.contains('y'));
        a.querySelector('p').textContent = 'uno';
        const li = document.createElement('span');
        li.id = 'new'; li.textContent = 'tres & <b>';
        a.appendChild(li);
        a.classList.add('z'); a.classList.remove('x');
        a.style.backgroundColor = 'red'; a.style.marginTop = '4px';
        console.log(a.innerHTML);
        console.log(a.getAttribute('class'), a.getAttribute('style'), a.style.marginTop);
        console.log(document.getElementById('new') === li, li.parentNode === a, a.firstElementChild.nextElementSibling.textContent);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), [
        "0:DIV 2 2 true",
        "0:<p>uno</p><p>two</p><span id=\"new\">tres &amp; &lt;b&gt;</span>",
        "0:y z background-color: red; margin-top: 4px 4px",
        "0:true true two",
    ]);
    assert!(s.page().dirty);
}

#[test]
fn inner_html_parses_and_moves() {
    let mut s = page("<ul id=l></ul><div id=d></div>");
    s.eval(r#"
        const l = document.getElementById('l');
        l.innerHTML = '<li>a</li><li class=b>b</li>';
        const d = document.getElementById('d');
        d.appendChild(l.querySelector('.b'));
        console.log(l.children.length, d.innerHTML, document.body.outerHTML);
        const f = document.createDocumentFragment();
        f.append('x', document.createElement('br'));
        d.appendChild(f);
        console.log(d.childNodes.length, d.innerHTML);
        d.remove();
        console.log(document.getElementById('d'), d.isConnected);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), [
        "0:1 <li class=\"b\">b</li> <body><ul id=\"l\"><li>a</li></ul><div id=\"d\"><li class=\"b\">b</li></div></body>",
        "0:3 <li class=\"b\">b</li>x<br>",
        "0:null false",
    ]);
}

#[test]
fn events_bubble_and_handlers_run() {
    let mut s = page("<div id=o onclick=\"console.log('attr', event.type)\"><button id=b>go</button></div>");
    s.eval(r#"
        const b = document.getElementById('b');
        b.addEventListener('click', e => { console.log('button', e.target === b); });
        document.addEventListener('click', e => { console.log('doc', e.currentTarget === document); e.preventDefault(); });
        document.addEventListener('keydown', e => console.log('key', e.key));
        window.addEventListener('load', () => console.log('loaded', document.readyState));
    "#, "t.js").unwrap();
    let b = s.page().dom.descendants(0).into_iter().find(|&n| s.page().dom.attr(n, "id") == Some("b")).unwrap();
    assert!(s.page().dom.activatable(b));
    assert_eq!(s.dispatch(&format!("click\t{b}")).unwrap(), 1);
    assert_eq!(s.dispatch("key\tkeydown\tArrowUp").unwrap(), 0);
    s.dispatch("ready").unwrap();
    assert_eq!(logs(&mut s), ["0:button true", "0:attr click", "0:doc true", "0:key ArrowUp", "0:loaded complete"]);
}

#[test]
fn timers_run_in_order() {
    let mut s = page("");
    s.eval(r#"
        setTimeout(() => console.log('t2'), 2);
        setTimeout(() => console.log('t0'), 0);
        let n = 0; const iv = setInterval(() => { if (++n == 2) clearInterval(iv); console.log('iv', n); }, 1);
        requestAnimationFrame(t => console.log('raf', typeof t));
        Promise.resolve().then(() => console.log('micro'));
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:micro"]);
    let mut waits = 0;
    loop {
        let next = s.dispatch("tick").unwrap();
        if next < 0 { break; }
        std::thread::sleep(std::time::Duration::from_millis(next as u64 + 1));
        waits += 1;
        assert!(waits < 50);
    }
    let l = logs(&mut s);
    assert_eq!(l.iter().filter(|x| x.starts_with("0:iv")).count(), 2, "{l:?}");
    assert!(l.iter().position(|x| x == "0:t0").unwrap() < l.iter().position(|x| x == "0:t2").unwrap());
    assert!(l.contains(&"0:raf number".to_string()));
}

#[test]
fn location_and_navigation() {
    let mut s = page("<a id=x href='../other.html'>o</a>");
    s.eval(r#"
        console.log(location.host, location.pathname, document.getElementById('x').href, innerWidth, innerHeight);
        location.href = 'next.html?q=1';
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:example.test /dir/page.html http://example.test/dir/../other.html 410 330"]);
    assert_eq!(s.page().navigate.as_deref(), Some("http://example.test/dir/next.html?q=1"));
}

#[test]
fn runaway_script_is_stopped() {
    let mut s = Script::new(Document::new(sex_html::parse::parse(""), "", (1, 1), clock), Limits { budget_ms: 200, load_budget_ms: 200, ..Limits::default() }).unwrap();
    assert_eq!(s.eval("for(;;){}", "loop.js"), Err(sex_js::Error::Thrown));
    s.eval("console.log('alive')", "t.js").unwrap();
    assert_eq!(logs(&mut s).last().unwrap(), "0:alive");
}

#[test]
fn the_language_works() {
    let mut s = page("");
    s.eval(r#"
        class A { #x = 2; get x() { return this.#x; } }
        const m = new Map([[1, 'a']]);
        const r = /(\d+)-(\d+)/.exec('10-20');
        console.log(new A().x, m.get(1), r[2], [3,1,2].sort().join(), JSON.parse('{"k":[true]}').k[0],
          Math.sqrt(2).toFixed(5), (1234.5678).toFixed(2), parseFloat('3.25e2'), 2 ** 10, BigInt(2) ** 64n,
          'é'.normalize('NFD').length, new Date(0).toISOString());
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:2 a 20 1,2,3 true 1.41421 1234.57 325 1024 18446744073709551616 2 1970-01-01T00:00:00.000Z"]);
}

#[derive(Default)]
struct Raw(Vec<String>);
impl sex_js::Page for Raw {
    fn call(&mut self, op: u32, args: &[&str]) -> Option<String> { if op == 1 { self.0.push(args.join("|")); } None }
    fn now_ns(&mut self) -> u64 { clock() }
}
#[test]
fn prelude_loads() {
    let wasm = sex_js::QUICKJS_WASM;
    let r = Script::with_engine(wasm, Raw::default(), Limits::default());
    if let Err(e) = r { panic!("{e:?}"); }
}

#[test]
fn startup_time() {
    let t = std::time::Instant::now();
    let _s = page("");
    eprintln!("STARTUP {:?}", t.elapsed());
}

#[test]
fn onclick_property_makes_clickable() {
    let mut s = page("<div id=d>x</div>");
    s.eval("const d = document.getElementById('d'); d.onclick = () => console.log('hit');", "t.js").unwrap();
    let d = s.page().dom.descendants(0).into_iter().find(|&n| s.page().dom.attr(n, "id") == Some("d")).unwrap();
    assert!(s.page().dom.activatable(d));
    s.dispatch(&format!("click\t{d}")).unwrap();
    assert_eq!(logs(&mut s), ["0:hit"]);
}

#[test]
fn canvas_commands_are_queued() {
    let mut s = page("<canvas id=c width=20 height=10></canvas>");
    s.eval(r#"
        const ctx = document.getElementById('c').getContext('2d');
        console.log(ctx === document.getElementById('c').getContext('2d'), document.getElementById('c').getContext('webgl'));
        ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 5, NaN); ctx.fillRect(1, 2, 3, 4);
        console.log(ctx.fillStyle, ctx.measureText('abcd').width > 0);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:true null", "0:red true"], "a 2d canvas has no webgl context");
    let q = &s.page().canvas_queue;
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].1, "fs\u{1f}c\u{1f}red\u{1e}fr\u{1f}1\u{1f}2\u{1f}3\u{1f}4");
}

#[test]
fn canvas_gradient_stops_and_resizes_reach_the_queue() {
    let mut s = page("<canvas id=c width=20 height=10></canvas>");
    s.eval(r#"
        const c = document.getElementById('c'), ctx = c.getContext('2d');
        const g = ctx.createLinearGradient(0, 0, 10, 0);
        ctx.fillStyle = g; g.addColorStop(0, 'red'); ctx.fillRect(0, 0, 1, 1);
        ctx.fillRect(0, 0, 2, 2);
        ctx.lineWidth = 3; c.width = c.width;
        console.log(ctx.lineWidth, ctx.fillStyle);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:1 #000000"]);
    let q = &s.page().canvas_queue;
    let cmds: Vec<&str> = q[0].1.split('\u{1e}').collect();
    assert_eq!(cmds, [
        "fs\u{1f}l\u{1f}0\u{1f}0\u{1f}10\u{1f}0\u{1f}0",
        "fs\u{1f}l\u{1f}0\u{1f}0\u{1f}10\u{1f}0\u{1f}1\u{1f}0\u{1f}red",
        "fr\u{1f}0\u{1f}0\u{1f}1\u{1f}1",
        "fr\u{1f}0\u{1f}0\u{1f}2\u{1f}2",
        "lw\u{1f}3",
        "sz\u{1f}20\u{1f}10",
    ]);
}

#[test]
fn webgl_draws_a_triangle_from_js() {
    let mut s = page("<canvas id=c width=8 height=8></canvas>");
    s.eval(r#"
        const gl = document.getElementById('c').getContext('webgl');
        const sh = (t, src) => { const x = gl.createShader(t); gl.shaderSource(x, src); gl.compileShader(x);
            if (!gl.getShaderParameter(x, gl.COMPILE_STATUS)) console.log('shader', gl.getShaderInfoLog(x)); return x; };
        const p = gl.createProgram();
        gl.attachShader(p, sh(gl.VERTEX_SHADER, 'attribute vec2 pos; varying vec2 v; void main() { v = pos; gl_Position = vec4(pos, 0.0, 1.0); }'));
        gl.attachShader(p, sh(gl.FRAGMENT_SHADER, 'precision mediump float; uniform vec4 tint; varying vec2 v; void main() { gl_FragColor = tint; }'));
        gl.linkProgram(p);
        console.log(gl.getProgramParameter(p, gl.LINK_STATUS), gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS), gl.getAttribLocation(p, 'pos'));
        gl.useProgram(p);
        gl.uniform4f(gl.getUniformLocation(p, 'tint'), 0, 1, 0, 1);
        const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b);
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
        gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
        gl.clearColor(1, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        const px = new Uint8Array(4); gl.readPixels(3, 3, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        console.log(Array.from(px).join(','), gl.getError(), gl.getParameter(gl.VERSION));
    "#, "t.js").unwrap();
    let l = logs(&mut s);
    assert_eq!(l[0], "0:true 1 0");
    assert!(l[1].starts_with("0:0,255,0,255 0 WebGL 1.0"), "{l:?}");
}

#[test]
fn webgl_page_picture_is_in_the_texture_before_the_next_draw() {
    let mut s = page("<img id=p src=pic.png><canvas id=c width=4 height=4></canvas>");
    s.page_mut().pictures.push(("pic.png".into(), std::rc::Rc::new(sex_image::Image { width: 1, height: 1, pixels: vec![0xFF20_4060] })));
    s.eval(r#"
        const gl = document.getElementById('c').getContext('webgl');
        const sh = (t, src) => { const x = gl.createShader(t); gl.shaderSource(x, src); gl.compileShader(x); return x; };
        const p = gl.createProgram();
        gl.attachShader(p, sh(gl.VERTEX_SHADER, 'attribute vec2 pos; void main() { gl_Position = vec4(pos, 0.0, 1.0); }'));
        gl.attachShader(p, sh(gl.FRAGMENT_SHADER, 'precision mediump float; uniform sampler2D t; void main() { gl_FragColor = texture2D(t, vec2(0.5)); }'));
        gl.linkProgram(p); gl.useProgram(p);
        gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
        gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
        gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, document.getElementById('p'));
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        const px = new Uint8Array(4); gl.readPixels(1, 1, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        console.log(Array.from(px).join(','));
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:32,64,96,255"]);
}

#[test]
fn mouse_events_reach_the_element_under_the_pointer() {
    let mut s = page("<body><div id=a><canvas id=c></canvas></div><p id=p>x</p></body>");
    s.eval(r#"
        const c = document.getElementById('c'), log = [];
        for (const t of ['pointerdown', 'mousedown', 'mouseup', 'click', 'dblclick', 'mouseover', 'mouseenter', 'mousemove', 'wheel', 'contextmenu', 'mouseleave'])
          c.addEventListener(t, e => log.push(t + ':' + e.clientX + ',' + e.clientY + ',' + e.button + ',' + e.buttons + (t === 'wheel' ? ',' + e.deltaY : '') + (t === 'click' ? ',' + e.offsetX + ',' + e.offsetY : '')));
        document.getElementById('a').addEventListener('click', e => log.push('bubbled'));
        document.getElementById('p').addEventListener('click', e => e.preventDefault());
        window.done = () => console.log(log.join(' '));
    "#, "t.js").unwrap();
    let id = |s: &mut Script<Document>, q: &str| { s.eval(&format!("console.log(document.getElementById('{q}')._id)"), "id.js").unwrap(); logs(s).pop().unwrap()[2..].to_string() };
    let (c, p) = (id(&mut s, "c"), id(&mut s, "p"));
    // The canvas is laid out at (10, 20) 100 by 50: hit testing finds it, not its parent.
    let (cn, pn): (usize, usize) = (c.parse().unwrap(), p.parse().unwrap());
    let doc = s.page_mut();
    doc.rects = vec![None; doc.dom.nodes.len()];
    let parent = doc.dom.nodes[cn].parent.unwrap();
    doc.rects[parent] = Some((0, 0, 410, 80));
    doc.rects[cn] = Some((10, 20, 100, 50));
    doc.rects[pn] = Some((0, 80, 410, 20));
    assert_eq!(doc.node_at(15, 25), Some(cn));
    assert_eq!(doc.node_at(5, 5), Some(parent));
    assert_eq!(doc.node_at(5, 90), Some(pn));
    assert_eq!(doc.node_at(500, 500), None);
    for m in ["move\t{c}\t15\t25\t0\t0\t0", "down\t{c}\t15\t25\t0\t1\t0", "up\t{c}\t15\t25\t0\t0\t0", "down\t{c}\t15\t25\t0\t1\t0", "up\t{c}\t16\t25\t0\t0\t0",
              "wheel\t{c}\t15\t25\t0\t0\t-2", "down\t{c}\t15\t25\t2\t2\t0", "up\t{c}\t15\t25\t2\t0\t0", "leave\t\t-1\t-1\t0\t0\t0"] {
        assert_eq!(s.dispatch(&format!("mouse\t{}", m.replace("{c}", &c))).unwrap(), 0, "{m}");
    }
    // A cancelled click answers 1, so the browser won't follow a link there.
    assert_eq!(s.dispatch(&format!("mouse\tdown\t{p}\t5\t90\t0\t1\t0")).unwrap(), 0);
    assert_eq!(s.dispatch(&format!("mouse\tup\t{p}\t5\t90\t0\t0\t0")).unwrap(), 1);
    s.eval("done(); console.log(document.elementFromPoint(15, 25) === document.getElementById('c'))", "t.js").unwrap();
    let l = logs(&mut s);
    assert_eq!(l[0], "0:mouseover:15,25,-1,0 mouseenter:15,25,-1,0 mousemove:15,25,-1,0 pointerdown:15,25,0,1 mousedown:15,25,0,1 mouseup:15,25,0,0 click:15,25,0,0,5,5 bubbled \
pointerdown:15,25,0,1 mousedown:15,25,0,1 mouseup:16,25,0,0 click:16,25,0,0,6,5 bubbled dblclick:16,25,0,0 wheel:15,25,0,0,200 \
pointerdown:15,25,2,2 mousedown:15,25,2,2 mouseup:15,25,2,0 contextmenu:15,25,2,0 mouseleave:-1,-1,0,0");
    assert_eq!(l[1], "0:true");
}

#[test]
fn fetch_init_xhr_and_cookies() {
    let mut s = page("<p>x</p>");
    // The page is http://example.test/dir/page.html; cookies are shared with the browser.
    let jar = std::rc::Rc::new(std::cell::RefCell::new(sex_fetch::Jar::new()));
    let url = sex_fetch::Url::parse("http://example.test/dir/page.html").unwrap();
    jar.borrow_mut().store(&url, "sid=1; Path=/; HttpOnly", 100);
    jar.borrow_mut().store(&url, "pref=dark; Path=/", 100);
    s.page_mut().cookies = Some(jar.clone());
    s.page_mut().wall = || 200;
    s.eval(r#"
        console.log(document.cookie);
        document.cookie = 'seen=yes; path=/';
        document.cookie = 'sid=stolen; path=/';
        console.log(document.cookie);
        fetch('/youtubei/v1/player', { method: 'post', headers: { 'Content-Type': 'application/json', 'X-Goog-Api-Format-Version': '2' }, body: JSON.stringify({ videoId: 'abc' }) })
          .then(r => r.json().then(j => console.log('fetch', r.status, r.headers.get('content-type'), r.url, j.ok)));
        const x = new XMLHttpRequest();
        x.open('GET', 'data.bin');
        x.responseType = 'arraybuffer';
        x.setRequestHeader('Range', 'bytes=0-3');
        const states = [];
        x.onreadystatechange = () => states.push(x.readyState);
        x.onload = () => console.log('xhr', x.status, x.getResponseHeader('Content-Range'), new Uint8Array(x.response).join(','), states.join(''));
        x.send();
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:pref=dark", "0:pref=dark; seen=yes"]);
    assert_eq!(jar.borrow().header(&url, 200).unwrap(), "sid=1; pref=dark; seen=yes", "scripts can't replace HttpOnly cookies");
    let asked = std::mem::take(&mut s.page_mut().fetches);
    assert_eq!(asked.len(), 2);
    assert_eq!((asked[0].url.as_str(), asked[0].method.as_str()), ("/youtubei/v1/player", "POST"));
    assert_eq!(asked[0].header_pairs(), [("content-type", "application/json"), ("x-goog-api-format-version", "2")]);
    assert_eq!(asked[0].body, br#"{"videoId":"abc"}"#);
    assert_eq!((asked[1].url.as_str(), asked[1].method.as_str(), asked[1].header_pairs()), ("data.bin", "GET", vec![("range", "bytes=0-3")]));
    s.page_mut().keep_head(asked[0].id, "http://example.test/youtubei/v1/player", "content-type: application/json\r\n");
    s.page_mut().keep_body(asked[0].id, 200, br#"{"ok":true}"#.to_vec());
    s.dispatch(&format!("fetched\t{}\t200\t11", asked[0].id)).unwrap();
    s.page_mut().keep_head(asked[1].id, "http://example.test/dir/data.bin", "content-range: bytes 0-3/10\r\n");
    s.page_mut().keep_body(asked[1].id, 206, vec![1, 2, 3, 4]);
    s.dispatch(&format!("fetched\t{}\t206\t4", asked[1].id)).unwrap();
    s.dispatch("flush").unwrap();
    assert_eq!(logs(&mut s), ["0:fetch 200 application/json http://example.test/youtubei/v1/player true", "0:xhr 206 bytes 0-3/10 1,2,3,4 234"]);
}

#[test]
fn forms_take_typing_clicks_and_submit() {
    let mut s = page("<body><form id=f action=search><label id=l>Find <input id=q name=q value=old></label>\
        <input type=checkbox id=c name=c><input type=radio name=r value=a id=ra checked><input type=radio name=r value=b id=rb>\
        <select id=s name=s><option>x</option><option value=yy>y</option></select><textarea id=t name=t>hi</textarea>\
        <button id=b>Go</button></form></body>");
    s.eval(r#"
        const $ = id => document.getElementById(id), log = [];
        $('q').addEventListener('input', e => log.push('input:' + e.data));
        $('q').addEventListener('change', () => log.push('change'));
        $('c').addEventListener('change', () => log.push('c:' + $('c').checked));
        $('f').addEventListener('submit', e => log.push('submit:' + (e.submitter ? e.submitter.id : '-')));
        window.id = el => el._id;
    "#, "t.js").unwrap();
    let id = |s: &mut Script<Document>, q: &str| { s.eval(&format!("console.log(id(document.getElementById('{q}')))"), "id.js").unwrap(); logs(s).pop().unwrap()[2..].to_string() };
    let q = id(&mut s, "q");
    // Pressing the field focuses it with the caret at the end; typing edits the value.
    s.dispatch(&format!("mouse\tdown\t{q}\t5\t5\t0\t1\t0")).unwrap();
    s.dispatch(&format!("mouse\tup\t{q}\t5\t5\t0\t0\t0")).unwrap();
    let qn: usize = q.parse().unwrap();
    assert_eq!(s.page_mut().dom.caret, Some((qn, 3)));
    for k in ["Backspace", "ArrowLeft", "X", "End", "!"] {
        assert_eq!(s.dispatch(&format!("key\tkeydown\t{k}\t\t0")).unwrap(), 1, "{k}");
    }
    assert_eq!(s.page_mut().dom.control_value(qn), "oXl!");
    assert_eq!(s.page_mut().dom.caret, Some((qn, 4)));
    // The value attribute stays the default.
    s.eval("console.log($('q').value, $('q').defaultValue, $('q').getAttribute('value'))", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:oXl! old old"]);
    // Checkbox, radio, select and label clicks.
    s.eval("$('c').click(); $('rb').click(); $('s').click(); console.log($('c').checked, $('ra').checked, $('rb').checked, $('s').value)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:true false true yy"]);
    // Enter in a text field submits the form as a GET to its action.
    s.eval("$('q').focus()", "t.js").unwrap();
    assert_eq!(s.dispatch("key\tkeydown\tEnter\t\t0").unwrap(), 1);
    assert_eq!(s.page_mut().navigate.take().as_deref(), Some("http://example.test/dir/search?q=oXl%21&c=on&r=b&s=yy&t=hi"));
    // So does the submit button, which is sent too when it has a name; FormData sees the same.
    s.eval("$('b').setAttribute('name', 'go'); $('b').click(); console.log([...new FormData($('f'))].length, log.join(' '))", "t.js").unwrap();
    assert_eq!(s.page_mut().navigate.take().as_deref(), Some("http://example.test/dir/search?q=oXl%21&c=on&r=b&s=yy&t=hi&go="));
    assert_eq!(logs(&mut s), ["0:5 input:null input:X input:! c:true change submit:- submit:b"]);
    // Blurring clears the caret; reset restores defaults.
    s.eval("$('q').blur(); $('f').reset(); console.log($('q').value, $('t').value)", "t.js").unwrap();
    assert_eq!(s.page_mut().dom.caret, None);
    assert_eq!(logs(&mut s), ["0:old hi"]);
}

#[test]
fn running_out_of_script_heap_says_so() {
    let dom = sex_html::parse::parse("<p>x</p>");
    let limits = Limits { memory_bytes: 64 << 20, heap_bytes: 8 << 20, ..Limits::default() };
    let mut s = Script::new(Document::new(dom, "http://example.test/", (410, 330), clock), limits).expect("engine");
    let _ = s.eval("const keep = []; for (;;) keep.push(new Array(100000).fill(1.5));", "big.js");
    let l = logs(&mut s);
    assert!(l.iter().any(|m| m.contains("out of memory")), "{l:?}");
    // The engine still works afterwards.
    s.eval("keep.length = 0; console.log('ok')", "t.js").unwrap();
}

#[test]
fn form_reset_restores_checkboxes_and_radios() {
    let mut s = page("<form id=f><input type=checkbox id=c checked><input type=radio name=r id=a checked><input type=radio name=r id=b></form>");
    s.eval("const $ = id => document.getElementById(id); $('c').click(); $('b').click(); \
console.log($('c').checked, $('a').checked, $('b').checked, $('c').defaultChecked); $('f').reset(); \
console.log($('c').checked, $('a').checked, $('b').checked)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:false false true true", "0:true true false"]);
}

#[test]
fn clicking_inside_a_button_submits() {
    let mut s = page("<form id=f action=go><input name=q value=x><button><span id=s>Go</span></button></form>");
    s.eval("let t; document.getElementById('f').addEventListener('submit', e => { t = e.submitter.localName; e.preventDefault(); }); \
document.getElementById('s').click(); console.log(t)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:button"]);
}

#[test]
fn form_post_navigates_with_its_body() {
    let mut s = page("<form id=f method=post action='/login#done'><input name=user value='a b'><input name=pw value='x&y'></form>");
    s.eval("document.getElementById('f').submit()", "t.js").unwrap();
    let d = s.page_mut();
    assert_eq!(d.navigate.as_deref(), Some("http://example.test/login#done"));
    assert_eq!(d.navigate_post.as_deref(), Some("user=a+b&pw=x%26y"));
}

/// Loads a module graph the way the browser does: ask what is missing, supply it, repeat.
fn run_module(s: &mut Script<Document>, src: &str, name: &str, files: &[(&str, &str)]) -> Result<(), sex_js::Error> {
    for _ in 0..8 {
        let need = s.module_needs(src, name).unwrap();
        if need.is_empty() { break; }
        for url in need {
            let body = files.iter().find(|f| f.0 == url).map(|f| f.1).unwrap_or_else(|| panic!("asked for {url}"));
            s.page_mut().modules.insert(url, body.to_string());
        }
    }
    s.eval_module(src, name)
}

#[test]
fn module_scripts_import_their_graph() {
    let mut s = page("<p>hi</p>");
    let files = [
        ("http://example.test/dir/a.js", "import { b } from './sub/b.js'; export const a = 'A' + b;"),
        ("http://example.test/dir/sub/b.js", "import * as c from '/c.js'; export const b = 'B' + c.default + import.meta.url.length;"),
        ("http://example.test/c.js", "export default 'C';"),
    ];
    run_module(&mut s, "import { a } from './a.js'; console.log(a); console.log(typeof window, this === undefined);", "http://example.test/dir/page.html#module1", &files).unwrap();
    assert_eq!(logs(&mut s), ["0:ABC32", "0:object true"]);
    // A second module script shares the loaded modules; a missing one is an error, not a crash.
    run_module(&mut s, "import { b } from './sub/b.js'; console.log('again', b);", "http://example.test/dir/page.html#module2", &files).unwrap();
    assert_eq!(logs(&mut s), ["0:again BC32"]);
    assert_eq!(s.eval_module("import x from './nope.js';", "http://example.test/dir/page.html#module3"), Err(sex_js::Error::Thrown));
    let l = logs(&mut s);
    assert!(l[0].contains("could not load module"), "{l:?}");
    // Top-level await and errors thrown while a module runs reach the log.
    s.eval_module("await null; throw new TypeError('late');", "http://example.test/m4").unwrap_err();
    let l = logs(&mut s);
    assert!(l[0].starts_with("2:TypeError: late"), "{l:?}");
}

#[test]
fn dynamic_import_waits_for_its_module() {
    let mut s = page("<p>hi</p>");
    // From a classic script: the import waits until the host gives the module (and its own imports).
    s.eval("import('./lazy.js').then(m => console.log('lazy', m.x), e => console.log('failed', e.message));", "http://example.test/dir/page.html#script1").unwrap();
    assert!(logs(&mut s).is_empty());
    assert_eq!(s.modules_wanted(), ["http://example.test/dir/lazy.js"]);
    s.page_mut().modules.insert("http://example.test/dir/lazy.js".into(), "import { y } from './y.js'; export const x = 'X' + y;".into());
    assert_eq!(s.modules_ready().unwrap(), 1);
    assert_eq!(s.modules_wanted(), ["http://example.test/dir/y.js"]);
    s.page_mut().modules.insert("http://example.test/dir/y.js".into(), "export const y = 'Y';".into());
    assert_eq!(s.modules_ready().unwrap(), 0);
    assert_eq!(logs(&mut s), ["0:lazy XY"]);
    // Loaded already: it runs at once. A fragment shares the source. A failed fetch is an error.
    s.eval("import('./lazy.js#again').then(m => console.log('again', m.x));", "http://example.test/dir/page.html#script2").unwrap();
    assert_eq!(logs(&mut s), ["0:again XY"]);
    s.eval("import('/gone.js').then(m => console.log('ok'), e => console.log('failed', e.name));", "http://example.test/dir/page.html#script3").unwrap();
    assert_eq!(s.modules_wanted(), ["http://example.test/gone.js"]);
    s.page_mut().modules.insert("http://example.test/gone.js".into(), sex_js::document::failed_module("http://example.test/gone.js"));
    s.modules_ready().unwrap();
    assert_eq!(logs(&mut s), ["0:failed TypeError"]);
    // An entry module knows its own address.
    s.eval_module("console.log(import.meta.url, new URL('./img.png', import.meta.url).href);", "http://example.test/dir/app.js").unwrap();
    assert_eq!(logs(&mut s), ["0:http://example.test/dir/app.js http://example.test/dir/img.png"]);
}

#[test]
fn defining_a_custom_element_upgrades_the_same_object() {
    let mut s = page("<x-late id=x></x-late>");
    s.eval(r#"
        const el = document.getElementById('x'); el.mine = 1;
        el.addEventListener('ping', () => console.log('heard'));
        customElements.define('x-late', class extends HTMLElement { constructor() { super(); this.made = true; } hi() { return 'hi'; } });
        console.log(document.getElementById('x') === el, el.mine, el.made, el.hi());
        el.dispatchEvent(new Event('ping'));
    "#, "t").unwrap();
    assert_eq!(logs(&mut s), ["0:true 1 true hi", "0:heard"]);
}

#[test]
fn old_dom_apis_are_there() {
    let mut s = page("<div id=a><p>one</p><!-- note --><p>two</p></div>");
    s.eval(r#"
        const w = document.createTreeWalker(document.getElementById('a'), NodeFilter.SHOW_ELEMENT);
        const seen = []; while (w.nextNode()) seen.push(w.currentNode.tagName);
        console.log(seen.join(','));
        const e = document.createEvent('CustomEvent'); e.initCustomEvent('hey', true, true, 5);
        document.addEventListener('hey', ev => console.log('hey', ev.detail, ev.bubbles));
        document.dispatchEvent(e);
        const d = document.implementation.createHTMLDocument('');
        d.body.innerHTML = '<form></form><form></form>';
        console.log(d.body.childNodes.length, document.querySelectorAll('form').length);
        const u = new URL('/x.js', document.baseURI); u.hash = 'r=1'; console.log(u.href, URL.canParse('nope'));
        const c = document.createComment('hidden'); document.body.appendChild(c);
        console.log(c.nodeType, c.data, document.body.textContent.includes('hidden'), typeof Comment, document.fonts.status, window instanceof Window && window instanceof EventTarget);
    "#, "t").unwrap();
    assert_eq!(logs(&mut s), ["0:P,P", "0:hey 5 true", "0:2 0", "0:http://example.test/x.js#r=1 false", "0:8 hidden false function loaded true"]);
}

#[test]
fn everyday_web_apis_are_there() {
    let mut s = page("<div id=a><my-card id=c title=hi></my-card></div>");
    s.eval(r#"
        'use strict';
        // A bare addEventListener is the window's, as in browsers.
        addEventListener('custom', e => console.log('bare', e.type));
        dispatchEvent(new Event('custom'));
        console.log(btoa('hello'), atob('aGVsbG8='), atob(btoa('\xff\x00')).length);
        console.log(new TextDecoder().decode(new TextEncoder().encode('héllo €')));
        const img = new Image(4, 5);
        console.log(img instanceof HTMLImageElement, img.getAttribute('width'));
        const input = document.createElement('input');
        input.type = 'checkbox';
        console.log(input.type, typeof performance.timing.responseStart, performance.getEntriesByType('navigation').length);
        const v = document.createElement('video');
        console.log(v instanceof HTMLVideoElement, v instanceof HTMLMediaElement, document.getElementById('a') instanceof HTMLDivElement);
        const c = structuredClone({ a: [1, { b: 2 }], m: new Map([[1, 2]]) });
        console.log(JSON.stringify(c.a), c.m.get(1));
        new IntersectionObserver(es => console.log('seen', es[0].isIntersecting, es[0].target.id)).observe(document.getElementById('a'));
        const ac = new AbortController(); ac.signal.addEventListener('abort', () => console.log('aborted')); ac.abort();
        new Blob(['ab', 'c']).text().then(t => console.log('blob', t));
        class MyCard extends HTMLElement {
            static get observedAttributes() { return ['title']; }
            constructor() { super(); this.made = true; }
            connectedCallback() { console.log('connected', this.id, this.made); }
            attributeChangedCallback(n, o, v) { console.log('attr', n, v); }
        }
        customElements.define('my-card', MyCard);
        const el = document.getElementById('c');
        console.log(el instanceof MyCard, customElements.get('my-card') === MyCard);
        const fresh = new MyCard();
        console.log(fresh.localName);
        document.body.appendChild(fresh);
        crypto.getRandomValues(new Uint8Array(4));
        console.log(crypto.randomUUID().length, typeof requestIdleCallback, CSS.supports('display', 'grid'));
    "#, "t.js").unwrap();
    s.dispatch("tick").unwrap();
    let l = logs(&mut s);
    assert_eq!(l, [
        "0:bare custom", "0:aGVsbG8= hello 2", "0:héllo €", "0:true 4", "0:checkbox number 1", "0:true true true",
        "0:[1,{\"b\":2}] 2", "0:aborted", "0:attr title hi", "0:connected c true", "0:true true", "0:my-card",
        "0:connected  true", "0:36 function true", "0:blob abc", "0:seen true a",
    ]);
}

#[test]
fn media_source_feeds_a_video() {
    let mut s = page("<video id=v></video>");
    s.eval(r#"
        console.log(MediaSource.isTypeSupported('video/mp4; codecs="avc1.4d401f"'), MediaSource.isTypeSupported('audio/mp4; codecs="mp4a.40.2"'),
            MediaSource.isTypeSupported('video/webm; codecs="vp9"'), MediaSource.isTypeSupported('video/mp4; codecs="hev1.1.6.L93.B0"'));
        const v = document.getElementById('v');
        const ms = new MediaSource();
        console.log(ms.readyState);
        ms.addEventListener('sourceopen', () => {
            const sb = ms.addSourceBuffer('video/mp4; codecs="avc1.4d401f"');
            const a = ms.addSourceBuffer('audio/mp4; codecs="mp4a.40.2"');
            console.log('open', ms.readyState, ms.sourceBuffers.length);
            sb.addEventListener('updateend', () => {
                console.log('updateend', sb.updating, sb.buffered.length, sb.buffered.end(0), v.buffered.length);
            });
            sb.appendBuffer(new Uint8Array([1, 2, 3]));
            try { sb.appendBuffer(new Uint8Array([4])); } catch (e) { console.log('busy', e.message); }
            window.sb = sb; window.a = a;
        });
        v.src = URL.createObjectURL(ms);
        window.ms = ms;
    "#, "t.js").unwrap();
    s.dispatch("tick").unwrap();
    // The append waits for the browser, bytes and all.
    let taken = std::mem::take(&mut s.page_mut().media_bytes);
    assert_eq!(taken.len(), 1);
    assert_eq!((taken[0].1, &taken[0].2[..]), (0, &[1u8, 2, 3][..]));
    let asked: Vec<String> = std::mem::take(&mut s.page_mut().media).into_iter().map(|(_, v, a)| format!("{v} {a}")).collect();
    assert_eq!(asked, ["mse ", "add video/mp4; codecs=\"avc1.4d401f\"", "add audio/mp4; codecs=\"mp4a.40.2\""]);
    let node = taken[0].0;
    s.dispatch(&format!("mse\t{node}\t0\t1\t0-2.5")).unwrap();
    s.dispatch(&format!("mse\t{node}\t1\t1\t0.5-3")).unwrap();
    s.eval("console.log('both', v_ranges()); function v_ranges() { const b = document.getElementById('v').buffered; return b.length + ' ' + b.start(0) + ' ' + b.end(0); }", "u.js").unwrap();
    s.eval("sb.timestampOffset = 5; sb.abort(); sb.changeType('video/webm; codecs=\"vp9\"'); console.log('offset', sb.timestampOffset);", "o.js").unwrap();
    let asked: Vec<String> = std::mem::take(&mut s.page_mut().media).into_iter().map(|(_, v, a)| format!("{v} {a}")).collect();
    assert_eq!(asked, ["offset 0 5", "abort 0", "type 0 video/webm; codecs=\"vp9\""]);
    s.eval("ms.endOfStream(); console.log(ms.readyState, ms.duration, document.getElementById('v').duration);", "w.js").unwrap();
    let l = logs(&mut s);
    assert_eq!(l, [
        "0:true true true false", "0:closed", "0:open open 2", "0:busy InvalidStateError",
        "0:updateend false 1 2.5 0", "0:both 1 0.5 2.5", "0:offset 5", "0:ended 3 3",
    ]);
}

#[test]
fn mutation_observers_see_changes() {
    let mut s = page("<div id=a><p id=p>one</p></div>");
    s.eval(r#"
        const a = document.getElementById('a'), p = document.getElementById('p');
        const seen = [];
        const mo = new MutationObserver(rs => { for (const r of rs) seen.push(r.type + ':' + (r.attributeName || '') + ':' + r.addedNodes.length + '/' + r.removedNodes.length + ':' + r.oldValue); console.log(seen.join(' ')); });
        mo.observe(a, { childList: true, subtree: true, attributes: true, attributeOldValue: true, attributeFilter: ['class', 'title'], characterData: true });
        p.className = 'x';
        p.setAttribute('data-skip', '1');
        p.title = 't';
        const s = document.createElement('span');
        a.appendChild(s);
        a.removeChild(s);
        p.firstChild.data = 'uno';
        a.innerHTML = '<b>new</b>';
        console.log('sync', seen.length);
        new MutationObserver(() => console.log('outside')).observe(document.body, { attributes: true });
        a.setAttribute('class', 'q');
        try { new MutationObserver(() => {}).observe(a, {}); } catch (e) { console.log(e.name); }
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), [
        "0:sync 0",
        "0:TypeError",
        "0:attributes:class:0/0:null attributes:title:0/0:null childList::1/0:null childList::0/1:null characterData::0/0:null childList::1/1:null attributes:class:0/0:null",
    ]);
}

#[test]
fn custom_elements_hear_attributes_and_removal() {
    let mut s = page("<div id=a></div>");
    s.eval(r#"
        class XBox extends HTMLElement {
            static get observedAttributes() { return ['size']; }
            connectedCallback() { console.log('in'); }
            disconnectedCallback() { console.log('out'); }
            attributeChangedCallback(n, o, v) { console.log('attr', n, o, v); }
        }
        customElements.define('x-box', XBox);
        const b = document.createElement('x-box');
        document.getElementById('a').appendChild(b);
        b.setAttribute('size', '2');
        b.setAttribute('other', '1');
        b.removeAttribute('size');
        b.remove();
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:in", "0:attr size null 2", "0:attr size 2 null", "0:out"]);
}

#[test]
fn shadow_roots_are_flattened_into_the_host() {
    let mut s = page("<div id=host><span>light</span></div>");
    s.eval(r#"
        const host = document.getElementById('host');
        const root = host.attachShadow({ mode: 'open' });
        root.innerHTML = '<style>b { color: red }</style><b id=in>shadow</b><slot></slot>';
        console.log(host.shadowRoot === root, root.host === host, root.nodeType, host.children.length, host.firstElementChild.tagName);
        console.log(host.innerHTML);
        const inner = root.getElementById('in');
        console.log(inner.getRootNode() === root, inner.getRootNode({ composed: true }) === document, inner.isConnected, host.contains(inner));
        console.log(root.querySelector('slot').assignedNodes().length, root.querySelector('slot').assignedElements()[0].tagName);
        host.addEventListener('click', e => console.log('host heard', e.composedPath().includes(root)));
        inner.click();
        host.textContent = 'plain';
        console.log(host.shadowRoot.firstElementChild.localName, host.childNodes.length, host.textContent.includes('plain'));
        try { host.attachShadow({ mode: 'open' }); } catch (e) { console.log(e.name); }
        const closed = document.createElement('div').attachShadow({ mode: 'closed' });
        console.log(closed.host.shadowRoot);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), [
        "0:true true 11 1 SPAN",
        "0:<span>light</span>",
        "0:true true true false",
        "0:1 SPAN",
        "0:host heard true",
        "0:style 1 true",
        "0:NotSupportedError",
        "0:null",
    ]);
}

#[test]
fn messages_reach_ports_and_the_window() {
    let mut s = page("");
    s.eval(r#"
        const ch = new MessageChannel();
        ch.port1.onmessage = e => console.log('port', JSON.stringify(e.data));
        ch.port2.postMessage({ a: [1] });
        window.addEventListener('message', e => console.log('window', e.data, e.origin, e.source === window));
        window.postMessage('hi', '*');
        window.postMessage('dropped', 'https://elsewhere.test');
        const b1 = new BroadcastChannel('c'), b2 = new BroadcastChannel('c');
        b2.onmessage = e => console.log('broadcast', e.data);
        b1.postMessage(5);
        console.log('sync');
    "#, "t.js").unwrap();
    s.dispatch("tick").unwrap();
    assert_eq!(logs(&mut s), ["0:sync", "0:port {\"a\":[1]}", "0:window hi http://example.test true", "0:broadcast 5"]);
}

#[test]
fn intl_formats_in_english() {
    let mut s = page("");
    let r = s.eval(r#"
        console.log(new Intl.NumberFormat('en-US').format(1234567.891), (1234.5).toLocaleString(), new Intl.NumberFormat('en', { style: 'percent' }).format(0.256));
        console.log(new Intl.NumberFormat('en', { notation: 'compact' }).format(1530000), new Intl.NumberFormat('en', { notation: 'compact' }).format(999), new Intl.NumberFormat('en', { style: 'currency', currency: 'USD' }).format(-3.5));
        console.log(Intl.DateTimeFormat().resolvedOptions().timeZone, new Intl.DateTimeFormat('en-US').format(Date.UTC(2026, 10, 9, 15, 4)));
        console.log(new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' }).format(Date.UTC(2026, 10, 9)), new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit' }).format(Date.UTC(2026, 10, 9, 15, 4)));
        console.log(new Intl.PluralRules('en').select(1), new Intl.PluralRules('en', { type: 'ordinal' }).select(22), new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(-1, 'day'), new Intl.RelativeTimeFormat('en').format(3, 'hours'));
        console.log(new Intl.ListFormat('en').format(['a', 'b', 'c']), ['b', 'a', 'C'].sort(new Intl.Collator().compare).join(''), 'a'.localeCompare('B'), new Intl.Locale('en-us').baseName);
    "#, "t.js");
    assert!(r.is_ok(), "{:?}", logs(&mut s));
    assert_eq!(logs(&mut s), [
        "0:1,234,567.891 1,234.5 26%",
        "0:1.5M 999 -$3.50",
        "0:UTC 11/9/2026",
        "0:Nov 9, 2026 3:04 PM",
        "0:one two yesterday in 3 hours",
        "0:a, b, and c abC -1 en-US",
    ]);
}

#[test]
fn smaller_platform_pieces() {
    let mut s = page("<div id=a><p>one</p><p>two</p></div><video id=v></video>");
    s.eval(r#"
        const d = new DOMParser().parseFromString('<p class=x>hi <b>there</b></p>', 'text/html');
        console.log(d.body.querySelector('.x').textContent, d.querySelector('b').tagName);
        const svg = new DOMParser().parseFromString('<svg viewBox="0 0 2 2"><path d="M0 0"/></svg>', 'image/svg+xml');
        console.log(svg.documentElement.localName, svg.documentElement.getAttribute('viewBox'));
        const e = new DOMException('gone', 'NotFoundError');
        console.log(e instanceof Error, e.name, e.code, DOMException.NOT_FOUND_ERR);
        const r = document.createRange();
        r.selectNodeContents(document.getElementById('a'));
        console.log(r.toString(), r.collapsed, r.createContextualFragment('<i>x</i>').firstChild.tagName);
        const rect = document.getElementById('a').getBoundingClientRect();
        console.log(rect instanceof DOMRect, typeof rect.toJSON().bottom);
        navigator.mediaCapabilities.decodingInfo({ type: 'media-source', video: { contentType: 'video/webm; codecs="vp9"', width: 1280, height: 720, framerate: 30 } })
            .then(i => console.log('decode', i.supported, i.smooth));
        navigator.mediaCapabilities.decodingInfo({ type: 'media-source', video: { contentType: 'video/mp4; codecs="hev1.1.6.L93.B0"', width: 1280, height: 720, framerate: 30 } })
            .then(i => console.log('hevc', i.supported));
        console.log(navigator.cookieEnabled, typeof navigator.sendBeacon, document.fullscreenEnabled, window.frameElement, document.getElementById('v').getVideoPlaybackQuality().droppedVideoFrames);
        const st = document.createElement('style');
        document.head.appendChild(st);
        st.sheet.insertRule('.y { color: red }', 0);
        console.log(st.textContent.trim(), document.styleSheets.length, st.sheet.cssRules.length);
        const an = document.getElementById('a').animate([{ opacity: 0 }, { opacity: 1 }], { duration: 0, fill: 'forwards' });
        an.finished.then(() => console.log('finished', document.getElementById('a').style.opacity, an.playState));
        window.onerror = m => console.log('onerror', m);
        document.getElementById('a').addEventListener('click', () => { throw new Error('boom'); });
        document.getElementById('a').click();
    "#, "t.js").unwrap();
    s.dispatch("tick").unwrap();
    let l = logs(&mut s);
    let l: Vec<&String> = l.iter().filter(|x| !x.starts_with("2:")).collect();
    assert_eq!(l, [
        "0:hi there B", "0:svg 0 0 2 2", "0:true NotFoundError 8 8", "0:onetwo false I", "0:true number",
        "0:true function false null 0", "0:.y { color: red } 1 1", "0:onerror boom",
        "0:decode true true", "0:hevc false", "0:finished 1 finished",
    ]);
}

#[test]
fn current_script_is_set_while_a_script_runs() {
    let mut s = page("<script id=s>1</script>");
    s.eval("console.log(document.getElementById('s')._id)", "t.js").unwrap();
    let id = logs(&mut s)[0].trim_start_matches("0:").to_string();
    s.dispatch(&format!("script\t{id}")).unwrap();
    s.eval("console.log(document.currentScript && document.currentScript.id)", "t.js").unwrap();
    s.dispatch("script\t").unwrap();
    s.eval("console.log(document.currentScript)", "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:s", "0:null"]);
}

#[test]
fn review_fixes_cycles_bound_formatters_shadow_lifecycle() {
    let mut s = page("<div id=a><p id=p></p></div>");
    s.eval(r#"
        new MutationObserver(() => {}).observe(document.body, { childList: true, subtree: true });
        const a = document.getElementById('a'), p = document.getElementById('p');
        try { p.appendChild(a); } catch (e) { console.log(e.name, a.isConnected, a.parentNode === document.body); }
        console.log([1000, 2000].map(new Intl.NumberFormat('en').format).join(' '), [0].map(new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).format)[0]);
        class XKid extends HTMLElement { connectedCallback() { console.log('kid in'); } disconnectedCallback() { console.log('kid out'); } }
        customElements.define('x-kid', XKid);
        const root = a.attachShadow({ mode: 'open' });
        root.innerHTML = '<x-kid></x-kid>';
        root.innerHTML = '';
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:HierarchyRequestError true true", "0:1,000 2,000 1/1/1970", "0:kid in", "0:kid out"]);
}

#[test]
fn scripts_the_page_inserts_run() {
    let mut s = page("<head></head><body><div id=a></div></body>");
    s.eval(r#"
        window.ran = [];
        const inline = document.createElement('script');
        inline.textContent = 'ran.push("inline " + (document.currentScript && document.currentScript.id)); var fromInline = 1;';
        inline.id = 'i';
        document.head.appendChild(inline);
        console.log(ran.join(), typeof fromInline);
        const ext = document.createElement('script');
        ext.src = 'player.js';
        ext.onload = () => console.log('loaded', ran.join('|'), window.player);
        document.head.appendChild(ext);
        const bad = document.createElement('script');
        bad.src = 'missing.js';
        bad.addEventListener('error', () => console.log('error event'));
        document.body.appendChild(bad);
        document.getElementById('a').innerHTML = '<script>ran.push("markup")<\/script>';
        const wrapper = document.createElement('div'), nested = document.createElement('script');
        nested.text = 'ran.push("nested")';
        wrapper.appendChild(nested);
        document.body.appendChild(wrapper);
        document.body.appendChild(inline);
    "#, "t.js").unwrap();
    assert_eq!(logs(&mut s), ["0:inline i number"]);
    let asked = std::mem::take(&mut s.page_mut().classic_scripts);
    assert!(s.page().fetches.is_empty());
    let urls: Vec<&str> = asked.iter().map(|f| f.url.as_str()).collect();
    assert_eq!(urls, ["player.js", "missing.js"]);
    assert!(asked.iter().all(|ask| s.page().classic_script_current(ask)));
    // The browser evaluates a classic resource without inserting its bytes in
    // fetch storage or going through the page's replaceable eval/fetch globals.
    s.eval("globalThis.fetch = () => { throw new Error('fetch hook'); }; globalThis.eval = () => { throw new Error('eval hook'); };", "hooks.js").unwrap();
    s.dispatch(&format!("script\t{}", asked[0].node)).unwrap();
    s.eval("ran.push('ext ' + document.currentScript.src.split('/').pop()); var player = 'ok';", "player.js").unwrap();
    s.dispatch("script\t").unwrap();
    s.dispatch(&format!("scriptend\t{}\t1", asked[0].node)).unwrap();
    s.dispatch(&format!("scriptend\t{}\t0", asked[1].node)).unwrap();
    assert!(s.page().bodies.is_empty());
    s.dispatch("tick").unwrap();
    let l: Vec<String> = logs(&mut s).into_iter().filter(|x| !x.starts_with("1:")).collect();
    assert_eq!(l, ["0:loaded inline i|nested|ext player.js ok", "0:error event"]);
}

#[test]
fn frames_talk_through_post_message() {
    // The page around: its iframe's window is a stand-in that hands messages to the browser.
    let mut top = page("<iframe id=f src='https://www.youtube.com/embed/x'></iframe>");
    top.eval(r#"
        const f = document.getElementById('f');
        window.addEventListener('message', e => console.log('top got', JSON.stringify(e.data), e.origin, e.source === f.contentWindow));
        f.addEventListener('load', () => console.log('frame loaded'));
        f.contentWindow.postMessage({event: 'listening', id: 1}, 'https://www.youtube.com');
        console.log(window.parent === window, window.top === window, document.referrer === '', f.contentDocument);
    "#, "t.js").unwrap();
    let node = top.page().dom.descendants(0).into_iter().find(|&n| top.page().dom.tag(n) == Some("iframe")).unwrap() as i64;
    assert_eq!(std::mem::take(&mut top.page_mut().outbox), [(node, r#"{"event":"listening","id":1}"#.to_string(), "https://www.youtube.com".to_string())]);
    top.dispatch(&format!("message\t{node}\thttps://www.youtube.com\t{{\"event\":\"onReady\"}}")).unwrap();
    top.dispatch(&format!("frameload\t{node}")).unwrap();
    top.dispatch("tick").unwrap();
    assert_eq!(logs(&mut top), ["0:true true true null", "0:top got {\"event\":\"onReady\"} https://www.youtube.com true", "0:frame loaded"]);

    // The page inside: parent, top, referrer and ancestorOrigins come from the browser.
    let dom = sex_html::parse::parse("<p>player</p>");
    let mut doc = Document::new(dom, "https://www.youtube.com/embed/x", (320, 180), clock);
    doc.referrer = "https://example.test/".into();
    doc.parent_origin = Some("https://example.test".into());
    let mut inner = Script::new(doc, Limits::default()).unwrap();
    inner.eval(r#"
        window.onmessage = e => console.log('inner got', e.data.event, e.origin, e.source === window.parent);
        console.log(window.parent !== window, window.top === window.parent, document.referrer, location.ancestorOrigins[0], location.ancestorOrigins.length);
        window.parent.postMessage({event: 'onReady'}, '*');
    "#, "t.js").unwrap();
    assert_eq!(std::mem::take(&mut inner.page_mut().outbox), [(-1, r#"{"event":"onReady"}"#.to_string(), "*".to_string())]);
    inner.dispatch("message\tparent\thttps://example.test\t{\"event\":\"listening\"}").unwrap();
    assert_eq!(logs(&mut inner), ["0:true true https://example.test/ https://example.test 1", "0:inner got listening https://example.test true"]);
}


#[test]
fn classic_script_requests_belong_to_connected_unchanged_document_nodes() {
    let source="var remote=document.createElement('script'); remote.src='https://cdn.test/player.js'; document.head.appendChild(remote);";
    let mut s=page("");s.eval(source,"create.js").unwrap();
    let ask=s.page_mut().classic_scripts.remove(0);
    assert!(s.page().fetches.is_empty());
    assert!(s.page().classic_script_current(&ask));
    s.eval("remote.src='https://other.test/changed.js'", "change.js").unwrap();
    assert!(!s.page().classic_script_current(&ask));
    s.eval("remote.src='https://cdn.test/player.js'; remote.remove()", "remove.js").unwrap();
    assert!(!s.page().classic_script_current(&ask));
    let mut other=page("");other.eval(source,"reload.js").unwrap();
    assert_eq!(other.page().classic_scripts[0].node,ask.node);
    assert!(!other.page().classic_script_current(&ask));
    assert!(other.page().classic_script_current(&other.page().classic_scripts[0]));
    other.eval("remote.type='module'", "type.js").unwrap();
    assert!(!other.page().classic_script_current(&other.page().classic_scripts[0]));
}
