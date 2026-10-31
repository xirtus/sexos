"""Actual third Quil activation, inherited workspace recovery, input and cleanup."""
import re

def open_selected(c):
    """Serialize a real row Open through its actual completion, then load.

    A time gap cannot establish absence of an asynchronous Open intent.
    This probe has only one pending row Open, so the completion fences the
    optional intent before focus/traversal or another Open may be issued.
    """
    offset=len(c['text']());c['q'].key('ret')
    c['wait'](r'\[linen.open.queued\] id=\d+ accepted=1',offset)
    c['wait'](r'\[linen.open.complete\] status=0',offset)
    intent=re.search(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id=(\d+) ok=1',c['text']()[offset:])
    if intent is None:return None
    object_id=int(intent[1])
    # A selected row that is not a supported document reports an explicit
    # failed load; it is not the saved object being sought.
    loaded=c['wait'](r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(object_id)+r'(?:\s|$)|\[quil.document.result\] outcome=Failed ',offset)
    if loaded[0].startswith('[quil.document.result]'):return None
    return dict(object=object_id,bytes=int(loaded[1]),hash=loaded[2])

def run(c,refresh,close,rows=None):
    """Exercise every remaining BOOT Quil, retaining legacy short-fixture support."""
    key,wait,text,focus=(c[k]for k in ('key','wait','text','focus'))
    extended=rows is not None
    chain=rows[2:]if extended else [dict(target=18,surface=307,frame=307,ordinal=2)]
    steps=[];first_frame=None;first_loaded=None
    for index,row in enumerate(chain):
        target,surface,frame_id=row['target'],row['surface'],row['frame']
        predecessor=rows[row['ordinal']-1]['target']if extended else 16
        start=len(text())
        key('grave_accent',r'\[shell.palette.item\] idx=0');key('j',r'\[shell.palette.select\] old=0 new=1')
        key('ret',rf'\[shell.app.relaunch\] predecessor={predecessor} successor={target} ')
        wait(rf'\[app.activation.committed\] logical=9 incarnation={row["ordinal"]+1} predecessor={predecessor} successor={target} .*runtime_mint=0',start)
        wait(rf'\[app.surface.presented\] target={target} surface={surface} .*ok=1',start)
        refresh();loaded=[]
        for expected in [c['first'],c['second']]:
            focus(200)
            for _ in range(20):
                key('j',r'\[shell.action.select_next_linen\] id=\d+')
                observed=open_selected(c)
                if observed and observed['object']==expected['object']:
                    if(observed['bytes'],observed['hash'])!=(expected['bytes'],expected['hash']):raise RuntimeError('reserve incarnation document changed')
                    loaded.append(dict(observed));break
                focus(200)
            else:raise RuntimeError('reserve incarnation selection unavailable')
        focus(frame_id);edit_start=len(text());key('t',r'\[quil.document.edit\]')
        frame=c['capture']('third-quil-edit'if index==0 else f'reserve-quil-{target}-edit','t#',target=target)
        second=c['second'];second['content']+='t'
        offset=len(text());c['ctrl']('s',r'\[quil.document.saved\].*dirty=0')
        m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(second['object']),offset)
        second.update(bytes=int(m[1]),hash=m[2]);saved=dict(second)
        close_start=len(text());close(target,frame_id)
        steps.append(dict(target=target,surface=surface,logical=9,incarnation=row['ordinal']+1,predecessor=predecessor,serial_start=start,edit_start=edit_start,save_start=offset,close_start=close_start,serial_end=len(text()),loaded=loaded,saved=saved,frame=frame))
        if index==0:first_frame=frame;first_loaded=[d['object']for d in loaded]
    # A fresh physical selection after the LAST cleanup must show exhaustion.
    key('grave_accent',r'\[shell.palette.item\] idx=0');selection_start=len(text())
    selection_pattern=r'\[shell.palette.select\] old=0 new=1(?=\r?\n|$)'
    key('j',selection_pattern);selection=wait(selection_pattern,selection_start);row_start=selection_start+selection.end()
    row=wait(r'\[launcher.text.row\] index=1 row=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED(?=\r?\n|$)',row_start)
    exhaustion_frame=c['capture']('third-quil-exhausted-launcher','Open Quil',target=9,kind='launcher');key('esc',r'\[command_palette.close\]')
    result=dict(target=18,surface=307,logical=9,incarnation=3,documents=first_loaded,edited_and_saved=True,pixel_errors=[],frame=first_frame,terminal=True,reserve_exhausted=True,exhaustion_frame=exhaustion_frame,exhaustion_selection_offset=selection_start+selection.start(),exhaustion_row_offset=row_start+row.start())
    if extended:result.update(chain=steps,final_target=chain[-1]['target'],append_count=len(steps),catalog=rows)
    return result
