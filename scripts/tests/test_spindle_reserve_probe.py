import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
from history_boot_fixture import boot
import spindle_reserve_probe as p


def fixture():
    rows=p.source_spindles(ROOT)
    # Independent expected actual third identity; no generated population.
    assert [r['target']for r in rows]==[12,17,24]
    text=boot({12:120,17:170,24:240})
    def add(line):
        nonlocal text
        text+=line+'\n'
    def capture(name,kind,target,expected):
        add('[test.capture.boundary] complete=1')
        return dict(sequence=len(frames)+1,name=name,kind=kind,target=target,expected=expected,
                    serial_end=len(text),serial_sha256=hashlib.sha256(text.encode()).hexdigest(),proof={'ok':True},screenshot=name+'.ppm')
    frames=[]
    add('[files.history.durable] logical=12 target=17 generation=7 bank=0 bytes=40 hash=0x11 barriers=3 readback=1')
    add('[sext.app.pool.ready] target=17 task=170 bytes=4096 zeroed=4096 workspace_records=1 reused=1 zero_owner=sext ok=1')
    step=dict(target=24,task=240,surface=313,frame_id=314,predecessor=17,logical=12,incarnation=3,serial_start=len(text))
    add('[app.activation.committed] logical=12 incarnation=3 predecessor=17 successor=24 task=240 surface=313 pages=10 runtime_alloc=0 runtime_mint=0 bsp=1')
    add('[shell.app.relaunch] predecessor=17 successor=24 task=240 surface=313 receipt_replayed=1')
    add('[files.history.recovered] logical=12 source=17 source_task=170 target=24 generation=7 bytes=40 source_hash=0x11 owned_hash=0x22 data_hash=0x33 boot_grant=53')
    add('[files.history.open] target=24 handle=91 mode=load seq=1')
    add('[files.history.close] target=24 handle=91 seq=2 ok=1')
    add('[spindle.history.receipt] target=24 task=240 kind=loaded handle=91 bytes=40 count=1 hash=0x22 error=0')
    for sid in [313,315,316,317]:add(f'[spindle.view.text] target=24 sid={sid} sequence=1 paint=1 cols=20 rows=3 bytes=60')
    step['recovery']=dict(source=17,source_task=170,generation=7,bytes=40,source_hash='0x11',owned_hash='0x22',data_hash='0x33',handle=91,count=1)
    step['recall_start']=len(text)
    add('[spindle.history.nav] dir=up idx=0 len=12 ok=1')
    frame=capture('reserve-spindle-24-recall','spindle',24,'> echo alpha17');frames.append(frame);step['recall_frame']=frame
    step['recall_execute_start']=len(text);commands=[]
    def save(generation):
        add(f'[files.history.open] target=24 handle=92 mode=save seq={generation*3}')
        add(f'[files.history.durable] logical=12 target=24 generation={generation} bank=0 bytes=50 hash=0x44 barriers=3 readback=1')
        add(f'[files.history.commit] target=24 handle=92 seq={generation*3+1} bytes=50 hash=0x44')
        add(f'[files.history.close] target=24 handle=92 seq={generation*3+2} ok=1')
        add('[spindle.history.receipt] target=24 task=240 kind=saved handle=92 bytes=50 count=2 hash=0x44 error=0')
        return dict(handle=92,bytes=50,count=2,hash='0x44',generation=generation,bank=0)
    for i,value in enumerate(['echo alpha17','echo alpha24','mkdoc thirdprobe','filldocx 5 33','catdocx 5','rmdoc 5']):
        start=len(text);add(f'[spindle.instance.command] target=24 task=240 name={value.split()[0]} ok=1')
        if i>=2:
            job=[f'Create({list(b"thirdprobe"+bytes(6))})','Fill(5, 33, true)','Verify(5, true)','Delete(5)'][i-2]
            fingerprint=p.fnv(bytes((37*n+11)&255 for n in range(33)))if i==4 else'0x0'
            add(f'[spindle.probe.result] target=24 kind=done job={job} value={[5,33,33,0][i-2]} error=0 hash={fingerprint}')
        saved=save(8+i);commands.append(dict(command=value,serial_start=start,serial_end=len(text),saved=saved))
    step.update(commands=commands,object=5)
    start=len(text);add('[spindle.instance.command] target=24 task=240 name=net-probe ok=1')
    add('[net.job.state] owner=Some(Identity { pd: 24, task: 240 }) ticket=4 from=Connecting to=Sending staged=1025 queued=1025 received=0 scope=private_job')
    add(f'[browser.net.job.bytes] ticket=4 length=1025 hash={p.fnv(p.ECHO)} exact=1 endpoint=controlled_peer tls=0 web=0')
    add('[browser.net.job.released] ticket=4 view=Verified private_job=0')
    add('[spindle.net.state] target=24 task=240 state=verified busy=0 generation=5 fixed_peer=1 web=0')
    saved=save(14);frame=capture('reserve-spindle-24-network','spindle',24,'NET / verified');frames.append(frame)
    peer=dict(endpoint=['127.0.0.1',46078],errors=[],completed=True,connections=[dict(request_hex=p.ECHO.hex(),response_hex=p.ECHO.hex(),received=1025,sent=1025,remote=['127.0.0.1',12345])])
    step['network']=dict(serial_start=start,serial_end=len(text),peer=peer,peer_file='spindle-24-echo-peer.json',peer_sha256=hashlib.sha256(p.peer_bytes(peer)).hexdigest(),frame=frame,saved=saved)
    step['close_start']=len(text);add('[task.finish] pd=24 task=240 reason=0 terminal=1')
    for role,holder,acked in [(1,3,1),(2,1,3),(4,11,7),(8,13,15),(16,10,31)]:add(f'[app.service.ack] target=24 task=240 holder={holder} role={role} acked={acked} required=31')
    add('[app.private.retired] target=24 task=240 pages=1 bytes=4096 acked=31 required=31 all_flushed=1 pool_reusable=1 messages=0 signals=0 metadata=retained runtime_alloc=0')
    add('[sext.app.pool.ready] target=24 task=240 bytes=4096 zeroed=4096 workspace_records=1 reused=1 zero_owner=sext ok=1');step['serial_end']=len(text)
    start=len(text);add('[shell.palette.item] idx=0 name=Open Spindle action=open')
    rowoffset=len(text);add('[launcher.text.row] index=0 row=0 name=Open Spindle status=CLOSED / RESERVE EXHAUSTED')
    frame=capture('third-spindle-exhausted-launcher','launcher',12,'Open Spindle');frames.append(frame)
    result=dict(roster=p.verify_boot_roster(ROOT,text),catalog=rows,chain=[step],exhaustion_start=start,exhaustion_row_offset=rowoffset,exhaustion_frame=frame,reserve_exhausted=True)
    return result,frames,text,rows


class ReserveProbe(unittest.TestCase):
    def check(self,items):return p.verify(*items)
    def test_complete_three_subject_join_uses_actual_unequal_boot_tasks(self):
        self.assertEqual(self.check(fixture())['targets'],[24])
    def test_chain_skip_and_identity_forgery_fail(self):
        for field,value in [('target',17),('task',24),('predecessor',12),('surface',302),('frame_id',306),('incarnation',2)]:
            values=list(fixture());values[0]['chain'][0][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):self.check(values)
        values=list(fixture());values[0]['chain']=[]
        with self.assertRaises(ValueError):self.check(values)
    def test_stale_command_windows_or_skipped_probe_fail(self):
        for mutate in [lambda s:s['commands'].pop(3),lambda s:s['commands'][3].update(serial_start=0),lambda s:s.update(recall_start=0),lambda s:s['commands'][4].update(command='catdocx 6')]:
            values=list(fixture());mutate(values[0]['chain'][0])
            with self.assertRaises(ValueError):self.check(values)
    def test_wrong_saved_generation_or_payload_metadata_fails(self):
        for field,value in [('generation',7),('bytes',51),('hash','0xff')]:
            values=list(fixture());values[0]['chain'][0]['commands'][0]['saved'][field]=value
            with self.assertRaises(ValueError):self.check(values)
    def test_missing_or_wrong_fresh_receipts_fail(self):
        for old,new in [('source_task=170','source_task=171'),('source_hash=0x11','source_hash=0x99'),('sid=317','sid=318'),('job=Fill(5, 33, true)','job=Fill(6, 33, true)'),('view=Verified private_job=0','view=Verified private_job=1'),('role=8 acked=15','role=4 acked=15'),('all_flushed=1','all_flushed=0'),('RESERVE EXHAUSTED','RESERVE AVAILABLE')]:
            values=list(fixture());values[2]=values[2].replace(old,new)
            # Even a recomputed prefix binding cannot repair a false receipt.
            for frame in values[1]:frame['serial_sha256']=hashlib.sha256(values[2][:frame['serial_end']].encode()).hexdigest()
            with self.subTest(old=old),self.assertRaises(ValueError):self.check(values)
    def test_capture_ref_prefix_task_and_missing_frame_fail(self):
        for mutate in [lambda r,f:f.pop(0),lambda r,f:r['chain'][0]['recall_frame'].update(target=17),lambda r,f:r['chain'][0]['network']['frame'].update(serial_sha256='wrong'),lambda r,f:r['exhaustion_frame'].update(proof={'ok':False})]:
            values=list(fixture());mutate(values[0],values[1])
            with self.assertRaises(ValueError):self.check(values)
    def test_wrong_peer_payload_or_extra_connection_fails_even_if_hash_recomputed(self):
        for mutate in [lambda p:p['connections'][0].update(request_hex='00'),lambda p:p['connections'][0].update(response_hex='00'),lambda p:p['connections'].append(dict(p['connections'][0])),lambda p:p['errors'].append('extra connection'),lambda p:p.update(completed=False)]:
            values=list(fixture());network=values[0]['chain'][0]['network'];mutate(network['peer']);network['peer_sha256']=hashlib.sha256(p.peer_bytes(network['peer'])).hexdigest()
            with self.assertRaises(ValueError):self.check(values)
    def test_raw_peer_file_is_bound_when_out_supplied(self):
        values=fixture();network=values[0]['chain'][0]['network']
        with tempfile.TemporaryDirectory()as directory:
            path=Path(directory)/network['peer_file'];path.write_bytes(p.peer_bytes(network['peer']))
            self.assertTrue(p.verify(*values,out=directory)['ok']);path.write_text('{}\n')
            with self.assertRaises(ValueError):p.verify(*values,out=directory)
    def test_stale_or_forged_exhaustion_offsets_fail(self):
        for field,value in [('exhaustion_start',0),('exhaustion_row_offset',0),('reserve_exhausted',False)]:
            values=list(fixture());values[0][field]=value
            with self.assertRaises(ValueError):self.check(values)
    def test_echo_pattern_is_independent_and_exact(self):
        expected=bytes((i*17+78)%256 for i in range(1025))
        self.assertEqual(p.ECHO,expected);self.assertEqual(len(p.ECHO),1025)


if __name__=='__main__':unittest.main()
