#!/usr/bin/env python3
"""BootGraph serial-log checker (host-side only).

Usage:
  scripts/check_bootgraph_log.py /tmp/sexos.log [--strict-clock] [--allow-fault]
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Tuple


REQUIRED_PD_MARKERS: List[Tuple[str, str]] = [
    ("sexdisplay", "sexdisplay"),
    ("sexdrive", "sexdrive"),
    ("silk-shell", "silkshell"),
    ("sexinput", "sexinput"),
    ("sexusb", "sexusb"),
    ("silkbar", "silkbar"),
    ("linen", "linen"),
    ("sexstore", "sexstore"),
    ("quil", "quil"),
    ("sexbell", "sexbell"),
    ("sexfiles", "sexfiles"),
    ("spindle", "spindle"),
    ("sext", "sext"),
]

# If A2 markers landed, require these grants with ok=1 (observed on full boot).
REQUIRED_A2_GRANTS_OK1: List[str] = [
    "from=kernel to=3 slot=SLOT_DISPLAY target=1 ok=1",
    "from=kernel to=3 slot=SLOT_SHELL target=3 ok=1",
    "from=kernel to=4 slot=SLOT_INPUT target=input_ring ok=1",
    "from=kernel to=4 slot=SLOT_SHELL target=3 ok=1",
    "from=kernel to=3 slot=SLOT_SHELL_INPUT_READY target=4 ok=1",
    "from=kernel to=6 slot=SLOT_DISPLAY target=1 ok=1",
    "from=kernel to=7 slot=SLOT_DISPLAY target=1 ok=1",
    "from=kernel to=3 slot=SLOT_LINEN target=7 ok=1 optional=1",
    "from=kernel to=3 slot=SLOT_QUIL target=9 ok=1 optional=1",
]

CRITICAL_EDGES: List[Tuple[str, str]] = [
    ("silkbar", "sexdisplay"),
    ("silk-shell", "sexdisplay"),
    ("silk-shell", "silkbar"),
    ("silk-shell", "linen"),
    ("sexinput", "silk-shell"),
    ("sexusb", "sexinput"),
    ("linen", "sexdisplay"),
    ("linen", "sexfiles"),
    ("quil", "sexfiles"),
    ("spindle", "silk-shell"),
    ("spindle", "quil"),
]

# These two startup sends are unconditional and instrumented in the current
# image. Other edges are input/activity dependent or not instrumented yet;
# absence there is reported as UNOBSERVED, never as a proven ordering check.
REQUIRED_STARTUP_EDGES = {("silkbar", "sexdisplay"), ("silk-shell", "sexdisplay")}

# Match the actual diagnostics in kernel/src/interrupts.rs and main.rs,
# including a trap's first line if the subsequent panic print is truncated.
FAULT_PATTERNS = [
    "#PF", "#GP", "panic", "fault.kill", "PAGE FAULT",
    "GENERAL PROTECTION", "GP FAULT", "DOUBLE FAULT", "TRIPLE FAULT",
    "FATAL", "EXCEPTION: INVALID OPCODE", "EXCEPTION: UD ENTER",
    "PKU SECURITY VIOLATION", "TEST FAILED:",
]


@dataclass
class GateResult:
    name: str
    passed: bool
    reason: str = ""


@dataclass
class PdRow:
    pd: str
    init_line: Optional[int]
    ready_line: Optional[int]
    state: str


@dataclass
class EdgeRow:
    edge: str
    receiver_ready_line: Optional[int]
    sender_send_line: Optional[int]
    result: str


def find_first_line(lines: List[str], needle: str) -> Optional[int]:
    for i, line in enumerate(lines, 1):
        if needle in line:
            return i
    return None


def find_all_lines(lines: List[str], needle: str) -> List[int]:
    out: List[int] = []
    for i, line in enumerate(lines, 1):
        if needle in line:
            out.append(i)
    return out


def bootgraph_gate(lines: List[str]) -> Tuple[GateResult, List[PdRow], Dict[str, int]]:
    rows: List[PdRow] = []
    ready_lines: Dict[str, int] = {}
    errs: List[str] = []

    for pd, token in REQUIRED_PD_MARKERS:
        init = find_first_line(lines, f"[{token}.init.start]")
        ready = find_first_line(lines, f"[{token}.ready]")
        if ready is not None:
            ready_lines[pd] = ready

        if len(find_all_lines(lines, f"[{token}.init.start]")) > 1 or len(find_all_lines(lines, f"[{token}.ready]")) > 1:
            state = "DUPLICATE"
            errs.append(f"{pd} duplicate init/ready marker")
        elif init is None or ready is None:
            state = "MISSING"
            errs.append(f"{pd} missing {'init' if init is None else 'ready'}")
        elif init >= ready:
            state = "ORDER_FAIL"
            errs.append(f"{pd} init>=ready ({init}>={ready})")
        else:
            state = "OK"

        rows.append(PdRow(pd=pd, init_line=init, ready_line=ready, state=state))

    if errs:
        return GateResult("BOOTGRAPH_GATE", False, "; ".join(errs)), rows, ready_lines
    return GateResult("BOOTGRAPH_GATE", True), rows, ready_lines


def cap_grant_gate(lines: List[str]) -> GateResult:
    begin = find_first_line(lines, "[bootgraph.phase25.begin]")
    complete = find_first_line(lines, "[bootgraph.phase25.complete]")
    if begin is None:
        return GateResult("CAP_GRANT_GATE", False, "missing phase25.begin")
    if complete is None:
        return GateResult("CAP_GRANT_GATE", False, "missing phase25.complete")
    if begin >= complete:
        return GateResult("CAP_GRANT_GATE", False, f"phase25 order invalid ({begin}>={complete})")

    # The current image always emits these grants. A truncated log without
    # grant rows must not receive a capability-proof PASS.
    for marker in REQUIRED_A2_GRANTS_OK1:
        grant_lines = find_all_lines(lines, f"[bootgraph.cap.grant {marker}]")
        if len(grant_lines) != 1:
            return GateResult("CAP_GRANT_GATE", False, f"expected one required grant: {marker} (found {len(grant_lines)})")
        if not begin < grant_lines[0] < complete:
            return GateResult("CAP_GRANT_GATE", False, f"grant outside phase25: {marker}")

    return GateResult("CAP_GRANT_GATE", True)


def parse_first_send_edges(lines: List[str]) -> Dict[Tuple[str, str], int]:
    edge_line: Dict[Tuple[str, str], int] = {}
    rx = re.compile(r"\[bootgraph\.edge\.send\s+from=([^\s]+)\s+to=([^\s\]]+).+first=1\]")
    for i, line in enumerate(lines, 1):
        m = rx.search(line)
        if not m:
            continue
        key = (m.group(1), m.group(2))
        if key not in edge_line:
            edge_line[key] = i
    return edge_line


def parse_defer_edges(lines: List[str]) -> Dict[Tuple[str, str, str], List[int]]:
    defer_lines: Dict[Tuple[str, str, str], List[int]] = {}
    rx = re.compile(
        r"\[bootgraph\.edge\.defer\s+from=([^\s]+)\s+to=([^\s]+)\s+slot=([^\s]+)\s+reason=missing_cap\]"
    )
    for i, line in enumerate(lines, 1):
        m = rx.search(line)
        if not m:
            continue
        key = (m.group(1), m.group(2), m.group(3))
        defer_lines.setdefault(key, []).append(i)
    return defer_lines


def order_gate(lines: List[str], ready_lines: Dict[str, int]) -> Tuple[GateResult, List[EdgeRow]]:
    phase_complete = find_first_line(lines, "[bootgraph.phase25.complete]")
    edges = parse_first_send_edges(lines)
    defers = parse_defer_edges(lines)
    rows: List[EdgeRow] = []
    errs: List[str] = []
    warnings: List[str] = []
    if phase_complete is None:
        errs.append("missing phase25.complete")
    # Shell readiness is now an unconditional boot dependency for sexinput.
    # A failed/missing wait must not hide as an activity-dependent absent edge.
    notify = find_all_lines(lines, "[silk-shell.ready.notify] to=sexinput")
    accepted = find_all_lines(lines, "[sexinput.shell.ready.accept] caller=3")
    shell_ready = ready_lines.get("silk-shell")
    for name, observations in (("shell ready notification", notify), ("input shell-ready acceptance", accepted)):
        if len(observations) != 1:
            errs.append(f"expected one {name} (found {len(observations)})")
        elif shell_ready is None or observations[0] <= shell_ready:
            errs.append(f"{name} before shell ready")
    input_send = edges.get(("sexinput", "silk-shell"))
    if input_send is not None and len(accepted) == 1 and input_send <= accepted[0]:
        errs.append("sexinput->silk-shell send before authenticated shell-ready acceptance")
    seen_first = set()
    send_rx = re.compile(r"\[bootgraph\.edge\.send from=([^\s\]]+) to=([^\s\]]+) slot=([^\s\]]+) op=([^\s\]]+) first=1\]")
    for line_no, line in enumerate(lines, 1):
        if "[bootgraph.edge.send" not in line:
            continue
        match = send_rx.search(line)
        if match is None:
            errs.append(f"malformed edge.send at line {line_no}")
            continue
        key = (match.group(1), match.group(2))
        if key in seen_first:
            errs.append(f"{key[0]}->{key[1]} duplicate first send at line {line_no}")
        seen_first.add(key)

    for (sender, receiver, slot), lines_for_edge in defers.items():
        if len(lines_for_edge) > 1:
            errs.append(f"{sender}->{receiver} slot={slot} duplicate defer markers ({len(lines_for_edge)})")
        first_defer = lines_for_edge[0]
        if phase_complete is not None and first_defer > phase_complete:
            warnings.append(f"{sender}->{receiver} slot={slot} defer after phase25.complete ({first_defer}>{phase_complete})")
        send_line = edges.get((sender, receiver))
        if send_line is not None and send_line > first_defer:
            pass
        elif send_line is None:
            warnings.append(f"{sender}->{receiver} slot={slot} defer without later edge.send")

    for sender, receiver in CRITICAL_EDGES:
        send_line = edges.get((sender, receiver))
        recv_ready = ready_lines.get(receiver)
        edge_name = f"{sender}->{receiver}"

        if send_line is None:
            required = (sender, receiver) in REQUIRED_STARTUP_EDGES
            result = "MISSING" if required else "UNOBSERVED"
            if required:
                errs.append(f"{edge_name} missing startup edge.send")
            rows.append(EdgeRow(edge=edge_name, receiver_ready_line=recv_ready, sender_send_line=send_line, result=result))
            continue
        if recv_ready is None:
            errs.append(f"{edge_name} missing receiver ready")
            rows.append(EdgeRow(edge=edge_name, receiver_ready_line=recv_ready, sender_send_line=send_line, result="FAIL"))
            continue

        ok = True
        why: List[str] = []
        if recv_ready >= send_line:
            ok = False
            why.append(f"receiver_ready({recv_ready})>=send({send_line})")
        if phase_complete is not None and phase_complete >= send_line:
            ok = False
            why.append(f"phase25.complete({phase_complete})>=send({send_line})")

        if ok:
            rows.append(EdgeRow(edge=edge_name, receiver_ready_line=recv_ready, sender_send_line=send_line, result="OK"))
        else:
            rows.append(EdgeRow(edge=edge_name, receiver_ready_line=recv_ready, sender_send_line=send_line, result="FAIL"))
            errs.append(f"{edge_name} {'; '.join(why)}")

    if errs:
        return GateResult("ORDER_GATE", False, "; ".join(errs)), rows
    if warnings:
        return GateResult("ORDER_GATE", True, "WARN: " + "; ".join(warnings)), rows
    return GateResult("ORDER_GATE", True), rows


def clock_gate(lines: List[str], strict_clock: bool) -> Tuple[GateResult, List[str]]:
    warnings: List[str] = []
    trace = "\n".join(line.rstrip("\r\n") for line in lines)
    # CIVIL evidence always selects the civil lane, including malformed or
    # incomplete evidence. It must never be rescued by old DEMO receipts.
    civil = any(prefix in trace for prefix in (
        "[boot.silkbar.clock.source]", "[silkbar.civil.",
        "[sexdisplay.clock.civil.", "[display.civil.",
    )) or any("[sexdisplay.clock.presentation]" in line and
              "[sexdisplay.clock.presentation] label=DEMO " not in line for line in lines)
    if civil:
        from silkbar_clock_gate import inspect_publications
        try:
            presentations = list(re.finditer(
                r"\[sexdisplay.clock.presentation\] label=UTC state=Unknown snapshot_only=1 "
                r"x=(\d+) y=(\d+) w=(\d+) h=(\d+) width=(\d+)\r?$", trace, re.MULTILINE))
            assert len(presentations) == trace.count("[sexdisplay.clock.presentation]") == 1, "missing/malformed UTC presentation"
            admitted = inspect_publications(trace)
            draws = list(re.finditer(
                r"\[display.civil.draw\] sequence=(\d+) state=(Unknown|Snapshot|Unavailable|Rollback|Exhausted) "
                r"seconds=(\d+) stale=([01]) pixels=written path=(full|strip) "
                r"x=(\d+) y=(\d+) w=(\d+) h=(\d+)\r?$", trace, re.MULTILINE))
            assert len(draws) == trace.count("[display.civil.draw]") and draws, "missing/malformed completed civil draw"
            snapshot_draws = 0
            for draw in draws:
                sequence, state, seconds, stale = int(draw[1]), draw[2], int(draw[3]), int(draw[4])
                assert presentations[0].start() < draw.start(), "civil draw precedes presentation"
                assert int(draw[8]) > 0 and int(draw[9]) > 0, "empty civil draw"
                if sequence == 0:
                    assert (state, seconds, stale) == ("Unknown", 0, 0), "initial civil draw invents time"
                    continue
                receipt = admitted.get(sequence)
                assert receipt and receipt['offset'] < draw.start(), "civil draw lacks prior exact admission"
                if stale:
                    assert (state, seconds, receipt['state']) == ("Unknown", 0, "Snapshot"), "stale civil draw retains/invents time"
                else:
                    assert (state, seconds) == (receipt['state'], receipt['seconds']), "civil draw differs from admission"
                    snapshot_draws += state == "Snapshot"
            assert snapshot_draws, "no admitted Snapshot with later completed draw"
            assert "[sexdisplay.render.live.ok]" in trace, "missing sexdisplay.render.live.ok"
            return GateResult("CLOCK_GATE", True,
                f"CIVIL admitted/completed snapshots={snapshot_draws}; log receipts only, no pixel or authenticated-time proof"), warnings
        except (AssertionError, ValueError, KeyError, IndexError, TypeError) as exc:
            return GateResult("CLOCK_GATE", False, "CIVIL: " + str(exc)), warnings
    # Canonical V1 runtime chain.
    send_lines = find_all_lines(lines, "[silkbar.clock.send]")
    recv_lines = find_all_lines(lines, "[sexdisplay.clock.recv]")
    redraw_lines = find_all_lines(lines, "[sexdisplay.clock.redraw]")
    live_ok_lines = find_all_lines(lines, "[sexdisplay.render.live.ok]")
    drop_clock_lines = find_all_lines(lines, "[silkbar.send_update.drop.clock]")
    fb_live_wait_lines = find_all_lines(lines, "[sexdisplay.fb.live.wait]")

    errs: List[str] = []
    if len(send_lines) < 1:
        errs.append("missing silkbar.clock.send")
    if len(recv_lines) < 1:
        errs.append("missing sexdisplay.clock.recv")
    if len(redraw_lines) < 1:
        errs.append("missing sexdisplay.clock.redraw")
    if len(live_ok_lines) < 1:
        errs.append("missing sexdisplay.render.live.ok")

    if errs:
        return GateResult("CLOCK_GATE", False, "; ".join(errs)), warnings

    # Diagnostic warnings for degraded-but-live behavior.
    if send_lines and not recv_lines:
        warnings.append("silkbar.clock.send present but sexdisplay.clock.recv absent")
    if recv_lines and not redraw_lines:
        warnings.append("sexdisplay.clock.recv present but sexdisplay.clock.redraw absent")
    if fb_live_wait_lines and not live_ok_lines:
        warnings.append("sexdisplay.fb.live.wait present but sexdisplay.render.live.ok absent")
    if len(drop_clock_lines) > 3:
        warnings.append(f"silkbar.send_update.drop.clock count high ({len(drop_clock_lines)})")

    if warnings:
        if strict_clock:
            return GateResult("CLOCK_GATE", False, "; ".join(warnings)), warnings
        return GateResult("CLOCK_GATE", True, "WARN: " + "; ".join(warnings)), warnings

    return GateResult("CLOCK_GATE", True), warnings


def fault_gate(lines: List[str], allow_fault: bool) -> GateResult:
    hits: List[str] = []
    folded_lines = [line.casefold() for line in lines]
    for pat in FAULT_PATTERNS:
        if any(pat.casefold() in line for line in folded_lines):
            hits.append(pat)
    if hits and not allow_fault:
        return GateResult("FAULT_GATE", False, "fault markers found: " + ", ".join(hits))
    if hits and allow_fault:
        return GateResult("FAULT_GATE", True, "WARN: faults allowed by --allow-fault")
    return GateResult("FAULT_GATE", True)


def print_pd_table(rows: List[PdRow]) -> None:
    print("PD,init_line,ready_line,state")
    for r in rows:
        il = "-" if r.init_line is None else str(r.init_line)
        rl = "-" if r.ready_line is None else str(r.ready_line)
        print(f"{r.pd},{il},{rl},{r.state}")


def print_edge_table(rows: List[EdgeRow]) -> None:
    print("EDGE,receiver_ready_line,sender_send_line,result")
    for r in rows:
        rr = "-" if r.receiver_ready_line is None else str(r.receiver_ready_line)
        ss = "-" if r.sender_send_line is None else str(r.sender_send_line)
        print(f"{r.edge},{rr},{ss},{r.result}")


def main() -> int:
    ap = argparse.ArgumentParser(description="BootGraph serial-log checker")
    ap.add_argument("log_path", help="Serial log path, e.g. /tmp/sexos.log")
    ap.add_argument("--strict-clock", action="store_true", help="Fail CLOCK_GATE on missing/incomplete tick markers")
    ap.add_argument("--allow-fault", action="store_true", help="Allow FAULT_GATE patterns without failing")
    args = ap.parse_args()

    log = Path(args.log_path)
    if not log.exists():
        print(f"BOOTGRAPH FAIL: INPUT_GATE log_not_found path={log}")
        return 1

    lines = log.read_text(errors="replace").splitlines()

    bg_gate, pd_rows, ready_map = bootgraph_gate(lines)
    cap_gate = cap_grant_gate(lines)
    ord_gate, edge_rows = order_gate(lines, ready_map)
    clk_gate, _ = clock_gate(lines, args.strict_clock)
    flt_gate = fault_gate(lines, args.allow_fault)

    gates = [bg_gate, cap_gate, ord_gate, clk_gate, flt_gate]
    failed = [g for g in gates if not g.passed]

    if failed:
        first = failed[0]
        print(f"BOOTGRAPH FAIL: {first.name} {first.reason}".strip())
    else:
        print("BOOTGRAPH PASS")

    for g in gates:
        status = "PASS" if g.passed else "FAIL"
        if g.reason:
            print(f"{g.name}: {status} {g.reason}")
        else:
            print(f"{g.name}: {status}")

    print_pd_table(pd_rows)
    print_edge_table(edge_rows)

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
