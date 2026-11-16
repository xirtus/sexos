"""QEMU placeholder-size WAV capture is parsed without altering its PCM artifact."""
import io
from pathlib import Path
import sys
import tempfile
import unittest
import wave
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from moan_gate import open_recording

class RecordingTests(unittest.TestCase):
    def capture(self,payload=b'\1\0\2\0'*8):
        out=io.BytesIO()
        with wave.open(out,'wb') as wav:
            wav.setnchannels(2);wav.setsampwidth(2);wav.setframerate(48000);wav.writeframes(payload)
        return bytearray(out.getvalue())
    def read(self,data):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'recording.wav';path.write_bytes(data)
            with open_recording(path) as wav:
                result=(wav.getnchannels(),wav.getframerate(),wav.readframes(wav.getnframes()))
            self.assertEqual(path.read_bytes(),data,'recording evidence must remain unchanged')
            return result
    def test_finalized_and_qemu_placeholder_recordings_preserve_exact_samples(self):
        data=self.capture();expected=bytes(data[44:]);self.assertEqual(self.read(data),(2,48000,expected))
        data[4:8]=data[40:44]=b'\0'*4
        self.assertEqual(self.read(data),(2,48000,expected))
    def test_placeholder_rejects_wrong_format_and_partial_pcm_frame(self):
        data=self.capture();data[4:8]=data[40:44]=b'\0'*4
        wrong=data.copy();wrong[22:24]=b'\1\0'
        with self.assertRaises(wave.Error):self.read(wrong)
        with self.assertRaises(wave.Error):self.read(data+b'\0')
        with self.assertRaises(wave.Error):self.read(data[:44])
    def test_placeholder_capture_ceiling(self):
        data=self.capture();data[4:8]=data[40:44]=b'\0'*4
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'oversized.wav'
            with path.open('wb') as file:
                file.write(data[:44]);file.truncate(44+16*1024*1024+4)
            with self.assertRaises(wave.Error):open_recording(path)

if __name__=='__main__':unittest.main()
