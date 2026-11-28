#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Ship one CI build to the x17 over the tether, unattended: system and apps together.

  tools/x17_release.py --run CI_RUN_ID --version N [--host 192.168.1.177] [--port 8792]

1. Downloads the run's `sexos-image` artifact and takes `iso_root/boot.img` and every
   program in `iso_root/programs/`.
2. Signs the boot image as release N with ~/sexos-release-key/release.key and serves it.
3. Over the Spindle tether (`sexwork/tether`, tether_host.py serve must be running and the
   x17 connected): `update URL`, then `restart`; waits for the x17 to come back and for
   Files to report version N healthy.
4. Writes every program of the same build into the x17's `/bin`, replacing older copies
   (an OTA only replaces the boot slot; `/bin` copies override the program store, so a
   release that skipped this would leave stale apps). Through the Tether app's bridge
   (`tether-bridge call fs_write`) when it is connected, else Spindle's push-file (16 MiB
   at most per file).
"""
import argparse, functools, http.server, json, os, re, shutil, subprocess, sys, tarfile, tempfile, threading, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORK = Path.home() / 'Projects/Sex/sexwork'
TETHER = WORK / 'tether'
BRIDGE = WORK / 'bridge/tether-bridge'
KEY = Path.home() / 'sexos-release-key/release.key'
PUSHFILE_MAX = 16 * 1024 * 1024


def sh(*cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


def tether(*args, timeout=60):
    return subprocess.run([sys.executable, str(TETHER / 'tether_host.py'), '--dir', str(TETHER / 'logs'), *args],
                          capture_output=True, text=True, timeout=timeout)


def state():
    try:
        return json.loads((TETHER / 'logs/state.json').read_text())
    except (OSError, ValueError):
        return {}


def log_text():
    c = TETHER / 'logs/current.log'
    return c.read_text(errors='replace') if c.exists() else ''


def wait_log(pattern, since, timeout):
    end = time.time() + timeout
    while time.time() < end:
        m = re.search(pattern, log_text()[since:])
        if m:
            return m
        time.sleep(2)
    return None


def bridge(method, params, timeout=900):
    r = subprocess.run([str(BRIDGE), 'call', method, json.dumps(params)], capture_output=True, text=True, timeout=timeout)
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {'ok': False, 'error': r.stdout + r.stderr}


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--run', required=True, help='GitHub Actions run id with a sexos-image artifact')
    p.add_argument('--version', required=True, type=int)
    p.add_argument('--host', default='192.168.1.177', help="this Mac's address as the x17 sees it")
    p.add_argument('--port', type=int, default=8792)
    p.add_argument('--skip-update', action='store_true', help='only push the programs')
    a = p.parse_args()

    work = Path(tempfile.mkdtemp(prefix='x17-release-'))
    print(f'[release] work dir {work}', flush=True)
    sh('gh', 'run', 'download', a.run, '-R', 'xirtus/sexos', '-n', 'sexos-image', '-D', str(work))
    sh('tar', '--zstd', '-xf', str(work / 'image.tar.zst'), '-C', str(work), 'iso_root/boot.img', 'iso_root/programs')
    (work / 'image.tar.zst').unlink()
    ota = work / 'ota'
    ota.mkdir()
    sh('cargo', 'run', '-q', '--release', '--manifest-path', str(ROOT / 'tools/sexos-release/Cargo.toml'), '--',
       'sign', str(KEY), str(a.version), str(work / 'iso_root/boot.img'), str(ota))
    shutil.copyfile(work / 'iso_root/boot.img', ota / 'sexos-boot.img')
    print('[release] ' + (ota / 'sexos-update.txt').read_text().replace('\n', ' '), flush=True)

    if not a.skip_update:
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(ota))
        server = http.server.ThreadingHTTPServer(('0.0.0.0', a.port), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        if not state().get('connected'):
            sys.exit('[release] the x17 is not on the Spindle tether (tether HOST in Spindle)')
        since = len(log_text())
        tether('send', f'update http://{a.host}:{a.port}/')
        m = wait_log(r'Version \d+ is in system slot \w|Not updated[^\n]*|update: [^\n]*|Up to date[^\n]*', since, 1800)
        if not m or 'is in system slot' not in m[0]:
            sys.exit(f'[release] update failed: {m[0] if m else "no answer in 30 minutes"}')
        print(f'[release] {m[0]}', flush=True)
        since = len(log_text())
        tether('send', 'restart')
        print('[release] restarting; waiting for the x17 to come back on the tether', flush=True)
        end = time.time() + 900
        while time.time() < end and not re.search(rf'\[files\.update\.healthy\] slot=\w trial=None version={a.version} ', log_text()):
            time.sleep(5)
        if not re.search(rf'\[files\.update\.healthy\] slot=\w trial=None version={a.version} ', log_text()):
            sys.exit('[release] version did not come up healthy within 15 minutes; the next start falls back by itself')
        print(f'[release] version {a.version} is running and accepted', flush=True)
        server.shutdown()

    programs = sorted((work / 'iso_root/programs').iterdir())
    use_bridge = BRIDGE.exists() and bridge('status', {}).get('result', {}).get('connected')
    print(f'[release] writing {len(programs)} programs to /bin via {"the Tether bridge" if use_bridge else "Spindle push-file"}', flush=True)
    failed = []
    for prog in programs:
        if not prog.is_file():
            continue
        if use_bridge:
            r = bridge('fs_write', {'path': f'/bin/{prog.name}', 'local_path': str(prog)})
            ok = r.get('ok') is True
        elif prog.stat().st_size <= PUSHFILE_MAX:
            r = tether('push-file', str(prog), f'/bin/{prog.name}', '--wait', '300', timeout=330)
            ok = r.returncode == 0
        else:
            ok, r = False, 'too large for push-file; open the Tether app and run again with --skip-update'
        print(f'[release]   {prog.name}: {"ok" if ok else "FAILED " + str(r)[:200]}', flush=True)
        if not ok:
            failed.append(prog.name)
    shutil.rmtree(work, ignore_errors=True)
    if failed:
        sys.exit(f'[release] programs not written: {", ".join(failed)}')
    print('[release] done: system and every app are from one build', flush=True)


if __name__ == '__main__':
    main()
