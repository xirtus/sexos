import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("history_roster_evidence", ROOT / "scripts/history_roster_evidence.py")
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)

ROWS = [
    (0, 12, 154, 153, 39, 42, None, 14),
    (1, 17, 302, 306, 50, 51, 49, 52),
    (2, 24, 313, 314, 80, 81, 82, 83),
]


def source(rows):
    result = f"pub const INCARNATIONS: [Incarnation; {len(rows)}] = [\n"
    for ordinal, target, surface, frame, lifecycle, close, activation, route in rows:
        result += (f"Incarnation {{ app: App::Spindle, ordinal: {ordinal}, target: {target}, surface: {surface}, "
                   f"shell_surface: Some({frame}), lifecycle_slot: {lifecycle}, close_slot: {close}, "
                   f"activation_slot: {'None' if activation is None else f'Some({activation})'}, route_slot: {route}, children: &[] }},\n")
    return result + "];\n"


def trace(rows):
    text = f"[boot.history.recovery] holder=11 slot=53 logical=12 participants={len(rows)} version=1 base=1808 sectors=18 held_query_verified=1 runtime_mint=0\n"
    for ordinal, target, *_ in rows:
        text += f"[boot.history.participant] index={ordinal} target={target} task={target*10} holder=11 immutable=1 runtime_mint=0\n"
    for i, (ordinal, target, surface, frame, lifecycle, close, activation, route) in enumerate(rows):
        text += (f"[boot.app.catalog] target={target} task={target*10} surface={surface} logical=12 incarnation={ordinal+1} "
                 f"predecessor={0 if i == 0 else rows[i-1][1]} lifecycle={lifecycle} close={close} route={route} "
                 f"dormant={int(i != 0)} grants_verified=1 data_only=1\n")
    return text + (f"[boot.app.catalog.complete] rows={len(rows)} actual_grants=1 runtime_mint=0\n"
                   "[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\n"
                   "[bootgraph.boot.handoff] target=sexdisplay id=1 entry=0x4000cae0\n")


class HistoryRoster(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / h.CATALOG
        self.path.parent.mkdir(parents=True)
        self.rows = ROWS[:2]
        self.path.write_text(source(self.rows))

    def check(self, text):
        return h.verify_boot_roster(self.root, text)

    def test_actual_source_nine_literal_spindles(self):
        self.assertEqual(h.source_spindles(ROOT), [
            dict(ordinal=0, target=12, surface=154, frame=153, lifecycle=39, close=42, activation=None, route=14),
            dict(ordinal=1, target=17, surface=302, frame=306, lifecycle=50, close=51, activation=49, route=52),
            dict(ordinal=2, target=24, surface=313, frame=314, lifecycle=130, close=131, activation=132, route=133),
            dict(ordinal=3, target=25, surface=318, frame=319, lifecycle=134, close=135, activation=136, route=137),
            dict(ordinal=4, target=26, surface=323, frame=324, lifecycle=138, close=139, activation=140, route=141),
            dict(ordinal=5, target=27, surface=328, frame=329, lifecycle=142, close=143, activation=144, route=145),
            dict(ordinal=6, target=28, surface=333, frame=334, lifecycle=146, close=147, activation=148, route=149),
            dict(ordinal=7, target=29, surface=338, frame=339, lifecycle=150, close=151, activation=152, route=153),
            dict(ordinal=8, target=40, surface=353, frame=354, lifecycle=194, close=195, activation=196, route=197),
        ])

    def test_current_full_catalog_joins_only_exact_spindle_tasks(self):
        # Independent current non-Spindle inventory, not generated from parser output.
        others = [(9,156,9,1,0,38,41,11,0), (15,300,15,1,0,40,43,43,0),
                  (16,301,9,2,9,45,46,47,1), (18,307,9,3,16,54,56,57,1),
                  (19,308,9,4,18,110,111,113,1), (20,309,9,5,19,114,115,117,1),
                  (21,310,9,6,20,118,119,121,1), (22,311,9,7,21,122,123,125,1),
                  (23,312,9,8,22,126,127,129,1),
                  (30,343,15,2,15,154,155,157,1), (31,344,15,3,30,158,159,161,1),
                  (32,345,9,9,23,162,163,165,1), (33,346,9,10,32,166,167,169,1),
                  (34,347,9,11,33,170,171,173,1), (35,348,9,12,34,174,175,177,1),
                  (36,349,9,13,35,178,179,181,1), (37,350,9,14,36,182,183,185,1),
                  (38,351,9,15,37,186,187,189,1), (39,352,9,16,38,190,191,193,1),
                  (41,358,41,1,0,240,241,243,0), (42,359,41,2,41,244,245,247,1),
                  (43,360,41,3,42,248,249,251,1), (44,361,41,4,43,252,253,255,1),
                  (45,362,41,5,44,420,421,423,1), (46,363,41,6,45,424,425,427,1), (47,364,41,7,46,428,429,431,1), (48,365,41,8,47,432,433,435,1), (49,366,41,9,48,436,437,439,1)]
        extra = ""
        for target, surface, logical, inc, pred, lifecycle, close, route, dormant in others:
            extra += (f"[boot.app.catalog] target={target} task={target*10} surface={surface} logical={logical} "
                      f"incarnation={inc} predecessor={pred} lifecycle={lifecycle} close={close} route={route} "
                      f"dormant={dormant} grants_verified=1 data_only=1\n")
        actual_rows=ROWS[:2]+[(2,24,313,314,130,131,132,133),(3,25,318,319,134,135,136,137),(4,26,323,324,138,139,140,141),(5,27,328,329,142,143,144,145),(6,28,333,334,146,147,148,149),(7,29,338,339,150,151,152,153),(8,40,353,354,194,195,196,197)]
        text = trace(actual_rows).replace("[boot.app.catalog.complete] rows=9", extra + "[boot.app.catalog.complete] rows=37")
        self.assertEqual(h.verify_boot_roster(ROOT, text)["participants"],
                         [dict(target=12, task=120), dict(target=17, task=170),dict(target=24,task=240),dict(target=25,task=250),dict(target=26,task=260),dict(target=27,task=270),dict(target=28,task=280),dict(target=29,task=290),dict(target=40,task=400)])
        # Descriptive Browser/Quil rows do not create Spindle writer authority.
        roster=h.verify_boot_roster(ROOT,text)
        self.assertEqual(h.verify_writer(dict(source=40,task=400,logical=12),roster),dict(target=40,task=400,logical=12))
        for writer in (dict(source=30,task=300,logical=12),dict(source=31,task=310,logical=12),dict(source=29,task=29,logical=12),dict(source=40,task=40,logical=12),dict(source=40,task=401,logical=12)):
            with self.assertRaises(ValueError):h.verify_writer(writer,roster)
        with self.assertRaises(ValueError):
            h.verify_boot_roster(ROOT,text.replace('[boot.history.participant] index=7 target=29 task=290',
                                                  '[boot.history.participant] index=7 target=29 task=29'))

        final_line=next(line for line in text.splitlines(keepends=True) if line.startswith('[boot.history.participant] index=8 '))
        for bad in (text.replace(final_line,''),text.replace(final_line,final_line+final_line),
                    text.replace('participants=9','participants=8'),
                    text.replace('index=8 target=40 task=400','index=8 target=40 task=40'),
                    text.replace('index=8 target=40 task=400',f'index=8 target=40 task={(1<<32)+400}'),
                    text.replace('index=8 target=40','index=8 target=41'),
                    text.replace('target=40 task=400 surface=353 logical=12 incarnation=9 predecessor=29',
                                 'target=40 task=400 surface=353 logical=12 incarnation=9 predecessor=28')):
            with self.subTest(bad=bad), self.assertRaises(ValueError):h.verify_boot_roster(ROOT,bad)

    def test_two_three_and_eight_explicit_rosters_with_unequal_tasks_join_writer(self):
        eight=ROWS[:2]+[(2,24,313,314,130,131,132,133),(3,25,318,319,134,135,136,137),
                       (4,26,323,324,138,139,140,141),(5,27,328,329,142,143,144,145),
                       (6,28,333,334,146,147,148,149),(7,29,338,339,150,151,152,153)]
        for rows in (ROWS[:2], ROWS, eight):
            self.path.write_text(source(rows))
            roster = self.check(trace(rows))
            self.assertEqual(roster["participants"], [{"target": row[1], "task": row[1]*10} for row in rows])
            self.assertEqual(roster["count"], len(rows))
            self.assertEqual(h.verify_writer(dict(source=rows[-1][1], task=rows[-1][1]*10, logical=12), roster),
                             dict(target=rows[-1][1], task=rows[-1][1]*10, logical=12))

    def test_atomic_record_after_interrupted_print_is_not_dropped(self):
        text = trace(self.rows).replace("[boot.history.participant] index=1", "[old.print] partial=[boot.history.participant] index=1")
        self.assertEqual(self.check(text)["participants"][-1], dict(target=17, task=170))

    def test_header_exact_and_no_legacy_fallback(self):
        original = trace(self.rows)
        for old, new in [("holder=11 slot=53", "holder=12 slot=53"), ("slot=53", "slot=54"),
                         ("logical=12 participants", "logical=17 participants"), ("participants=2", "participants=1"),
                         ("participants=2", "participants=3"), ("version=1", "version=0"),
                         ("base=1808", "base=1809"), ("sectors=18", "sectors=19"),
                         ("held_query_verified=1", "held_query_verified=0"),
                         ("held_query_verified=1 ", ""), ("runtime_mint=0", "runtime_mint=1")]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                self.check(original.replace(old, new, 1))
        with self.assertRaises(ValueError):
            self.check(original.replace("participants=2 version=1", "original=12 task=120 successor=17 successor_task=170"))

    def test_participant_missing_duplicate_unknown_reordered_and_wide(self):
        original = trace(self.rows)
        line = original.splitlines(keepends=True)[2]
        bad = [original.replace(line, ""), original + line, original.replace(line, line + line),
               original.replace("index=1 target=17", "index=1 target=24"),
               original.replace("index=1 target=17", "index=0 target=17"),
               original.replace("index=1 target=17 task=170", "index=1 target=17 task=17"),
               original.replace("index=1 target=17 task=170", f"index=1 target=17 task={(1<<32)+170}"),
               original.replace("index=1 target=17", f"index=1 target={(1<<32)+17}"),
               original.replace("immutable=1", "immutable=0", 1)]
        lines = original.splitlines(keepends=True)
        lines[1], lines[2] = lines[2], lines[1]
        bad.append("".join(lines))
        for text in bad:
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.check(text)

    def test_each_catalog_source_field_and_task_checked(self):
        original = trace(self.rows)
        line = next(line for line in original.splitlines(keepends=True) if line.startswith("[boot.app.catalog] target=17"))
        for field, value in [("target", 24), ("task", 120), ("task", 1<<32), ("surface", 999), ("logical", 17),
                             ("incarnation", 1), ("predecessor", 9), ("lifecycle", 39), ("close", 42),
                             ("route", 14), ("dormant", 0), ("grants_verified", 0), ("data_only", 0)]:
            import re
            changed = re.sub(rf"\b{field}=\d+", f"{field}={value}", line)
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.check(original.replace(line, changed))
        for changed in ("", line+line):
            with self.assertRaises(ValueError): self.check(original.replace(line, changed))

    def test_catalog_seal_handoff_order_and_uniqueness(self):
        lines = trace(self.rows).splitlines(keepends=True)
        for index in (0, 1, 2, 3, 4, 5, 6):
            moved = lines.copy()
            moved.append(moved.pop(index))
            with self.subTest(index=index), self.assertRaises(ValueError): self.check("".join(moved))
        for index in (0, 5, 6):
            with self.assertRaises(ValueError): self.check("".join(lines + [lines[index]]))
        with self.assertRaises(ValueError): self.check("".join(lines[:-1]))

    def test_malformed_truncated_and_suffix_records_never_silently_skip(self):
        original = trace(self.rows)
        line = original.splitlines(keepends=True)[2]
        for changed in (line.rstrip(), line.rstrip()+" extra=1\n", line.replace("task=170", "task=-1"),
                        line.replace("task=170", "task=no"), line.replace("task=170", f"task={1<<64}")):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                self.check(original + changed)
        with self.assertRaises(ValueError): self.check(original.replace(line, line.rstrip()+" bogus\n"))

    def test_source_order_duplicates_nonliteral_and_zero_rejected(self):
        base = source(self.rows)
        for bad in (base.replace("ordinal: 1", "ordinal: 2"), base.replace("target: 17", "target: 12"),
                    base.replace("target: 17", "target: 0"), base.replace("target: 17", "target: NEXT"),
                    base.replace("surface: 302", "surface: 154"), base.replace("Some(306)", "Some(153)"),
                    base.replace("lifecycle_slot: 50", "lifecycle_slot: 39")):
            self.path.write_text(bad)
            with self.assertRaises(ValueError): h.source_spindles(self.root)

    def test_writer_unknown_wrong_task_logical_and_types_fail(self):
        roster = self.check(trace(self.rows))
        for meta in (dict(source=24, task=240), dict(source=17, task=17), dict(source=12, task=170),
                     dict(source=17, task=170, logical=17), dict(source="17", task=170),
                     dict(source=17, task=True), dict(source=17), dict(source=17, task=(1<<32)+170)):
            with self.subTest(meta=meta), self.assertRaises(ValueError): h.verify_writer(meta, roster)


if __name__ == "__main__":
    unittest.main()
