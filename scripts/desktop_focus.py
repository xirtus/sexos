"""Focus real desktop frames while preserving Spindle's app-local Tab key."""
import re


def focus_surface(sid, text, key):
    for _ in range(24):
        ids = re.findall(r'\[shell.focus.set\] id=(\d+)', text())
        current = int(ids[-1]) if ids else 0
        if current == sid:
            return
        # Re-evaluate after every transition: a Tab cycle can enter Spindle.
        if current in (153, 306) or sid in (153, 306):
            # Wait for the toggle's focus outcome. Its focus change and action
            # record arrive in either order; re-reading focus before the outcome
            # would see stale focus and toggle back. Either outcome record may be
            # cut by interleaved serial output, so match each by its own tail.
            key('scroll_lock', r'ToggleSpindle ok=\d reason=\w+ frame=\d+ sid=\d+|old=\d+ new=\d+ frame=\d+ reason=ToggleSpindle')
        else:
            key('tab', r'\[shell.focus.set\] id=\d+')
    raise RuntimeError('bounded focus traversal failed for '+str(sid))
