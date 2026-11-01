#!/usr/bin/env bash
# Production timer/inventory/ownership helpers, no privileged context emulation.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SELF_WAIT_GATE_OUT:-}" ]]; then
    TEST_DIR="$SELF_WAIT_GATE_OUT"
    mkdir "$TEST_DIR"
else
    TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-self-wait.XXXXXX")"
    trap 'rm -rf "$TEST_DIR"' EXIT
fi
rustc --edition=2021 --test "$ROOT_DIR/tests/self_wait.rs" -o "$TEST_DIR/tests" 2>"$TEST_DIR/tests.compile.log"
"$TEST_DIR/tests" | tee "$TEST_DIR/tests.log"
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
import json,re,subprocess,sys
from pathlib import Path
root,out=map(Path,sys.argv[1:])
scheduler=(root/'kernel/src/scheduler.rs').read_text()
interrupts=(root/'kernel/src/interrupts.rs').read_text()
init=(root/'kernel/src/init.rs').read_text()
helper=(root/'kernel/src/self_wait.rs').read_text()

def code(s):
    return re.sub(r'\s+','',re.sub(r'//[^\n]*','',s))
def fn(s,name):
    matches=re.findall(r'^(?:pub )?(?:unsafe )?fn '+name+r'\b.*?^}',s,re.M|re.S)
    if len(matches)!=1:raise ValueError('missing/ambiguous actual '+name)
    return code(matches[0])
def require(s,text):
    if code(text) not in s:raise ValueError('missing source custody condition: '+text)
def wiring(s,irq,boot):
    body=code(s)
    require(body,'static SELF_WAIT_TASKS:self_wait::BootInventory<Task,{self_wait::BOOT_MAIN_CAPACITY}>=self_wait::BootInventory::new();')
    inventory=fn(s,'seal_self_wait_inventory')
    require(inventory,'assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"SELF_WAIT_REGISTRATION_AFTER_BOOT");')
    require(inventory,'for id in 1..crate::ipc::MAX_DOMAINS as u32')
    require(inventory,'let task=pd.main_task.load(Ordering::Acquire);if task.is_null(){continue;}')
    require(inventory,'assert!(SELF_WAIT_TASKS.register(count,task),"BOOT_SELF_WAIT_INVENTORY");count+=1;')
    require(inventory,'assert!(SELF_WAIT_TASKS.seal(),"BOOT_SELF_WAIT_SEAL");')
    require(inventory,'serial_println!("[boot.self_wait.sealed] tasks={} capacity={} max_ticks={} runtime_registration=0",count,self_wait::BOOT_MAIN_CAPACITY,self_wait::MAX_TICKS);')
    if inventory.index('SELF_WAIT_TASKS.register(')>=inventory.index('SELF_WAIT_TASKS.seal('):raise ValueError('registration follows seal')
    if body.count('SELF_WAIT_TASKS.register(')!=1 or body.count('SELF_WAIT_TASKS.seal(')!=1:raise ValueError('runtime/repeated inventory mutation')
    boot=code(boot)
    if boot.count('crate::scheduler::seal_self_wait_inventory();')!=1 or boot.index('crate::scheduler::seal_self_wait_inventory();')>=boot.index('crate::boot_memory::seal_all();'):raise ValueError('actual init inventory seal misplaced')
    wait=fn(s,'wait_for_message_ticks')
    require(wait,'if !SELF_WAIT_TASKS.contains(task)||ticks==0||ticks>self_wait::MAX_TICKS{return false;}')
    require(wait,'if pd.main_task.load(Ordering::Acquire)!=task{return false;}')
    order=['interrupts::disable();','SELF_WAIT_TASKS.contains(task)','wait_deadline.arm(now,ticks)','state.park(core.core_id)']
    positions=[wait.index(x)for x in order]
    if positions!=sorted(positions):raise ValueError('arm/park/exact ownership order changed')
    poll=fn(s,'poll_self_wait_deadlines')
    require(poll,'SELF_WAIT_TASKS.visit(|task|unsafe{')
    require(poll,'if let Some(deadline)=(*task).wait_deadline.take_due(now){')
    require(poll,'wake_owned_thread(task);')
    if any(x in poll for x in ['DOMAIN_REGISTRY','register(','alloc','Vec','Box','loop{','while','unpark_thread(','.cancel(']):raise ValueError('timer poll not bounded retained inventory wake')
    if poll.count('SELF_WAIT_TASKS.visit(')!=1:raise ValueError('multiple timer inventory passes')
    unpark=fn(s,'unpark_thread')
    if unpark.index('wait_deadline.cancel()')>=unpark.index('wake_owned_thread(task_ptr)'):raise ValueError('IPC wake no longer cancels prior deadline first')
    finish=fn(s,'finish_current_task')
    if finish.index('wait_deadline.cancel()')>=finish.index('state.exit()'):raise ValueError('exit does not cancel deadline')
    irq=code(irq)
    call='crate::scheduler::poll_self_wait_deadlines(TICKS.load(Ordering::Relaxed));'
    if irq.count(call)!=1 or irq.index(call)>=irq.index('ifstack_frame.code_segment.0asu64&3==0{'):raise ValueError('kernel HLT timer no longer polls deadline')

wiring(scheduler,interrupts,init)
def once(s,a,b):
    if s.count(a)!=1:raise ValueError('missing/ambiguous mutation: '+a)
    return s.replace(a,b,1)
controls=[
 ('scheduler-fixed32',once(scheduler,'BootInventory<Task,{self_wait::BOOT_MAIN_CAPACITY}>','BootInventory<Task,32>'),interrupts,init),
 ('receipt-fixed32',once(scheduler,',count,self_wait::BOOT_MAIN_CAPACITY,self_wait::MAX_TICKS);',',count,32,self_wait::MAX_TICKS);'),interrupts,init),
 ('runtime-register',scheduler+'\nfn forbidden_runtime_registration(){SELF_WAIT_TASKS.register(0,core::ptr::null_mut());}\n',interrupts,init),
 ('timer-cancels-later-arm',once(scheduler,'            wake_owned_thread(task);','            unpark_thread(task);'),interrupts,init),
 ('kernel-timer-omitted',scheduler,once(interrupts,'    crate::scheduler::poll_self_wait_deadlines(TICKS.load(Ordering::Relaxed));',''),init),
 ('actual-init-unsealed',scheduler,interrupts,once(init,'    crate::scheduler::seal_self_wait_inventory();','')),
]
for name,s,irq,boot in controls:
    try:wiring(s,irq,boot)
    except ValueError as error:(out/(name+'.log')).write_text(str(error)+'\n')
    else:raise ValueError('source wiring control survived: '+name)

# Actual production helper mutation, exact selected runtime regression. A
# missing mutation or compile failure is an infrastructure failure, never PASS.
fixture=(root/'tests/self_wait.rs').read_text()
for capacity,test in [(32,'all_64_registered_pointers_are_visited_once_in_slot_order'),(65,'boot_main_capacity_is_literal_64')]:
    name='capacity-'+str(capacity)
    altered,n=re.subn(r'pub const BOOT_MAIN_CAPACITY\s*:\s*usize\s*=\s*64\s*;',f'pub const BOOT_MAIN_CAPACITY:usize={capacity};',helper)
    if n!=1:raise ValueError('missing actual capacity mutation')
    module=out/(name+'-production.rs');module.write_text(altered)
    source=fixture.replace('"../kernel/src/self_wait.rs"',json.dumps(str(module.resolve()))).replace('"../kernel/src/task_ownership.rs"',json.dumps(str(root/'kernel/src/task_ownership.rs')))
    path=out/(name+'.rs');path.write_text(source);binary=out/name
    built=subprocess.run(['rustc','--edition=2021','--test',str(path),'-o',str(binary)],capture_output=True,text=True,timeout=30)
    (out/(name+'.compile.log')).write_text(built.stdout+built.stderr)
    if built.returncode:raise ValueError(name+' mutant did not compile')
    result=subprocess.run([str(binary),'--exact',test],capture_output=True,text=True,timeout=30)
    log=result.stdout+result.stderr;(out/(name+'.log')).write_text(log)
    if result.returncode!=101 or 'running 1 test'not in log or f'test {test} ... FAILED'not in log:raise ValueError(name+' escaped exact runtime negative control')
report=dict(ok=True,capacity=64,compiled_capacity_mutants_rejected=2,source_wiring_controls_rejected=len(controls),scope='actual atomic helpers and source wiring; no new BOOT tasks, x86 timer, stack pivot, SMP or latency proof')
(out/'result.json').write_text(json.dumps(report,indent=2)+'\n')
print('[gate.self_wait_host.controls] '+json.dumps(report,sort_keys=True))
PY
echo '[gate.self_wait_host.result] PASS scope=production_atomics_and_source_wiring_not_live_x86_timer_or_SMP'
