# SPDX-License-Identifier: GPL-3.0-only
"""Run one of Polyamour's own mesh operators on a BMesh: the BMesh goes to a
temporary object, the operator runs in its Edit Mode, the result comes back.
The scene's selection, active object and mode are restored after."""


def run_on_copy(bm, idname, select_mode='VERT', **opts):
    from polyamour import polyops as P
    from polyamour._base import first, req
    from polyamour.mesh import Mesh
    if not P.get().live:
        raise RuntimeError('this bmesh operator needs a live Polyamour')
    ids = req('ids')
    mode, active, selected = ids['mode'], ids['active'], ids['selected']
    if mode != 'OBJECT':
        req('mode_set', 'OBJECT')
    mname = first(req('mesh_new', '__bmesh_tmp'))
    me = Mesh(mname)
    bm.to_mesh(me)
    me._flush_now()
    oname = first(req('add_object', '__bmesh_tmp', data=mname))
    try:
        req('mode_set', 'EDIT')
        req('mesh_select', mname, mode=P.Raw(select_mode))
        req('op', idname, **opts)
        req('mode_set', 'OBJECT')
        bm.clear()
        bm.from_mesh(Mesh(mname))
    finally:
        req('delete', oname)
        req('remove', 'mesh', mname)
        if selected:
            req('select', *selected)
        else:
            req('deselect')
        req('active', active if active else P.Raw('none'))
        if mode != 'OBJECT':
            req('mode_set', mode)
