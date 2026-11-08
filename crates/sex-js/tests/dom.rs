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
