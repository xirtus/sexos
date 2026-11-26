"""Input producers keep command contents out of normal serial diagnostics."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class TerminalPrivateInput(unittest.TestCase):
    def test_native_terminal_receipts_do_not_format_typed_text(self):
        source = (ROOT / 'apps/spindle/src/main.rs').read_text()
        for receipt in ('spindle.line.append', 'spindle.text.append',
                        'spindle.input.recv', 'spindle.line.enter', 'spindle.line.edit.ok',
                        'spindle.input.progress', 'spindle.probe.input', 'spindle.key.recv',
                        'spindle.input.echo.ok'):
            lines = [line for line in source.splitlines() if f'[{receipt}]' in line]
            self.assertTrue(lines, receipt)
            self.assertFalse(any(re.search(r' ch=| text=| key=\{| code=\{|scancode=\{|down=\{', line) for line in lines), receipt)
        self.assertIn('!private_launch{let idx=hist.push', source)
        self.assertIn('command_receipt_name(cmd_name)', source)
        self.assertIn('line.clear_private();', source)
        self.assertIn('self.prev_buf.fill(0);', source)
        self.assertIn('self.nav_saved.fill(0);', source)

    def test_shell_yarn_receipts_and_saved_state_exclude_private_input(self):
        source = (ROOT / 'servers/silk-shell/src/main.rs').read_text()
        for receipt in ('spindle.key.char', 'spindle.line.edit',
                        'spindle.command.echo', 'spindle.structured.dispatch',
                        'silk-shell.main_dispatch.spindle.reach'):
            lines = [line for line in source.splitlines() if f'[{receipt}]' in line]
            self.assertTrue(lines, receipt)
            self.assertFalse(any(re.search(r' ch=\{| args=\{| cmd=\{|scancode=\{', line) for line in lines), receipt)
        self.assertIn('if !private_launch { yarn_append_output', source)
        self.assertIn('SPINDLE_VI_PREV_BUF = [0u8; YARN_CMD_BUF_CAP]', source)
        self.assertIn('is_browser_launch_candidate(&yarn.cmd_buf[..yarn.cmd_len])', source)

    def test_old_history_snapshots_drop_private_launches_on_load(self):
        source = (ROOT / 'apps/spindle/src/history_persistence.rs').read_text()
        self.assertIn('is_private_browser_launch(entry)', source)
        self.assertIn('if dropped{self.mark_dirty();}', source)


if __name__ == '__main__':
    unittest.main()
