"""Independent PCM/archive oracle rejects bypass even with valid checksum."""
from pathlib import Path
import struct
import sys
import unittest
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from moan_offline_ui_gate import FRAMES, expected_pcm, project_oracle


def seal(data):
    struct.pack_into('<Q', data, 16, len(data))
    checksum = 0xcbf29ce484222325
    for byte in data[:24] + data[32:]:
        checksum = ((checksum ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    struct.pack_into('<Q', data, 24, checksum)
    return bytes(data)


def fixture(channels=(False, True, False, False), version=1):
    data = bytearray(64)
    data[:8] = b'MOANPRJ\0'
    struct.pack_into('<HHI', data, 8, version, 0, 48000)
    struct.pack_into('<HHHH', data, 32, 4, 4, 3, 100 if version == 3 else 0)
    struct.pack_into('<QQQ', data, 40, 0, 0, FRAMES)
    for swapped in channels:
        data.extend(struct.pack('<I', FRAMES) + expected_pcm(swapped))
        if version >= 2:
            data.extend(bytes(4))
    for source in range(4):
        data.extend(struct.pack('<HHhBB', 1, 100, 0, 0, 0) + bytes(22))
        data.extend(struct.pack('<HHQIIHHII', source, 0, 0, 0, FRAMES, 100, 0, 0, 0))
    return seal(data)


class OfflineArchive(unittest.TestCase):
    def test_actual_archive_versions_and_non_bypass(self):
        for version in (1, 2, 3):
            self.assertTrue(project_oracle(fixture(version=version))['clap_non_bypass_swap'])

    def test_bypass_changed_original_and_dropped_channel_refused(self):
        for channels in ((False, False, False, False), (True, True, False, False), (False, True, True, False)):
            with self.assertRaises(AssertionError):
                project_oracle(fixture(channels))
        data = bytearray(fixture())
        data[68:70] = data[70:72]
        with self.assertRaises(AssertionError):
            project_oracle(seal(data))

    def test_wrong_track_mapping_and_mutation_refused(self):
        data = bytearray(fixture())
        data[-32:-30] = struct.pack('<H', 0)
        with self.assertRaises(AssertionError):
            project_oracle(seal(data))
        data = bytearray(fixture())
        data[100] ^= 1
        with self.assertRaises(AssertionError):
            project_oracle(seal(data))
        with self.assertRaises(AssertionError):
            project_oracle(fixture() + b'junk')


if __name__ == '__main__':
    unittest.main()
