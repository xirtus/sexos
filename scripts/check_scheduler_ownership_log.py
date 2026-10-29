#!/usr/bin/env python3
"""Require observed guest custody transitions; this is not exhaustive SMP proof."""
import argparse
from pathlib import Path
import re
import sys


def validate(text, require_last=False):
    errors = []
    events = {name: [] for name in ("park", "detached_wake", "post_stack_release", "user_resumed")}
    rx = re.compile(r"\[scheduler\.ownership\] event=(\w+) task=(\d+) count=(\d+)")
    for match in rx.finditer(text):
        if match[1] in events:
            events[match[1]].append(int(match[3]))
    for name, counts in events.items():
        if not counts or counts[0] != 1 or any(a >= b for a, b in zip(counts, counts[1:])):
            errors.append(f"missing, duplicate or nonmonotonic {name} observations")
    if re.search(r"panic|fault\.kill|PAGE FAULT|GP FAULT|DOUBLE FAULT|CUSTODY_MISMATCH|STACK_NOT_RELEASED", text, re.I):
        errors.append("fault or custody tripwire")
    if require_last:
        begin = list(re.finditer(r"\[scheduler\.proof\.last_runnable\.begin\] task=(\d+) held=(\d+) lane=bsp", text))
        wake = list(re.finditer(r"\[scheduler\.proof\.last_runnable\.timer_wake\] task=(\d+)", text))
        passed = list(re.finditer(r"\[scheduler\.proof\.last_runnable\.pass\] task=(\d+) restored=(\d+) ownership=preserved", text))
        if not (len(begin) == len(wake) == len(passed) == 1):
            errors.append("missing/duplicate last-runnable proof stage")
        else:
            a, b, c = begin[0], wake[0], passed[0]
            if not (a.start() < b.start() < c.start() and a[1] == b[1] == c[1] and a[2] == c[2] and int(a[2]) > 0):
                errors.append("last-runnable identity, custody count or stage order mismatch")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--require-last-runnable", action="store_true")
    args = parser.parse_args()
    errors = validate(args.log.read_text(errors="replace"), args.require_last_runnable)
    for error in errors:
        print(f"[scheduler.ownership.runtime] FAIL {error}")
    if not errors:
        print(f"[scheduler.ownership.runtime] PASS last_runnable_required={int(args.require_last_runnable)} scope=observed_BSP_transitions")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
