"""Run with python3 -m unittest discover -s crates/sex-voice/adapters -p 'test_*.py'."""

import io
import json
import struct
import unittest
from unittest.mock import patch

import mosslanding as bridge


def submit(job_id=11, seed=0, voice_id=0, reference_id=0, model="active", text="Hello"):
    language = b"en"
    model = model.encode()
    text = text.encode()
    return (bridge.header(1, job_id) + struct.pack("<B3xQQQIBBH", 2, seed, voice_id,
                                                  reference_id, 8000, len(language), len(model), len(text))
            + language + model + text)


class VoiceAdapterTest(unittest.TestCase):
    def setUp(self):
        self.adapter = bridge.Adapter(fixture=True)

    def tearDown(self):
        self.adapter.close()

    def test_fixture_result_and_audio(self):
        queued = self.adapter.handle(submit(seed=42, model="fixture-v1"))
        self.assertEqual(queued[5], 4)
        self.assertEqual(queued[16], 1)
        running = self.adapter.handle(bridge.header(3, 11))
        self.assertEqual(running[16], 2)
        complete = self.adapter.handle(bridge.header(3, 11))
        self.assertEqual(complete[5], 5)
        self.assertEqual(struct.unpack_from("<Q", complete, 16)[0], 11)
        self.assertEqual(self.adapter.asset(11), bridge.FIXTURE_WAV)
        self.assertEqual(complete[40:72], bridge.hashlib.sha256(bridge.FIXTURE_WAV).digest())

    def test_cancel_never_publishes_asset(self):
        self.adapter.handle(submit(job_id=12))
        cancelled = self.adapter.handle(bridge.header(2, 12))
        self.assertEqual(cancelled[16], 4)
        self.assertEqual(self.adapter.handle(bridge.header(3, 12))[16], 4)
        self.assertIsNone(self.adapter.asset(12))

    def test_rejects_malformed_and_duplicate(self):
        frame = bytearray(submit())
        frame[4] = 99
        with self.assertRaises(bridge.ProtocolError):
            self.adapter.handle(bytes(frame))
        frame[4] = 1
        with self.assertRaises(bridge.ProtocolError):
            self.adapter.handle(bytes(frame) + b"trailing")
        self.adapter.handle(bytes(frame))
        with self.assertRaises(bridge.ProtocolError):
            self.adapter.handle(bytes(frame))

    def test_real_backend_mapping_without_model(self):
        class DeferredPool:
            def submit(self, callback, *args):
                self.callback = (callback, args)
            def shutdown(self, wait):
                pass

        real = bridge.Adapter(fixture=False, voices={8: "voice.wav"})
        real.pool.shutdown(wait=True)
        real.pool = DeferredPool()
        self.addCleanup(real.close)
        queued = real.handle(submit(job_id=20, voice_id=8))
        self.assertEqual(queued[16], 1)
        requests = []

        def fake_urlopen(request, timeout):
            url = request.full_url if hasattr(request, "full_url") else request
            if url.endswith("/api/synthesize"):
                requests.append(json.loads(request.data))
                return io.BytesIO(bridge.FIXTURE_WAV)
            return io.BytesIO(json.dumps({"model_id": "MOSS-TTS-v1.5"}).encode())

        with patch.object(bridge.urllib.request, "urlopen", side_effect=fake_urlopen):
            callback, args = real.pool.callback
            callback(*args)
        answer = real.handle(bridge.header(3, 20))
        self.assertEqual(answer[5], 5)
        self.assertEqual(real.asset(20), bridge.FIXTURE_WAV)
        self.assertEqual(requests[0]["voice"], "voice.wav")
        self.assertEqual(requests[0]["mode"], "clone")


if __name__ == "__main__":
    unittest.main()
