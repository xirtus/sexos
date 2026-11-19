#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Reads a SexOS hardware report (Spindle `hw save`, or a saved boot log) from a
real machine and answers, device by device, whether each driver came up.

There is no driver lab: real hardware results come from Andreas's machines (the
Alienware x17 R1 first). This is the collection path for them:

  1. On the machine: Spindle `cd /media/STICK` then `hw save` writes hw-report.txt.
  2. The file reaches the project (chat upload, or Sex/sexwork/hw-reports/ on the Mac).
  3. A thread files it:  python3 scripts/hw_report.py FILE --machine x17r1 --file
     which copies it to hw-reports/x17r1/DATE-N.txt with a .md verdict beside it and
     compares it with the previous report of that machine (fixed, broken, changed).
  4. The PR that adds it shows the verdict in CI (driver-fuzz.yml, hw-report job).

Without --file it prints the verdict as Markdown. --json prints the parsed report.
The checks live in CHECKS below: one row per question the runbook asks
(docs/handoff/X17_BARE_METAL_RUNBOOK_2026_11_18.md), each with the lines that
answer it. A device-reported blob in a line (`edid=` hex) is also kept as a
fuzz seed under fuzz/drivers/seeds/ with --seeds, so real hardware feeds the fuzzers.
"""
import argparse
import datetime
import json
import os
import re
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPORTS = os.path.join(ROOT, "hw-reports")
LINE = re.compile(r"\[([a-z0-9_.]+)\]\s*(.*)")
FIELD = re.compile(r'([A-Za-z0-9_]+)=("([^"]*)"|\S+)')

# (id, question, good tags, bad tags, what bad means). A tag matches a line's
# [tag] exactly or as a prefix ending in '.'; "tag key=value" also needs that field.
CHECKS = [
    ("census", "Which devices are there", ["hw.device", "hw.census"], ["hw.warning"], "a device has no driver"),
    ("vmd", "RAID mode (VMD) opened", ["kernel.vmd.done"], ["kernel.vmd.skip"], "Intel VMD was not opened: no SSD behind it"),
    ("nvme", "The SSD answered", ["sexdrive.nvme.drive"], ["sexdrive.nvme.refused", "sexdrive.nvme.err"], "the NVMe drive was refused"),
    ("nvme_format", "SSD sector format", ["sexdrive.nvme.namespace"], ["sexdrive.nvme.namespace.refused"], "4 KiB sectors (not supported yet)"),
    ("nvme_irq", "SSD interrupts", ["sexdrive.nvme.irq mode=msix"], ["sexdrive.nvme.irq.silent"], "the interrupt never arrived; sexdrive polls"),
    ("raid", "RAID volume served", ["sexdrive.raid.serve"], ["sexdrive.raid.refused"], "the RAID set was not served"),
    ("disk", "Whose disk", ["sexdrive.disk.layout", "files.fs.foreign"], [], ""),
    ("windows", "Windows volumes (read only)", ["files.media.mounted"], ["files.media.bitlocker"], "BitLocker: that volume stays closed (expected on many laptops)"),
    ("iommu", "DMA isolation", ["iommu.enabled"], ["iommu.absent"], "no IOMMU: devices can reach all memory"),
    ("display", "Intel display", ["display.intel"], ["display.intel.failed"], "the Intel display driver did not take over"),
    ("panel", "Panel width", ["display.chrome.metrics"], [], ""),
    ("gpu", "NVIDIA GPU found and powered", ["gpu.found", "gpu.power"], ["gpu.failed"], "the dGPU was not handled"),
    ("wifi", "Wi-Fi up", ["wifi.ready"], ["wifi.failed"], "Wi-Fi did not come up"),
    ("sound", "Sound (HDA codec)", ["bell.hda.ready", "bell.hda.codec"], ["bell.hda.failed"], "no sound device"),
    ("usb", "USB controller and devices", ["sexusb.ready", "sexusb.enum.summary"], ["sexusb.xhci.start.fail", "sexusb.device.fail"], "a USB device failed to enumerate"),
    ("touchpad", "Touchpad", ["sexinput.touchpad.ready", "sexinput.touchpad.found"], ["sexinput.touchpad.reject", "sexinput.touchpad.stopped"], "the touchpad was refused or stopped"),
    ("acpi", "ACPI tables and AML", ["sexacpi.tables"], ["sexacpi.ready aml=0", "sexacpi.fadt.refused", "sexacpi.aml.fatal"], "firmware AML did not load"),
    ("battery", "Battery shown", ["display.panel.battery"], [], ""),
    ("crash", "Nothing crashed", [], ["panic_handler", "crash.record", "service.restarted", "sexacpi.aml.panic", "sexuvc.panic"], "a driver or service stopped"),
]


def parse(text):
    """Every [tag] line, with its key=value fields, in order."""
    lines = []
    for raw in text.splitlines():
        m = LINE.search(raw)
        if not m:
            continue
        tag, rest = m.group(1), m.group(2)
        fields = {k: (q if q is not None and v.startswith('"') else v) for k, v, q in FIELD.findall(rest)}
        lines.append({"tag": tag, "fields": fields, "text": raw.strip()})
    return lines


def _match(line, pattern):
    tag, _, need = pattern.partition(" ")
    if not (line["tag"] == tag or line["tag"].startswith(tag + ".")):
        return False
    for pair in need.split():
        k, _, v = pair.partition("=")
        if line["fields"].get(k) != v:
            return False
    return True


def verdict(lines):
    """One row per check: ok (good lines only), bad (any bad line), or unknown."""
    rows = []
    for cid, question, good, bad, meaning in CHECKS:
        g = [l["text"] for l in lines if any(_match(l, p) for p in good)]
        b = [l["text"] for l in lines if any(_match(l, p) for p in bad)]
        state = "bad" if b else ("ok" if g else ("ok" if not good else "unknown"))
        rows.append({"id": cid, "question": question, "state": state, "meaning": meaning if b else "",
                     "evidence": (b or g)[:3]})
    return rows


def devices(lines):
    """PCI census rows: bdf -> vendor:device and the support verdict."""
    out = {}
    for l in lines:
        if l["tag"] == "hw.device":
            words = l["text"].split("]", 1)[1].split()
            bdf = words[0] if words else "?"
            ident = next((w for w in words[1:] if re.fullmatch(r"[0-9a-f]{4}:[0-9a-f]{4}", w)), "")
            out[bdf] = {"id": ident, "support": l["fields"].get("support", "")}
    return out


def compare(old_rows, new_rows):
    before = {r["id"]: r["state"] for r in old_rows}
    changes = []
    for r in new_rows:
        was = before.get(r["id"])
        if was and was != r["state"]:
            kind = "fixed" if r["state"] == "ok" else ("broken" if was == "ok" else "changed")
            changes.append((kind, r["question"], was, r["state"]))
    return changes


ICON = {"ok": "ok", "bad": "BAD", "unknown": "?"}


def markdown(name, rows, devs, changes=None):
    out = [f"# Hardware report: {name}", ""]
    n = {s: sum(r["state"] == s for r in rows) for s in ICON}
    out.append(f"{n['ok']} ok, {n['bad']} bad, {n['unknown']} without an answer in the report.")
    out.append("")
    if changes:
        out.append("## Since the previous report")
        out += [f"- **{k}**: {q} ({a} -> {b})" for k, q, a, b in changes]
        out.append("")
    out += ["| Check | State | Evidence |", "|---|---|---|"]
    for r in rows:
        ev = "<br>".join(e.replace("|", "\\|")[:140] for e in r["evidence"])
        note = f" {r['meaning']}." if r["meaning"] else ""
        out.append(f"| {r['question']} | {ICON[r['state']]}{note} | `{ev}` |" if ev else f"| {r['question']} | {ICON[r['state']]}{note} | |")
    if devs:
        out += ["", f"## PCI devices ({len(devs)})", "", "| Address | Id | Support |", "|---|---|---|"]
        out += [f"| {bdf} | {d['id']} | {d['support']} |" for bdf, d in sorted(devs.items())]
    return "\n".join(out) + "\n"


SEED_FIELDS = {"edid": "drv_display_intel"}


def seeds(lines, dest):
    """Device-reported hex blobs become fuzz seeds for the matching target."""
    n = 0
    for l in lines:
        for key, target in SEED_FIELDS.items():
            v = l["fields"].get(key)
            if v and re.fullmatch(r"(?:[0-9a-fA-F]{2})+", v):
                d = os.path.join(dest, target)
                os.makedirs(d, exist_ok=True)
                with open(os.path.join(d, f"hw-{l['tag']}-{n}.bin"), "wb") as f:
                    f.write(bytes.fromhex(v))
                n += 1
    return n


def previous(machine, exclude=None):
    d = os.path.join(REPORTS, machine)
    if not os.path.isdir(d):
        return None
    names = sorted(f for f in os.listdir(d) if f.endswith(".txt") and f != exclude)
    return os.path.join(d, names[-1]) if names else None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("report", nargs="?", help="hw-report.txt (or a boot log)")
    ap.add_argument("--machine", default="x17r1", help="machine name, the folder under hw-reports/")
    ap.add_argument("--file", action="store_true", help="copy into hw-reports/MACHINE/ with a .md verdict")
    ap.add_argument("--json", action="store_true", help="print the parsed report as JSON")
    ap.add_argument("--seeds", metavar="DIR", help="write device-reported blobs as fuzz seeds into DIR")
    ap.add_argument("--check-all", metavar="DIR", help="print the verdict of every report under DIR")
    a = ap.parse_args(argv)
    if a.check_all:
        for machine in sorted(os.listdir(a.check_all)):
            d = os.path.join(a.check_all, machine)
            if not os.path.isdir(d):
                continue
            for f in sorted(x for x in os.listdir(d) if x.endswith(".txt")):
                with open(os.path.join(d, f), errors="replace") as fh:
                    lines = parse(fh.read())
                print(markdown(f"{machine}/{f}", verdict(lines), devices(lines)))
        return 0
    if not a.report:
        ap.error("a report file is needed")
    with open(a.report, errors="replace") as fh:
        text = fh.read()
    lines = parse(text)
    rows, devs = verdict(lines), devices(lines)
    if a.json:
        print(json.dumps({"lines": len(lines), "checks": rows, "devices": devs}, indent=1))
        return 0
    if a.seeds:
        print(f"[hw_report.seeds] written={seeds(lines, a.seeds)}", file=sys.stderr)
    prev = previous(a.machine)
    changes = None
    if prev:
        with open(prev, errors="replace") as fh:
            changes = compare(verdict(parse(fh.read())), rows)
    if a.file:
        d = os.path.join(REPORTS, a.machine)
        os.makedirs(d, exist_ok=True)
        day = datetime.date.today().isoformat()
        k = 1
        while os.path.exists(os.path.join(d, f"{day}-{k}.txt")):
            k += 1
        base = os.path.join(d, f"{day}-{k}")
        shutil.copyfile(a.report, base + ".txt")
        with open(base + ".md", "w") as fh:
            fh.write(markdown(f"{a.machine} {day} #{k}", rows, devs, changes))
        print(base + ".md")
        return 0
    sys.stdout.write(markdown(a.machine, rows, devs, changes))
    return 0


if __name__ == "__main__":
    sys.exit(main())
