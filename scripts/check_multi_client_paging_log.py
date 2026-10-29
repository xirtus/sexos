#!/usr/bin/env python3
"""Check observed two-client paging/reclamation on the parked-AP BSP lane."""
import argparse
from pathlib import Path
import re
import sys


def validate(text, quota=False):
    errors = []
    markers = ('boot.paging.genesis', 'boot.memory.authority.sealed', 'sext.client.ready',
               'paging.client.begin', 'paging.client.negative', 'paging.fault.trap',
               'paging.fault.pending', 'paging.leaf.commit', 'paging.fault.completed',
               'sext.fault.commit', 'sext.commit.negative', 'paging.client.page',
               'paging.client.recovered', 'paging.leaf.reclaim', 'sext.release',
               'sext.reclaim.negative', 'sext.reuse.negative', 'sext.release.denied',
               'paging.client.released', 'paging.client.reuse', 'paging.client.retained',
               'paging.client.quota.begin', 'paging.fault.rejected', 'sext.fault.reject')
    rows = {marker: [] for marker in markers}
    for marker in markers:
        pattern = rf'\[{re.escape(marker)}\](?: ([^\r\n]*))?(?:\r?$)'
        found = list(re.finditer(pattern, text, re.M))
        if text.count('[' + marker + ']') != len(found):
            errors.append('malformed record ' + marker)
        for m in found:
            fields = {}
            for part in (m[1] or '').split():
                if '=' not in part:
                    errors.append('partial record ' + marker)
                    continue
                key, value = part.split('=', 1)
                if key in fields:
                    errors.append('duplicate field ' + marker)
                fields[key] = value
            fields['_pos'] = m.start()
            rows[marker].append(fields)

    def count(marker, expected):
        if len(rows[marker]) != expected:
            errors.append(f'{marker} count {len(rows[marker])}, expected {expected}')

    def n(row, key):
        try:
            return int(row[key], 16 if row[key].startswith('0x') else 10)
        except (KeyError, ValueError):
            errors.append('missing/invalid numeric field ' + key)
            return -1

    def eq(row, **fields):
        for key, value in fields.items():
            if str(row.get(key)) != str(value):
                errors.append(f'wrong {key}: expected {value}, saw {row.get(key)}')

    for marker, expected in {
        'boot.paging.genesis': 2, 'boot.memory.authority.sealed': 1, 'sext.client.ready': 2,
        'paging.client.begin': 2, 'paging.client.negative': 1,
        'paging.fault.trap': 10 if quota else 9, 'paging.fault.pending': 10 if quota else 9,
        'paging.leaf.commit': 9, 'paging.fault.completed': 9, 'sext.fault.commit': 9,
        'sext.commit.negative': 9, 'paging.client.page': 8, 'paging.client.recovered': 1,
        'paging.leaf.reclaim': 4, 'sext.release': 5, 'sext.reclaim.negative': 4,
        'sext.reuse.negative': 4, 'sext.release.denied': 2,
        'paging.client.released': 1, 'paging.client.reuse': 1, 'paging.client.retained': 1,
        'paging.client.quota.begin': int(quota), 'paging.fault.rejected': int(quota),
        'sext.fault.reject': int(quota),
    }.items():
        count(marker, expected)
    if errors:
        return errors
    seal = rows['boot.memory.authority.sealed'][0]
    eq(seal, runtime_alloc=0, runtime_map='capability_leaf', runtime_mint=0)
    grants, bases, ready, begins = {}, {}, {}, {}
    for g in rows['boot.paging.genesis']:
        target = n(g, 'target')
        if target in grants or target not in (10, 15):
            errors.append('duplicate/foreign target grant')
        grants[target] = g
        eq(g, pager=14, pool_pages=8, arena_pages=8, quota=4, target_pkey=target,
           leaves_absent=8, tables_preallocated=1)
        if g['_pos'] >= seal['_pos']:
            errors.append('grant after seal')
    for r in rows['sext.client.ready']:
        target = n(r, 'target')
        if target in ready:
            errors.append('duplicate client readiness')
        ready[target] = r
        eq(r, frames=8, quota=4, pool_bytes=32768, arena_bytes=32768, zero_owner='sext')
        if n(r, 'target') not in grants or r['_pos'] <= seal['_pos']:
            errors.append('unsealed pager readiness')
    for b in rows['paging.client.begin']:
        target, base = n(b, 'pd'), n(b, 'base')
        if target in bases or target not in grants or base <= 0 or base % 4096:
            errors.append('client identity/address mismatch')
        bases[target] = base
        begins[target] = b
        eq(b, bytes=32768)
        if b['_pos'] <= seal['_pos']:
            errors.append('client admitted before seal')
    if set(bases) != {10, 15} or abs(bases.get(10, 0) - bases.get(15, 0)) < 32768:
        errors.append('client arenas alias')
        return errors
    eq(rows['paging.client.negative'][0], invalid_slot=1, high_alias=1, wrong_query=1, wrong_commit=1, wrong_reject=1, wrong_reclaim=1, ok=1)
    negative = rows['paging.client.negative'][0]
    if not seal['_pos'] < negative['_pos'] < begins[15]['_pos'] or set(ready) != {10, 15}:
        errors.append('client negative/readiness stage mismatch')
    pending, leaves, complete, commits = {}, {}, {}, {}
    last_token = 0
    tasks = {}
    traps = rows['paging.fault.trap']
    for index, p in enumerate(rows['paging.fault.pending']):
        token, target, task = n(p, 'token'), n(p, 'target'), n(p, 'task')
        if token <= last_token or target not in grants or task <= 0 or token in pending:
            errors.append('fault identity/token order')
        last_token = token
        if target in tasks and tasks[target] != task:
            errors.append('fault task changed')
        tasks[target] = task
        pending[token] = p
        t = traps[index]
        eq(t, pd=target, eligible=1)
        if n(t, 'addr') != n(p, 'page') or n(t, 'rip') != n(p, 'rip') or not begins.get(target, {'_pos': len(text)})['_pos'] < t['_pos'] < p['_pos']:
            errors.append('trap/pending address/RIP/order mismatch')
        offset = n(p, 'page') - bases.get(target, 0)
        if offset not in range(0, 8 * 4096, 4096):
            errors.append('fault outside client extent')
        expected_error = 6 if target == 15 and offset in (4096, 12288) else 4
        if n(t, 'error') != expected_error:
            errors.append('read/write fault mismatch')
    for target in (10, 15):
        expected_pages = [0, 1, 2, 3] + ([0] + ([4] if quota else []) if target == 15 else [])
        observed_pages = [(n(p, 'page') - bases[target]) // 4096 for p in pending.values() if n(p, 'target') == target]
        if observed_pages != expected_pages:
            errors.append('unexpected client fault sequence')
    if len(set(tasks.values())) != 2:
        errors.append('clients share a task')
    if errors:
        return errors
    active, by_page, retired = {}, {}, {}
    reclaimed, reuse_predecessors, commit_used, reclaim_used = {}, {}, {}, {}
    cross_reuses, self_reuses = 0, 0
    events = sorted([(r['_pos'], 'commit', r) for r in rows['paging.leaf.commit']] +
                    [(r['_pos'], 'reclaim', r) for r in rows['paging.leaf.reclaim']])
    for pos, kind, r in events:
        token, target, frame, page = (n(r, k) for k in ('token', 'target', 'frame', 'page'))
        if not 0 <= frame < 8 or target not in grants:
            errors.append('frame/target outside authority')
            continue
        if kind == 'commit':
            eq(r, rights=6, pkey=target, zero=1, runtime_alloc=0)
            p = pending.get(token)
            if not p or n(p, 'target') != target or n(p, 'page') != page or p['_pos'] >= pos:
                errors.append('leaf does not match pending fault')
            if token in leaves or frame in active or (target, page) in by_page:
                errors.append('live alias or duplicate commit')
            if frame in retired:
                old = retired[frame]
                reuse_predecessors[token] = old
                cross_reuses += n(old, 'target') != target
                self_reuses += n(old, 'target') == target
            active[frame] = r
            by_page[target, page] = frame
            leaves[token] = r
            commit_used[token] = sum(n(v, 'target') == target for v in active.values())
            if ready[target]['_pos'] >= pos:
                errors.append('leaf before pager client readiness')
            if sum(n(v, 'target') == target for v in active.values()) > 4:
                errors.append('kernel per-client quota exceeded')
        else:
            # The production marker is emitted only after clear + invalidation.
            eq(r, cleared=1, tlb=1, runtime_alloc=0)
            old = active.get(frame)
            if not old or any(n(old, k) != n(r, k) for k in ('token', 'target', 'page', 'frame')):
                errors.append('reclaim does not own exact live binding')
            else:
                del active[frame]
                by_page.pop((target, page), None)
                retired[frame] = r
                if token in reclaimed:
                    errors.append('duplicate reclaim generation')
                reclaimed[token] = r
                reclaim_used[token] = sum(n(v, 'target') == target for v in active.values())
    if (cross_reuses, self_reuses) != (3, 1):
        errors.append('missing cross-client or same-address reuse')
    if errors:
        return errors
    for marker, dest in [('paging.fault.completed', complete), ('sext.fault.commit', commits)]:
        for r in rows[marker]:
            token = n(r, 'token')
            if token in dest or token not in leaves:
                errors.append('duplicate/unknown completion')
                continue
            dest[token] = r
            leaf, p = leaves[token], pending[token]
            eq(r, target=n(leaf, 'target'))
            if r['_pos'] <= leaf['_pos']:
                errors.append('completion before PTE commit')
            if marker == 'paging.fault.completed':
                eq(r, task=n(p, 'task'), resume=1)
                if n(r, 'saved_rip') != n(p, 'rip'):
                    errors.append('instruction RIP changed')
            else:
                eq(r, frame=n(leaf, 'frame'), page=leaf['page'], zeroed=4096, ok=1)
                if n(r, 'used') != commit_used[token]:
                    errors.append('sext per-client quota accounting')
    if set(complete) != set(leaves) or set(commits) != set(leaves):
        errors.append('incomplete exact-token completion evidence')
    if errors:
        return errors
    negative_tokens = [n(r, 'token') for r in rows['sext.commit.negative']]
    if len(set(negative_tokens)) != len(negative_tokens) or set(negative_tokens) != set(leaves):
        errors.append('commit negative witnesses do not cover exact generations')
    for r in rows['sext.commit.negative']:
        token = n(r, 'token')
        eq(r, exact=1, conflicting_frame=1, zero_token=1, unknown_token=1, ok=1)
        if token not in commits or r['_pos'] <= commits[token]['_pos']:
            errors.append('commit replay proof mismatch')
    for r in rows['paging.client.page']:
        target, index = n(r, 'pd'), n(r, 'page')
        eq(r, zero=1, value=1, tail=1, ok=1, first='write' if target == 15 and index % 2 else 'read')
        matches = [l for l in leaves.values() if n(l, 'target') == target and n(l, 'page') == bases.get(target, 0) + index * 4096]
        if target not in bases or not 0 <= index < 4 or not matches:
            errors.append('client page proof outside mapped range')
        elif r['_pos'] <= complete[n(matches[0], 'token')]['_pos']:
            errors.append('client instruction proof before wake')
        elif n(matches[0], 'token') in reclaimed and r['_pos'] >= reclaimed[n(matches[0], 'token')]['_pos']:
            errors.append('client instruction proof after its binding was reclaimed')
    for target in (10, 15):
        if sorted(n(r, 'page') for r in rows['paging.client.page'] if n(r, 'pd') == target) != list(range(4)):
            errors.append('missing/duplicate client page')
    fresh = [r for r in rows['sext.release'] if n(r, 'replay') == 0]
    replay = [r for r in rows['sext.release'] if n(r, 'replay') == 1]
    if len(fresh) != 4 or len(replay) != 1:
        errors.append('release replay cardinality')
    fresh_tokens = [n(r, 'token') for r in fresh]
    if len(set(fresh_tokens)) != 4 or set(fresh_tokens) != set(reclaimed):
        errors.append('release witnesses do not cover exact reclaims')
    for r in fresh:
        eq(r, status=0)
        token = n(r, 'token')
        if token in reclaim_used:
            eq(r, used=reclaim_used[token])
        target, page = n(r, 'target'), n(r, 'page')
        expected_request = (page - bases.get(target, 0)) // 4096 + 1 if target == 10 else 1
        eq(r, request=expected_request)
        if token in complete and token in reclaimed and complete[token]['_pos'] >= reclaimed[token]['_pos']:
            errors.append('reclaim preceded fault completion')
        match = [x for x in rows['paging.leaf.reclaim'] if n(x, 'token') == n(r, 'token')]
        if len(match) != 1 or match[0]['_pos'] >= r['_pos'] or any(n(match[0], k) != n(r, k) for k in ('target', 'frame', 'page')):
            errors.append('free before exact PTE detach')
    reclaim_negatives = [n(r, 'token') for r in rows['sext.reclaim.negative']]
    if len(set(reclaim_negatives)) != 4 or set(reclaim_negatives) != set(reclaimed):
        errors.append('reclaim negative witnesses do not cover exact generations')
    for r in rows['sext.reclaim.negative']:
        old = reclaimed.get(n(r, 'token'))
        if old and (n(r, 'target'), n(r, 'frame')) != (n(old, 'target'), n(old, 'frame')):
            errors.append('reclaim negative binding mismatch')
        eq(r, stale_commit=1, duplicate_reclaim=1, ok=1)
        if not any(n(x, 'token') == n(r, 'token') and x['_pos'] < r['_pos'] for x in fresh):
            errors.append('missing retired generation denial')
    reuse_tokens = [n(r, 'token') for r in rows['sext.reuse.negative']]
    if len(set(reuse_tokens)) != 4 or set(reuse_tokens) != set(reuse_predecessors):
        errors.append('reuse negative witnesses do not cover exact generations')
    for token, predecessor in reuse_predecessors.items():
        release = next((r for r in fresh if n(r, 'token') == n(predecessor, 'token')), None)
        if not release or release['_pos'] >= leaves[token]['_pos']:
            errors.append('frame reused before sext confirmed exact reclaim')
    for r in rows['sext.reuse.negative']:
        eq(r, stale_commit=1, stale_reclaim=1, new_commit=1, ok=1)
        token, oldtoken = n(r, 'token'), n(r, 'old_token')
        old, new = leaves.get(oldtoken), leaves.get(token)
        predecessor = reuse_predecessors.get(token)
        if not predecessor or oldtoken != n(predecessor, 'token'):
            errors.append('reuse probe does not name the immediately retired generation')
        if not old or not new or oldtoken >= token or n(old, 'frame') != n(new, 'frame') or n(r, 'frame') != n(new, 'frame') or r['_pos'] <= new['_pos']:
            errors.append('post-reuse stale generation probe mismatch')
        elif n(r, 'old_target') != n(old, 'target') or n(r, 'target') != n(new, 'target'):
            errors.append('post-reuse target mismatch')
    eq(rows['paging.client.released'][0], pd=10, pages=3, retained=1, ok=1)
    eq(rows['paging.client.recovered'][0], pd=15, pages=4, retries=4, retained=4, ok=1)
    reuse = rows['paging.client.reuse'][0]
    eq(reuse, pd=15, page=0, refault=1, zero=1, replay_safe=1, neighbors=3, ok=1)
    retained = rows['paging.client.retained'][0]
    eq(retained, pd=10, pages=1, cross_client_reuse=1, ok=1)
    if retained['_pos'] <= reuse['_pos']:
        errors.append('neighbor not checked after reuse')
    for r in rows['sext.release.denied']:
        eq(r, target=15, status=3)
    if sorted(n(r, 'request') for r in rows['sext.release.denied']) != [0, 1]:
        errors.append('missing zero/conflicting release denials')
    if replay:
        r = replay[0]
        original = next((x for x in fresh if n(x, 'token') == n(r, 'token')), None)
        if not original or any(n(original, k) != n(r, k) for k in ('target', 'request', 'page', 'token', 'frame')):
            errors.append('release replay does not match original exact request/binding')
        eq(r, target=15, request=1, status=0, used=4)
        if not any(n(l, 'target') == 15 and n(l, 'page') == bases[15] and l['_pos'] < r['_pos'] and n(l, 'token') != n(r, 'token') for l in leaves.values()):
            errors.append('duplicate release not tested after remapping')
    # Client summaries must follow their actual instructions and ACK stages.
    released = rows['paging.client.released'][0]
    recovered = rows['paging.client.recovered'][0]
    bell_pages = [r for r in rows['paging.client.page'] if n(r, 'pd') == 10]
    kaleido_pages = [r for r in rows['paging.client.page'] if n(r, 'pd') == 15]
    bell_releases = [r for r in fresh if n(r, 'target') == 10]
    own_release = [r for r in fresh if n(r, 'target') == 15]
    if len(bell_releases) != 3 or any(r['_pos'] >= released['_pos'] for r in bell_releases + bell_pages) or released['_pos'] >= begins[15]['_pos']:
        errors.append('released client summary stage mismatch')
    if len(own_release) != 1 or any(r['_pos'] >= recovered['_pos'] for r in kaleido_pages) or (own_release and recovered['_pos'] >= own_release[0]['_pos']):
        errors.append('recovered client summary stage mismatch')
    refaults = [l for token, l in leaves.items() if token in reuse_predecessors and n(reuse_predecessors[token], 'target') == n(l, 'target')]
    if len(refaults) != 1 or n(refaults[0], 'target') != 15 or n(refaults[0], 'page') != bases[15]:
        errors.append('missing exact page-zero self refault')
    elif not replay or not complete[n(refaults[0], 'token')]['_pos'] < replay[0]['_pos'] < reuse['_pos']:
        errors.append('reuse summary/replayed release preceded refault completion')
    for denial in rows['sext.release.denied']:
        expected_page = bases[15] + (4096 if n(denial, 'request') == 1 else 0)
        if n(denial, 'page') != expected_page or not replay or not replay[0]['_pos'] < denial['_pos'] < reuse['_pos']:
            errors.append('release denial does not belong to post-refault requests')
    if {n(r, 'target') for r in active.values()} != {10, 15} or sum(n(r, 'target') == 10 for r in active.values()) != 1 or sum(n(r, 'target') == 15 for r in active.values()) != 4:
        errors.append('wrong retained live mapping inventory')
    if quota:
        p = pending.get(last_token)
        reject = rows['paging.fault.rejected'][0]
        policy = rows['sext.fault.reject'][0]
        eq(rows['paging.client.quota.begin'][0], pd=15, page=4, in_range=1)
        eq(reject, target=15, task=tasks.get(15), token=last_token, terminal=1)
        eq(policy, target=15, token=last_token, reason='Quota', used=4, quota=4)
        begin = rows['paging.client.quota.begin'][0]
        if n(policy, 'page') != bases[15] + 4 * 4096 or not p or not retained['_pos'] < begin['_pos'] < traps[-1]['_pos'] < p['_pos'] < reject['_pos'] < policy['_pos']:
            errors.append('quota rejection page/stage mismatch')
        if not p or n(p, 'page') != bases[15] + 4 * 4096 or last_token in leaves:
            errors.append('quota rejection did not deny exact pending fifth page')
        after = text[reject['_pos']:]
        if re.search(rf'task\.running id={tasks.get(15)}\b', after) or '[paging.client.page]' in after or '[paging.client.reuse]' in after or '[paging.client.recovered]' in after:
            errors.append('rejected client resurrected')
        if not re.search(r'\[silkbar\.clock\.send\][^\n]*status=0x0(?:\s|$)', after):
            errors.append('no service progress after rejection')
    if re.search(r'panic|fault\.kill|PAGE FAULT|GP FAULT|DOUBLE FAULT|\[sext\.fatal\]|\[paging\.client\.fatal\]|\[paging\.fault\.terminal\]', text, re.I):
        errors.append('fatal or spurious fault')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log', type=Path)
    parser.add_argument('--quota', action='store_true')
    args = parser.parse_args()
    errors = validate(args.log.read_text(errors='replace'), args.quota)
    for error in errors:
        print('[paging.multi.runtime] FAIL ' + error)
    if not errors:
        print('[paging.multi.runtime] PASS clients=2 recovered=9 reclaimed=4 cross_client_reuse=3 self_reuse=1 scope=observed_BSP_instructions')
    return bool(errors)


if __name__ == '__main__':
    sys.exit(main())
