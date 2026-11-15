#!/usr/bin/env python3
"""Actual Moan offline controls qualification; no live/hostile sandbox claim."""
import hashlib
import struct

FRAMES = 48000


def expected_pcm(swapped=False):
    data = bytearray()
    for n in range(FRAMES):
        pair = (((n % 127) - 63) * 128, ((n % 97) - 48) * 96)
        data.extend(struct.pack('<hh', *(pair[::-1] if swapped else pair)))
    return bytes(data)


def project_oracle(data, channels=(False, True, False, False)):
    """Parse the actual saved archive independently; verify every PCM sample.

    The fixture imports asymmetric stereo. CLAP swap must be non-bypass, crash
    and cancellation add nothing, recovery swaps back, VST3 default unity is
    tested only as UI/format plumbing (the parameterized native proof is separate).
    """
    assert 64 <= len(data) <= 16 * 1024 * 1024 and data[:8] == b'MOANPRJ\0'
    version, flags, rate = struct.unpack_from('<HHI', data, 8)
    assert version in (1, 2, 3) and flags == 0 and rate == 48000
    assert struct.unpack_from('<Q', data, 16)[0] == len(data)
    checksum = 0xcbf29ce484222325
    for byte in data[:24] + data[32:]:
        checksum = ((checksum ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    assert checksum == struct.unpack_from('<Q', data, 24)[0]
    sources, tracks, selected, master = struct.unpack_from('<HHHH', data, 32)
    assert sources == tracks == len(channels) and selected == tracks - 1
    assert master == (100 if version == 3 else 0)
    assert struct.unpack_from('<QQQ', data, 40) == (0, 0, FRAMES)
    at = 64
    hashes = []
    for swapped in channels:
        assert struct.unpack_from('<I', data, at)[0] == FRAMES
        at += 4
        pcm = data[at:at + FRAMES * 4]
        assert pcm == expected_pcm(swapped), 'wrong/stale/bypass rendered PCM or changed original'
        hashes.append(hashlib.sha256(pcm).hexdigest())
        at += FRAMES * 4
        if version >= 2:
            assert data[at:at + 4] == bytes(4), 'unexpected MIDI source'
            at += 4
    for source in range(tracks):
        assert len(data) - at >= 62
        # Exactly one ordinary track/region, unity volume, pan zero, no FX.
        assert struct.unpack_from('<HHhBB', data, at) == (1, 100, 0, 0, 0)
        assert data[at + 8:at + 30] == bytes(22)
        at += 30
        assert struct.unpack_from('<HHQIIHHII', data, at) == (source, 0, 0, 0, FRAMES, 100, 0, 0, 0)
        at += 32
    assert at == len(data), 'trailing archive bytes'
    return dict(ok=True, tracks=tracks, frames=FRAMES, pcm_sha256=hashes,
                original_preserved=True, clap_non_bypass_swap=True,
                vst3_default_unity_plumbing=True)


def main():
    import argparse
    import importlib.util
    import json
    import os
    from pathlib import Path
    import re
    import shutil
    import subprocess
    import sys
    import time
    own = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out', 'app-root'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--seed-elf', type=Path, default=own / 'target/x86_64-sex-app/release/moanrack')
    args = parser.parse_args()
    app_root = args.app_root.resolve()
    # Reuse the exact workstation input/geometry helpers of the staged app cohort.
    sys.path.insert(0, str(app_root / 'scripts'))
    from install_program import install
    from quil_autosave_gate import boot
    from moan_workstation_gate import Probe
    from moan_gate_geometry import ready, refresh, origin
    from program_egui_gate import client_rect, colours
    spec = importlib.util.spec_from_file_location('plugin_strip_oracle', own / 'scripts/moan_plugin_pixels.py')
    pixels = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(pixels)
    pattern = re.compile(pixels.PAINT.pattern.replace('moanrack.paint', 'moan.plugin.paint'), re.M)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    assert not any(out.iterdir()), 'fresh evidence directory required'
    sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    programs = [(n, app_root / 'iso_root/programs' / n) for n in ('moan', 'moanplug', 'loom')]
    programs.append(('moanrack', args.seed_elf.resolve()))
    inputs = {str(p): sha(p) for p in [args.iso, args.source_disk, *[p for _, p in programs]]}
    for root in (own, app_root):
        for directory in ('apps/moan', 'apps/moanrack', 'apps/moanplug', 'crates/moan-plugin', 'crates/moan-core', 'crates/serenade-core', 'crates/weft', 'crates/sex-app', 'crates/sex-pdx', 'scripts'):
            for path in sorted((root / directory).rglob('*')):
                if path.is_file() and '__pycache__' not in path.parts and ('target' not in path.relative_to(root / directory).parts):
                    inputs[str(path)] = sha(path)
    (out / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
    for label, root in (('runner', own), ('app', app_root)):
        (out / (label + '-source-commit.txt')).write_text(subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True))
    (out / 'programs').mkdir()
    for name, path in programs:
        shutil.copyfile(path, out / 'programs' / name)
    disk = out / 'nvme.img'
    shutil.copyfile(args.source_disk, disk)
    install(disk, [(name, path.read_bytes()) for name, path in programs])
    os.environ['SEXOS_GATE_BOUND'] = '1600'
    g = boot(args.iso, disk, out, 'moan-offline-ui', '1280x800')
    shots = []
    try:
        c = Probe(g, out, '1280x800')
        text, q = g['text'], g['q']
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 180)
        seeded = c.run('run moanrack @/ --seed-moan-ui')
        g['wait'](r'\[moanrack.ui.seed\] ok=1 frames=48000 fixtures=4 developer_only=1', seeded, 180)
        child = g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) ', seeded, 120)
        g['wait'](rf'\[app.private.retired\] target={child[1]} task={child[2]} ', seeded, 120)
        start, ft, ftask, fsurface = c.launch('files', 'files')
        g['wait'](r'\[files.app.ready\].*portal=1', start, 180)
        start, target, task, surface = c.launch('moan')
        g['wait'](r'\[moan.ready\]', start, 180)
        ready(g, surface)

        def select_file(filename, since):
            g['wait'](r'\[files.app.question\] program=moan mode=Open', since, 120)
            c.focus(fsurface)
            c.chord('ctrl', 'h')
            c.chord('ctrl', 'f')
            c.type(filename)
            q.key('ret')
            g['wait'](rf'\[files.app.found\] path={re.escape(filename)} by=name', since, 120)
            g['wait'](rf'\[files.app.search\] base=/ query={re.escape(filename)} results=1 .* finished=1', since, 120)
            q.key('home')
            q.key('ret')
            g['wait'](rf'\[files.app.found.opened\] path=/{re.escape(filename)}', since, 120)
            q.key('ret')
            g['wait'](rf'\[files.pick.granted\] row=\d+ path=/{re.escape(filename)}', since, 120)
            c.focus(surface)

        def choose_plugin(filename):
            since = len(text())
            c.click(surface, 'Choose_plugin')
            select_file(filename, since)

        def shot(name, expected):
            since, tile, size = refresh(g, surface)
            client = client_rect(tile, *size)
            # The fourth region lowers the status strip to the client bottom.
            # Park on the global bar so the hardware cursor cannot cover any
            # checked app pixels; keep the complete status-strip RGB oracle.
            assert client[1] > 40, 'cursor parking must be outside app pixels'
            parked = len(text())
            # Shell deliberately rejects large jumps into the top-left corner.
            # Use the middle of the bar and require its actual acceptance.
            c.move(320, 10)
            g['wait'](r'\[shell.abs.normalize\] raw_x=\d+ raw_y=\d+ sx=(?:319|320) sy=(?:9|10) accepted=1 reason=ok', parked, 120)
            g['wait'](rf'\[moan.plugin.paint\] target={target} task={task} surface={surface} .* text_hex={expected.encode().hex()}\r?\n', since, 120)
            g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w={size[0]} h={size[1]}\r?\n', since, 120)
            deadline = time.monotonic() + 15
            attempts = []
            for n in range(64):
                path = out / f'{name}-attempt-{n+1:03d}.ppm'
                q.cmd('screendump', {'filename': str(path)})
                try:
                    view = pixels.model_view(text(), target, task, surface, size, expected, since, pattern=pattern)
                    proof = pixels.visible_pixels(path.read_bytes(), client, view)
                    assert len(colours(path.read_bytes(), client)) > 8
                    attempts.append(dict(file=path.name, ok=True, proof=proof))
                    final = out / f'{name}.ppm'
                    shutil.copyfile(path, final)
                    shots.append(dict(file=final.name, sha256=sha(final), proof=proof))
                    break
                except (ValueError, AssertionError) as error:
                    attempts.append(dict(file=path.name, ok=False, error=str(error)))
                if time.monotonic() >= deadline:
                    break
                time.sleep(.1)
            (out / f'{name}-captures.json').write_text(json.dumps(attempts, indent=2) + '\n')
            assert attempts[-1]['ok'], (name, attempts)

        start = c.command(surface, 'ctrl', 'o')
        select_file('source.wav', start)
        # Import emits selection/control state; the read-only layout command
        # emits the aggregate project receipt after the completed import.
        imported, _, _ = refresh(g, surface)
        g['wait'](r'\[moan.project\] tracks=1 frames=48000 ', imported, 120)
        c.click(surface, 'Instruments')
        choose_plugin('swap.clap')
        start = len(text())
        c.click(surface, 'Render_folder')
        g['wait'](r'\[files.app.question\] program=moan mode=Folder', start, 120)
        c.focus(fsurface)
        c.chord('ctrl', 'h')
        c.chord('ctrl', 'f')
        c.type('renders')
        q.key('ret')
        g['wait'](r'\[files.app.found\] path=renders by=name', start, 120)
        g['wait'](r'\[files.app.search\] base=/ query=renders results=1 .* finished=1', start, 120)
        q.key('home')
        q.key('ret')
        g['wait'](r'\[files.app.list\] dir=/renders count=0', start, 120)
        c.chord('ctrl', 's')
        g['wait'](r'\[files.pick.granted\] row=\d+ path=/renders\r?\n', start, 120)
        c.focus(surface)

        children = []
        def begin():
            since = len(text())
            c.click(surface, 'Offline_render')
            g['wait'](r'\[moan.plugin.started\]', since, 120)
            m = g['wait'](rf'\[shell.launch.event\] parent={target} event=Started \{{ cookie: (\d+), row: (\d+), task: (\d+) \}} sent=1', since, 120)
            children.append(tuple(map(int, m.groups())))
            return since, children[-1]

        start, child = begin()
        g['wait'](r'\[moan.plugin.imported\] tracks=2 frames=48000 clipped=0 format=Clap', start, 180)
        g['wait'](r'\[moanplug.rendered\] frames=48000 blocks=188 ticks=\d+ format=clap offline=1', start, 120)
        shot('clap-new-track-original-kept', 'Offline render ready')
        choose_plugin('crash.clap')
        start, child = begin()
        g['wait'](rf'fault.kill \[cpu.fault.terminal\] pd={child[1]} kind=invalid_opcode ', start, 180)
        g['wait'](r'\[moan.plugin.state\] status=Failed retired=1', start, 180)
        assert '[moan.plugin.imported]' not in text()[start:]
        shot('crash-original-project-alive', 'Offline render failed; original region unchanged')
        choose_plugin('hang.clap')
        start, child = begin()
        shot('offline-render-active', 'Offline render running')
        guard = c.command(surface, 'ctrl', 'n')
        c.click(surface, 'Cancel_render')
        g['wait'](rf'\[shell.launch.kill\] parent={target} child={child[1]} task={child[2]} ok=1', start, 180)
        g['wait'](r'\[moan.plugin.state\] status=Failed retired=1', start, 180)
        assert '[moan.project.new]' not in text()[guard:] and '[moan.plugin.imported]' not in text()[start:]
        shot('cancel-retired-project-kept', 'Offline render failed; original region unchanged')
        choose_plugin('swap.clap')
        start, child = begin()
        g['wait'](r'\[moan.plugin.imported\] tracks=3 frames=48000 clipped=0 format=Clap', start, 180)
        shot('recovery-new-editable-track', 'Offline render ready')
        choose_plugin('gain.vst3')
        c.click(surface, 'VST3')
        start, child = begin()
        g['wait'](r'\[moan.plugin.imported\] tracks=4 frames=48000 clipped=0 format=Vst3', start, 180)
        g['wait'](r'\[moanplug.rendered\].*format=vst3 offline=1', start, 120)
        shot('vst3-default-unity-format-control', 'Offline render ready')
        start = c.command(surface, 'ctrl', 'z')
        g['wait'](r'\[moan.project\] tracks=3 ', start, 120)
        start = c.command(surface, 'ctrl', 'shift', 'z')
        g['wait'](r'\[moan.project\] tracks=4 ', start, 120)
        start = c.command(surface, 'ctrl', 's')
        g['wait'](r'\[files.app.question\] program=moan mode=Save', start, 120)
        c.focus(fsurface)
        c.chord('ctrl', 'h')
        c.chord('ctrl', 's')
        for _ in range(24):
            q.key('backspace')
            time.sleep(.08)
        c.type('offline.moan')
        q.key('ret')
        saved = g['wait'](r'\[moan.project.saved\] bytes=(\d+) tracks=4 clips=4 hash=(0x[0-9a-f]+) synced=1', start, 240)
        c.close(surface, target, task)
        c.focus(fsurface)
        start = len(text())
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={ft} task={ftask} ', start, 180)
        q.cmd('quit')
        g['proc'].wait(30)
    finally:
        g['stop']()
    extractor = out / 'sexfs-extract'
    subprocess.run(['rustc', '--edition=2021', '-O', '-A', 'dead_code', '-A', 'unused_attributes', str(app_root / 'scripts/sexfs_extract.rs'), '-o', str(extractor)], check=True)
    project = out / 'offline.moan'
    subprocess.run([str(extractor), str(disk), '/offline.moan', str(project)], check=True)
    assert project.stat().st_size == int(saved[1])
    audio = project_oracle(project.read_bytes())
    assert len({row for _, row, _ in children}) < len(children), 'no actual row reuse'
    assert len({(row, task) for _, row, task in children}) == len(children), 'child task reused'
    assert all(sha(Path(path)) == digest for path, digest in inputs.items()), 'source or immutable input changed'
    result = dict(ok=True, scope=__doc__, archive=audio, screenshots=shots, children=children,
                  crash_recovered=True, cancel_retired=True, editable_import_undo_redo=True,
                  no_hostile_sandbox_claim=True, no_live_insert_claim=True)
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(ok=True, screenshots=len(shots), tracks=audio['tracks'])))


if __name__ == '__main__':
    main()
