#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Builds the public SexOS hardware compatibility list (docs/HARDWARE.md and
docs/hardware.json) from two sources:

  * crates/sex-hwdb: what each driver supports on master. Every exact id in
    its census table (`exact()`), and the class rules (`by_class()`) that
    decide for ids the table does not name.
  * hw-reports/: what real machines reported. The newest report of each
    machine (Spindle `hw save`, filed by scripts/hw_report.py) lists every PCI
    device it saw (`[hw.device]` lines) and answers the runbook's questions
    (sound up, Wi-Fi up, SSD answered...). A device's row says which machines
    it was seen on and whether its driver came up there.

  python3 scripts/hardware_list.py            rewrite docs/HARDWARE.md and docs/hardware.json
  python3 scripts/hardware_list.py --check    exit 1 if they are out of date (prints what changed)
  python3 scripts/hardware_list.py --stdout   print the Markdown

Nothing here is hand-maintained: add a device by adding its row to sex-hwdb,
add a machine by filing its report.
"""
import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import hw_report  # noqa: E402

HWDB = os.path.join(ROOT, "crates", "sex-hwdb", "src", "lib.rs")
REPORTS = os.path.join(ROOT, "hw-reports")
OUT_MD = os.path.join(ROOT, "docs", "HARDWARE.md")
OUT_JSON = os.path.join(ROOT, "docs", "hardware.json")

# sex_hwdb::Service names (Service::name) and what a reader calls them.
SERVICES = {
    "Drive": ("sexdrive", "Storage"),
    "Net": ("sexnet", "Network"),
    "Bell": ("sexbell", "Sound"),
    "Usb": ("sexusb", "USB and Thunderbolt"),
    "Display": ("sexdisplay", "Graphics and display"),
    "Input": ("sexinput", "Input buses"),
    "Saddle": ("saddle", "Harness SDK drivers"),
    "Kernel": ("kernel", "Platform"),
    "Nothing": ("none", "Other"),
}
ORDER = ["Drive", "Display", "Net", "Bell", "Usb", "Input", "Saddle", "Kernel", "Nothing"]
SUPPORT = {
    "Driven": ("driven", "Works: a SexOS driver drives it"),
    "Firmware": ("firmware", "Works through firmware setup only (boot framebuffer), no native driver yet"),
    "Platform": ("platform", "Platform plumbing; nothing to drive"),
    "InProgress": ("in_progress", "Driver in progress (open branch or untried on real hardware)"),
    "Planned": ("planned", "In the driver plan, not started"),
    "Unsupported": ("none", "Not supported"),
}
# Which runbook check (hw_report.CHECKS) says whether a device's driver came up
# on a machine; by service, refined by PCI class.
CHECK_FOR = {"Drive": "nvme", "Bell": "sound", "Usb": "usb", "Input": "touchpad"}


def _strip_comments(src):
    return "\n".join(l.split("//", 1)[0] for l in src.splitlines())


def _fn_body(src, name):
    m = re.search(r"fn " + name + r"\b[^{]*\{", src)
    if not m:
        raise SystemExit(f"sex-hwdb: fn {name} not found")
    depth, i = 1, m.end()
    while depth:
        depth += {"{": 1, "}": -1}.get(src[i], 0)
        i += 1
    return src[m.end():i - 1]


def hwdb(src=None):
    """(devices, classes, vendors) parsed from sex-hwdb's lib.rs."""
    if src is None:
        with open(HWDB) as f:
            src = f.read()
    vendors = {}
    for ids, name in re.findall(r"((?:0x[0-9a-fA-F]{4}\s*\|?\s*)+)=>\s*\"([^\"]+)\"", _strip_comments(_fn_body(src, "vendor_name"))):
        for v in re.findall(r"0x([0-9a-fA-F]{4})", ids):
            vendors[v.lower()] = name
    classes_named = {}
    for c, s, p, name in re.findall(r"\((0x[0-9a-f]{2}|_), (0x[0-9a-f]{2}|_), (0x[0-9a-f]{2}|_)\)\s*=>\s*\"([^\"]+)\"", _fn_body(src, "class_name")):
        classes_named[(c, s, p)] = name

    devices = []
    body = _fn_body(src, "exact")
    section = ""
    for raw in body.splitlines():
        line = raw.strip()
        if line.startswith("//"):
            section = line.lstrip("/ ").rstrip(".")
            continue
        m = re.search(r"=>\s*known\(\"([^\"]*)\",\s*(\w+),\s*(\w+)\)", line)
        if not m:
            continue
        model, service, support = m.groups()
        for v, d in re.findall(r"\(0x([0-9a-fA-F]{4}),\s*0x([0-9a-fA-F]{4})\)", line.split("=>")[0]):
            devices.append({"vendor": v.lower(), "device": d.lower(), "vendor_name": vendors.get(v.lower(), "Unknown vendor"),
                            "model": model, "service": service, "support": support, "group": section})

    classes = []
    for line in _strip_comments(_fn_body(src, "by_class")).splitlines():
        if " => " not in line:
            continue
        left, right = line.strip().rsplit(" => ", 1)
        r = re.match(r"\((\w+), (\w+)\)", right.strip())
        if not r:
            continue
        service, support = r.groups()
        pats, _, guard = left.partition(" if ")
        found = re.findall(r"\((0x[0-9a-f]{2}|_), (0x[0-9a-f]{2}|_), (0x[0-9a-f]{2}|_)\)", pats)
        if pats.strip() == "_":
            found = [("_", "_", "_")]
        who = ""
        if guard:
            ids = [vendors.get(x.lower(), x) for x in re.findall(r"0x([0-9a-fA-F]{4})", guard)]
            if "serial_io_i2c" in guard:
                who = "Intel Serial IO I2C functions"
            elif "!=" in guard and ids:
                who = "not " + " or ".join(ids)
            elif ids:
                who = " or ".join(ids)
        for c, s_, p in found:
            name = classes_named.get((c, s_, p)) or classes_named.get((c, s_, "_")) or classes_named.get((c, "_", "_")) or "device"
            if c == "_":
                name = "anything else"
            classes.append({"class": c, "subclass": s_, "prog_if": p, "name": name, "only": who, "service": service, "support": support})
    return devices, classes, vendors


def machines(reports_dir=REPORTS):
    """The newest report of each machine under hw-reports/, parsed."""
    out = []
    if not os.path.isdir(reports_dir):
        return out
    for machine in sorted(os.listdir(reports_dir)):
        d = os.path.join(reports_dir, machine)
        if not os.path.isdir(d):
            continue
        names = sorted(f for f in os.listdir(d) if f.endswith(".txt"))
        if not names:
            continue
        with open(os.path.join(d, names[-1]), errors="replace") as f:
            lines = hw_report.parse(f.read())
        seen = {}
        for l in lines:
            if l["tag"] != "hw.device":
                continue
            words = l["text"].split("]", 1)[1].split()
            ident = next((w for w in words[1:] if re.fullmatch(r"[0-9a-f]{4}:[0-9a-f]{4}", w)), None)
            if ident:
                seen[ident] = {"bdf": words[0], "class": l["fields"].get("class", ""), "support": l["fields"].get("support", "")}
        checks = {r["id"]: r["state"] for r in hw_report.verdict(lines)}
        out.append({"machine": machine, "report": f"hw-reports/{machine}/{names[-1]}", "reports": len(names),
                    "devices": seen, "checks": checks})
    return out


def check_for(dev, cls=""):
    if dev["service"] == "Net":
        return "wifi" if cls.startswith("0280") or "Wi-Fi" in dev.get("model", "") else None
    if dev["service"] == "Display":
        return "gpu" if dev["vendor"] == "10de" else "display"
    return CHECK_FOR.get(dev["service"])


def build(src=None, reports_dir=REPORTS):
    devices, classes, vendors = hwdb(src)
    ms = machines(reports_dir)
    known = set()
    for dev in devices:
        ident = f"{dev['vendor']}:{dev['device']}"
        known.add(ident)
        dev["seen"] = []
        for m in ms:
            hit = m["devices"].get(ident)
            if hit:
                c = check_for(dev, hit["class"])
                dev["seen"].append({"machine": m["machine"], "support_reported": hit["support"],
                                    "check": c, "state": m["checks"].get(c, "unknown") if c else "unknown"})
    unlisted = []
    for m in ms:
        for ident, hit in sorted(m["devices"].items()):
            if ident not in known:
                unlisted.append({"id": ident, "machine": m["machine"], "class": hit["class"], "support_reported": hit["support"],
                                 "vendor_name": vendors.get(ident[:4], "Unknown vendor")})
    return {"devices": devices, "classes": classes, "machines": ms, "unlisted": unlisted}


STATE = {"ok": "works", "bad": "**failed**", "unknown": "not checked"}


def markdown(data):
    out = ["# SexOS hardware compatibility list", "",
           "<!-- Generated by scripts/hardware_list.py from crates/sex-hwdb and hw-reports/. Do not edit by hand. -->", "",
           "What SexOS drives today, device by device, and what real machines have shown. "
           "Two sources feed it, and nothing in it is written by hand:", "",
           "- **Support** is what the driver table in `crates/sex-hwdb` says about master: the same table "
           "BOOT prints as `[hw.device]` lines on every boot.",
           "- **Seen on** is what real machines reported: each machine's newest Spindle `hw save` report "
           "under `hw-reports/` (see its README to send one). `works` means the driver came up on that "
           "machine; `failed` means it was there and did not.", "",
           "Any device not named below falls under the class rules at the end. To add a device, add its row "
           "to `crates/sex-hwdb`; to write its driver, start from the driver SDK (`docs/sdk/DRIVER_SDK.md`).", ""]
    n = len(data["devices"])
    works = sum(hw_report_support_works(d["support"]) for d in data["devices"])
    out += [f"{n} named devices, {works} of them working on master. "
            f"{len(data['machines'])} real machine{'s' if len(data['machines']) != 1 else ''} reported.", ""]
    out += ["## Support levels", "", "| Support | Meaning |", "|---|---|"]
    out += [f"| {name} | {meaning} |" for name, meaning in SUPPORT.values()]
    out.append("")

    out += ["## Real machines", ""]
    if data["machines"]:
        out += ["| Machine | Newest report | Checks that work | Failed | Not answered |", "|---|---|---|---|---|"]
        for m in data["machines"]:
            st = list(m["checks"].values())
            out.append(f"| {m['machine']} | `{m['report']}` | {st.count('ok')} | {st.count('bad')} | {st.count('unknown')} |")
    else:
        out.append("No real-machine report yet. The first will come from the Alienware x17 R1 "
                   "(`docs/handoff/X17_BARE_METAL_RUNBOOK_2026_11_18.md`).")
    out.append("")

    by = {}
    for d in data["devices"]:
        by.setdefault(d["service"], []).append(d)
    for service in ORDER + sorted(set(by) - set(ORDER)):
        rows = by.get(service)
        if not rows:
            continue
        svc, title = SERVICES.get(service, (service.lower(), service))
        out += [f"## {title} (`{svc}`)", "", "| Device | PCI id | Support | Seen on |", "|---|---|---|---|"]
        for d in sorted(rows, key=lambda d: (d["vendor_name"], d["model"], d["device"])):
            seen = ", ".join(f"{s['machine']}: {STATE[s['state']]}" for s in d["seen"]) or ""
            out.append(f"| {d['vendor_name']} {d['model']} | `{d['vendor']}:{d['device']}` | {SUPPORT[d['support']][0]} | {seen} |")
        out.append("")

    if data["unlisted"]:
        out += ["## Seen on real machines, not named in the table", "",
                "These went by the class rules below on the machine that reported them.", "",
                "| PCI id | Vendor | Class | Machine | Support reported |", "|---|---|---|---|---|"]
        out += [f"| `{u['id']}` | {u['vendor_name']} | {u['class']} | {u['machine']} | {u['support_reported']} |" for u in data["unlisted"]]
        out.append("")

    out += ["## Any other device, by kind", "",
            "Ids the table does not name are decided by their PCI class (first matching rule wins).", "",
            "| Kind | PCI class | Only for | Driver | Support |", "|---|---|---|---|---|"]
    for c in data["classes"]:
        cls = "any" if c["class"] == "_" else ".".join(x[2:] if x != "_" else "*" for x in (c["class"], c["subclass"], c["prog_if"]))
        svc = SERVICES.get(c["service"], (c["service"].lower(), ""))[0]
        out.append(f"| {c['name']} | {cls} | {c['only']} | {svc} | {SUPPORT[c['support']][0]} |")
    return "\n".join(out) + "\n"


def hw_report_support_works(support):
    return support in ("Driven", "Firmware", "Platform")


def as_json(data):
    keep = {"devices": data["devices"], "classes": data["classes"], "unlisted": data["unlisted"],
            "machines": [{k: m[k] for k in ("machine", "report", "reports", "checks")} for m in data["machines"]]}
    return json.dumps(keep, indent=1, sort_keys=True) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="exit 1 if docs/HARDWARE.md or docs/hardware.json is stale")
    ap.add_argument("--stdout", action="store_true", help="print the Markdown instead of writing files")
    ap.add_argument("--reports", default=REPORTS, help="the hw-reports directory")
    a = ap.parse_args(argv)
    data = build(reports_dir=a.reports)
    md, js = markdown(data), as_json(data)
    if a.stdout:
        sys.stdout.write(md)
        return 0
    if a.check:
        stale = []
        for path, text in ((OUT_MD, md), (OUT_JSON, js)):
            try:
                with open(path) as f:
                    if f.read() != text:
                        stale.append(path)
            except FileNotFoundError:
                stale.append(path)
        for p in stale:
            print(f"[hardware_list.stale] file={os.path.relpath(p, ROOT)} fix=\"python3 scripts/hardware_list.py\"")
        if not stale:
            print(f"[hardware_list.ok] devices={len(data['devices'])} machines={len(data['machines'])}")
        return 1 if stale else 0
    for path, text in ((OUT_MD, md), (OUT_JSON, js)):
        with open(path, "w") as f:
            f.write(text)
    print(f"[hardware_list.written] devices={len(data['devices'])} classes={len(data['classes'])} machines={len(data['machines'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
