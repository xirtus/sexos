#!/usr/bin/env python3
"""Observed BSP task-exit custody, complete teardown and surviving-client reuse."""
import argparse
from pathlib import Path
import re
import sys
from check_multi_client_paging_log import validate as validate_initial


def validate(text, quota=False):
    marker = '[paging.client.quota.begin]' if quota else '[paging.client.finish]'
    if text.count(marker) != 1:
        return ['missing/duplicate terminal client trigger']
    boundary = text.index(marker)
    errors = validate_initial(text[:boundary])
    if errors:
        return ['initial paging: ' + error for error in errors]
    records = {}
    for match in re.finditer(r'\[((?:paging\.|sext\.)[a-zA-Z0-9_.]+|task\.finish|silkbar\.clock\.send)\](?: ([^\r\n]*))?', text):
        fields = {}
        for part in (match[2] or '').split():
            if '=' not in part:
                errors.append('partial record ' + match[1])
            else:
                key, value = part.split('=', 1)
                if key in fields:
                    errors.append('duplicate field ' + match[1])
                fields[key] = value
        fields['_pos'] = match.start()
        records.setdefault(match[1], []).append(fields)

    def rows(marker):
        return records.get(marker, [])

    def one(marker):
        found = rows(marker)
        if len(found) != 1:
            errors.append(f'{marker}: expected one record, saw {len(found)}')
            return {}
        return found[0]

    def num(row, key):
        try:
            value = row[key]
            return int(value, 16 if value.startswith('0x') else 10)
        except (KeyError, ValueError, AttributeError):
            errors.append('missing/invalid ' + key)
            return -1

    def eq(row, **expected):
        for key, value in expected.items():
            if str(row.get(key)) != str(value):
                errors.append(f'{key}: expected {value}, saw {row.get(key)}')

    for name, expected in {
        'paging.fault.trap': 13 if quota else 12,
        'paging.fault.pending': 13 if quota else 12,
        'paging.leaf.commit': 12, 'paging.fault.completed': 12,
        'sext.fault.commit': 12, 'sext.commit.negative': 12,
        'sext.reuse.negative': 7,
    }.items():
        if len(rows(name)) != expected:
            errors.append(f'{name}: incomplete/extra observed stage')

    trigger = one('paging.client.quota.begin' if quota else 'paging.client.finish')
    if quota:
        terminal = one('paging.fault.rejected')
        eq(terminal, target=15, terminal=1)
        quota_policy = one('sext.fault.reject')
        eq(quota_policy, target=15, reason='Quota', used=4, quota=4)
    else:
        terminal = one('task.finish')
        eq(trigger, pd=15, resident=4, reason='proof')
        eq(terminal, pd=15, reason=0, terminal=1)
        if rows('paging.fault.rejected') or rows('sext.fault.reject'):
            errors.append('ordinary self-finish unexpectedly used fault rejection')
    initial_tasks = {num(r, 'target'): num(r, 'task') for r in rows('paging.fault.pending') if r['_pos'] < boundary}
    task = num(terminal, 'task')
    if task != initial_tasks.get(15):
        errors.append('terminal task differs from original client main task')
    opposite = 'paging.client.finish' if quota else 'paging.client.quota.begin'
    if rows(opposite) or (quota and rows('task.finish')):
        errors.append('mixed terminal profiles')
    if task <= 0 or terminal.get('_pos', -1) <= boundary:
        errors.append('terminal task/trigger mismatch')
    notice = one('paging.exit.notice')
    eq(notice, target=15, task=task, pager=14, detached=1, accepted=1)
    closed = one('paging.target.cleanup')
    eq(closed, target=15, task=task, leaves=4, closed=1, detached=1, all_flushed=1, runtime_alloc=0)
    freed = one('sext.cleanup')
    eq(freed, target=15, task=task, released=4, remaining=0, closed=1, ok=1)
    wait = one('sext.cleanup.wait')
    eq(wait, caller=10, target=15, request=4, status=0)
    summary = one('paging.client.exit_reuse')
    eq(summary, pd=10, pages=3, retained=1, dead_target=15, ok=1)
    ordered = [terminal, notice, closed, freed, wait, summary]
    if not all(a.get('_pos', -1) < b.get('_pos', -1) for a, b in zip(ordered, ordered[1:])):
        errors.append('exit/custody/cleanup/ACK/reuse ordering')
    old = {}
    for r in rows('paging.leaf.commit'):
        if r['_pos'] < boundary and num(r, 'target') == 15:
            old[num(r, 'frame')] = r
    cleanup = rows('paging.leaf.cleanup')
    if len(old) != 4 or len(cleanup) != 4:
        errors.append('incomplete client teardown')
    mask, seen = 0, set()
    for r in cleanup:
        frame = num(r, 'frame')
        eq(r, target=15, task=task, finalized=1, cleared=1, tlb=1, runtime_alloc=0)
        if frame in seen or frame not in old or not 0 <= frame < 8:
            errors.append('foreign/duplicate cleanup frame')
            continue
        seen.add(frame)
        mask |= 1 << frame
        original = old[frame]
        for field in ('token', 'target', 'page', 'frame'):
            if num(r, field) != num(original, field):
                errors.append('cleanup lease identity mismatch ' + field)
        if not notice.get('_pos', -1) < r['_pos'] < closed.get('_pos', -1):
            errors.append('frame released outside authorized cleanup boundary')
    if num(closed, 'frames') != mask or num(freed, 'bitmap') != mask:
        errors.append('cleanup bitmap does not cover exact old bindings')
    after = text[terminal.get('_pos', len(text)):]
    if re.search(rf'task\.running id={task}\b', after):
        errors.append('ended task ran again')
    if any(num(r, 'target') == 15 and r['_pos'] > boundary for r in rows('paging.leaf.commit')):
        errors.append('closed target acquired a new mapping')
    commits = [r for r in rows('paging.leaf.commit') if r['_pos'] > boundary]
    if len(commits) != 3:
        errors.append('expected three surviving-client refaults')
    pages = rows('paging.client.exit_reuse.page')
    if len(pages) != 3 or sorted(num(r, 'page') for r in pages) != [0, 1, 2]:
        errors.append('missing/duplicate surviving-client instruction proof')
    bases = {num(r, 'pd'): num(r, 'base') for r in rows('paging.client.begin')}
    tokens, frames = set(), set()
    last_token = max((num(r, 'token') for r in rows('paging.fault.pending') if r['_pos'] < wait.get('_pos', -1)), default=0)
    for index, r in enumerate(commits):
        token, frame = num(r, 'token'), num(r, 'frame')
        eq(r, target=10, rights=6, pkey=10, zero=1, runtime_alloc=0)
        if token in tokens or frame in frames or frame not in old:
            errors.append('survivor did not reuse unique dead-client backing')
            continue
        tokens.add(token); frames.add(frame)
        if token <= last_token:
            errors.append('survivor fault token reused or out of order')
        last_token = token
        if r['_pos'] <= wait.get('_pos', -1):
            errors.append('frame reused before confirmed cleanup')
        if num(r, 'page') != bases.get(10, 0) + index * 4096:
            errors.append('survivor page is not the exact next aligned client page')
        matches = [x for x in pages if num(x, 'page') == index]
        pending = [x for x in rows('paging.fault.pending') if num(x, 'token') == token]
        complete = [x for x in rows('paging.fault.completed') if num(x, 'token') == token]
        policy = [x for x in rows('sext.fault.commit') if num(x, 'token') == token]
        negative = [x for x in rows('sext.reuse.negative') if num(x, 'token') == token]
        replay = [x for x in rows('sext.cleanup.replay') if num(x, 'new_token') == token]
        commit_negative = [x for x in rows('sext.commit.negative') if num(x, 'token') == token]
        if not all(len(group) == 1 for group in (matches, pending, complete, policy, negative, replay, commit_negative)):
            errors.append('missing/duplicate exact-generation survivor join')
            continue
        p, c, s, ng, rp, user = pending[0], complete[0], policy[0], negative[0], replay[0], matches[0]
        eq(p, target=10, task=initial_tasks.get(10))
        traps = [x for x in rows('paging.fault.trap') if x['_pos'] > boundary and num(x, 'pd') == 10 and num(x, 'addr') == num(r, 'page')]
        if len(traps) != 1:
            errors.append('missing survivor read fault')
        else:
            eq(traps[0], pd=10, error='0x4', eligible=1)
            if num(traps[0], 'rip') != num(p, 'rip') or not wait.get('_pos', -1) < traps[0]['_pos'] < p['_pos']:
                errors.append('survivor trap RIP/order mismatch')
        eq(c, target=10, task=num(p, 'task'), resume=1)
        if num(p, 'page') != num(r, 'page') or num(c, 'saved_rip') != num(p, 'rip') or not p['_pos'] < r['_pos'] < c['_pos'] < user['_pos']:
            errors.append('survivor instruction/frame/RIP ordering')
        eq(s, target=10, frame=frame, zeroed=4096, ok=1, used=index+2)
        if num(s, 'page') != num(r, 'page'):
            errors.append('survivor pager page differs from installed leaf')
        eq(commit_negative[0], exact=1, conflicting_frame=1, zero_token=1, unknown_token=1, ok=1)
        if s['_pos'] <= r['_pos'] or commit_negative[0]['_pos'] <= s['_pos']:
            errors.append('survivor pager completion ordering')
        eq(ng, frame=frame, old_target=15, target=10, old_token=num(old[frame], 'token'), stale_commit=1, stale_reclaim=1, new_commit=1, ok=1)
        eq(rp, target=15, task=task, frame=frame, new_target=10, preserved=1, ok=1)
        if num(rp, 'bitmap') != mask or not commit_negative[0]['_pos'] < ng['_pos'] < rp['_pos']:
            errors.append('cleanup replay changed its result or preceded reuse')
        eq(user, pd=10, page=index, zero=1, tail=1, value=1, neighbor=1, ok=1)
        if summary.get('_pos', -1) <= user['_pos']:
            errors.append('survivor summary before instruction proof')
    if len(rows('sext.cleanup.replay')) != 3:
        errors.append('cleanup replay proof count')
    if quota:
        pending = [r for r in rows('paging.fault.pending') if r['_pos'] > boundary and num(r, 'target') == 15]
        if len(pending) != 1:
            errors.append('missing exact quota request')
        else:
            p = pending[0]
            eq(trigger, pd=15, page=4, in_range=1)
            eq(terminal, task=num(p, 'task'), token=num(p, 'token'))
            eq(quota_policy, target=15, token=num(p, 'token'), page=p.get('page'))
            quota_traps = [r for r in rows('paging.fault.trap') if r['_pos'] > boundary and num(r, 'pd') == 15]
            if len(quota_traps) != 1:
                errors.append('missing exact quota trap')
            else:
                qt = quota_traps[0]
                eq(qt, pd=15, error='0x4', eligible=1)
                if num(qt, 'addr') != num(p, 'page') or num(qt, 'rip') != num(p, 'rip') or not boundary < qt['_pos'] < p['_pos']:
                    errors.append('quota trap page/RIP/stage mismatch')
            if num(p, 'task') != initial_tasks.get(15) or not terminal.get('_pos', -1) < quota_policy.get('_pos', -1) < closed.get('_pos', -1):
                errors.append('quota terminal policy stage/identity mismatch')
            if num(p, 'page') != bases.get(15, 0) + 4 * 4096 or not boundary < p['_pos'] < terminal.get('_pos', -1):
                errors.append('quota request page/order mismatch')
    if not re.search(r'\[silkbar\.clock\.send\][^\n]*status=0x0(?:\s|$)', text[summary.get('_pos', len(text)):]):
        errors.append('no desktop service progress after cleanup and reuse')
    if re.search(r'panic|fault\.kill|PAGE FAULT|GP FAULT|DOUBLE FAULT|\[sext\.fatal\]|\[paging\.client\.fatal\]|\[paging\.fault\.terminal\]', text, re.I):
        errors.append('fatal/spurious fault')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log', type=Path)
    parser.add_argument('--quota', action='store_true')
    args = parser.parse_args()
    errors = validate(args.log.read_text(errors='replace'), args.quota)
    for error in errors:
        print('[paging.exit_cleanup.runtime] FAIL ' + error)
    if not errors:
        print('[paging.exit_cleanup.runtime] PASS dead_client_pages=4 survivor_reuses=3 retained=1 cached_cleanup_replay=3 scope=observed_BSP_instructions')
    return bool(errors)


if __name__ == '__main__':
    sys.exit(main())
