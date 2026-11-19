# SPDX-License-Identifier: GPL-3.0-only
# Handlers and timers: the active object bobs as the frame changes (press
# Space to play), and a timer prints three times.
import math

import bpy

target = bpy.context.active_object.name


@bpy.app.handlers.persistent
def bob(scene, depsgraph=None):
    ob = bpy.data.objects.get(target)
    if ob is not None:
        ob.location.z = math.sin(scene.frame_current / 8) * 0.5


# Remove an earlier copy of this handler before adding it again.
for h in list(bpy.app.handlers.frame_change_post):
    if getattr(h, "__name__", "") == "bob":
        bpy.app.handlers.frame_change_post.remove(h)
bpy.app.handlers.frame_change_post.append(bob)

ticks = [0]


def tick():
    ticks[0] += 1
    print("timer", ticks[0])
    return None if ticks[0] >= 3 else 1.0


bpy.app.timers.register(tick, first_interval=1.0)
print("Handler and timer registered for", target)
