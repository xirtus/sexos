"""Prove a saved name reaches Linen automatically, then inspect its real glyphs."""
import re
from check_desktop_pixels import final_view


def after_save(c, start, count, name):
    wait,text=(c[k]for k in ['wait','text'])
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
    frame=c['capture']('auto-list-'+name,kind='linen')
    c['focus'](201)
    return dict(name=name,count=count,selection_preserved=True,background_focus_preserved=True,
                manual_rescan=False,pixel_errors=[],frame=frame)
