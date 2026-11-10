"""The shared boot helper owns its guest until it returns a stop closure."""
import pathlib,sys,tempfile,unittest
from contextlib import ExitStack
from unittest.mock import Mock,patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import quil_autosave_gate as gate

class BootSetupCleanupTests(unittest.TestCase):
 def run_case(self,stage=None):
  with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
   out=pathlib.Path(directory);proc=Mock();proc.poll.return_value=None
   def kill():proc.poll.return_value=-9
   proc.kill.side_effect=kill;sockets=[]
   def launch(args,stdout,stderr):
    # Preserve diagnostic files just as a real failed guest would.
    stdout.write('diagnostic QEMU output\n');stdout.flush()
    serial=pathlib.Path(args[args.index('-serial')+1].removeprefix('file:'))
    serial.write_text('diagnostic serial output\n')
    sock=pathlib.Path(args[args.index('-qmp')+1].split(',')[0].removeprefix('unix:'))
    sockets.append(sock)
    if stage!='socket':sock.touch()
    return proc
   stack.enter_context(patch.object(gate.subprocess,'Popen',side_effect=launch))
   qmp=stack.enter_context(patch.object(gate,'QMP'))
   wait=stack.enter_context(patch.object(gate,'wait_serial',return_value=Mock()))
   if stage=='qmp':qmp.side_effect=RuntimeError('QMP handshake failed')
   if stage=='ready':wait.side_effect=TimeoutError('ready receipt absent')
   if stage=='interrupt':wait.side_effect=KeyboardInterrupt()
   if stage=='socket':stack.enter_context(patch.object(gate.time,'monotonic',side_effect=[0,0,0,16]))
   try:
    if stage:
     error=KeyboardInterrupt if stage=='interrupt' else RuntimeError if stage=='qmp' else TimeoutError
     with self.assertRaises(error):gate.boot(out/'image.iso',out/'disk.img',out,'setup','1280x800')
     proc.kill.assert_called_once();proc.wait.assert_called_once();self.assertFalse(sockets[0].exists())
    else:
     guest=gate.boot(out/'image.iso',out/'disk.img',out,'setup','1280x800')
     proc.kill.assert_not_called();proc.wait.assert_not_called();self.assertTrue(sockets[0].exists())
     self.assertIs(guest['proc'],proc);guest['stop']();guest['stop']()
     proc.kill.assert_called_once();proc.wait.assert_called_once();self.assertFalse(sockets[0].exists())
    self.assertEqual((out/'setup-serial.log').read_text(),'diagnostic serial output\n')
    self.assertEqual((out/'setup-qemu.log').read_text(),'diagnostic QEMU output\n')
   finally:
    for sock in sockets:sock.unlink(missing_ok=True)
 def test_qmp_socket_timeout_cleans_guest(self):self.run_case('socket')
 def test_qmp_handshake_failure_cleans_guest(self):self.run_case('qmp')
 def test_readiness_failure_cleans_guest(self):self.run_case('ready')
 def test_interrupt_during_setup_cleans_guest(self):self.run_case('interrupt')
 def test_success_transfers_cleanup_to_caller(self):self.run_case()

if __name__=='__main__':unittest.main()
