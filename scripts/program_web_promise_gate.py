#!/usr/bin/env python3
"""Native QuickJS rejection identity, checkpoint/task ordering, muting and VM lifetime.
Two genuine HTTP origins; no Promise replacement, fetch/eval hook or source rewrite.
"""
import argparse, hashlib, http.server, json, os, re, shutil, threading, time
from pathlib import Path
from quil_autosave_gate import boot
ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser()
    for n in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + n, required=True, type=Path)
    p.add_argument('--program', type=Path, default=ROOT / 'iso_root/programs/web')
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    seen, servers = [], []

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            seen.append(dict(port=self.server.server_address[1], path=self.path,
                             origin=self.headers.get('Origin'), referer=self.headers.get('Referer')))
            mime, extra = 'text/html', []
            if self.path == '/web':
                body, mime = a.program.read_bytes(), 'application/octet-stream'
            elif self.path == '/page':
                body = ('''<!doctype html><title>Native Promise events</title><h1>Native Promise events</h1><script>
                  const nativePromise=Promise;var initialized=false,reason={identity:1};
                  var initializer=Promise.resolve().then(()=>{initialized=true;throw reason});
                  var late=Promise.reject('gate-late');
                  onunhandledrejection=e=>{
                    if(e.reason===reason)console.log('promise initializer',e.promise===initializer,e.isTrusted,e.cancelable);
                    else if(e.reason==='gate-late'){console.log('promise late',e.promise===late);setTimeout(()=>late.catch(()=>{}),0);}
                    else if(e.reason==='gate-default'){console.log('promise default',e.defaultPrevented);return;}
                    else if(e.reason==='owned timer'||e.reason==='owned async'||e.reason==='readable resource'||e.reason==='owned thenable'||e.reason==='owned getter')console.log('promise owned',e.reason);
                    else console.log('FAIL parent rejection',e.reason);
                    e.preventDefault();
                  };
                  onrejectionhandled=e=>console.log('promise handled',e.promise===late,e.reason==='gate-late',e.cancelable,e.isTrusted);
                  Promise.reject('gate-default');Promise.reject('sync caught').catch(()=>{});
                  var micro=Promise.reject('micro caught');queueMicrotask(()=>micro.catch(()=>{}));
                  setTimeout(()=>Promise.reject('owned timer'),0);(async()=>{await 0;throw 'owned async'})();
                  console.log('promise body',initialized,Promise===nativePromise,typeof PromiseRejectionEvent);
                </script><script>
                  console.log('promise following',initialized);
                  window.addEventListener('message',e=>{if(e.data==='frame isolated'){console.log('promise frame message',e.origin===''' + json.dumps(bbase) + ''');document.getElementById('owner-frame').remove();console.log('promise frame removed');}});
                  function resource(path,cors){const s=document.createElement('script');s.src=''' + json.dumps(bbase) + '''+path;if(cors)s.crossOrigin='anonymous';s.onload=()=>{console.log('promise resource loaded',path);if(path==='/opaque.js'){for(const v of [opaqueThenable,opaqueGetter,opaqueProxy]){Promise.resolve(v);new Promise(r=>r(v));(async()=>{await v})();(async function*(){await v})().next();}queueMicrotask(opaqueBoundNative);setTimeout(opaqueBoundNative,0);Promise.resolve().finally(opaqueFinally);new Promise(opaqueExecutor);(async()=>{await opaqueCtorPromise})();(async function*(){await opaqueCtorPromise})().next();(async function*(){})().return(opaqueCtorPromise);const pending=Promise.resolve(opaqueThenable);pending.then();pending.then().then();Promise.resolve({then(){throw 'owned thenable'}});Promise.resolve({get then(){throw 'owned getter'}});}};s.onerror=()=>console.log('FAIL resource',path);document.head.appendChild(s);}
                  resource('/opaque.js',false);resource('/readable.js',true);
                </script><iframe id="owner-frame" src="''' + bbase + '''/frame"></iframe>''').encode()
            elif self.path == '/opaque.js':
                body, mime = b"Promise.reject('opaque immediate');setTimeout(()=>Promise.reject('opaque timer'),0);(async()=>{await 0;throw 'opaque async'})();globalThis.opaqueThenable={then(){throw 'opaque thenable'}};globalThis.opaqueGetter={get then(){throw 'opaque getter'}};globalThis.opaqueProxy=new Proxy({}, {get(){throw 'opaque proxy'}});globalThis.opaqueBoundNative=Promise.reject.bind(Promise,'opaque native bound');globalThis.opaqueFinally=()=>{throw 'opaque finally'};globalThis.opaqueExecutor=()=>{throw 'opaque executor'};globalThis.opaqueCtorPromise=Promise.resolve(1);Object.defineProperty(opaqueCtorPromise,'constructor',{get(){throw 'opaque constructor'}});", 'application/javascript'
            elif self.path == '/readable.js':
                body, mime = b"Promise.reject('readable resource');", 'application/javascript'
                extra = [('Access-Control-Allow-Origin', abase)]
            elif self.path == '/frame':
                body = b"""<!doctype html><script>const p=Promise.reject('frame private');onunhandledrejection=e=>{e.preventDefault();if(e.reason!=='frame private')console.log('FAIL destroyed frame',e.reason);else {console.log('promise frame isolated',e.promise===p,e.reason==='frame private');parent.postMessage('frame isolated','*')}};setTimeout(()=>Promise.reject('destroyed frame timer'),2000);</script>"""
            elif self.path == '/cancel':
                body = b"""<!doctype html><script>onunhandledrejection=e=>console.log('FAIL old document',e.reason);setTimeout(()=>Promise.reject('old timer'),2000);location.href='/next';</script>"""
            elif self.path == '/next':
                body = b"""<!doctype html><script>onunhandledrejection=e=>{e.preventDefault();console.log('promise replacement',e.reason)};Promise.reject('new document');</script>"""
            else:
                self.send_response(404); self.end_headers(); return
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(body)))
            for k, v in extra:
                self.send_header(k, v)
            self.end_headers(); self.wfile.write(body)

        def log_message(self, *args):
            pass

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True

    for _ in range(2):
        server = Server(('127.0.0.1', 0), Handler)
        servers.append(server)
        threading.Thread(target=server.serve_forever, daemon=True).start()
    abase, bbase = [f'http://10.0.2.2:{s.server_address[1]}' for s in servers]
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024**3)
    os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
    os.environ.setdefault('SEXOS_GATE_BOUND', '900')
    g = boot(a.iso.resolve(), disk, a.out, 'promise', a.resolution)
    q, t = g['q'], g['text']

    def command(line):
        g['focus'](153); time.sleep(.3)
        for ch in line:
            if ch in ':_+':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'},
                      {'type': 'qcode', 'data': {':': 'semicolon', '_': 'minus', '+': 'equal'}[ch]}], 'hold-time': 80})
            else:
                q.key({' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}.get(ch, ch))
            time.sleep(.04)
        q.key('ret')

    def close(start):
        ready = g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)\r?\n', start, 120)
        target, task = ready[1], ready[2]
        surface = g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)\r?\n', start, 60)[1]
        g['focus'](int(surface)); q.key('f11')
        g['wait'](rf'\[web.closed\] target={target} task={task}\r?\n', start, 60)
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ', start, 120)

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        start = len(t()); command(f'fetch {abase}/web /bin/web')
        saved = g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)', start, 240)
        assert saved[1].startswith('Saved'), saved[1]
        start = len(t()); command(f'run +net web {abase}/page')
        for mark in ('promise body false true function', 'promise following true',
                     'promise initializer true true true', 'promise late true',
                     'promise handled true true false true', 'promise default false',
                     'promise owned owned timer', 'promise owned owned async',
                     'promise owned readable resource', 'promise owned owned thenable', 'promise owned owned getter', 'promise frame isolated true true',
                     'promise frame message true', 'promise frame removed', 'promise resource loaded /opaque.js',
                     'promise resource loaded /readable.js'):
            g['wait'](r'\[web.console\] level=0 ' + re.escape(mark) + r'\r?\n', start, 300)
        assert '[web.console] level=2 Unhandled promise rejection: gate-default\n' in t()[start:].replace('\r', '')
        time.sleep(3)
        assert '[web.console] level=0 FAIL' not in t()[start:]
        assert 'opaque immediate' not in t()[start:] and 'opaque timer' not in t()[start:] and 'opaque async' not in t()[start:]
        readable = [r for r in seen if r['path'] == '/readable.js']
        opaque = [r for r in seen if r['path'] == '/opaque.js']
        assert len(readable) == len(opaque) == 1, (readable, opaque)
        assert readable[0]['origin'] == abase and opaque[0]['origin'] is None
        assert readable[0]['referer'] == opaque[0]['referer'] == abase + '/'
        close(start)
        start = len(t()); command(f'run +net web {abase}/cancel')
        g['wait'](r'\[web.console\] level=0 promise replacement new document\r?\n', start, 300)
        time.sleep(3)
        assert '[web.console] level=0 FAIL' not in t()[start:]
        close(start)
        assert '[web.fatal]' not in t() and 'KERNEL PANIC' not in t()
        proof = dict(ok=True, iso_sha256=hashlib.sha256(a.iso.read_bytes()).hexdigest(),
                     program_sha256=hashlib.sha256(a.program.read_bytes()).hexdigest(), requests=seen)
        (a.out / 'result.json').write_text(json.dumps(proof, indent=2) + '\n')
        print(proof)
    finally:
        g['stop']()
        for server in servers:
            server.shutdown(); server.server_close()


if __name__ == '__main__':
    main()
