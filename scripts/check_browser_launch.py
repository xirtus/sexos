#!/usr/bin/env python3
"""Actual Shell Browser launcher policy; held replies/desktop effects are doubles.

The source catalog is the current three-Browser catalog, not a synthesized
future grant inventory. This host gate does not prove kernel activation or
native pixels. Every production function is freshly extracted from Main.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


FUNCTIONS = (
    'native_launcher_status', 'launcher_live_action', 'launcher_row_title',
    'launcher_focus_existing', 'quil_activation_edge', 'app_activation_info',
    'app_reserve_remaining', 'app_launcher_status', 'browser_activation_info',
    'browser_launcher_status', 'browser_reserve_label', 'browser_active_frame',
    'open_browser_in_active_scene', 'quil_activation_info', 'spindle_activation_info',
    'palette_item_status', 'palette_execute_selected', 'command_palette_selected_accent',
)
CASES = (
    'browser_row_preserves_existing_indices_and_webstub_identity',
    'live_or_closing_predecessor_never_activates_or_focuses_future_sibling',
    'held_exact_catalog_is_required_before_browser_focus_restore_or_bring_here',
    'ordered_ready_grants_activate_30_then_31_and_exhaust_honestly',
    'wrong_task_slot_surface_and_retry_cannot_skip_or_commit_an_edge',
    'full_focus_and_old_input_close_view_custody_never_retarget_to_new_instance',
)


def masked(source):
    # Keep offsets/newlines; braces inside Rust strings, chars or comments are
    # not function/item delimiters. Unsupported extraction fails compilation.
    tokens = r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\''
    return re.sub(tokens, lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]), source, flags=re.S)


def extract(source, kind, name):
    clean = masked(source)
    pattern = r'^(?:pub )?(?:unsafe )?' + kind + r'\s+' + re.escape(name) + r'\b'
    found = list(re.finditer(pattern, clean, re.M))
    assert len(found) == 1, f'{kind} {name}: missing/ambiguous production item'
    start = found[0].start()
    if kind == 'const':
        begin = clean.index('=', found[0].end()) + 1
        stack = []
        for pos in range(begin, len(clean)):
            char = clean[pos]
            if char in '([{':
                stack.append(char)
            elif char in ')]}':
                assert stack and stack.pop() == {')': '(', ']': '[', '}': '{'}[char], name
            elif char == ';' and not stack:
                return source[start:pos + 1]
        raise AssertionError(name + ': unterminated constant')
    # Preserve actual derive/repr attributes on Frame/Tab/Command definitions.
    while start > 0:
        prior = source.rfind('\n', 0, start - 1) + 1
        if source[prior:start].strip().startswith('#['):
            start = prior
        else:
            break
    begin = clean.index('{', found[0].end())
    depth = 0
    for pos in range(begin, len(clean)):
        if clean[pos] == '{':
            depth += 1
        elif clean[pos] == '}':
            depth -= 1
            if depth == 0:
                return source[start:pos + 1]
    raise AssertionError(name + ': unterminated production body')


def production(source):
    items = [('struct', name) for name in ('ShellTab', 'ShellFrame', 'CommandDef')]
    items += [('enum', name) for name in ('Command', 'LifecycleState')]
    items += [('const', name) for name in ('MAX_TABS_PER_FRAME', 'FRAME_FLAG_MINIMIZED',
                                          'SURFACE_ID_BROWSER', 'BROWSER_FRAME_ID', 'COMMAND_LIST')]
    items += [('fn', name) for name in FUNCTIONS]
    return '\n\n'.join(extract(source, kind, name) for kind, name in items) + '\n'


def run(out, fixture, name, body, catalog, *, enabled=True, expected=None):
    prod = out / (name + '-production.rs')
    prod.write_text(body)
    binary = out / name
    env = {**os.environ, 'BROWSER_LAUNCH_PRODUCTION': str(prod), 'BROWSER_LAUNCH_CATALOG': str(catalog)}
    command = ['rustc', '--edition=2021', '--test']
    if enabled:
        command += ['--cfg', 'browser_launch_catalog']
    compiled = subprocess.run(command + [str(fixture), '-o', str(binary)], env=env,
                              text=True, capture_output=True)
    (out / (name + '.compile.log')).write_text(compiled.stdout + compiled.stderr)
    assert compiled.returncode == 0, name + ': compile failure is not mutant rejection; see compile log'
    command = [str(binary), '--test-threads=1']
    if expected:
        command += ['--exact', expected]
    result = subprocess.run(command, text=True, capture_output=True)
    log = result.stdout + result.stderr
    (out / (name + '.log')).write_text(log)
    if expected:
        assert result.returncode == 101 and 'running 1 test' in log, name + ': missing runtime rejection'
        assert f'test {expected} ... FAILED' in log, name + ': wrong rejection case'
        print('[browser.launch.control] rejected=' + name + ' case=' + expected)
        return 1
    count = 8 if enabled else 5
    assert result.returncode == 0 and f'test result: ok. {count} passed;' in log, name + ': fixture failed/count changed'
    for case in CASES if enabled else CASES[:3]:
        assert f'test {case} ... ok' in log, name + ': missing ' + case
    print(log, end='')
    return count


def check(root, out):
    main_path = root / 'servers/silk-shell/src/main.rs'
    fixture_path = root / 'tests/browser_launch_main.rs'
    catalog_path = root / 'crates/sex-pdx/src/app_catalog.rs'
    source = main_path.read_text()
    actual = production(source)
    original_fixture = fixture_path.read_text()
    dependencies = [main_path, fixture_path, catalog_path, Path(__file__).resolve(), root / 'scripts/launcher_host_gate.sh']

    def path(match):
        target = (fixture_path.parent / match[1]).resolve()
        assert target.is_relative_to(root) and target.is_file(), 'fixture module escaped source root'
        dependencies.append(target)
        return '#[path="' + str(target) + '"]'

    fixture = out / 'fixture.rs'
    fixture.write_text(re.sub(r'#\[path\s*=\s*"([^"]+)"\]', path, original_fixture))
    catalog = catalog_path.read_text()
    rows = re.findall(r'Incarnation\s*\{\s*app:\s*App::Browser,\s*ordinal:\s*(\d+),\s*target:\s*(\d+),\s*surface:\s*(\d+),', catalog)
    assert rows == [('0', '15', '300'), ('1', '30', '343'), ('2', '31', '344')], 'expected actual three-Browser source catalog'
    contents = out / 'source-catalog.rs'
    # include! requires module contents without inner documentation comments;
    # preserve all actual data/types/functions. No row synthesis or rewriting.
    contents.write_text('\n'.join('//' + line[3:] if line.startswith('//!') else line for line in catalog.splitlines()) + '\n')
    plain = run(out, fixture, 'actual-default', actual, contents, enabled=False)
    complete = run(out, fixture, 'actual-three-browser-catalog', actual, contents)
    controls = [
        ('browser-focus-excluded', 'if row.lifecycle_slot!=e.slot || row.frame()!=frame',
         'if row.app==app_catalog::App::Browser || row.lifecycle_slot!=e.slot || row.frame()!=frame', CASES[0]),
        ('task-equals-domain', '(i.successor,i.task,i.surface)!=(e.target,e.task,e.surface)',
         '(i.successor,i.task,i.surface)!=(e.target,e.target,e.surface)', CASES[3]),
        ('commit-state-unchecked', 'if active.state!=2 || browser_activation_info(true)!=Ok(active)',
         'if browser_activation_info(true)!=Ok(active)', CASES[4]),
        ('skip-rejected-edge', 'let info=quil_activation_edge(slot,row.target,row.surface,row.lifecycle_slot,false)?;',
         'let info=match quil_activation_edge(slot,row.target,row.surface,row.lifecycle_slot,false){Ok(info)=>info,Err(_)=>continue};', CASES[4]),
    ]
    for name, old, new, test in controls:
        assert actual.count(old) == 1, name + ': missing/ambiguous mutation anchor'
        run(out, fixture, name, actual.replace(old, new, 1), contents, expected=test)
    result = dict(ok=True, source_catalog='actual_three_browser_rows', default_cases=plain,
                  complete_cases=complete, compiled_mutants_rejected=len(controls),
                  scope='actual_extracted_Main_SDK_scripted_held_and_desktop_callbacks_not_kernel_activation_or_native_pixels',
                  source_sha256={str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in sorted(set(dependencies))})
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print('[browser.launch.host] PASS source_catalog=actual3 default_cases=5 complete_cases=8 compiled_mutants=4 native_activation=0')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    if args.out:
        out = args.out.resolve()
        out.mkdir(parents=True, exist_ok=True)
        check(root, out)
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-browser-launch-') as directory:
            check(root, Path(directory))


if __name__ == '__main__':
    main()
