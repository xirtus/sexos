#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""
Test suite for ACE-Step 1.5 Bridge & Mock Server.
"""

import io
import json
import os
import re
import socket
import subprocess
import sys
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import wave

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(os.path.dirname(SCRIPT_DIR))
BRIDGE_SCRIPT = os.path.join(PROJECT_ROOT, "scripts", "start_moan_ace_bridge.sh")
MOCK_SERVER = os.path.join(PROJECT_ROOT, "scripts", "mock_ace_server.py")


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class TestMoanAceBridge(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        self_check = subprocess.run(["chmod", "+x", BRIDGE_SCRIPT, MOCK_SERVER])
        assert self_check.returncode == 0

    def test_status_human_and_json(self):
        # 1. Test human output
        res_human = subprocess.run(
            [BRIDGE_SCRIPT, "--status"],
            capture_output=True,
            text=True,
            check=True,
        )
        out_human = res_human.stdout
        self.assertIn("OS:", out_human)
        self.assertIn("Arch:", out_human)
        self.assertIn("RAM (MB):", out_human)
        self.assertIn("GPU Backend:", out_human)
        self.assertIn("Recommended Tier:", out_human)
        self.assertIn("Model Directory:", out_human)
        self.assertIn("Models Exist:", out_human)

        # 2. Test JSON output
        res_json = subprocess.run(
            [BRIDGE_SCRIPT, "--status", "--json"],
            capture_output=True,
            text=True,
            check=True,
        )
        status_data = json.loads(res_json.stdout)
        self.assertIn("os", status_data)
        self.assertIn("arch", status_data)
        self.assertIn("ram_mb", status_data)
        self.assertIn("gpu_backend", status_data)
        self.assertIn("recommended_tier", status_data)
        self.assertIn("model_dir", status_data)
        self.assertIn("models_exist", status_data)
        self.assertIsInstance(status_data["ram_mb"], int)
        self.assertGreater(status_data["ram_mb"], 0)
        self.assertIn(
            status_data["recommended_tier"],
            ["tier0", "tier2-lite", "tier3-studio"],
        )
        self.assertIn(
            status_data["gpu_backend"],
            ["metal", "mlx", "cuda", "rocm", "cpu"],
        )

    def test_ram_tier_recommendations(self):
        # Simulated low RAM (<8GB) -> tier0
        env_tier0 = os.environ.copy()
        env_tier0["MOAN_SIMULATED_RAM_MB"] = "4096"
        res = subprocess.run(
            [BRIDGE_SCRIPT, "--status", "--json"],
            capture_output=True,
            text=True,
            env=env_tier0,
            check=True,
        )
        data = json.loads(res.stdout)
        self.assertEqual(data["ram_mb"], 4096)
        self.assertEqual(data["recommended_tier"], "tier0")

        # Simulated mid RAM (16GB) -> tier2-lite
        env_tier2 = os.environ.copy()
        env_tier2["MOAN_SIMULATED_RAM_MB"] = "16384"
        res = subprocess.run(
            [BRIDGE_SCRIPT, "--status", "--json"],
            capture_output=True,
            text=True,
            env=env_tier2,
            check=True,
        )
        data = json.loads(res.stdout)
        self.assertEqual(data["ram_mb"], 16384)
        self.assertEqual(data["recommended_tier"], "tier2-lite")

        # Simulated high RAM (64GB) -> tier3-studio
        env_tier3 = os.environ.copy()
        env_tier3["MOAN_SIMULATED_RAM_MB"] = "65536"
        res = subprocess.run(
            [BRIDGE_SCRIPT, "--status", "--json"],
            capture_output=True,
            text=True,
            env=env_tier3,
            check=True,
        )
        data = json.loads(res.stdout)
        self.assertEqual(data["ram_mb"], 65536)
        self.assertEqual(data["recommended_tier"], "tier3-studio")

    def test_download_lite_dry_run(self):
        res = subprocess.run(
            [BRIDGE_SCRIPT, "--download", "lite", "--dry-run"],
            capture_output=True,
            text=True,
            check=True,
        )
        manifest = json.loads(res.stdout)
        self.assertEqual(manifest["pack"], "lite")
        filenames = [f["name"] for f in manifest["files"]]
        self.assertIn("Qwen3-Embedding-0.6B-Q8_0.gguf", filenames)
        self.assertIn("acestep-v15-turbo-Q4_K_M.gguf", filenames)
        self.assertIn("vae-BF16.gguf", filenames)

        # Check total size is ~2.4GB (between 2.0GB and 2.8GB)
        total_size = manifest["total_bytes"]
        self.assertGreater(total_size, 2.0 * (1024**3))
        self.assertLess(total_size, 2.8 * (1024**3))

        # Check URLs and SHA256 presence
        for f in manifest["files"]:
            self.assertTrue(f["url"].startswith("https://huggingface.co/"))
            self.assertEqual(len(f["sha256"]), 64)
            self.assertIn(f["url"], manifest["urls"])

    def test_download_studio_dry_run(self):
        res = subprocess.run(
            [BRIDGE_SCRIPT, "--download", "studio", "--dry-run"],
            capture_output=True,
            text=True,
            check=True,
        )
        manifest = json.loads(res.stdout)
        self.assertEqual(manifest["pack"], "studio")
        filenames = [f["name"] for f in manifest["files"]]
        self.assertIn("acestep-5Hz-lm-4B-Q8_0.gguf", filenames)
        self.assertIn("acestep-v15-xl-turbo-Q8_0.gguf", filenames)

        # Check total size is ~7.7GB (between 7.0GB and 8.5GB)
        total_size = manifest["total_bytes"]
        self.assertGreater(total_size, 7.0 * (1024**3))
        self.assertLess(total_size, 8.5 * (1024**3))

        for f in manifest["files"]:
            self.assertTrue(f["url"].startswith("https://huggingface.co/"))
            self.assertEqual(len(f["sha256"]), 64)
            self.assertIn(f["url"], manifest["urls"])

    def test_mock_ace_server_lifecycle_and_endpoints(self):
        """Spins up mock_ace_server.py on an ephemeral port (0), exercises all endpoints,

        and verifies the returned RIFF WAV is 48 kHz stereo PCM.
        """
        proc = subprocess.Popen(
            [sys.executable, MOCK_SERVER, "--port", "0"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        port = None
        start_time = time.time()
        try:
            # Read port from stdout
            while time.time() - start_time < 5.0:
                line = proc.stdout.readline()
                if not line:
                    time.sleep(0.05)
                    continue
                match = re.search(r"MOCK_ACE_SERVER_PORT=(\d+)", line)
                if match:
                    port = int(match.group(1))
                    break

            self.assertIsNotNone(port, "Failed to capture ephemeral port from mock server")
            base_url = f"http://127.0.0.1:{port}"

            # 1. GET /props
            req_props = urllib.request.Request(f"{base_url}/props")
            with urllib.request.urlopen(req_props, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                props_data = json.loads(resp.read().decode("utf-8"))
                self.assertEqual(props_data.get("status"), "ready")
                self.assertEqual(props_data.get("sample_rate"), 48000)
                self.assertEqual(props_data.get("channels"), 2)
                self.assertIn("synth", props_data.get("capabilities", []))

            # 2. POST /lm
            lm_payload = json.dumps({
                "prompt": "melodic techno groove",
                "bpm": 128,
                "key": "D minor",
                "duration": 15.0,
            }).encode("utf-8")
            req_lm = urllib.request.Request(
                f"{base_url}/lm",
                data=lm_payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_lm, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                lm_data = json.loads(resp.read().decode("utf-8"))
                self.assertIsInstance(lm_data, list)
                self.assertGreater(len(lm_data), 0)
                item = lm_data[0]
                self.assertEqual(item.get("bpm"), 128)
                self.assertEqual(item.get("key"), "D minor")
                self.assertIn("lyrics", item)
                self.assertTrue(item["lyrics"].startswith("["))

            # 3. POST /synth?wav=1 -> 48 kHz stereo WAV verification
            synth_payload = json.dumps({
                "duration": 0.5,
                "bpm": 128,
                "seed": 101,
            }).encode("utf-8")
            req_synth = urllib.request.Request(
                f"{base_url}/synth?wav=1",
                data=synth_payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_synth, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(resp.headers.get("Content-Type"), "audio/wav")
                wav_bytes = resp.read()

                # Validate WAV format strictly
                with wave.open(io.BytesIO(wav_bytes), "rb") as wf:
                    self.assertEqual(wf.getnchannels(), 2, "Must be 2-channel stereo")
                    self.assertEqual(wf.getframerate(), 48000, "Must be 48,000 Hz")
                    self.assertEqual(wf.getsampwidth(), 2, "Must be 16-bit PCM (2 bytes/sample)")
                    self.assertGreater(wf.getnframes(), 0)

            # 4. POST /understand
            req_understand = urllib.request.Request(
                f"{base_url}/understand",
                data=b"{}",
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_understand, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                und_data = json.loads(resp.read().decode("utf-8"))
                self.assertIn("bpm", und_data)
                self.assertIn("key", und_data)
                self.assertIn("caption", und_data)
                self.assertIn("lyrics", und_data)

            # 5. GET /health (acestep-api)
            req_health = urllib.request.Request(f"{base_url}/health")
            with urllib.request.urlopen(req_health, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                health_data = json.loads(resp.read().decode("utf-8"))
                self.assertEqual(health_data.get("code"), 200)

            # 6. POST /release_task (acestep-api)
            rel_payload = json.dumps({"prompt": "cinematic strings", "duration": 0.5}).encode("utf-8")
            req_rel = urllib.request.Request(
                f"{base_url}/release_task",
                data=rel_payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_rel, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                rel_data = json.loads(resp.read().decode("utf-8"))
                self.assertEqual(rel_data.get("code"), 200)
                task_id = rel_data.get("data", {}).get("task_id")
                self.assertIsNotNone(task_id)

            # 7. POST /query_result (acestep-api)
            query_payload = json.dumps({"task_id": task_id}).encode("utf-8")
            req_query = urllib.request.Request(
                f"{base_url}/query_result",
                data=query_payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_query, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                q_data = json.loads(resp.read().decode("utf-8"))
                self.assertEqual(q_data.get("code"), 200)
                results = q_data.get("data", [])
                self.assertGreater(len(results), 0)
                res_item = results[0]
                self.assertEqual(res_item.get("task_id"), task_id)
                self.assertEqual(res_item.get("status"), 1)
                audio_url = res_item.get("audio_url")
                self.assertTrue(audio_url.startswith("/v1/audio"))

            # 8. GET /v1/audio (acestep-api audio retrieval)
            req_audio = urllib.request.Request(f"{base_url}{audio_url}")
            with urllib.request.urlopen(req_audio, timeout=3.0) as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(resp.headers.get("Content-Type"), "audio/wav")
                retrieved_wav = resp.read()

                with wave.open(io.BytesIO(retrieved_wav), "rb") as wf:
                    self.assertEqual(wf.getnchannels(), 2, "Must be 2-channel stereo")
                    self.assertEqual(wf.getframerate(), 48000, "Must be 48,000 Hz")
                    self.assertEqual(wf.getsampwidth(), 2, "Must be 16-bit PCM")
                    self.assertGreater(wf.getnframes(), 0)

        finally:
            proc.terminate()
            try:
                proc.wait(timeout=3.0)
            except Exception:
                proc.kill()
            if proc.stdout:
                proc.stdout.close()
            if proc.stderr:
                proc.stderr.close()

    def test_serve_mode_mock_via_bridge(self):
        """Test starting mock server via start_moan_ace_bridge.sh --serve --mode mock."""
        free_port = find_free_port()
        proc = subprocess.Popen(
            [BRIDGE_SCRIPT, "--serve", "--mode", "mock", "--port", str(free_port)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        base_url = f"http://127.0.0.1:{free_port}"
        started = False
        start_time = time.time()
        try:
            while time.time() - start_time < 5.0:
                try:
                    with urllib.request.urlopen(f"{base_url}/health", timeout=1.0) as resp:
                        if resp.status == 200:
                            started = True
                            break
                except Exception:
                    time.sleep(0.1)

            self.assertTrue(started, "Bridge --serve --mode mock failed to start and respond to /health")
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=3.0)
            except Exception:
                proc.kill()
            if proc.stdout:
                proc.stdout.close()
            if proc.stderr:
                proc.stderr.close()


if __name__ == "__main__":
    unittest.main()
