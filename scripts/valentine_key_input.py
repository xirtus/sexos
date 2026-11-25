"""Type fixture text through QEMU's PC keycodes."""
import time

def send_text(q, value, delay=0.06):
    for char in value:
        if char in ('@', ':'):
            key = '2' if char == '@' else 'semicolon'
            q.cmd('send-key', {'keys': [
                {'type': 'qcode', 'data': 'shift'},
                {'type': 'qcode', 'data': key},
            ], 'hold-time': 80})
        else:
            q.key({' ': 'spc', '.': 'dot', '/': 'slash'}.get(char, char))
        time.sleep(delay)
