#!/usr/bin/env python3
"""Separate normal-image cold recovery of an accepted pending-I/O Close session.

Never builds. --check only reads sealed artifacts; no guest or device access.
"""
import argparse
import gzip
import hashlib
import json
import math
import re
import subprocess
import threading
import time
from pathlib import Path

import alpha_evidence as alpha
from check_document_disk import recover, fnv
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from https_asset_gate import script_dependencies
from pending_io_close import BASELINE, SNAPSHOT
from quil_third_probe import open_selected

MAX_SECONDS = 420
MAX_SERIAL = 64 * 1024 * 1024


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def contract(root):
    c = alpha.contract(root)
    names = script_dependencies(root, ('scripts/pending_io_cold_gate.py',
                                       'scripts/tests/test_pending_io_cold_gate.py'))
    c['source_sha256'].update({p: alpha.sha(Path(root) / p) for p in names})
    return c


def accepted_session(root, iso, binding, session):
    alpha.replay(root, iso, binding, session)
    session = Path(session).resolve()
    manifest = json.loads((session / 'manifest.json').read_text())
    result = json.loads((session / 'result.json').read_text())
    pending = result.get('pending_io_close')
    if not manifest['options'].get('pending_io_close') or not pending:
        raise ValueError('accepted live pending-I/O component required')
    seed = gzip.decompress((session / 'nvme.img.gz').read_bytes())
    if digest(seed) != manifest['final_disk_sha256']:
        raise ValueError('live terminal seed hash differs')
    document = pending['document']
    found = recover(seed, document)
    payload = found.pop('payload')
    if payload not in (BASELINE, SNAPSHOT) or found != pending['terminal_document']:
        raise ValueError('live terminal document is not exact baseline or snapshot')
    chain = dict(session=str(session), manifest_sha256=alpha.sha(session / 'manifest.json'),
                 result_sha256=alpha.sha(session / 'result.json'), seed_sha256=digest(seed),
                 document=document, recovered=found, bytes=len(payload), hash=hex(fnv(payload)))
    return seed, payload, chain


def native_load(text, start, end, document, payload, expected_recovered):
    if type(start) is not int or type(end) is not int or not 0 <= start < end <= len(text):
        raise ValueError('cold Open offsets outside serial log')
    if not re.search(r'\[quil.incarnation.start\] target=9 task=9 surface=156 fresh=1', text[:start]):
        raise ValueError('cold Open lacks fresh Quil9 incarnation')
    if re.search(r'\[quil.incarnation.start\] target=(?:16|18) |\[task.finish\] pd=9 ', text[:end]):
        raise ValueError('cold Open is not the fresh primary Quil')
    span = text[start:end]
    patterns = (
        ('[linen.open.queued]', r'\[linen.open.queued\] id=(\d+) accepted=1(?=\r?\n|$)'),
        ('[linen.quil.disk_doc.intent]', r'\[linen.quil.disk_doc.intent\] object_id=(\d+) path_id=(\d+) ok=1(?=\r?\n|$)'),
        ('[linen.open.complete]', r'\[linen.open.complete\] status=0(?=\r?\n|$)'),
        ('[files.document.recovered]', r'\[files.document.recovered\] object=(\d+) identity=(\d+) generation=(\d+) bank=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+)(?=\r?\n|$)'),
        ('[quil.persist.load.ok]', r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)(?=\r?\n|$)'),
    )
    records = []
    for tag, pattern in patterns:
        matches = list(re.finditer(pattern, span))
        if span.count(tag) != 1 or len(matches) != 1:
            raise ValueError('cold Open requires one exact native ' + tag)
        records.append(matches[0])
    queued, intent, complete, recovered, load = records
    if int(queued[1]) != int(intent[1]) or int(intent[2]) != document:
        raise ValueError('cold Open row/path identity differs')
    if not queued.start() < intent.start() < complete.start() or not intent.start() < load.start():
        raise ValueError('cold native receipts precede actual Open/intent')
    observed_recovered = dict(zip(('identity', 'generation', 'bank', 'bytes'), map(int, recovered.groups()[1:5])))
    observed_recovered['hash'] = recovered[6]
    if int(recovered[1]) != document or observed_recovered != expected_recovered:
        raise ValueError('cold provider journal metadata differs from terminal seed')
    if not intent.start() < recovered.start() < load.start():
        raise ValueError('cold provider recovery must precede Quil load after Open intent')
    if tuple(load.groups()) != (str(len(payload)), hex(fnv(payload)), str(document)):
        raise ValueError('cold native load differs from terminal seed')
    return dict(open_start=start, open_end=end, load_offset=start + load.start(),
                object=document, bytes=len(payload), hash=hex(fnv(payload)), target=9)


def capture_native_load(reader, start, document, payload, expected_recovered):
    # Serial output continues while the guest runs. Bind both offsets and
    # receipt parsing to one immutable read, never to a later file length.
    snapshot = reader()
    return native_load(snapshot, start, len(snapshot), document, payload, expected_recovered)


def unchanged_document(disk, document, payload, expected_recovered):
    found = recover(disk, document)
    actual_payload = found.pop('payload')
    if actual_payload != payload or found != expected_recovered:
        raise ValueError('cold final durable document metadata/payload differs')
    return found


def guest_command(iso, out, resolution, model):
    sock = Path('/tmp') / ('sex-pending-cold-' + digest(str(out).encode())[:16] + '.sock')
    w, h = resolution.split('x')
    return ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku', '-rtc', alpha.RTC,
            '-m', '512M', '-vga', 'none', '-device', f'VGA,xres={w},yres={h}',
            '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
            '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={out / "nvme.img"},format=raw',
            '-device', 'nvme,serial=sexos01,drive=nvm', '-serial', f'file:{out / "serial.log"}',
            '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none', '-no-reboot',
            '-no-shutdown', '-nic', 'none', '-netdev', 'user,id=peer', '-device',
            model + ',netdev=peer,mac=52:54:00:12:34:56'], sock


def raw_files(out):
    return {str(p.relative_to(out)): alpha.sha(p) for p in out.rglob('*')
            if p.is_file() and p.name not in ('manifest.json', 'verification.json', 'nvme.img')}


def replay(root, iso, binding, session, out):
    seed, payload, chain = accepted_session(root, iso, binding, session)
    c = contract(root)
    alpha.require_inputs(root, binding, c)
    out = Path(out).resolve()
    m = json.loads((out / 'manifest.json').read_text())
    result = json.loads((out / 'result.json').read_text())
    if m.get('error') is not None or result.get('ok') is not True or m.get('stopped') is not True:
        raise ValueError('failed or live cold attempt remains diagnostic')
    if m['chain'] != chain or m['source_sha256'] != c['source_sha256'] or m['binding'] != alpha.frozen_binding(root, iso, binding):
        raise ValueError('cold source/image/live-session chain differs')
    if m['resolution'] not in ('1280x800', '1024x768') or m['model'] not in ('e1000', 'e1000e'):
        raise ValueError('unknown cold platform profile')
    command, _ = guest_command(iso, out, m['resolution'], m['model'])
    if command != m['command'] or m['raw_sha256'] != raw_files(out):
        raise ValueError('cold command or immutable raw closure differs')
    for name in m['raw_sha256']:
        p = out / name
        if p.is_symlink() or not p.resolve().is_relative_to(out):
            raise ValueError('unsafe cold artifact')
    if gzip.decompress((out / 'seed.img.gz').read_bytes()) != seed:
        raise ValueError('cold boot seed is not exact terminal live bytes')
    final = gzip.decompress((out / 'nvme.img.gz').read_bytes())
    if digest(final) != m['final_disk_sha256']:
        raise ValueError('cold final disk hash differs')
    unchanged_document(final, chain['document'], payload, chain['recovered'])
    text = alpha.serial_text(out / 'serial.log')
    if len(text.encode()) > MAX_SERIAL or re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow|\[fault.kill\]|\[sexfiles.defer.drop\]', text):
        raise ValueError('cold guest/log failure')
    load = result['load']
    if load != native_load(text, load['open_start'], load['open_end'], chain['document'], payload, chain['recovered']):
        raise ValueError('cold native load receipt differs')
    index = json.loads((out / 'captures.json').read_text())
    if index.get('version') != alpha.VERSION or index.get('complete') is not True or index.get('serial_offset_units') != alpha.SERIAL_UNITS or len(index['frames']) != 1:
        raise ValueError('cold immutable frame registry incomplete')
    frame = index['frames'][0]
    if frame != result['frame'] or (frame['sequence'], frame['name'], frame['kind'], frame['target'], frame['expected'], frame['region']) != (1, 'pending-cold-recovered', 'quil', 9, 'pendclose', None):
        raise ValueError('cold frame identity differs')
    if not load['open_end'] <= frame['serial_end'] <= len(text) or frame['source_sha256'] != c['source_sha256']:
        raise ValueError('cold frame predates load or has foreign source')
    prefix = text[:frame['serial_end']]
    screenshot = out / frame['screenshot']
    if screenshot.is_symlink() or not screenshot.resolve().is_relative_to(out) or digest(prefix.encode()) != frame['serial_sha256'] or alpha.sha(screenshot) != frame['screenshot_sha256']:
        raise ValueError('cold screenshot/prefix changed')
    proof = alpha.verify_frame(prefix, screenshot.read_bytes(), c, tuple(map(int, m['resolution'].split('x'))), 'quil', 'pendclose', 9)
    if not proof['ok'] or proof != frame['proof']:
        raise ValueError('cold full glyph/clock proof differs')
    from clock_source_evidence import validate_clock_sources
    for source in validate_clock_sources(text):
        if not math.floor(m['host_utc_start']) - 1 <= int(source['seconds']) <= math.ceil(m['host_utc_end']) + 1:
            raise ValueError('cold clock outside actual host bracket')
    if not 0 < m['elapsed_seconds'] <= MAX_SECONDS + 15:
        raise ValueError('cold process exceeded bound')
    return dict(ok=True, cold_recovery_verified=True, chain=chain, load=load, frame=frame,
                scope='Separate normal-image cold boot, exact live terminal disk seed, native Linen Open/Quil9 load and full Quil/civil-clock pixels. No rollback or general crash-consistency claim.')


def run(root, iso, binding, session, out, resolution, model):
    seed, payload, chain = accepted_session(root, iso, binding, session)
    c = contract(root)
    alpha.require_inputs(root, binding, c)
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise ValueError('fresh cold output directory required')
    disk = out / 'nvme.img'
    disk.write_bytes(seed)
    (out / 'seed.img.gz').write_bytes(gzip.compress(seed, mtime=0))
    command, sock = guest_command(iso, out, resolution, model)
    m = dict(chain=chain, source_sha256=c['source_sha256'], binding=alpha.frozen_binding(root, iso, binding),
             command=command, resolution=resolution, model=model, host_utc_start=time.time(), error=None, stopped=False)
    (out / 'manifest.json').write_text(json.dumps(m, indent=2) + '\n')
    captures = alpha.Captures(out, c, tuple(map(int, resolution.split('x'))))
    proc = None
    watchdog = None
    q = None
    began = time.monotonic()
    log = out / 'serial.log'
    def text():
        return alpha.serial_text(log) if log.exists() else ''
    def health():
        if proc.poll() is not None or time.monotonic() - began >= MAX_SECONDS:
            raise RuntimeError('cold guest exited or timed out')
        if log.exists() and log.stat().st_size > MAX_SERIAL:
            raise RuntimeError('cold serial limit exceeded')
        if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', text()):
            raise RuntimeError('cold guest fault')
    def wait(pattern, start=0, timeout=90):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            health()
            match = re.search(pattern, text()[start:])
            if match:
                return match
            time.sleep(.05)
        raise RuntimeError('cold wait timed out: ' + pattern)
    def key(k, pattern):
        start = len(text())
        q.key_sequence([k])
        return wait(pattern, start)
    def focus(sid):
        return focus_surface(sid, text, key)
    try:
        with (out / 'qemu.log').open('wb') as output:
            proc = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        def expire():
            if proc.poll() is None:
                proc.kill()
        watchdog = threading.Timer(MAX_SECONDS, expire)
        watchdog.daemon = True
        watchdog.start()
        until = time.monotonic() + 15
        while not sock.exists():
            health()
            if time.monotonic() > until:
                raise RuntimeError('cold QMP unavailable')
            time.sleep(.05)
        q = QMP(sock)
        if resolution == '1024x768':
            wait(r'SexOS compact 1024x768', timeout=10)
            q.key('down')
            q.key('ret')
        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[linen.disk.publish.done\]')
        for _ in range(20):
            focus(200)
            key('j', r'\[shell.action.select_next_linen\] id=\d+')
            start = len(text())
            got = open_selected(dict(q=q, wait=wait, text=text))
            if got and got['object'] == chain['document']:
                load = capture_native_load(text, start, chain['document'], payload, chain['recovered'])
                break
        else:
            raise RuntimeError('cold document never selected through Linen')
        focus(201)
        candidate = captures.candidate('pending-cold-recovered')
        until = time.monotonic() + 60
        while time.monotonic() < until:
            health()
            q.cmd('screendump', dict(filename=str(candidate)))
            prefix = text()
            pixels = candidate.read_bytes()
            try:
                proof = alpha.verify_frame(prefix, pixels, c, captures.resolution, 'quil', 'pendclose', 9)
            except ValueError:
                proof = dict(ok=False)
            if proof['ok']:
                frame = captures.commit('pending-cold-recovered', candidate, prefix, proof, 'quil', 'pendclose', 9, None, pixels)
                break
            time.sleep(.15)
        else:
            raise RuntimeError('cold full Quil/clock pixels did not settle')
        (out / 'result.json').write_text(json.dumps(dict(ok=True, load=load, frame=frame), indent=2) + '\n')
    except BaseException as error:
        m['error'] = type(error).__name__ + ': ' + str(error)
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=m['error'])) + '\n')
        raise
    finally:
        if proc is not None:
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
            m['stopped'] = proc.poll() is not None
        if watchdog is not None:
            watchdog.cancel()
        if q is not None:
            q.file.close()
            q.sock.close()
        sock.unlink(missing_ok=True)
        # No reads/hashes of the final disk until the sole guest writer is gone.
        final = disk.read_bytes()
        (out / 'nvme.img.gz').write_bytes(gzip.compress(final, mtime=0))
        m.update(final_disk_sha256=digest(final), host_utc_end=time.time(), elapsed_seconds=time.monotonic() - began)
        captures.write(m['error'] is None and m['stopped'])
        m['raw_sha256'] = raw_files(out)
        (out / 'manifest.json').write_text(json.dumps(m, indent=2) + '\n')
    return replay(root, iso, binding, session, out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'binding', 'session', 'out'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--resolution', choices=('1280x800', '1024x768'), default='1280x800')
    parser.add_argument('--model', choices=('e1000', 'e1000e'), default='e1000e')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[1]
    values = (root, args.iso.resolve(strict=True), args.binding.resolve(strict=True),
              args.session.resolve(strict=True), args.out.resolve())
    result = replay(*values) if args.check else run(*values, args.resolution, args.model)
    print(json.dumps(result, indent=2))
    return result


if __name__ == '__main__':
    main()
