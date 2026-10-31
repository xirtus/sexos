import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
import spindle_reserve_probe as p


# Independent literal catalog, separate from the concurrently growing production
# catalog. BOOT task IDs deliberately differ from domain IDs for every edge.
SPECS=[
    (12,154,153,39,42,None,14,[160,161,162]),
    (17,302,306,50,51,49,52,[303,304,305]),
    (24,313,314,130,131,132,133,[315,316,317]),
    (25,318,319,134,135,136,137,[320,321,322]),
    (26,323,324,138,139,140,141,[325,326,327]),
    (27,328,329,142,143,144,145,[330,331,332]),
    (28,333,334,146,147,148,149,[335,336,337]),
    (29,338,339,150,151,152,153,[340,341,342]),
]


def fixture(count=3):
    specs=SPECS[:count];rows=[];source=[]
    text=f'[boot.history.recovery] holder=11 slot=53 logical=12 participants={count} version=1 base=1808 sectors=18 held_query_verified=1 runtime_mint=0\n'
    for i,(target,surface,frame,life,close,activation,route,children) in enumerate(specs):
        rows.append(dict(ordinal=i,target=target,surface=surface,frame=frame,lifecycle=life,close=close,activation=activation,route=route))
        act='None'if activation is None else f'Some({activation})'
        source.append(f'Incarnation {{ app: App::Spindle, ordinal: {i}, target: {target}, surface: {surface}, shell_surface: Some({frame}), lifecycle_slot: {life}, close_slot: {close}, activation_slot: {act}, route_slot: {route}, children: &{children} }},')
        text+=f'[boot.history.participant] index={i} target={target} task={target*10} holder=11 immutable=1 runtime_mint=0\n'
    catalog=p.ROOT/'crates/sex-pdx/src/app_catalog.rs';catalog.parent.mkdir(parents=True,exist_ok=True)
    catalog.write_text(f'pub const INCARNATIONS: [Incarnation; {count}] = [\n'+ '\n'.join(source)+'\n];\n')
    for i,row in enumerate(rows):
        target=row['target'];pre=0 if i==0 else rows[i-1]['target']
        text+=f'[boot.app.catalog] target={target} task={target*10} surface={row["surface"]} logical=12 incarnation={i+1} predecessor={pre} lifecycle={row["lifecycle"]} close={row["close"]} route={row["route"]} dormant={int(i!=0)} grants_verified=1 data_only=1\n'
    text+=f'[boot.app.catalog.complete] rows={count} actual_grants=1 runtime_mint=0\n[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\n[bootgraph.boot.handoff] target=sexdisplay id=1 entry=0x4000cae0\n'
    def add(line):
        nonlocal text
        text+=line+'\n'
    frames=[];steps=[]
    def capture(name,kind,target,expected):
        add('[test.capture.boundary] complete=1')
        frame=dict(sequence=len(frames)+1,name=name,kind=kind,target=target,expected=expected,
                    serial_end=len(text),serial_sha256=hashlib.sha256(text.encode()).hexdigest(),proof={'ok':True},screenshot=name+'.ppm')
        frames.append(frame);return frame
    generation=7;prior_size=40;prior_hash='0x11'
    add('[files.history.durable] logical=12 target=17 generation=7 bank=0 bytes=40 hash=0x11 barriers=3 readback=1')
    add('[sext.app.pool.ready] target=17 task=170 bytes=4096 zeroed=4096 workspace_records=1 reused=1 zero_owner=sext ok=1')
    for row in rows[2:]:
        target=row['target'];task=target*10;ordinal=row['ordinal'];pre=rows[ordinal-1]['target'];surface=row['surface']
        step=dict(target=target,task=task,surface=surface,frame_id=row['frame'],predecessor=pre,logical=12,incarnation=ordinal+1,serial_start=len(text))
        add(f'[app.activation.committed] logical=12 incarnation={ordinal+1} predecessor={pre} successor={target} task={task} surface={surface} pages=10 runtime_alloc=0 runtime_mint=0 bsp=1')
        add(f'[shell.app.relaunch] predecessor={pre} successor={target} task={task} surface={surface} receipt_replayed=1')
        add(f'[files.history.recovered] logical=12 source={pre} source_task={pre*10} target={target} generation={generation} bytes={prior_size} source_hash={prior_hash} owned_hash=0x22 data_hash=0x33 boot_grant=53')
        add(f'[files.history.open] target={target} handle=91 mode=load seq=1')
        add(f'[files.history.close] target={target} handle=91 seq=2 ok=1')
        add(f'[spindle.history.receipt] target={target} task={task} kind=loaded handle=91 bytes={prior_size} count=1 hash=0x22 error=0')
        for sid in [surface]+specs[ordinal][-1]:add(f'[spindle.view.text] target={target} sid={sid} sequence=1 paint=1 cols=20 rows=3 bytes=60')
        step['recovery']=dict(source=pre,source_task=pre*10,generation=generation,bytes=prior_size,source_hash=prior_hash,owned_hash='0x22',data_hash='0x33',handle=91,count=1)
        step['recall_start']=len(text)
        add('[spindle.history.nav] dir=up idx=0 len=12 ok=1')
        step['recall_frame']=capture(f'reserve-spindle-{target}-recall','spindle',target,f'> echo alpha{pre}')
        step['recall_execute_start']=len(text);commands=[]
        def save():
            nonlocal generation
            generation+=1
            add(f'[files.history.open] target={target} handle=92 mode=save seq={generation*3}')
            add(f'[files.history.durable] logical=12 target={target} generation={generation} bank=0 bytes=50 hash=0x44 barriers=3 readback=1')
            add(f'[files.history.commit] target={target} handle=92 seq={generation*3+1} bytes=50 hash=0x44')
            add(f'[files.history.close] target={target} handle=92 seq={generation*3+2} ok=1')
            add(f'[spindle.history.receipt] target={target} task={task} kind=saved handle=92 bytes=50 count=2 hash=0x44 error=0')
            return dict(handle=92,bytes=50,count=2,hash='0x44',generation=generation,bank=0)
        for i,value in enumerate([f'echo alpha{pre}',f'echo alpha{target}','mkdoc thirdprobe','filldocx 5 33','catdocx 5','rmdoc 5']):
            start=len(text);add(f'[spindle.instance.command] target={target} task={task} name={value.split()[0]} ok=1')
            if i>=2:
                job=[f'Create({list(b"thirdprobe"+bytes(6))})','Fill(5, 33, true)','Verify(5, true)','Delete(5)'][i-2]
                fingerprint=p.fnv(bytes((37*n+11)&255 for n in range(33)))if i==4 else'0x0'
                add(f'[spindle.probe.result] target={target} kind=done job={job} value={[5,33,33,0][i-2]} error=0 hash={fingerprint}')
            saved=save();commands.append(dict(command=value,serial_start=start,serial_end=len(text),saved=saved))
        step.update(commands=commands,object=5)
        start=len(text);ticket=ordinal+2;add(f'[spindle.instance.command] target={target} task={task} name=net-probe ok=1')
        add(f'[net.job.state] owner=Some(Identity {{ pd: {target}, task: {task} }}) ticket={ticket} from=Connecting to=Sending staged=1025 queued=1025 received=0 scope=private_job')
        add(f'[browser.net.job.bytes] ticket={ticket} length=1025 hash={p.fnv(p.ECHO)} exact=1 endpoint=controlled_peer tls=0 web=0')
        add(f'[browser.net.job.released] ticket={ticket} view=Verified private_job=0')
        add(f'[spindle.net.state] target={target} task={task} state=verified busy=0 generation=5 fixed_peer=1 web=0')
        saved=save();frame=capture(f'reserve-spindle-{target}-network','spindle',target,'NET / verified')
        peer=dict(endpoint=['127.0.0.1',46078],errors=[],completed=True,connections=[dict(request_hex=p.ECHO.hex(),response_hex=p.ECHO.hex(),received=1025,sent=1025,remote=['127.0.0.1',12345+target])])
        step['network']=dict(serial_start=start,serial_end=len(text),peer=peer,peer_file=f'spindle-{target}-echo-peer.json',peer_sha256=hashlib.sha256(p.peer_bytes(peer)).hexdigest(),frame=frame,saved=saved)
        start=len(text);add(f'[spindle.instance.command] target={target} task={task} name=echo ok=1');saved=save()
        step['terminal_echo']=dict(command=f'echo alpha{target}',serial_start=start,serial_end=len(text),saved=saved)
        prior_size=saved['bytes'];prior_hash=saved['hash']
        step['close_start']=len(text);add(f'[task.finish] pd={target} task={task} reason=0 terminal=1')
        for role,holder,acked in [(1,3,1),(2,1,3),(4,11,7),(8,13,15),(16,10,31)]:add(f'[app.service.ack] target={target} task={task} holder={holder} role={role} acked={acked} required=31')
        add(f'[app.private.retired] target={target} task={task} pages=1 bytes=4096 acked=31 required=31 all_flushed=1 pool_reusable=1 messages=0 signals=0 metadata=retained runtime_alloc=0')
        add(f'[sext.app.pool.ready] target={target} task={task} bytes=4096 zeroed=4096 workspace_records=1 reused=1 zero_owner=sext ok=1');step['serial_end']=len(text);steps.append(step)
    start=len(text);add('[shell.palette.item] idx=0 name=Open Spindle action=open')
    rowoffset=len(text);add('[launcher.text.row] index=0 row=0 name=Open Spindle status=CLOSED / RESERVE EXHAUSTED')
    frame=capture('third-spindle-exhausted-launcher','launcher',12,'Open Spindle')
    result=dict(roster=p.verify_boot_roster(p.ROOT,text),catalog=rows,chain=steps,exhaustion_start=start,exhaustion_row_offset=rowoffset,exhaustion_frame=frame,reserve_exhausted=True)
    return result,frames,text,rows


class ReserveProbe(unittest.TestCase):
    def setUp(self):
        directory=tempfile.TemporaryDirectory();self.addCleanup(directory.cleanup)
        patcher=patch.object(p,'ROOT',Path(directory.name));patcher.start();self.addCleanup(patcher.stop)
    def check(self,items):return p.verify(*items)
    def refresh_prefixes(self,values):
        for frame in values[1]:
            frame['serial_sha256']=hashlib.sha256(values[2][:frame['serial_end']].encode()).hexdigest()
    def test_full_eight_roster_has_six_exact_terminal_recovery_edges(self):
        values=fixture(8)
        self.assertEqual(self.check(values)['targets'],[24,25,26,27,28,29])
        chain=values[0]['chain']
        self.assertEqual([step['task']for step in chain],[240,250,260,270,280,290])
        self.assertEqual([step['terminal_echo']['command']for step in chain],
                         ['echo alpha24','echo alpha25','echo alpha26','echo alpha27','echo alpha28','echo alpha29'])
        self.assertEqual([step['terminal_echo']['saved']['generation']for step in chain],[15,23,31,39,47,55])
        self.assertEqual([step['recovery']['generation']for step in chain],[7,15,23,31,39,47])
    def test_full_chain_missing_stale_or_misordered_terminal_echo_fails(self):
        mutations=[lambda s:s.pop('terminal_echo'),
                   lambda s:s['terminal_echo'].update(command='echo alpha17'),
                   lambda s:s.update(terminal_echo=copy.deepcopy(s['commands'][1])),
                   lambda s:s['terminal_echo'].update(serial_start=s['network']['serial_start']),
                   lambda s:s['terminal_echo'].update(serial_end=s['serial_end']),
                   lambda s:s['terminal_echo']['saved'].update(generation=s['network']['saved']['generation'])]
        for index in [0,2,5]:
            for mutation in mutations:
                values=list(fixture(8));mutation(values[0]['chain'][index])
                with self.subTest(index=index,mutation=mutation),self.assertRaises(ValueError):self.check(values)
    def test_full_chain_wrong_recovery_source_generation_and_middle_skip_fail(self):
        for index in [1,3,5]:
            for field,value in [('source',17),('source_task',170),('generation',7),('source_hash','0xff')]:
                values=list(fixture(8));values[0]['chain'][index]['recovery'][field]=value
                with self.subTest(index=index,field=field),self.assertRaises(ValueError):self.check(values)
        values=list(fixture(8));values[0]['chain'].pop(2)
        with self.assertRaises(ValueError):self.check(values)
        # Alter actual current recovery receipt while retaining all frame bindings.
        for old,new in [('source=24 source_task=240 target=25','source=17 source_task=170 target=25'),
                        ('target=25 generation=15 bytes=50 source_hash','target=25 generation=14 bytes=50 source_hash')]:
            values=list(fixture(8));self.assertIn(old,values[2]);values[2]=values[2].replace(old,new)
            self.refresh_prefixes(values)
            with self.assertRaises(ValueError):self.check(values)
    def test_full_chain_terminal_command_and_save_receipts_mandatory(self):
        for index in [0,5]:
            for old,new in [('name=echo ok=1','name=noop ok=1'),('kind=saved','kind=stale'),
                            ('barriers=3','barriers=0')]:
                values=list(fixture(8));terminal=values[0]['chain'][index]['terminal_echo']
                start,end=terminal['serial_start'],terminal['serial_end']
                span=values[2][start:end];self.assertIn(old,span)
                values[2]=values[2][:start]+span.replace(old,new)+values[2][end:]
                self.refresh_prefixes(values)
                with self.subTest(index=index,old=old),self.assertRaises(ValueError):self.check(values)
    def test_full_chain_network_release_and_cleanup_not_replaceable(self):
        for old,new in [('ticket=9 view=Verified private_job=0','ticket=9 view=Verified private_job=1'),
                        ('target=29 task=290 holder=13 role=8','target=29 task=290 holder=11 role=8'),
                        ('pd=29 task=290 reason=0 terminal=1','pd=29 task=290 reason=0 terminal=0')]:
            values=list(fixture(8));self.assertIn(old,values[2]);values[2]=values[2].replace(old,new)
            self.refresh_prefixes(values)
            with self.subTest(old=old),self.assertRaises(ValueError):self.check(values)
    def test_full_chain_binds_every_raw_peer_file(self):
        values=fixture(8)
        with tempfile.TemporaryDirectory()as directory:
            for step in values[0]['chain']:
                net=step['network'];(Path(directory)/net['peer_file']).write_bytes(p.peer_bytes(net['peer']))
            self.assertTrue(p.verify(*values,out=directory)['ok'])
            for step in values[0]['chain']:
                net=step['network'];path=Path(directory)/net['peer_file'];raw=path.read_bytes()
                path.write_bytes(raw+b' ')
                with self.subTest(target=step['target']),self.assertRaises(ValueError):p.verify(*values,out=directory)
                path.write_bytes(raw)
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
