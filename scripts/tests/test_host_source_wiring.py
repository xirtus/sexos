"""Exercise the actual shell-gate validators against source mutations."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class HostSourceWiring(unittest.TestCase):
    def validate(self, gate, transform=lambda value: value):
        script = (ROOT / 'scripts' / gate).read_text()
        marker = "<<'PYCODE'" if 'linen' in gate else "<<'PY'"
        end = '\nPYCODE\n' if 'linen' in gate else '\nPY\n'
        code = script.split(marker, 1)[1].split(end, 1)[0].lstrip('\n')
        with tempfile.TemporaryDirectory() as name:
            root = pathlib.Path(name)
            for relative in ('sexos_contract.toml', 'crates/sex-pdx/src/display_ready.rs',
                             'servers/sexdisplay/src/main.rs', 'servers/silk-shell/src/main.rs',
                             'servers/silk-shell/src/linen_async.rs', 'servers/silkbar/src/main.rs',
                             'kernel/src/init.rs'):
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                source = (ROOT / relative).read_text()
                target.write_text(transform(source) if relative == 'servers/silk-shell/src/main.rs' else source)
            return subprocess.run(['python3', '-c', code, str(root), str(root / 'production.rs')],
                                  cwd=root, capture_output=True, text=True).returncode

    def test_actual_validators_accept_original_and_broker_mutable_receive(self):
        for gate in ('display_ready_protocol_gate.sh', 'linen_text_delivery_gate.sh'):
            self.assertEqual(self.validate(gate), 0)
            self.assertEqual(self.validate(gate, lambda s: s.replace('let mut msg = match pop_startup_message()', 'let msg = match pop_startup_message()').replace('clipboard_poll(); autofill_routes::poll(); app_input_poll();', 'clipboard_poll(); app_input_poll();')), 0)

    def test_readiness_and_fifo_order_cannot_disappear_or_move(self):
        ready = '(*core::ptr::addr_of_mut!(STARTUP_INBOX)).wait_for_display()'
        mutations = (
            lambda s: s.replace(ready, 'missing_ready()'),
            lambda s: s.replace('sex_rt::heap_init();', '').replace(ready, 'sex_rt::heap_init(); ' + ready),
            lambda s: s.replace('let mut msg = match pop_startup_message()', 'let unrelated = match pop_startup_message()'),
        )
        for mutate in mutations:
            self.assertNotEqual(self.validate('display_ready_protocol_gate.sh', mutate), 0)

    def test_every_original_poll_and_order_remain_required(self):
        calls = ('shell_registration_reply_poll', 'clipboard_intake_poll', 'clipboard_poll',
                 'app_input_poll', 'desktop_state_poll', 'launcher_focus_poll', 'spindle_view_poll',
                 'document_view_poll', 'palette_visibility_poll', 'palette_text_poll',
                 'linen_text_poll', 'linen_async_poll')
        for call in calls:
            with self.subTest(call=call):
                self.assertNotEqual(self.validate('linen_text_delivery_gate.sh', lambda s: s.replace(call + '();', '')), 0)
        for replacement in ('app_input_poll(); autofill_routes::poll(); clipboard_poll();',
                            'clipboard_poll(); unrelated(); app_input_poll();',
                            'clipboard_poll(); if false { autofill_routes::poll(); } app_input_poll();'):
            self.assertNotEqual(self.validate('linen_text_delivery_gate.sh', lambda s: s.replace('clipboard_poll(); autofill_routes::poll(); app_input_poll();', replacement)), 0)


if __name__ == '__main__':
    unittest.main()
