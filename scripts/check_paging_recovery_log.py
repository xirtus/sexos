#!/usr/bin/env python3
"""Join observed faults, authorized leaf commits and retried user instructions.

This is bounded BSP execution evidence, not paging/replay/SMP verification.
"""
import argparse
from pathlib import Path
import re
import sys


def validate(text, quota=False):
    errors = []

    def matches(pattern):
        return list(re.finditer(pattern, text))

    genesis = matches(r"\[boot\.paging\.genesis\] pager=(\d+) target=(\d+) pool_pages=8 arena_pages=8 quota=8 target_pkey=(\d+) leaves_absent=8 tables_preallocated=1")
    client = matches(r"\[paging\.client\.begin\] pd=(\d+) base=(0x[0-9a-f]+) bytes=32768")
    if len(genesis) != 1 or len(client) != 1:
        return ["missing/duplicate boot authority or real client probe"]
    g, c = genesis[0], client[0]
    pager, target, pkey = map(int, g.groups())
    base = int(c[2], 16)
    if pager == 0 or target == 0 or pager == target or int(c[1]) != target or base == 0 or base % 4096 or not 0 < pkey < 16:
        errors.append("boot/client capability identity or address mismatch")
    seal = matches(r"\[boot\.memory\.authority\.sealed\] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0")
    ready = matches(rf"\[sext\.ready\] target={target} frames=8 quota=4 pool_bytes=32768 arena_bytes=32768 zero_owner=sext")
    if len(seal) != 1 or len(ready) != 1 or not g.start() < seal[0].start() < c.start() or (seal and ready and ready[0].start() < seal[0].start()):
        errors.append("missing boot seal or active ring-3 pager readiness")
    client_negative = matches(r"\[paging\.client\.negative\] invalid_slot=1 high_alias=1 wrong_query=1 wrong_commit=1 wrong_reject=1 ok=1")
    if len(client_negative) != 1 or (seal and not seal[0].start() < client_negative[0].start() < c.start()):
        errors.append("missing actual client capability denial probes")
    traps = matches(r"\[paging\.fault\.trap\] pd=(\d+) addr=(0x[0-9a-f]+) rip=(0x[0-9a-f]+) error=(0x[0-9a-f]+) eligible=(\d+)")
    pending = matches(r"\[paging\.fault\.pending\] target=(\d+) task=(\d+) token=(\d+) page=(0x[0-9a-f]+) rip=(0x[0-9a-f]+)")
    leaves = matches(r"\[paging\.leaf\.commit\] token=(\d+) target=(\d+) page=(0x[0-9a-f]+) frame=(\d+) rights=(\d+) pkey=(\d+) zero=(\d+) runtime_alloc=(\d+)")
    completed = matches(r"\[paging\.fault\.completed\] target=(\d+) task=(\d+) token=(\d+) resume=(\d+) saved_rip=(0x[0-9a-f]+)")
    commits = matches(r"\[sext\.fault\.commit\] target=(\d+) token=(\d+) page=(0x[0-9a-f]+) frame=(\d+) used=(\d+) zeroed=(\d+) ok=(\d+)")
    negatives = matches(r"\[sext\.commit\.negative\] token=(\d+) exact=1 conflicting_frame=1 zero_token=1 unknown_token=1 ok=1")
    pages = matches(r"\[paging\.client\.page\] pd=(\d+) page=(\d+) first=(read|write) zero=1 value=1 tail=1 ok=1")
    expected = 5 if quota else 4
    for prefix, records in (("paging.fault.trap", traps), ("paging.fault.pending", pending),
                            ("paging.leaf.commit", leaves), ("paging.fault.completed", completed),
                            ("sext.fault.commit", commits), ("sext.commit.negative", negatives),
                            ("paging.client.page", pages)):
        if text.count("[" + prefix + "]") != len(records):
            errors.append("malformed or split evidence record: " + prefix)
    if len(traps) != expected or len(pending) != expected or any(len(rows) != 4 for rows in (leaves, completed, commits, negatives, pages)):
        errors.append("missing/duplicate trap, token, commit, completion or retried-instruction evidence")
    if len(traps) == len(pending) == expected and all(len(rows) == 4 for rows in (leaves, completed, commits, negatives, pages)):
        tokens = [int(row[3]) for row in pending]
        task = int(pending[0][2])
        if task == 0 or any(int(row[2]) != task for row in pending):
            errors.append("all faults must belong to the same nonzero saved client task")
        if tokens[0] == 0 or any(a >= b for a, b in zip(tokens, tokens[1:])):
            errors.append("fault tokens must be nonzero and strictly increasing")
        frames = set()
        for index, (trap, request) in enumerate(zip(traps, pending)):
            address = base + index * 4096
            if (int(trap[1]), int(trap[2], 16), int(trap[4], 16), int(trap[5])) != (target, address, 6 if index & 1 else 4, 1):
                errors.append(f"wrong fault access/address for page {index}")
            if int(request[1]) != target or int(request[4], 16) != address or request[5] != trap[3] or not c.start() < trap.start() < request.start():
                errors.append(f"fault identity, saved RIP or ordering mismatch for page {index}")
            if index == 4:
                continue
            token = int(request[3])
            leaf, done, commit, negative, page = leaves[index], completed[index], commits[index], negatives[index], pages[index]
            frame = int(leaf[4])
            if (int(leaf[1]), int(leaf[2]), int(leaf[3], 16), int(leaf[5]), int(leaf[6]), int(leaf[7]), int(leaf[8])) != (token, target, address, 6, pkey, 1, 0) or not 0 <= frame < 8 or frame in frames:
                errors.append(f"unauthorized/aliased frame or leaf for page {index}")
            frames.add(frame)
            if (int(done[1]), done[2], int(done[3]), int(done[4]), done[5]) != (target, request[2], token, 1, trap[3]):
                errors.append(f"completion does not belong to the saved task/frame for page {index}")
            if (int(commit[1]), int(commit[2]), int(commit[3], 16), int(commit[4]), int(commit[5]), int(commit[6]), int(commit[7])) != (target, token, address, frame, index + 1, 4096, 1) or int(negative[1]) != token:
                errors.append(f"sext allocation, zeroing or duplicate-denial mismatch for page {index}")
            if (int(page[1]), int(page[2]), page[3]) != (target, index, "write" if index & 1 else "read"):
                errors.append(f"missing real retried instruction result for page {index}")
            if not request.start() < leaf.start() < done.start() < page.start() or not leaf.start() < commit.start() < negative.start() or not ready or ready[0].start() > leaf.start():
                errors.append(f"publication/commit/completion ordering mismatch for page {index}")
    recovered = matches(rf"\[paging\.client\.recovered\] pd={target} pages=4 retries=4 retained=4 ok=1")
    if len(recovered) != 1 or (pages and recovered[0].start() < pages[-1].start()):
        errors.append("missing retained user data after four recoveries")
    rejected = matches(r"\[paging\.fault\.rejected\] target=(\d+) task=(\d+) token=(\d+) terminal=1")
    policy = matches(r"\[sext\.fault\.reject\] target=(\d+) token=(\d+) page=(0x[0-9a-f]+) reason=Quota used=4 quota=4")
    if quota:
        begin = matches(rf"\[paging\.client\.quota\.begin\] pd={target} page=4 in_range=1")
        if len(begin) != 1 or len(rejected) != 1 or len(policy) != 1 or len(pending) != 5:
            errors.append("missing exact terminal quota rejection")
        else:
            request, reject, decision = pending[4], rejected[0], policy[0]
            if (int(reject[1]), reject[2], reject[3]) != (target, request[2], request[3]) or (int(decision[1]), decision[2], int(decision[3], 16)) != (target, request[3], base + 4 * 4096):
                errors.append("quota rejection consumed the wrong fault/task")
            if not recovered or not recovered[0].start() < begin[0].start() < request.start() < reject.start() < decision.start():
                errors.append("quota rejection stage order mismatch")
            if not re.search(r"\[silkbar\.clock\.send\][^\n]* status=0x0(?:\s|$)", text[decision.end():]):
                errors.append("missing unrelated domain progress after client rejection")
            task = int(request[2])
            after = text[reject.end():]
            if re.search(rf"task\.running id={task}\b", after) or "[paging.client.page]" in after or "[paging.client.recovered]" in after:
                errors.append("rejected client task executed again")
    elif rejected or policy or "[paging.client.quota.begin]" in text:
        errors.append("unexpected rejected fault in normal recovery image")
    if re.search(r"panic|fault\.kill|PAGE FAULT|GP FAULT|DOUBLE FAULT|\[sext\.fatal\]|\[paging\.fault\.terminal\]", text, re.I):
        errors.append("unexpected fatal fault or pager failure")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--quota", action="store_true")
    args = parser.parse_args()
    errors = validate(args.log.read_text(errors="replace"), args.quota)
    for error in errors:
        print("[paging.recovery.runtime] FAIL " + error)
    if not errors:
        print(f"[paging.recovery.runtime] PASS recovered=4 quota_required={int(args.quota)} scope=observed_BSP_instructions")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
