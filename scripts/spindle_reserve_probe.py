"""Real remaining Spindle activation, private history, probe, echo and cleanup.

No capability is inferred from the source rows: the actual held BOOT roster is
verified first. Native replay owns source/image binding and full pixel replay.
"""
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import re
import socket
import threading
import time
from history_roster_evidence import source_spindles, verify_boot_roster

ROOT = Path(__file__).resolve().parents[1]
ECHO = bytes((17*i+78)&255 for i in range(1025))
END = r"(?=\r?\n)"


def fnv(data):
    value=0xcbf29ce484222325
    for byte in data:value=((value^byte)*0x100000001b3)&((1<<64)-1)
    return hex(value)


def peer_bytes(peer):return (json.dumps(peer,sort_keys=True,indent=2)+'\n').encode()


def check_peer(peer):
    if peer.get('errors')!=[] or peer.get('completed')is not True or len(peer.get('connections',[]))!=1:
        raise ValueError('echo peer incomplete, extra connection or error')
    row=peer['connections'][0]
    if row.get('request_hex')!=ECHO.hex() or row.get('response_hex')!=ECHO.hex() or row.get('received')!=1025 or row.get('sent')!=1025:
        raise ValueError('echo peer exact payload differs')
    if peer.get('endpoint')!=['127.0.0.1',46078]:raise ValueError('echo peer endpoint differs')


@contextmanager
def echo_peer(path):
    """One bounded real TCP exchange; an extra connection is an error, not retry."""
    path=Path(path)
    if path.exists():raise ValueError('peer evidence must be new')
    peer=dict(endpoint=['127.0.0.1',46078],connections=[],errors=[],completed=False)
    stop=threading.Event();connections=[]
    listener=socket.socket();listener.bind(('127.0.0.1',46078));listener.listen(1);listener.settimeout(.1)
    def worker():
        try:
            deadline=time.monotonic()+180
            while not stop.is_set():
                if time.monotonic()>deadline:raise TimeoutError('echo accept deadline')
                try:conn,remote=listener.accept();break
                except socket.timeout:continue
            else:return
            connections.append(conn);conn.settimeout(30);data=bytearray()
            while True:
                part=conn.recv(512)
                if not part:break
                data.extend(part)
                if len(data)>len(ECHO):raise ValueError('oversized echo request')
            if bytes(data)!=ECHO:raise ValueError('wrong echo request')
            sent=0
            for offset in range(0,len(ECHO),13):
                part=ECHO[offset:offset+13];conn.sendall(part);sent+=len(part)
            conn.shutdown(socket.SHUT_WR);conn.close()
            peer['connections'].append(dict(request_hex=bytes(data).hex(),response_hex=ECHO.hex(),received=len(data),sent=sent,remote=list(remote)))
            peer['completed']=True
            while not stop.is_set():
                try:extra,_=listener.accept();extra.close();raise ValueError('unexpected extra echo connection')
                except socket.timeout:continue
        except Exception as error:
            if not (stop.is_set() and isinstance(error,OSError)):peer['errors'].append(repr(error))
    thread=threading.Thread(target=worker,daemon=True);thread.start()
    try:yield peer
    finally:
        stop.set();listener.close()
        for conn in connections:
            try:conn.shutdown(socket.SHUT_RDWR)
            except OSError:pass
            conn.close()
        thread.join(timeout=2)
        if thread.is_alive():peer['errors'].append('peer worker did not stop')
        with path.open('xb')as file:file.write(peer_bytes(peer))
    check_peer(peer)


def _one(text,pattern,start,end):
    if type(start)is not int or type(end)is not int or not 0<=start<end<=len(text):raise ValueError('invalid Spindle evidence window')
    records=list(re.finditer(pattern+END,text[start:end]))
    if len(records)!=1:raise ValueError('missing/duplicate current Spindle receipt: '+pattern)
    return records[0]


def _save(text,target,task,start,end):
    saved=_one(text,rf'\[spindle.history.receipt\] target={target} task={task} kind=saved handle=([1-9]\d*) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0',start,end)
    durable=_one(text,rf'\[files.history.durable\] logical=12 target={target} generation=([1-9]\d*) bank=([01]) bytes={saved[2]} hash={saved[4]} barriers=3 readback=1',start,end)
    opened=_one(text,rf'\[files.history.open\] target={target} handle={saved[1]} mode=save seq=([1-9]\d*)',start,end)
    committed=_one(text,rf'\[files.history.commit\] target={target} handle={saved[1]} seq=([1-9]\d*) bytes={saved[2]} hash={saved[4]}',start,end)
    closed=_one(text,rf'\[files.history.close\] target={target} handle={saved[1]} seq=([1-9]\d*) ok=1',start,end)
    if not opened.start()<durable.start()<committed.start()<closed.start()<saved.start() or not int(opened[1])<int(committed[1])<int(closed[1]) or not 24<=int(saved[2])<=4096 or not 0<=int(saved[3])<=128:raise ValueError('durable save order/size')
    return dict(handle=int(saved[1]),bytes=int(saved[2]),count=int(saved[3]),hash=saved[4],generation=int(durable[1]),bank=int(durable[2]))


def _frame(frame,frames,text,kind,target,expected,start,end):
    if not isinstance(frame,dict) or sum(candidate==frame for candidate in frames)!=1:raise ValueError('missing/duplicate immutable capture reference')
    if (frame.get('kind'),frame.get('target'),frame.get('expected'))!=(kind,target,expected):raise ValueError('capture identity/expectation differs')
    offset=frame.get('serial_end')
    if type(offset)is not int or not start<offset<=end or offset>len(text):raise ValueError('capture is stale/outside action')
    if frame.get('serial_sha256')!=hashlib.sha256(text[:offset].encode()).hexdigest() or frame.get('proof',{}).get('ok')is not True:raise ValueError('capture prefix/proof mismatch')


def verify(result,frames,text,rows,*,out=None):
    if rows!=source_spindles(ROOT) or len(rows)<3 or result.get('catalog')!=rows:raise ValueError('different source Spindle roster')
    roster=verify_boot_roster(ROOT,text)
    if result.get('roster')!=roster:raise ValueError('different held BOOT roster')
    tasks={p['target']:p['task']for p in roster['participants']}
    chain=result.get('chain')
    if not isinstance(chain,list) or len(chain)!=len(rows)-2:raise ValueError('skipped reserve incarnation')
    previous=0;previous_terminal=None
    for step,row in zip(chain,rows[2:]):
        target=row['target'];task=tasks[target];pre=rows[row['ordinal']-1]['target'];pre_task=tasks[pre]
        identity=dict(target=target,task=task,surface=row['surface'],frame_id=row['frame'],predecessor=pre,logical=rows[0]['target'],incarnation=row['ordinal']+1)
        if any(step.get(k)!=v for k,v in identity.items()):raise ValueError('reserve identity differs')
        a,b,c,d,z=(step.get(k)for k in('serial_start','recall_start','recall_execute_start','close_start','serial_end'))
        if any(type(v)is not int for v in(a,b,c,d,z)) or not previous<=a<b<c<d<z<=len(text):raise ValueError('reserve windows reversed/stale')
        previous=z
        _one(text,rf'\[app.activation.committed\] logical=12 incarnation={row["ordinal"]+1} predecessor={pre} successor={target} task={task} surface={row["surface"]} pages=[1-9]\d* runtime_alloc=0 runtime_mint=0 bsp=1',a,b)
        _one(text,rf'\[shell.app.relaunch\] predecessor={pre} successor={target} task={task} surface={row["surface"]} receipt_replayed=1',a,b)
        prior=list(re.finditer(rf'\[files.history.durable\] logical=12 target={pre} generation=(\d+) bank=([01]) bytes=(\d+) hash=(0x[0-9a-f]+) barriers=3 readback=1'+END,text[:a]))
        if not prior:raise ValueError('missing predecessor durable history')
        p=prior[-1]
        if previous_terminal is not None and (int(p[1]),int(p[2]),int(p[3]),p[4])!=(previous_terminal['generation'],previous_terminal['bank'],previous_terminal['bytes'],previous_terminal['hash']):
            raise ValueError('successor did not recover predecessor terminal echo generation')
        recovery=_one(text,rf'\[files.history.recovered\] logical=12 source={pre} source_task={pre_task} target={target} generation={p[1]} bytes={p[3]} source_hash={p[4]} owned_hash=(0x[0-9a-f]+) data_hash=(0x[0-9a-f]+) boot_grant=53',a,b)
        loaded=_one(text,rf'\[spindle.history.receipt\] target={target} task={task} kind=loaded handle=([1-9]\d*) bytes={p[3]} count=(\d+) hash={recovery[1]} error=0',a,b)
        opened=_one(text,rf'\[files.history.open\] target={target} handle={loaded[1]} mode=load seq=([1-9]\d*)',a,b)
        closed=_one(text,rf'\[files.history.close\] target={target} handle={loaded[1]} seq=([1-9]\d*) ok=1',a,b)
        if not recovery.start()<opened.start()<closed.start()<loaded.start() or int(opened[1])>=int(closed[1]):raise ValueError('load before authorized recovery/fresh handle completion')
        recovered=dict(source=pre,source_task=pre_task,generation=int(p[1]),bytes=int(p[3]),source_hash=p[4],owned_hash=recovery[1],data_hash=recovery[2],handle=int(loaded[1]),count=int(loaded[2]))
        if step.get('recovery')!=recovered:raise ValueError('recovery result metadata differs')
        _one(text,rf'\[sext.app.pool.ready\] target={pre} task={pre_task} bytes=[1-9]\d* zeroed=[1-9]\d* workspace_records=\d+ reused=1 zero_owner=sext ok=1',0,a)
        for sid in [row['surface']]+_children(ROOT,target):
            if not re.search(rf'\[spindle.view.text\] target={target} sid={sid} sequence=\d+ paint=\d+ cols=\d+ rows=\d+ bytes=\d+'+END,text[a:b]):raise ValueError('missing current pane')
        recall='echo alpha'+str(pre)
        _one(text,rf'\[spindle.history.nav\] dir=up idx=0 len={len(recall)} ok=1',b,c)
        _frame(step['recall_frame'],frames,text,'spindle',target,'> '+recall,b,c)
        commands=step.get('commands',[])
        if len(commands)!=6:raise ValueError('incomplete command/probe sequence')
        obj=step.get('object')
        if type(obj)is not int or not 3<=obj<15:raise ValueError('invalid fresh probe object')
        expected=[recall,'echo alpha'+str(target),'mkdoc thirdprobe',f'filldocx {obj} 33',f'catdocx {obj}',f'rmdoc {obj}']
        last=c;generation=int(p[1])
        for i,(cmd,wanted)in enumerate(zip(commands,expected)):
            start,end=cmd.get('serial_start'),cmd.get('serial_end')
            if cmd.get('command')!=wanted or type(start)is not int or type(end)is not int or not last<=start<end<=d:raise ValueError('wrong/missing/reordered command')
            last=end
            _one(text,rf'\[spindle.instance.command\] target={target} task={task} name={wanted.split()[0]} ok=1',start,end)
            saved=_save(text,target,task,start,end)
            if cmd.get('saved')!=saved or saved['generation']<=generation:raise ValueError('wrong durable generation/save metadata')
            generation=saved['generation']
            if i>=2:
                job=[re.escape('Create('+str(list(b'thirdprobe'+bytes(6)))+')'),rf'Fill\({obj}, 33, true\)',rf'Verify\({obj}, true\)',rf'Delete\({obj}\)'][i-2]
                value=[obj,33,33,0][i-2]
                receipt=_one(text,rf'\[spindle.probe.result\] target={target} kind=done job={job} value={value} error=0 hash=(0x[0-9a-f]+)',start,end)
                if i==4 and receipt[1]!=fnv(bytes((37*n+11)&255 for n in range(33))):raise ValueError('probe payload fingerprint differs')
        net=step['network'];n,e=net['serial_start'],net['serial_end']
        if not last<=n<e<=d:raise ValueError('network window overlaps prior command/close')
        _one(text,rf'\[spindle.instance.command\] target={target} task={task} name=net-probe ok=1',n,e)
        data=_one(text,rf'\[browser.net.job.bytes\] ticket=([1-9]\d*) length=1025 hash={fnv(ECHO)} exact=1 endpoint=controlled_peer tls=0 web=0',n,e)
        ticket=int(data[1])
        if not re.search(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: {target}, task: {task} \}}\) ticket={ticket} [^\r\n]*scope=private_job'+END,text[n:n+data.start()]):raise ValueError('echo ticket lacks exact service owner')
        released=_one(text,rf'\[browser.net.job.released\] ticket={ticket} view=Verified private_job=0',n,e)
        verified=_one(text,rf'\[spindle.net.state\] target={target} task={task} state=verified busy=0 generation=\d+ fixed_peer=1 web=0',n,e)
        if not data.start()<released.start()<verified.start():raise ValueError('verification precedes positive RELEASE')
        net_saved=_save(text,target,task,n,e)
        if net.get('saved')!=net_saved or net_saved['generation']<=generation:raise ValueError('network command history not durable')
        check_peer(net['peer']);raw=peer_bytes(net['peer'])
        if net.get('peer_sha256')!=hashlib.sha256(raw).hexdigest() or net.get('peer_file')!=f'spindle-{target}-echo-peer.json':raise ValueError('peer transcript binding differs')
        if out is not None and (Path(out)/net['peer_file']).read_bytes()!=raw:raise ValueError('raw peer transcript differs')
        _frame(net['frame'],frames,text,'spindle',target,'NET / verified',n+verified.end(),e)
        terminal=step.get('terminal_echo')
        if not isinstance(terminal,dict):raise ValueError('missing terminal echo')
        t,u=terminal.get('serial_start'),terminal.get('serial_end')
        if terminal.get('command')!='echo alpha'+str(target) or type(t)is not int or type(u)is not int or not e<=t<u<=d:raise ValueError('terminal echo outside post-network/pre-close window')
        _one(text,rf'\[spindle.instance.command\] target={target} task={task} name=echo ok=1',t,u)
        terminal_saved=_save(text,target,task,t,u)
        if terminal.get('saved')!=terminal_saved or terminal_saved['generation']<=net_saved['generation']:raise ValueError('terminal echo lacks fresh durable save')
        # Nothing may replace the controlled final history before Close.
        final_durable=list(re.finditer(rf'\[files.history.durable\] logical=12 target={target} [^\r\n]*'+END,text[t:d]))
        if len(final_durable)!=1:raise ValueError('terminal echo is not final durable history')
        previous_terminal=terminal_saved
        finished=_one(text,rf'\[task.finish\] pd={target} task={task} reason=0 terminal=1',d,z)
        ack_positions=[]
        for role,holder in [(1,3),(2,1),(4,11),(8,13),(16,10)]:
            ack=_one(text,rf'\[app.service.ack\] target={target} task={task} holder={holder} role={role} acked=\d+ required=31',d,z)
            ack_positions.append(ack.start())
        retired=_one(text,rf'\[app.private.retired\] target={target} task={task} pages=[1-9]\d* bytes=[1-9]\d* acked=31 required=31 all_flushed=1 pool_reusable=1 messages=\d+ signals=\d+ metadata=retained runtime_alloc=0',d,z)
        pool=_one(text,rf'\[sext.app.pool.ready\] target={target} task={task} bytes=[1-9]\d* zeroed=[1-9]\d* workspace_records=\d+ reused=1 zero_owner=sext ok=1',d,z)
        if not finished.start()<min(ack_positions)<=max(ack_positions)<retired.start()<pool.start():raise ValueError('cleanup ACK/retirement/pool ordering')
    start=result.get('exhaustion_start');frame=result['exhaustion_frame']
    if type(start)is not int or not previous<=start<frame['serial_end']:raise ValueError('exhaustion before last close')
    spanend=frame['serial_end']
    if not re.search(r'\[shell.palette.item\] idx=0 [^\r\n]*'+END,text[start:spanend]):raise ValueError('missing fresh palette open')
    offset=result.get('exhaustion_row_offset')
    if type(offset)is not int or not start<=offset<spanend or not re.match(r'\[launcher.text.row\] index=0 row=0 name=Open Spindle status=CLOSED / RESERVE EXHAUSTED'+END,text[offset:spanend]):raise ValueError('exhaustion row binding differs')
    current=list(re.finditer(r'\[launcher.text.row\] index=0 row=0 name=Open Spindle status=([^\r\n]*)'+END,text[start:spanend]))
    if not current or current[-1][1]!='CLOSED / RESERVE EXHAUSTED' or result.get('reserve_exhausted')is not True:raise ValueError('current exhaustion row differs')
    _frame(frame,frames,text,'launcher',12,'Open Spindle',start,spanend)
    return dict(ok=True,targets=[step['target']for step in chain],network_bytes=1025,terminal=True,reserve_exhausted=True)


def _children(root,target):
    source=(Path(root)/'crates/sex-pdx/src/app_catalog.rs').read_text()
    blocks=[b for b in re.findall(r'Incarnation\s*\{([^{}]+)\}',source) if re.search(rf'\btarget:\s*{target}\s*,',b)]
    if len(blocks)!=1:raise ValueError('missing literal Spindle pane catalog')
    match=re.search(r'children:\s*&\[([0-9, ]+)\]',blocks[0])
    children=[int(v.strip())for v in match[1].split(',')if v.strip()]if match else[]
    if len(children)!=3 or len(set(children))!=3 or any(v==0 for v in children):raise ValueError('Spindle requires exact three child panes')
    return children


def run(c,close,command):
    root,out=Path(c['root']),Path(c['out'])
    rows=source_spindles(root);roster=verify_boot_roster(root,c['text']())
    if len(rows)<3:raise ValueError('no remaining Spindle reserve')
    tasks={p['target']:p['task']for p in roster['participants']}
    text,wait,key,focus=(c[k]for k in('text','wait','key','focus'));steps=[]
    for row in rows[2:]:
        target=row['target'];task=tasks[target];pre=rows[row['ordinal']-1]['target'];pre_task=tasks[pre]
        step=dict(target=target,task=task,surface=row['surface'],frame_id=row['frame'],predecessor=pre,logical=12,incarnation=row['ordinal']+1,serial_start=len(text()))
        start=step['serial_start']
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        key('ret',rf'\[shell.app.relaunch\] predecessor={pre} successor={target} task={task} ')
        wait(rf'\[app.activation.committed\] logical=12 incarnation={row["ordinal"]+1} predecessor={pre} successor={target} task={task} surface={row["surface"]} ',start)
        recovered=wait(rf'\[files.history.recovered\] logical=12 source={pre} source_task={pre_task} target={target} generation=(\d+) bytes=(\d+) source_hash=(0x[0-9a-f]+) owned_hash=(0x[0-9a-f]+) data_hash=(0x[0-9a-f]+) boot_grant=53'+END,start)
        loaded=wait(rf'\[spindle.history.receipt\] target={target} task={task} kind=loaded handle=([1-9]\d*) bytes={recovered[2]} count=(\d+) hash={recovered[4]} error=0'+END,start)
        step['recovery']=dict(source=pre,source_task=pre_task,generation=int(recovered[1]),bytes=int(recovered[2]),source_hash=recovered[3],owned_hash=recovered[4],data_hash=recovered[5],handle=int(loaded[1]),count=int(loaded[2]))
        for sid in [row['surface']]+_children(root,target):wait(rf'\[spindle.view.text\] target={target} sid={sid} ',start)
        focus(row['frame']);step['recall_start']=len(text());recall='echo alpha'+str(pre)
        key('up',rf'\[spindle.history.nav\] dir=up idx=0 len={len(recall)} ok=1')
        step['recall_frame']=c['capture'](f'reserve-spindle-{target}-recall','> '+recall,kind='spindle',target=target)
        commands=[];step['recall_execute_start']=len(text())
        c['q'].key('ret');wait(rf'\[spindle.instance.command\] target={target} task={task} name=echo ok=1',step['recall_execute_start'])
        def finish_command(value,begin):
            wait(rf'\[spindle.history.receipt\] target={target} task={task} kind=saved .*error=0',begin)
            snapshot=text();end=len(snapshot);saved=_save(snapshot,target,task,begin,end)
            commands.append(dict(command=value,serial_start=begin,serial_end=end,saved=saved))
        finish_command(recall,step['recall_execute_start'])
        value='echo alpha'+str(target)
        begin=command(value,target,rf'\[spindle.instance.command\] target={target} task={task} name=echo ok=1');finish_command(value,begin)
        value='mkdoc thirdprobe'
        begin=command(value,target,rf'\[spindle.probe.result\] target={target} kind=done job=Create\([^\r\n]+\) value=\d+ error=0')
        snapshot=text()
        match=_one(snapshot,rf'\[spindle.probe.result\] target={target} kind=done job=Create\([^\r\n]+\) value=(\d+) error=0 hash=0x[0-9a-f]+',begin,len(snapshot))
        obj=int(match[1]);step['object']=obj;finish_command(value,begin)
        for value,job in [(f'filldocx {obj} 33',rf'Fill\({obj}, 33, true\)'),(f'catdocx {obj}',rf'Verify\({obj}, true\)'),(f'rmdoc {obj}',rf'Delete\({obj}\)')]:
            begin=command(value,target,rf'\[spindle.probe.result\] target={target} kind=done job={job} value=\d+ error=0');finish_command(value,begin)
        step['commands']=commands
        peer_file=f'spindle-{target}-echo-peer.json'
        with echo_peer(out/peer_file)as peer:
            begin=command('net-probe',target,rf'\[spindle.instance.command\] target={target} task={task} name=net-probe ok=1')
            wait(rf'\[spindle.net.state\] target={target} task={task} state=verified busy=0 ',begin,timeout=180)
            wait(rf'\[spindle.history.receipt\] target={target} task={task} kind=saved .*error=0',begin)
            frame=c['capture'](f'reserve-spindle-{target}-network','NET / verified',kind='spindle',target=target)
        end=len(text());step['network']=dict(serial_start=begin,serial_end=end,peer=peer,peer_file=peer_file,peer_sha256=hashlib.sha256(peer_bytes(peer)).hexdigest(),frame=frame,saved=_save(text(),target,task,begin,end))
        value='echo alpha'+str(target)
        begin=command(value,target,rf'\[spindle.instance.command\] target={target} task={task} name=echo ok=1')
        wait(rf'\[spindle.history.receipt\] target={target} task={task} kind=saved .*error=0',begin)
        snapshot=text();end=len(snapshot)
        step['terminal_echo']=dict(command=value,serial_start=begin,serial_end=end,saved=_save(snapshot,target,task,begin,end))
        step['close_start']=len(text());close(target,row['frame']);step['serial_end']=len(text());steps.append(step)
    exhaustion_start=len(text());key('grave_accent',r'\[shell.palette.item\] idx=0')
    row=wait(r'\[launcher.text.row\] index=0 row=0 name=Open Spindle status=CLOSED / RESERVE EXHAUSTED'+END,exhaustion_start)
    frame=c['capture']('third-spindle-exhausted-launcher','Open Spindle',target=12,kind='launcher')
    key('esc',r'\[command_palette.close\]')
    result=dict(roster=roster,catalog=rows,chain=steps,exhaustion_start=exhaustion_start,exhaustion_row_offset=exhaustion_start+row.start(),exhaustion_frame=frame,reserve_exhausted=True)
    verify(result,[f for step in steps for f in [step['recall_frame'],step['network']['frame']]]+[frame],text(),rows,out=out)
    return result
