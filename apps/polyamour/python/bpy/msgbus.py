# SPDX-License-Identifier: GPL-3.0-only
"""bpy.msgbus: accepted and kept, but Polyamour does not publish property
changes yet, so subscribers are not called (use bpy.app.handlers)."""

_subs = []


def subscribe_rna(key, owner, args, notify, options=set()):
    _subs.append((key, owner, args, notify))


def publish_rna(key):
    for k, owner, args, notify in list(_subs):
        if k == key:
            notify(*args)


def clear_by_owner(owner):
    _subs[:] = [s for s in _subs if s[1] is not owner]
