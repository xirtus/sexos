# SPDX-License-Identifier: GPL-3.0-only
"""Audit a protected typing interval; never confuse userspace and kernel scope."""
import re
USERSPACE=[
 r'\[pdx\.listen\.raw\.wrapper\][^\n]*\ba[012]=',
 r'\[(?:sexinput\.(?:ps2\.scancode|key\.(?:send|rejected)|kbd\.recv)|input\.keyboard\.key(?:down|up)\.ok)\][^\n]*\b(?:raw|key|code|mod|modifiers)=',
 r'\[(?:silk-shell\.(?:hid\.(?:raw|recv)|key\.(?:recv|route))|silk\.key\.route|shell\.(?:key\.ev_key\.received|browser\.input|key\.action|kbd\.ui\.(?:consume|action)))[^\n]*\b(?:a[012]|key|code|scancode|mod|value|printable)=',
 r'\[(?:usb\.keyboard\.shell\.recv|shell\.media\.key|(?:bell|atlas|collar)\.key\.recv)\][^\n]*\b(?:key|code|mod)=',
 r'\[silk\.text\.append\][^\n]*\bch=',
 r'\[sexusb\.input\.rejected\][^\n]*\barg[012]=',
 r'\[(?:keyboard_cursor\.key|keyboard\.cursor\.debug\.(?:keytrace|move|click))\][^\n]*\b(?:key|code)=',
 # CLI echo remains intentionally useful, but can never receive vault-focused keys.
 r'\[spindle\.(?:input\.recv|line\.enter|key\.char|key\.insert|echo)[^\n]*',
]
KERNEL=r'\[ps2\.(?:port60\.read|input_ring\.(?:enqueue|drop))\][^\n]*\bsc='

def audit_interval(trace):
    userspace=[line for line in trace.splitlines() if any(re.search(pattern,line) for pattern in USERSPACE)]
    kernel=[line for line in trace.splitlines() if re.search(KERNEL,line)]
    return dict(userspace_safe=not userspace,userspace_payload_lines=userspace,kernel_payload_lines=kernel,
                # A clean interval cannot prove the startup-budget kernel policy safe.
                complete_keyboard_privacy_proven=False,kernel_policy='separate source exception/review required')

def require_normal_image(g):
    for component in ('sexinput','shell','sexusb'):
        g['wait'](rf'\[input.payload.mode\] component={component} mode=(?:proof|redacted)',0,120)
        values=re.findall(rf'\[input.payload.mode\] component={component} mode=(proof|redacted)(?=\s|$)',g['text']())
        if values!=['redacted']:raise ValueError('private typing requires an explicitly normal metadata-only userspace image')
    if re.search(r'\[(?:keyboard_cursor\.gate|proof\.gate\.state)\] enabled=1',g['text']()):
        raise ValueError('private typing cannot use synthetic-input/keyboard-cursor diagnostic profiles')
