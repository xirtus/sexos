"""Prove a saved name reaches Linen automatically, then inspect its real glyphs."""
import re
import time
from check_desktop_pixels import final_view, verify


def after_save(c, start, count, name):
    wait,text,q=(c[k]for k in ['wait','text','q'])
    wait(r'\[linen.workspace.changed\] target=9 task=\d+ refresh=queued read_only=1',start)
    wait(r'\[linen.remote.snapshot.ok\] count='+str(count)+r' unique=1 complete=1 selection_preserved=1',start)
    ids=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
    if not ids or ids[-1]!='201':raise RuntimeError('background refresh stole Quil focus')
    # Snapshot already completed before any navigation. This uses plain Tab/J,
    # never the launcher Rescan action or a diagnostic storage command.
    c['focus'](200)
    for _ in range(count+1):
        try:
            view,labels=final_view(text())
            if any(identity==view['selected']and label==name for identity,_,label in labels):break
        except ValueError:pass
        c['key']('j',r'\[shell.action.select_next_linen\] id=[1-9]\d*')
    else:raise RuntimeError('new saved name absent from automatic list: '+name)
    path=c['out']/('auto-list-'+name+'.ppm');end=time.monotonic()+90;errors=['not painted']
    while time.monotonic()<end:
        q.cmd('screendump',{'filename':str(path)})
        errors=verify(text(),path.read_bytes(),c['font'],resolution=c['resolution'])
        if not errors:break
        time.sleep(.15)
    if errors:raise RuntimeError('automatic filename glyphs: '+str(errors))
    c['focus'](201)
    return dict(name=name,count=count,selection_preserved=True,background_focus_preserved=True,
                manual_rescan=False,pixel_errors=errors)
