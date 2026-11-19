#!/usr/bin/env python3
"""Markdown summary of a soak_gate.py run (soak.json): hours, rounds, crashes injected,
and whether free program memory, free frames and the kernel heap stayed flat (first
rounds against the last)."""
import json, sys


def trend(rounds, key, warm=3):
    have = [r[key] for r in rounds if key in r]
    if len(have) <= warm:
        return None
    first, last = have[:warm], have[-warm:]
    return dict(first=max(first) if key != 'heap_blocks' else min(first), last=last[-1],
                low=min(have), high=max(have))


def report(record):
    rounds = record.get('rounds', [])
    lines = [f"## Soak: {'passed' if record.get('ok') else 'FAILED'}", '',
             f"{record.get('minutes', 0) / 60:.2f} h, {len(rounds)} rounds, {record.get('cores')} cores, inject={record.get('inject')}"]
    if record.get('why'):
        lines.append(f"Stopped: `{record['why']}`")
    crashed = [r['crashed'] for r in rounds if 'crashed' in r]
    if crashed:
        lines.append(f"Services stopped and restarted: {len(crashed)} ({', '.join(sorted(set(crashed)))})")
    lines += ['', '| measure | first rounds | last | low | high |', '| --- | --- | --- | --- | --- |']
    for key in ('free_pages', 'free_frames', 'heap_blocks', 'heap_used', 'key_ms'):
        t = trend(rounds, key)
        if t:
            lines.append(f"| {key} | {t['first']} | {t['last']} | {t['low']} | {t['high']} |")
    return '\n'.join(lines)


if __name__ == '__main__':
    print(report(json.load(open(sys.argv[1]))))
