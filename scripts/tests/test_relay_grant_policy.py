"""Compile actual Files policy bodies; this is not a live Files exploit test."""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def function(source, name):
    matches = list(re.finditer(r'\bfn ' + re.escape(name) + r'\(', source))
    if len(matches) != 1:
        raise ValueError('missing/ambiguous production function: ' + name)
    start = source.rfind('\n', 0, matches[0].start()) + 1
    opening = source.index('{', matches[0].end())
    depth = 0
    for end in range(opening, len(source)):
        if source[end] == '{':
            depth += 1
        elif source[end] == '}':
            depth -= 1
            if depth == 0:
                return source[start:end + 1]
    raise ValueError('unterminated production function: ' + name)


def constant(source, name):
    matches = re.findall(r'^(?:pub )?const ' + re.escape(name) + r':[^;]+;', source, re.M)
    if len(matches) != 1:
        raise ValueError('missing/ambiguous production constant: ' + name)
    return matches[0]


def harness(root):
    files = (root / 'servers/sexfiles/src/fs_service.rs').read_text()
    launch = (root / 'crates/sex-pdx/src/launch.rs').read_text()
    store = (root / 'crates/sex-pdx/src/program_store.rs').read_text()
    wire = (root / 'crates/sex-pdx/src/fs.rs').read_text()
    relay = (root / 'crates/sex-pdx/src/console_relay.rs').read_text()
    package = (root / 'crates/sex-pdx/src/package.rs').read_text()
    binding = re.findall(r'^struct Binding \{[^\n]+\}', files, re.M)
    if len(binding) != 1:
        raise ValueError('production Binding layout changed')
    return ('#![allow(dead_code)]\nmod sex_pdx {\n'
            + 'pub mod fs { ' + constant(wire, 'PATH_BYTES') + constant(wire, 'ARGS_MAX') + '}\n'
            + 'pub mod console_relay { ' + constant(relay, 'GRANT_TOKEN') + '}\n'
            + 'pub mod package { ' + constant(package, 'GRANT_TOKEN') + '}\n'
            + 'pub mod program_store { ' + constant(store, 'BIN') + constant(store, 'NAME_BYTES')
            + function(store, 'pack_name') + '}\n'
            + 'pub mod launch { ' + function(launch, 'bin_path') + function(launch, 'args_allowed') + '}\n}\n'
            + 'pub use sex_pdx::{package, program_store};\nuse sex_pdx::fs as w;\n'
            + constant(files, 'ROOT_MAX') + constant(files, 'PICKED_MAX') + binding[0] + '\n'
            + '\n'.join(function(files, name) for name in ('holds_launch', 'holds_packages', 'confine', 'program_prepare'))
            + CASES)


CASES = r'''
fn binding(args: &[u8], picked: &[u8]) -> Binding {
    let mut b = Binding { task: 1, root: [0; ROOT_MAX], len: 10, args: [0; w::ARGS_MAX],
        args_len: args.len(), picked: [0; PICKED_MAX], picked_len: picked.len() };
    b.root[..10].copy_from_slice(b"/data/demo");
    b.args[..args.len()].copy_from_slice(args); b.picked[..picked.len()].copy_from_slice(picked); b
}
fn prepare(parent: &[u8], picked: &[u8], program: &[u8], args: &[u8]) -> Option<Vec<u8>> {
    let mut out = [0; w::PATH_BYTES];
    program_prepare(&binding(parent, picked), program, args, &mut out).map(|n| out[..n].to_vec())
}
#[test] fn ordinary_ungranted_launch_remains_allowed() {
    assert_eq!(prepare(b"", b"", b"/bin/term", b"hello world"), Some(b"/bin/term".to_vec()));
}
#[test] fn unheld_relay_grant_is_refused() {
    for args in [b"+launch".as_slice(), b"x +launch y", b"  +launch  "] {
        assert_eq!(prepare(b"", b"", b"/bin/term", args), None);
    }
}
#[test] fn holder_can_delegate_relay_with_identical_tokenization() {
    for parent in [b"+launch".as_slice(), b"ordinary   +launch more"] {
        assert!(prepare(parent, b"", b"/bin/term", b"x +launch y").is_some());
    }
    for parent in [b"foo+launch".as_slice(), b"+launchX", b"+launch\t"] {
        assert!(!holds_launch(&binding(parent, b"")));
        assert_eq!(prepare(parent, b"", b"/bin/term", b"+launch"), None);
    }
}
#[test] fn lookalikes_do_not_create_relay_authority() {
    for args in [b"foo+launch".as_slice(), b"+launchX", b"+launch\t"] {
        assert!(prepare(b"", b"", b"/bin/term", args).is_some());
        assert!(!holds_launch(&binding(args, b"")));
    }
}
#[test] fn packages_and_root_paths_keep_their_existing_checks() {
    assert_eq!(prepare(b"+launch", b"", b"/bin/term", b"+launch +packages"), None);
    assert_eq!(prepare(b"+launch", b"", b"/bin/term", b"+launch @/"), None);
    assert_eq!(prepare(b"@/allowed", b"", b"/bin/term", b"@/"), None);
    assert_eq!(prepare(b"", b"", b"/bin/term", b"@/secret"), None);
    assert!(prepare(b"+launch +packages @/", b"", b"/bin/term", b"+launch +packages @/secret").is_some());
}
#[test] fn held_paths_remain_bounded_and_delegable() {
    assert!(prepare(b"@/shared/item", b"", b"/bin/term", b"@/shared/item").is_some());
    assert_eq!(prepare(b"@/shared/item", b"", b"/bin/term", b"@/shared/itemx"), None);
    assert!(prepare(b"", b"@/picked", b"/bin/term", b"@/picked/file").is_some());
    assert_eq!(prepare(b"@/", b"", b"/bin/term", b"@/shared/../secret"), None);
}
#[test] fn program_path_checks_remain_unchanged() {
    assert_eq!(prepare(b"@/tools", b"", b"@/tools/helper", b""), Some(b"/tools/helper".to_vec()));
    assert_eq!(prepare(b"", b"", b"@/tools/helper", b""), None);
    for path in [b"/etc/secret".as_slice(), b"/bin/../term", b"/bin/a b"] {
        assert_eq!(prepare(b"+launch", b"", path, b"+launch"), None);
    }
}
'''


class RelayGrantPolicy(unittest.TestCase):
    def test_actual_production_policy_functions(self):
        with tempfile.TemporaryDirectory(prefix='sexos-relay-policy-') as directory:
            root = Path(directory)
            source, binary = root / 'policy.rs', root / 'policy-tests'
            source.write_text(harness(ROOT))
            compiled = subprocess.run(['rustc', '--edition=2021', '--test', str(source), '-o', str(binary)],
                                      capture_output=True, text=True)
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            checked = subprocess.run([str(binary)], capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)


if __name__ == '__main__':
    unittest.main()
