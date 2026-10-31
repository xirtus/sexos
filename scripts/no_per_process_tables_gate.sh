#!/usr/bin/env bash
# Repository-wide static absence check for per-process translation roots and a
# Unix-style process model. SexOS has one GLOBAL_VAS: every translation-root
# load must reinstall the single boot-established root. This scans every Rust
# source of every workspace member; it is static evidence, not a runtime proof.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "$ROOT_DIR" <<'PY'
import re,sys,tomllib,tempfile,shutil
from pathlib import Path

# Each permitted translation-root effect, bound to its enclosing function and
# to evidence that it reinstalls the one global root rather than a new one.
ALLOWED={
 'cr3_write':[('kernel/src/hal/x86_64.rs','tlb_flush_local','let (p4_frame, flags) = Cr3::read();\n            Cr3::write(p4_frame, flags);')],
 'page_table_root':[('kernel/src/memory/manager.rs','init','let level_4_table = unsafe { active_level_4_table(VirtAddr::new(hhdm_offset)) };\n    let mut mapper = unsafe { OffsetPageTable::new(level_4_table')],
 'ttbr_write':[('kernel/src/arch/aarch64/mmu.rs','init','TTBR0_ADDR = t0_l0 as u64;'),
               ('kernel/src/arch/aarch64/smp.rs','sex_arm_ap_main','let b0 = super::mmu::ttbr0_pa();\n        let b1 = super::mmu::ttbr1_pa();')],
}
PATTERNS={
 'cr3_write':re.compile(r'Cr3::write|\bmov\s+cr3\b|\bmov\s+%?\w+\s*,\s*%?cr3\b',re.I),
 'page_table_root':re.compile(r'OffsetPageTable::new|RecursivePageTable::new|MappedPageTable::new'),
 'ttbr_write':re.compile(r'msr\s+TTBR[01]_EL1',re.I),
 'process_model':re.compile(r'\bfn\s+(fork|vfork|execve?|waitpid|getpid|getppid|setsid|kill_process)\b|\bstruct\s+(Process|ProcessTable)\b|\bper_process_(cr3|pml4|page_table)\b'),
 'task_translation_root':re.compile(r'\b(cr3|pml4|ttbr0|page_table_root)\s*:\s*(u64|usize|PhysAddr|PhysFrame)',re.I),
}

def enclosing(text,offset):
    starts=[m for m in re.finditer(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(?:extern\s+"C"\s+)?fn\s+(\w+)',text,re.M) if m.start()<=offset]
    return starts[-1].group(1) if starts else ''

def scan(root):
    members=tomllib.load(open(root/'Cargo.toml','rb'))['workspace']['members']
    files=sorted({p for m in members for p in (root/m).rglob('*.rs') if 'target' not in p.parts})
    files+=sorted(p for p in (root/'kernel/src/arch').rglob('*.rs'))
    errors=[];seen={k:0 for k in ALLOWED}
    for path in sorted(set(files)):
        rel=str(path.relative_to(root));text=path.read_text(errors='replace')
        for kind,pattern in PATTERNS.items():
            for match in pattern.finditer(text):
                line=text.count('\n',0,match.start())+1
                if text[text.rfind('\n',0,match.start())+1:match.start()].lstrip().startswith('//'):continue
                ok=False
                for site,function,evidence in ALLOWED.get(kind,()):
                    if rel==site and enclosing(text,match.start())==function and evidence in text:ok=True;break
                if ok:seen[kind]+=1
                else:errors.append(f'{rel}:{line}: {kind}: {text.splitlines()[line-1].strip()[:100]}')
    expected={'cr3_write':1,'page_table_root':1,'ttbr_write':4}
    for kind,count in expected.items():
        if seen[kind]!=count:errors.append(f'{kind}: expected {count} sanctioned global-root sites, found {seen[kind]}')
    return len(set(files)),errors

root=Path(sys.argv[1]);count,errors=scan(root)
if errors:
    print('\n'.join(errors));print('[no_per_process_tables.static] FAIL');sys.exit(1)
print(f'[no_per_process_tables.static] PASS files={count}')

# Negative controls: each injected per-process construct must be rejected.
MUTANTS={
 'scheduler_switches_cr3':('kernel/src/scheduler.rs','\nfn switch_space(f: PhysFrame, g: Cr3Flags) { unsafe { Cr3::write(f, g); } }\n'),
 'fresh_root_mapper':('kernel/src/pd/create.rs','\nfn private_space(t: &mut PageTable) { let _ = unsafe { OffsetPageTable::new(t, VirtAddr::new(0)) }; }\n'),
 'task_cr3_field':('kernel/src/scheduler.rs','\nstruct Shadow { cr3: u64 }\n'),
 'unix_fork':('servers/silk-shell/src/main.rs','\nfn fork() -> u32 { 0 }\n'),
 'asm_cr3_load':('kernel/src/interrupts.rs','\nfn load() { unsafe { core::arch::asm!("mov cr3, rax"); } }\n'),
}
rejected=0
with tempfile.TemporaryDirectory() as temporary:
    for name,(target,addition) in MUTANTS.items():
        copy=Path(temporary)/name
        shutil.copytree(root,copy,ignore=shutil.ignore_patterns('target','target_debug','.git','docs','vendor','attic','archive','limine','iso_root','logs','sex-src','sex-repo','relibc','*.iso','*.img'))
        (copy/target).write_text((copy/target).read_text()+addition)
        _,mutant_errors=scan(copy)
        if not mutant_errors:print(f'[no_per_process_tables.negative] FAIL {name} accepted');sys.exit(1)
        rejected+=1
print(f'[no_per_process_tables.negative] PASS injected_rejected={rejected}')
print('[no_per_process_tables.result] PASS scope=static_workspace_and_arch_sources_not_runtime_translation_or_hostile_code')
PY
