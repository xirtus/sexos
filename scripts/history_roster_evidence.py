"""Source-bound BOOT history metadata joins; catalog data alone is not authority."""
import re
from pathlib import Path

CATALOG = "crates/sex-pdx/src/app_catalog.rs"
U32_MAX = (1 << 32) - 1
U64_MAX = (1 << 64) - 1


def _source(root):
    text = (Path(root) / CATALOG).read_text()
    text = re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.S)
    match = re.search(r"pub\s+const\s+INCARNATIONS\s*:\s*\[Incarnation;\s*(\d+)\]\s*=\s*\[(.*?)\];", text, re.S)
    if not match:
        raise ValueError("missing literal incarnation catalog")
    return int(match[1]), match[2]


def source_spindles(root):
    """Read literal Spindle rows without treating their identities as grants."""
    _, source = _source(root)
    rows = []
    pattern = re.compile(
        r"\s*app:\s*App::Spindle,\s*ordinal:\s*(\d+),\s*target:\s*(\d+),"
        r"\s*surface:\s*(\d+),\s*shell_surface:\s*(None|Some\(\s*\d+\s*\)),"
        r"\s*lifecycle_slot:\s*(\d+),\s*close_slot:\s*(\d+),"
        r"\s*activation_slot:\s*(None|Some\(\s*\d+\s*\)),\s*route_slot:\s*(\d+),"
        r"\s*children:\s*&\[\s*(?:\d+\s*(?:,\s*\d+\s*)*,?\s*)?\]\s*,?\s*"
    )
    for block in re.findall(r"Incarnation\s*\{([^{}]*)\}", source):
        if not re.search(r"app\s*:\s*App::Spindle\b", block):
            continue
        match = pattern.fullmatch(block)
        if not match:
            raise ValueError("nonliteral or malformed Spindle catalog row")
        ordinal, target, surface, frame, lifecycle, close, activation, route = match.groups()
        row = dict(ordinal=int(ordinal), target=int(target), surface=int(surface),
                   lifecycle=int(lifecycle), close=int(close), route=int(route))
        row["frame"] = row["surface"] if frame == "None" else int(re.search(r"\d+", frame)[0])
        row["activation"] = None if activation == "None" else int(re.search(r"\d+", activation)[0])
        rows.append(row)
    if not 1 <= len(rows) <= 64 or len(rows) != len(re.findall(r"app\s*:\s*App::Spindle\b", source)):
        raise ValueError("missing or unsupported Spindle roster")
    for i, row in enumerate(rows):
        if row["ordinal"] != i or not 1 <= row["target"] <= U32_MAX:
            raise ValueError("Spindle identity/order")
        if any(not 1 <= row[k] <= U64_MAX for k in ("surface", "frame")):
            raise ValueError("Spindle surface/frame")
        if any(not 1 <= row[k] <= 1024 for k in ("lifecycle", "close", "route")):
            raise ValueError("Spindle slot")
        if (row["activation"] is None) != (i == 0) or (i and not 1 <= row["activation"] <= 1024):
            raise ValueError("Spindle activation lineage")
    for key in ("target", "surface", "frame", "lifecycle", "close", "route"):
        if len({row[key] for row in rows}) != len(rows):
            raise ValueError("duplicate Spindle " + key)
    return rows


def _records(text, tag, fields):
    """Count every marker, including atomic records following interrupted prints."""
    result = []
    pattern = re.compile(" ".join(re.escape(name) + r"=([0-9]+)" for name in fields))
    for marker in re.finditer(re.escape("[" + tag + "]"), text):
        end = text.find("\n", marker.end())
        if end < 0:
            raise ValueError("truncated " + tag)
        payload = text[marker.end():end].removesuffix("\r")
        match = pattern.fullmatch(payload[1:]) if payload.startswith(" ") else None
        if not match:
            raise ValueError("malformed " + tag)
        record = dict(zip(fields, map(int, match.groups())))
        if any(value > U64_MAX for value in record.values()):
            raise ValueError("overflow " + tag)
        result.append((marker.start(), record))
    return result


def verify_boot_roster(root, text):
    rows = source_spindles(root)
    declared, _ = _source(root)
    headers = _records(text, "boot.history.recovery", (
        "holder", "slot", "logical", "participants", "version", "base", "sectors", "held_query_verified", "runtime_mint"))
    participants = _records(text, "boot.history.participant", (
        "index", "target", "task", "holder", "immutable", "runtime_mint"))
    catalogs = _records(text, "boot.app.catalog", (
        "target", "task", "surface", "logical", "incarnation", "predecessor", "lifecycle",
        "close", "route", "dormant", "grants_verified", "data_only"))
    complete = _records(text, "boot.app.catalog.complete", ("rows", "actual_grants", "runtime_mint"))
    if len(headers) != 1 or len(participants) != len(rows) or len(complete) != 1:
        raise ValueError("missing/duplicate history roster or catalog completion")
    expected = dict(holder=11, slot=53, logical=rows[0]["target"], participants=len(rows),
                    version=1, base=1808, sectors=18, held_query_verified=1, runtime_mint=0)
    if headers[0][1] != expected:
        raise ValueError("history header mismatch")
    if complete[0][1] != dict(rows=declared, actual_grants=1, runtime_mint=0) or len(catalogs) != declared:
        raise ValueError("incomplete verified catalog")
    seals = list(re.finditer(r"\[boot.memory.authority.sealed\] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\r?\n", text))
    if len(seals) != 1 or text.count("[boot.memory.authority.sealed]") != 1:
        raise ValueError("missing/duplicate/malformed memory seal")
    handoffs = list(re.finditer(r"\[bootgraph.boot.handoff\] target=[A-Za-z0-9_-]+ id=[0-9]+ entry=0x[0-9a-fA-F]+\r?\n", text))
    if not handoffs or len(handoffs) != text.count("[bootgraph.boot.handoff]"):
        raise ValueError("missing/malformed boot handoff")
    if not headers[0][0] < min(pos for pos, _ in participants) or not (
        max(pos for pos, _ in participants + catalogs) < complete[0][0] < seals[0].start() < handoffs[0].start()
    ):
        raise ValueError("history/catalog/seal/handoff ordering")
    by_target = {}
    tasks = set()
    for _, catalog in catalogs:
        if not 1 <= catalog["target"] <= U32_MAX or not 1 <= catalog["task"] <= U32_MAX:
            raise ValueError("catalog identity width")
        if catalog["target"] in by_target or catalog["task"] in tasks:
            raise ValueError("duplicate catalog identity")
        by_target[catalog["target"]] = catalog
        tasks.add(catalog["task"])
    normalized = []
    for i, (row, (_, participant)) in enumerate(zip(rows, participants)):
        catalog = by_target.get(row["target"])
        if catalog is None:
            raise ValueError("missing Spindle catalog")
        wanted = dict(target=row["target"], task=catalog["task"], surface=row["surface"],
                      logical=rows[0]["target"], incarnation=i + 1, predecessor=0 if i == 0 else rows[i-1]["target"],
                      lifecycle=row["lifecycle"], close=row["close"], route=row["route"], dormant=int(i != 0),
                      grants_verified=1, data_only=1)
        if catalog != wanted:
            raise ValueError("Spindle catalog/source mismatch")
        if participant != dict(index=i, target=row["target"], task=catalog["task"], holder=11, immutable=1, runtime_mint=0):
            raise ValueError("history participant/catalog mismatch")
        normalized.append(dict(target=row["target"], task=catalog["task"]))
    return {**{k: v for k, v in expected.items() if k != "participants"},
            "count": len(rows), "participants": normalized}


def verify_writer(meta, roster):
    """Join durable writer metadata to an already verified held BOOT roster."""
    source, task = meta.get("source"), meta.get("task")
    if type(source) is not int or type(task) is not int or not 1 <= source <= U32_MAX or not 1 <= task <= U32_MAX:
        raise ValueError("journal writer identity")
    if "logical" in meta and (type(meta["logical"]) is not int or meta["logical"] != roster["logical"]):
        raise ValueError("journal logical identity")
    if dict(target=source, task=task) not in roster["participants"]:
        raise ValueError("journal writer outside BOOT recovery roster")
    return dict(target=source, task=task, logical=roster["logical"])
