import copy
import pathlib
import sys
import threading
import unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from check_alpha_session import session_errors,boot_lifecycle_errors,quil_close_errors,terminal_pending_errors
from alpha_session import save_receipt,watch_save,submit_save
from check_document_disk import fnv


def accepted():
    kinds=['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat','linen_refresh']
    return dict(ok=True,acceptance=True,keyboard=True,pointer=True,edit_during_save_dirty=True,
        documents=[dict(object=5),dict(object=6)],
        soak=dict(acceptance=True,elapsed_seconds=1800.1,cycles=12,
                  pending_quil_close=True,pending_spindle_close=True,boot_successors=[16,17],cleanup_errors=[],
                  samples=[dict(kind=k,seconds=.1)for _ in range(12)for k in kinds]))


def close_fixture(pending=0,early_close=False):
    serial='.'*12000+'\n'
    facts=dict(attempt_start=len(serial),declined_pending=1,terminal_pending=pending,
        confirmed_pending=pending,save_revision=25,dirty_revision=26,document=6,
        snapshot_bytes=3,snapshot_hash=f'{fnv(b"abc"):#x}',
        terminal_scope='pending_io_teardown' if pending else 'safe_completed_save_close',save_complete=None)
    def line(key,value):
        nonlocal serial
        if key:facts[key]=len(serial)
        serial+=value+'\n'
    line('save_begin','[quil.document.begin] kind=save object=6 revision=25 bytes=3')
    line('dirty_edit','[quil.document.edit] revision=26 pending=1 bytes=4')
    line(None,'[shell.app.close.request] target=9 task=9 slot=1')
    line('declined','[quil.close.declined] dirty=1 pending=1 revision=26')
    line('shell_declined','[shell.app.close.declined] target=9 task=9 reason=unsaved next_close=confirmed')
    if early_close:line(None,'[quil.document.close] pending=0')
    if not pending:
        saved=len(serial);line(None,f'[quil.persist.save.ok] bytes=3 hash={fnv(b"abc"):#x} doc=6')
        complete=len(serial);line(None,'[quil.document.saved] saved_revision=25 revision=26 dirty=1')
        facts['save_complete']=dict(begin=facts['save_begin'],saved=saved,completed=complete,
            revision=25,current_revision=26,dirty=1,bytes=3,hash=f'{fnv(b"abc"):#x}')
    facts['confirm_start']=len(serial)
    line(None,'[shell.app.close.request] target=9 task=9 slot=1 confirmed=1')
    line('confirmed',f'[quil.close.confirmed] dirty=1 pending={pending}')
    line('document_close',f'[quil.document.close] pending={pending}')
    return serial,facts


def accepted_v2(pending=0):
    r=accepted();soak=r['soak'];serial,facts=close_fixture(pending)
    soak.update(evidence_version=2,quil_close=facts,pending_save_close_guard=True,
        pending_quil_close=bool(pending),remaining_obligations=[] if pending else ['Quil pending-I/O terminal teardown'])
    soak['cycle_windows']=[];samples=[]
    kinds=[(k,None)for k in ['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat']]
    kinds += [(k,p)for p in ['save_admission','cycle_end']for k in ['linen_input_receipt','linen_refresh']]
    for cycle in range(1,13):
        start=(cycle-1)*150+.1;offset=(cycle-1)*1000
        soak['cycle_windows'].append(dict(cycle=cycle,start_seconds=start,end_seconds=start+140,
                                         serial_start=offset,serial_end=offset+900))
        for i,(kind,phase)in enumerate(kinds,1):
            sample=dict(sample_id=len(samples)+1,cycle=cycle,kind=kind,seconds=.1,
                action_seconds=start+i,session_seconds=start+i+.1,recorded_seconds=start+i+.2,
                serial_start=offset+i*10,serial_end=offset+i*10+5)
            if phase:sample['phase']=phase
            samples.append(sample)
    soak.update(samples=samples,active_elapsed_seconds=1680.,idle_elapsed_seconds=soak['elapsed_seconds']-1680.)
    return r,serial

class AlphaSessionTests(unittest.TestCase):
    def test_complete_sustained_metadata_passes(self):self.assertEqual(session_errors(accepted()),[])
    def test_short_rehearsal_or_missing_acceptance_cannot_pass(self):
        for field,value in [('elapsed_seconds',1799.9),('elapsed_seconds',float('nan')),('elapsed_seconds',True),('acceptance',False),('cycles',1),('pending_quil_close',False),('boot_successors',[9,12])]:
            r=accepted();r['soak'][field]=value;self.assertTrue(session_errors(r),(field,value))
        r=accepted();r['acceptance']=False;self.assertTrue(session_errors(r))
    def test_missing_or_invalid_samples_and_duplicate_object_reject(self):
        for sample in [None,-1,float('inf'),True]:
            r=accepted()
            if sample is None:r['soak']['samples'].pop()
            else:r['soak']['samples'][0]['seconds']=sample
            self.assertTrue(session_errors(r))
        r=accepted();r['documents'][1]['object']=5;self.assertTrue(session_errors(r))
    def test_third_reserve_needs_its_own_recovery_and_terminal_evidence(self):
        r=accepted();r['soak']['boot_successors']=[16,17,18]
        self.assertTrue(session_errors(r))
        r['soak']['third_quil']=dict(target=18,surface=307,logical=9,incarnation=3,
            edited_and_saved=True,pixel_errors=[],terminal=True,reserve_exhausted=True,documents=[5,6])
        self.assertEqual(session_errors(r),[])
        r['soak']['third_quil']['target']=16;self.assertTrue(session_errors(r))
    def test_reboot_lifecycle_support_does_not_relabel_rehearsal_as_sustained(self):
        r=accepted();r['acceptance']=False;r['soak']['acceptance']=False;r['soak']['elapsed_seconds']=1
        self.assertEqual(boot_lifecycle_errors(r),[]);self.assertTrue(session_errors(r))
        r['soak']['boot_successors']=[16,17,18];self.assertTrue(boot_lifecycle_errors(r))

class AlphaEvidenceV2Tests(unittest.TestCase):
    def test_completed_save_close_is_explicitly_not_pending_teardown(self):
        result,serial=accepted_v2()
        self.assertEqual(session_errors(result),[])
        self.assertEqual(quil_close_errors(result['soak'],serial),[])
        self.assertFalse(result['soak']['pending_quil_close'])
        self.assertEqual(result['soak']['remaining_obligations'],['Quil pending-I/O terminal teardown'])
        legacy=copy.deepcopy(result);legacy['soak']['evidence_version']=1
        self.assertTrue(session_errors(legacy))
    def test_true_pending_teardown_has_distinct_positive_facts(self):
        result,serial=accepted_v2(1)
        self.assertEqual(session_errors(result),[])
        self.assertEqual(quil_close_errors(result['soak'],serial),[])
        self.assertTrue(result['soak']['pending_quil_close'])
        self.assertEqual(result['soak']['remaining_obligations'],[])
    def test_pending_claim_and_scope_cannot_be_fabricated(self):
        for name,value in [('pending_quil_close',True),('remaining_obligations',[]),('pending_save_close_guard',False)]:
            result,_=accepted_v2();result['soak'][name]=value
            self.assertTrue(session_errors(result),name)
        for name,value in [('declined_pending',0),('terminal_pending',1),('terminal_scope','pending_io_teardown'),('save_complete',None),('declined_pending',True)]:
            result,_=accepted_v2();result['soak']['quil_close'][name]=value
            self.assertTrue(session_errors(result),name)
    def test_exact_receipts_reject_foreign_task_and_false_pending(self):
        result,serial=accepted_v2()
        for old,new in [('target=9 task=9','target=9 task=16'),
                        ('pending=1 revision=26','pending=0 revision=26'),
                        ('saved_revision=25 revision=26','saved_revision=24 revision=26'),
                        ('hash='+result['soak']['quil_close']['snapshot_hash'],'hash=0x0')]:
            self.assertTrue(quil_close_errors(result['soak'],serial.replace(old,new)),old)
    def test_declined_close_cannot_already_teardown_or_use_stale_offset(self):
        result,_=accepted_v2();serial,facts=close_fixture(early_close=True)
        result['soak']['quil_close']=facts
        self.assertTrue(quil_close_errors(result['soak'],serial))
        result,serial=accepted_v2();result['soak']['quil_close']['declined']-=1
        self.assertTrue(quil_close_errors(result['soak'],serial))
    def test_per_cycle_evidence_rejects_duplicate_samples_and_idle_soak(self):
        result,_=accepted_v2();result['soak']['samples'][1]=dict(result['soak']['samples'][0])
        self.assertTrue(session_errors(result))
        result,_=accepted_v2()
        for s in result['soak']['samples']:s['cycle']=1
        self.assertTrue(session_errors(result))
        result,_=accepted_v2();result['soak'].update(elapsed_seconds=2400,idle_elapsed_seconds=720)
        self.assertTrue(session_errors(result))
    def test_window_bounds_cycle_identity_and_duration_accounting(self):
        for key,value in [('start_seconds',-1),('end_seconds',2000),('cycle',True),('serial_end',0)]:
            result,_=accepted_v2();result['soak']['cycle_windows'][0][key]=value
            self.assertTrue(session_errors(result),(key,value))
        for key,value in [('active_elapsed_seconds',1),('idle_elapsed_seconds',float('nan')),('cycles',11)]:
            result,_=accepted_v2();result['soak'][key]=value
            self.assertTrue(session_errors(result),key)
    def test_sample_requires_own_time_and_serial_window(self):
        for key,value in [('session_seconds',-1),('seconds',20),('serial_start',12001),('recorded_seconds',float('inf'))]:
            result,_=accepted_v2();result['soak']['samples'][0][key]=value
            self.assertTrue(session_errors(result),(key,value))
    def test_pending_teardown_needs_its_own_exact_fresh_terminal_interval(self):
        serial=('[shell.app.close.request] target=9 task=9 slot=1\n'
                '[quil.document.close] pending=1\n[task.finish] pd=9 task=9 reason=0 terminal=1\n'
                '[shell.app.close.request] target=12 task=12 slot=1\n'
                '[spindle.probe.closed] target=12 pending=1 terminal=1\n'
                '[task.finish] pd=12 task=12 reason=0 terminal=1\n')
        self.assertEqual(terminal_pending_errors(serial),[])
        self.assertTrue(terminal_pending_errors(serial.replace('quil.document.close] pending=1','quil.document.close] pending=0')))
        self.assertTrue(terminal_pending_errors(serial.replace('spindle.probe.closed] target=12 pending=1','spindle.probe.closed] target=12 pending=0'),include_quil=False))
        stale='[spindle.probe.closed] target=12 pending=1 terminal=1\n'+serial.replace('[spindle.probe.closed] target=12 pending=1 terminal=1\n','')
        self.assertTrue(terminal_pending_errors(stale,include_quil=False))
        self.assertTrue(terminal_pending_errors(serial.replace('pd=12 task=12','pd=12 task=17'),include_quil=False))


class SaveObservationTests(unittest.TestCase):
    def test_exact_snapshot_object_revision_and_fresh_offset(self):
        body='abc';start='prefix\n'
        serial=start+'[quil.document.begin] kind=save object=6 revision=2 bytes=3\n'+f'[quil.persist.save.ok] bytes=3 hash={fnv(body.encode()):#x} doc=6\n'+'[quil.document.saved] saved_revision=2 revision=2 dirty=0\n'
        receipt=save_receipt(serial,len(start),6,body)
        self.assertEqual(receipt['revision'],2)
        self.assertIsNone(save_receipt(serial,len(serial),6,body))
        self.assertIsNone(save_receipt(serial,len(start),5,body))
        self.assertIsNone(save_receipt(serial,len(start),6,'abd'))
        self.assertIsNone(save_receipt(serial.replace('saved_revision=2','saved_revision=3'),len(start),6,body))
    def test_observer_cancellation_and_reader_failure_stop_thread(self):
        for reader,expected in [(lambda:'',[]),(lambda:(_ for _ in ()).throw(ValueError('invalid serial')),['ValueError'])]:
            stop=threading.Event();observed=[];errors=[]
            thread=threading.Thread(target=watch_save,args=(reader,0,6,'abc',stop,observed,errors))
            thread.start()
            if not expected:stop.set()
            thread.join(timeout=1)
            self.assertFalse(thread.is_alive());self.assertEqual(errors,expected);self.assertEqual(observed,[])
    def test_save_has_explicit_control_up_and_barrier_before_next_input(self):
        trace=[]
        class Qmp:
            def cmd(self,operation,args):trace.append((operation,args))
        def wait(pattern,start):trace.append(('wait',pattern,start))
        submit_save(Qmp(),wait,42);trace.append(('refresh',))
        self.assertEqual(trace[0][0],'input-send-event')
        self.assertNotIn('hold-time',trace[0][1])
        self.assertEqual([(e['data']['key']['data'],e['data']['down'])for e in trace[0][1]['events']],
            [('ctrl',True),('s',True),('s',False),('ctrl',False)])
        self.assertIn('0x9d',trace[2][1]);self.assertEqual(trace[2][2],42)
        self.assertEqual(trace[-1],('refresh',))
    def test_missing_control_break_aborts_before_navigation(self):
        trace=[]
        class Qmp:
            def cmd(self,operation,args):trace.append(operation)
        def wait(pattern,start):
            if '0x9d'in pattern:raise TimeoutError('Control release absent')
        with self.assertRaises(TimeoutError):
            submit_save(Qmp(),wait,42)
            trace.append('refresh')
        self.assertEqual(trace,['input-send-event'])
if __name__=='__main__':unittest.main()
