"""Focus real desktop frames with completed, action-specific keyboard receipts."""
import re
import time


_TAB_RESULT = (r'\[shell\.kbd\.ui\.result\] action=AccessFocusNext ok=1 reason=\S+ frame=\d+ sid=\d+'
               r'|\[shell\.app\.switch\] old=\d+ new=\d+ live=1 shortcut=alt_tab')
_SPINDLE_RESULT = r'\[shell\.kbd\.ui\.result\] action=ToggleSpindle ok=1 reason=\S+ frame=\d+ sid=\d+'


def _completed(trace, code, action, proof=False):
    # Input's send receipt may race Shell's handling, so do not impose ordering
    # between producers. Normal images preserve edges without secret key codes.
    raw = proof or re.search(r'\[input\.keyboard\.key(?:down|up)\.ok\] code=', trace) is not None
    payload = rf'code={code} source=(ps2|usb)' if raw else r'source=(ps2|usb) payload=redacted'
    downs = re.finditer(rf'\[input\.keyboard\.keydown\.ok\] {payload}\r?\n', trace)
    # Pair in Input's own order and on the same device. Cross-producer Shell
    # receipt order stays unconstrained: Input may log release before Shell runs.
    paired = False
    for down in downs:
        source = down[1]
        up_payload = rf'code={code} source={source}' if raw else rf'source={source} payload=redacted'
        if re.search(rf'\[input\.keyboard\.keyup\.ok\] {up_payload}\r?\n', trace[down.end():]):
            paired = True
            break
    if not paired:
        return False
    if action == 'AccessFocusNext' and re.search(
            r'\[shell\.app\.switch\] old=\d+ new=\d+ live=1 shortcut=alt_tab\r?\n', trace):
        return True  # Clipboard supplies one explicit Alt+Tab packet.
    if raw:
        make = re.search(rf'\[shell\.kbd\.ui\.action\] scancode={code} action={action}(?:\s|$)', trace)
        if not make:
            return False
        trace = trace[make.end():]
    return re.search(rf'\[shell\.kbd\.ui\.result\] action={action} ok=1 reason=\S+ frame=\d+ sid=\d+\r?\n', trace) is not None


def focus_surface(sid, text, key):
    for _ in range(24):
        ids = re.findall(r'\[shell.focus.set\] id=(\d+)', text())
        current = int(ids[-1]) if ids else 0
        if current == sid:
            return
        # Serialize every make/action/release before another traversal or Close.
        # A delayed app-open focus receipt cannot acknowledge this keyboard act.
        spindle = current in (153, 306) or sid in (153, 306)
        code, action = (70, 'ToggleSpindle') if spindle else (15, 'AccessFocusNext')
        proof = bool(re.search(r'\[input\.payload\.mode\] component=sexinput mode=proof(?:\s|$)', text()))
        start = len(text())
        key('scroll_lock' if spindle else 'tab', _SPINDLE_RESULT if spindle else _TAB_RESULT)
        end = time.monotonic() + 5
        while not _completed(text()[start:], code, action, proof=proof):
            if time.monotonic() >= end:
                raise RuntimeError('missing completed keyboard focus action: ' + action)
            time.sleep(.01)
    raise RuntimeError('bounded focus traversal failed for ' + str(sid))
