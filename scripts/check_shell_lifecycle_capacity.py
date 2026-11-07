#!/usr/bin/env python3
"""Compile actual Shell lifecycle metadata behavior against finite roster cases.

This host check does not create capabilities, run a guest, or prove native
rendering. Initial SID/state and current later-frame inventories are independent
reviewed goldens. Future rows are descriptive capacity fixtures only.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
INITIAL = [(100, 'Allocated'), (101, 'Allocated'), (102, 'Allocated'),
           (103, 'Allocated'), (200, 'Visible'), (201, 'Visible'),
           (202, 'Visible'), (203, 'Visible'), (204, 'Visible'),
           (153, 'Visible'), (144, 'Mapped'), (146, 'Allocated'),
           (147, 'Allocated'), (148, 'Allocated'), (149, 'Allocated'),
           (150, 'Allocated'), (151, 'Allocated')]
NATIVE = {
    12: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314],
    17: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314,
         319, 324, 329, 334, 339],
    19: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314,
         319, 324, 329, 334, 339, 343, 344],
    28: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314,
         319, 324, 329, 334, 339, 343, 344,
         345, 346, 347, 348, 349, 350, 351, 352, 354],
    32: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314,
         319, 324, 329, 334, 339, 343, 344,
         345, 346, 347, 348, 349, 350, 351, 352, 354,
         358, 359, 360, 361],
    37: [300, 301, 306, 307, 308, 309, 310, 311, 312, 314,
         319, 324, 329, 334, 339, 343, 344,
         345, 346, 347, 348, 349, 350, 351, 352, 354,
         358, 359, 360, 361, 362, 363, 364, 365, 366],
}
# Independently reviewed (PD, Shell frame SID, lifecycle slot), not Task IDs.
# Existing17 records retain their exact values; Browser30/31 add two frameless
# content surfaces. No authority or runtime task namespace is inferred here.
ROSTER = {19: [
    (9,201,38), (12,153,39), (15,300,40), (16,301,45), (17,306,50),
    (18,307,54), (19,308,110), (20,309,114), (21,310,118), (22,311,122),
    (23,312,126), (24,314,130), (25,319,134), (26,324,138), (27,329,142),
    (28,334,146), (29,339,150), (30,343,154), (31,344,158),
], 28: [
    (9,201,38), (12,153,39), (15,300,40), (16,301,45), (17,306,50),
    (18,307,54), (19,308,110), (20,309,114), (21,310,118), (22,311,122),
    (23,312,126), (24,314,130), (25,319,134), (26,324,138), (27,329,142),
    (28,334,146), (29,339,150), (30,343,154), (31,344,158),
    (32,345,162), (33,346,166), (34,347,170), (35,348,174),
    (36,349,178), (37,350,182), (38,351,186), (39,352,190), (40,354,194),
], 32: [
    (9,201,38), (12,153,39), (15,300,40), (16,301,45), (17,306,50),
    (18,307,54), (19,308,110), (20,309,114), (21,310,118), (22,311,122),
    (23,312,126), (24,314,130), (25,319,134), (26,324,138), (27,329,142),
    (28,334,146), (29,339,150), (30,343,154), (31,344,158),
    (32,345,162), (33,346,166), (34,347,170), (35,348,174),
    (36,349,178), (37,350,182), (38,351,186), (39,352,190), (40,354,194),
    (41,358,240), (42,359,244), (43,360,248), (44,361,252),
], 37: [
    (9,201,38), (12,153,39), (15,300,40), (16,301,45), (17,306,50),
    (18,307,54), (19,308,110), (20,309,114), (21,310,118), (22,311,122),
    (23,312,126), (24,314,130), (25,319,134), (26,324,138), (27,329,142),
    (28,334,146), (29,339,150), (30,343,154), (31,344,158),
    (32,345,162), (33,346,166), (34,347,170), (35,348,174),
    (36,349,178), (37,350,182), (38,351,186), (39,352,190), (40,354,194),
    (41,358,240), (42,359,244), (43,360,248), (44,361,252),
    (45,362,420), (46,363,424), (47,364,428), (48,365,432), (49,366,436),
]}



def one(pattern, source, label):
    matches = re.findall(pattern, source, re.M | re.S)
    if len(matches) != 1:
        raise ValueError(f'expected one actual {label}, found {len(matches)}')
    return matches[0]


def actual_function(source, name, constant=False):
    qualifier = 'const' if constant else 'unsafe'
    return one(r'^' + qualifier + r' fn ' + name + r'\b.*?^}', source, name)


def extract(source):
    inventory = one(r'^const BOOT_LIFECYCLE_SURFACES:.*?^\];', source, 'initial inventory')
    names = re.findall(r'\((SURFACE_ID_\w+), LifecycleState::\w+\)', inventory)
    if len(names) != len(set(names)):
        raise ValueError('duplicate initial SID name')
    constants = [one(r'^(?:pub )?const ' + name + r':\s*u64\s*=.*?;', source, name)
                 for name in names]
    parts = [
        one(r'^#\[derive\(Debug, Clone, Copy, PartialEq, Eq\)\]\n#\[repr\(u8\)\]\nenum LifecycleState\b.*?^}', source, 'state enum'),
        '#[derive(Debug, Clone, Copy)]\n' + one(r'^struct SurfaceLifecycle\b.*?^}', source, 'metadata struct'),
        *constants, inventory,
        actual_function(source, 'native_lifecycle_surface_count', True),
        one(r'^const LIFECYCLE_MAX_SURFACES:.*?;', source, 'capacity'),
        one(r'^static mut LIFECYCLE_TABLE:[^\n]+;$', source, 'retained table'),
        one(r'^static mut LIFECYCLE_GENERATION:.*?;', source, 'generation'),
    ]
    parts.extend(actual_function(source, name) for name in
                 ['lifecycle_register', 'lifecycle_state', 'set_lifecycle_state',
                  'surface_generation', 'lifecycle_init_all'])
    return '\n'.join(parts)


def fixture(actual, native, roster=None):
    initial = ','.join(f'({sid},LifecycleState::{state})' for sid, state in INITIAL)
    roster_gold = 'None' if roster is None else 'Some(&' + repr(roster) + ')'
    return '''#![allow(dead_code,static_mut_refs)]
#[path="catalog.rs"] mod app_catalog;
static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
macro_rules! serial_println {($($a:tt)*) => {LOG.lock().unwrap().push(format!($($a)*));}}
''' + actual + f'''
const INITIAL_GOLD: [(u64,LifecycleState);17] = [{initial}];
const NATIVE_GOLD: &[u64] = &{native!r};
const ROSTER_GOLD: Option<&[(u32,u64,usize)]> = {roster_gold};
''' + r'''
unsafe fn reset() {
    LIFECYCLE_TABLE = [None; LIFECYCLE_MAX_SURFACES];
    LIFECYCLE_GENERATION = 1;
    LOG.lock().unwrap().clear();
    lifecycle_init_all();
}
unsafe fn snapshot() -> Vec<(u64,LifecycleState,u64)> {
    LIFECYCLE_TABLE.iter().flatten().map(|(sid,r)|(*sid,r.state,r.generation)).collect()
}
unsafe fn fill() {
    reset();
    for &sid in NATIVE_GOLD {
        assert!(lifecycle_register(sid,LifecycleState::Visible), "no retained slot for {sid}");
    }
}
#[test] fn initial_inventory_states() { unsafe {
    reset();
    assert_eq!(snapshot(), INITIAL_GOLD.iter().map(|&(sid,s)|(sid,s,1)).collect::<Vec<_>>());
    let log=LOG.lock().unwrap();
    assert!(log.iter().any(|s|s=="[silk.lifecycle.init] ok=1 surfaces=17 states=Visible|Mapped|Allocated"));
    assert!(log.iter().any(|s|s=="[silk.lifecycle.surface.live] ok=1 surfaces=17"));
}}
#[test] fn complete_catalog_frames_fit_and_next_sid_is_refused() { unsafe {
    // Select by already-recorded frame IDs, independently of the production
    // ordinal/family counting predicate. These are metadata associations only.
    if let Some(expected)=ROSTER_GOLD {
        let catalog_metadata:Vec<_>=app_catalog::INCARNATIONS.iter().map(|r|(r.target,r.frame(),r.lifecycle_slot)).collect();
        assert_eq!(catalog_metadata,expected);
    }
    let actual:Vec<_>=app_catalog::INCARNATIONS.iter().map(|r|r.frame())
        .filter(|sid|!INITIAL_GOLD.iter().any(|&(initial,_)|initial==*sid)).collect();
    assert_eq!(actual,NATIVE_GOLD);
    fill(); // In a fixed32 mutant, future rows must fail real registration here.
    assert_eq!(native_lifecycle_surface_count(),NATIVE_GOLD.len());
    assert_eq!(LIFECYCLE_MAX_SURFACES,17+NATIVE_GOLD.len());
    assert_eq!(snapshot().len(),LIFECYCLE_MAX_SURFACES);
    let last=*NATIVE_GOLD.last().unwrap();
    assert_eq!(lifecycle_state(last),Some(LifecycleState::Visible));
    let before=snapshot();
    assert!(!lifecycle_register(999_999,LifecycleState::Visible));
    assert!(!set_lifecycle_state(999_999,LifecycleState::Closing));
    assert_eq!(snapshot(),before);
}}
#[test] fn duplicate_reinitializes_only_existing_slot() { unsafe {
    fill();let sid=*NATIVE_GOLD.last().unwrap();
    assert!(set_lifecycle_state(sid,LifecycleState::Closing));
    assert!(surface_generation(sid).unwrap()>1);
    // Preserve the existing explicit registration/re-init contract, including
    // generation reset; capacity work must not silently alter that behavior.
    assert!(lifecycle_register(sid,LifecycleState::Mapped));
    let entries=snapshot();assert_eq!(entries.len(),LIFECYCLE_MAX_SURFACES);
    assert_eq!(entries.iter().filter(|r|r.0==sid).count(),1);
    assert_eq!(entries.last(),Some(&(sid,LifecycleState::Mapped,1)));
    assert_eq!(&entries[..17],&INITIAL_GOLD.map(|(sid,s)|(sid,s,1)));
}}
#[test] fn tombstones_keep_identity_and_generation_without_freeing_capacity() { unsafe {
    fill();let sid=*NATIVE_GOLD.last().unwrap();let mut previous=surface_generation(sid).unwrap();
    for state in [LifecycleState::Closing,LifecycleState::Tombstoned,LifecycleState::Destroyed] {
        assert!(set_lifecycle_state(sid,state));
        let generation=surface_generation(sid).unwrap();assert!(generation>previous);previous=generation;
        assert_eq!(lifecycle_state(sid),Some(state));
        assert!(!lifecycle_register(999_999,LifecycleState::Visible));
        assert_eq!(snapshot().len(),LIFECYCLE_MAX_SURFACES);
    }
    let before=snapshot();assert!(set_lifecycle_state(sid,LifecycleState::Destroyed));assert_eq!(snapshot(),before);
}}
'''


def expanded(catalog, count, extra):
    match = re.search(r'(pub const INCARNATIONS: \[Incarnation; )\d+(\] = \[)(.*?)(^\];)', catalog, re.M | re.S)
    if not match:
        raise ValueError('actual literal incarnation catalog missing')
    ordinals = list(map(int,re.findall(r'app:\s*App::Browser,\s*ordinal:\s*(\d+)',catalog)))
    expected = {12:[0],17:[0],19:[0,1,2],28:[0,1,2],32:[0,1,2],37:[0,1,2]}.get(count)
    if ordinals != expected:raise ValueError('actual Browser lineage needs independent review')
    rows = []
    for index in range(extra):
        rows.append('    Incarnation { app: App::Browser, ordinal: %d, target: %d, surface: %d, shell_surface: None, lifecycle_slot: %d, close_slot: %d, activation_slot: Some(%d), route_slot: %d, children: &[] },\n' %
                    (index+len(ordinals), 100+index, 500+index, 200+4*index, 201+4*index, 202+4*index, 203+4*index))
    replacement = match[1]+str(count+extra)+match[2]+match[3]+''.join(rows)+match[4]
    return catalog[:match.start()]+replacement+catalog[match.end():]


def run_case(out, name, actual, catalog, native, failed_test=None, roster=None):
    directory=out/name;directory.mkdir()
    (directory/'catalog.rs').write_text(catalog)
    (directory/'tests.rs').write_text(fixture(actual,native,roster))
    binary=directory/'tests'
    compile_result=subprocess.run(['rustc','--edition=2021','--test',str(directory/'tests.rs'),'-o',str(binary)],
                                  capture_output=True,text=True,timeout=30)
    (directory/'compile.log').write_text(compile_result.stdout+compile_result.stderr)
    if compile_result.returncode:
        raise ValueError(f'{name}: compilation failed; see {directory / "compile.log"}')
    args=[str(binary),'--test-threads=1']
    if failed_test:args.extend(['--exact',failed_test])
    result=subprocess.run(args,capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(directory/'tests.log').write_text(output)
    if failed_test:
        if result.returncode!=101 or f'{failed_test} ... FAILED' not in output:
            raise ValueError(f'{name}: compiled counterexample was not rejected')
        return 1
    if result.returncode or '4 passed; 0 failed' not in output:
        raise ValueError(f'{name}: actual behavior failed; see {directory / "tests.log"}')
    return 4


def check(root, out):
    source=(root/'servers/silk-shell/src/main.rs').read_text()
    catalog=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
    count=int(one(r'^pub const INCARNATIONS: \[Incarnation; (\d+)\]',catalog,'roster count'))
    if count not in NATIVE:raise ValueError('current roster needs independent native-frame golden review')
    native=NATIVE[count];actual=extract(source)
    roster=ROSTER.get(count)
    passed=run_case(out,'current',actual,catalog,native,roster=roster)
    # Force at least two new Browser reservations and an actual retained budget
    # greater than32, even when run before the eight-Spindle roster lands.
    extra=max(2,17-len(native));future=expanded(catalog,count,extra)
    future_native=native+list(range(500,500+extra))
    future_roster=None if roster is None else roster+[(100+i,500+i,200+4*i)for i in range(extra)]
    passed+=run_case(out,'future',actual,future,future_native,roster=future_roster)
    fixed=re.sub(r'^const LIFECYCLE_MAX_SURFACES:.*?;', 'const LIFECYCLE_MAX_SURFACES: usize = 32;',actual,flags=re.M)
    run_case(out,'fixed32-mutant',fixed,future,future_native,'complete_catalog_frames_fit_and_next_sid_is_refused',roster=future_roster)
    omitted=actual.replace('[(u64, LifecycleState); 17]','[(u64, LifecycleState); 16]',1)
    omitted=omitted.replace('    (SURFACE_ID_APP, LifecycleState::Allocated),\n','',1)
    if omitted==actual:raise ValueError('initial omission mutant not constructed')
    run_case(out,'omitted-initial-mutant',omitted,catalog,native,'initial_inventory_states',roster=roster)
    wrong=actual.replace('(SURFACE_ID_APP, LifecycleState::Allocated)', '(SURFACE_ID_APP, LifecycleState::Visible)',1)
    if wrong==actual:raise ValueError('initial state mutant not constructed')
    run_case(out,'wrong-initial-state-mutant',wrong,catalog,native,'initial_inventory_states',roster=roster)
    return dict(ok=True,source_catalog_rows=count,tests=passed,compiled_mutants_rejected=3,boot_surfaces=17,
                native_frames=len(native),capacity=17+len(native),future_capacity=17+len(future_native),
                scope='actual_shell_retained_metadata_host_not_guest_authority',evidence=str(out))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new directory retaining extracted code, binaries and logs')
    args=parser.parse_args()
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False)
        result=check(args.root.resolve(),args.out.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-shell-lifecycle-') as directory:
            result=check(args.root.resolve(),Path(directory))
            result.pop('evidence')
    print('[shell.lifecycle.capacity] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
