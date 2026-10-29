import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("memory_log", Path(__file__).resolve().parents[1] / "check_memory_authority_log.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def good():
    rows = [(7, 32, 1, 262144, 15, 0), (9, 32, 2, 262144, 15, 0), (2, 32, 3, 524288, 15, 0),
            (5, 17, 4, 4096, 7, 2), (2, 17, 4, 4096, 7, 5),
            (2, 33, 5, 3145728, 7, 1), (1, 33, 5, 3145728, 3, 2)]
    return "\n".join([f"[boot.memory.region] pd={p} slot={s} region={r} bytes={b} rights={q} peer={x}"
                      for p, s, r, b, q, x in rows] + [
        "[memory.authority.negative.selftest] denied=15 idempotent=1024 output_zero=1 ok=1",
        "[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0",
        "task.running id=1", "[sexusb.memory.dma.ready] region_id=1 length=262144",
        "[sexdrive.dma.region] slot=32 bytes=524288 reusable_buffers=2",
        "[sexfiles.bridge.diskfs.buf.ready] buf_va=0x1000 region_id=4 length=4096 peer_pd=2",
        "[sexfiles.memory.authority.negative] empty=1 wrong_kind=1 dma_right=1 ok=1",
        "[runtime.heap.ready] backing=elf_owned bytes=65536 runtime_mapping=0"])


class MemoryLogTests(unittest.TestCase):
    def test_complete(self):
        self.assertFalse(checker.validate(good()))

    def test_empty_and_missing_evidence(self):
        self.assertTrue(checker.validate(""))
        for marker in ("boot.memory.region", "authority.sealed", "negative.selftest", "dma.ready", "authority.negative"):
            self.assertTrue(checker.validate(good().replace(marker, "omitted")))

    def test_read_only_frame_and_budget(self):
        for before, after in (("rights=3", "rights=7"), ("bytes=524288", "bytes=1048576"), ("rights=15", "rights=7")):
            self.assertTrue(checker.validate(good().replace(before, after)))

    def test_peer_and_duplicate_slot(self):
        self.assertTrue(checker.validate(good().replace("peer=5", "peer=7")))
        self.assertTrue(checker.validate(good() + "\n" + good().splitlines()[0]))
        self.assertTrue(checker.validate(good().replace("peer_pd=2", "peer_pd=999")))

    def test_frame_rights_belong_to_drive_producer(self):
        swapped = good().replace("region=5 bytes=3145728 rights=7", "region=5 bytes=3145728 rights=TMP")
        swapped = swapped.replace("region=5 bytes=3145728 rights=3", "region=5 bytes=3145728 rights=7")
        self.assertTrue(checker.validate(swapped.replace("rights=TMP", "rights=3")))

    def test_late_genesis(self):
        rows = good().splitlines()
        rows.append(rows.pop(0))
        self.assertTrue(checker.validate("\n".join(rows)))

    def test_denial_zero_output_and_quarantine(self):
        self.assertTrue(checker.validate(good().replace("output_zero=1", "output_zero=0")))
        self.assertTrue(checker.validate(good() + "\n[sexdrive.dma.quarantine] ready=0"))


if __name__ == "__main__":
    unittest.main()
