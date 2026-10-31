"""Focus real desktop frames while preserving Spindle's app-local Tab key."""
import re
import time


def focus_surface(sid, text, key):
    for _ in range(24):
        ids = re.findall(r'\[shell.focus.set\] id=(\d+)', text())
        current = int(ids[-1]) if ids else 0
        if current == sid:
            return
        # Re-evaluate after every transition: a Tab cycle can enter Spindle.
        if current in (153, 306) or sid in (153, 306):
            # The toggle's focus change and its action record arrive in either
            # order, and outcome records can be cut by interleaved serial
            # output. Wait for a focus change after the key itself; re-reading
            # stale focus would toggle back.
            start = len(text())
            key('scroll_lock', r'\[shell.action.spindle\] toggle')
            end = time.monotonic()+5
            while time.monotonic() < end and not re.search(r'\[shell.focus.set\] id=\d+', text()[start:]):
                time.sleep(.05)
        else:
            key('tab', r'\[shell.focus.set\] id=\d+')
    raise RuntimeError('bounded focus traversal failed for '+str(sid))
