import pathlib
import re
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from desktop_focus import focus_surface, _completed


def receipt(old, new, code=15, action='AccessFocusNext', release=True):
    return (f'[input.keyboard.keydown.ok] code={code} source=ps2\n'
            f'[shell.kbd.ui.action] scancode={code} action={action} focused={old} frame=3 sid={old}\n'
            f'[shell.focus.set] id={new}\n'
            f'[shell.kbd.ui.result] action={action} ok=1 reason=ok frame=3 sid={new}\n'
            + (f'[input.keyboard.keyup.ok] code={code} source=ps2\n' if release else ''))


class FocusTests(unittest.TestCase):
    def test_entering_spindle_during_traversal_uses_global_control(self):
        state = {'sid': 200, 'shown': True, 'trace': '[shell.focus.set] id=200\n'}; keys = []
        def key(code, pattern):
            keys.append(code); old = state['sid']
            if code == 'scroll_lock': state.update(shown=False, sid=200)
            elif old == 200: state['sid'] = 300
            elif old == 300: state['sid'] = 153 if state['shown'] else 201
            else: self.fail('Tab was sent to app-local completion')
            emitted = receipt(old, state['sid'], 70, 'ToggleSpindle') if code == 'scroll_lock' else receipt(old, state['sid'])
            self.assertRegex(emitted, pattern); state['trace'] += emitted
        focus_surface(201, lambda: state['trace'], key)
        self.assertEqual(keys, ['tab', 'tab', 'scroll_lock', 'tab', 'tab'])

    def test_shown_unfocused_spindle_can_be_minimized_then_restored(self):
        state = {'sid': 200, 'shown': True, 'trace': '[shell.focus.set] id=200\n'}; keys = []
        def key(code, pattern):
            self.assertEqual(code, 'scroll_lock'); keys.append(code); old = state['sid']
            state['shown'] = not state['shown']; state['sid'] = 153 if state['shown'] else 200
            state['trace'] += receipt(old, state['sid'], 70, 'ToggleSpindle')
        focus_surface(153, lambda: state['trace'], key)
        self.assertEqual(len(keys), 2)

    def test_current_successor_needs_no_input(self):
        focus_surface(306, lambda: '[shell.focus.set] id=306', lambda *args: self.fail())

    def test_nonprogressing_guest_is_bounded(self):
        stream = '[shell.focus.set] id=200\n'; keys = []
        def key(*args):
            nonlocal stream
            keys.append(args); stream += receipt(200, 200)
        with self.assertRaisesRegex(RuntimeError, 'bounded focus traversal'):
            focus_surface(201, lambda: stream, key)
        self.assertEqual(len(keys), 24)

    def test_pending_needle_autofocus_cannot_ack_queued_tab_or_skip_release(self):
        stream = '[shell.focus.set] id=201\n'; keys = []; release_pending = False
        def text(): return stream
        def key(code, pattern):
            nonlocal stream, release_pending
            self.assertFalse(release_pending, 'next key preceded previous release')
            self.assertEqual(code, 'tab'); keys.append(code)
            if len(keys) == 1:
                # Exact real race: pending app-open focuses Needle before Tab
                # arrives, and that Tab subsequently moves focus back to Quil.
                stream += '[shell.focus.set] id=359\n'
                self.assertIsNone(re.search(pattern, stream.splitlines()[-1]))
                stream += receipt(359, 201, release=False)
            else: stream += receipt(201, 359, release=False)
            release_pending = True
        def release(_):
            nonlocal stream, release_pending
            stream += '[input.keyboard.keyup.ok] code=15 source=ps2\n'; release_pending = False
        with patch('desktop_focus.time.sleep', side_effect=release): focus_surface(359, text, key)
        self.assertEqual(keys, ['tab', 'tab']); self.assertFalse(release_pending)
        self.assertEqual(re.findall(r'\[shell.focus.set\] id=(\d+)', stream)[-1], '359')

    def test_only_generic_focus_or_wrong_make_or_missing_release_cannot_complete(self):
        cases = ['[shell.focus.set] id=359\n', receipt(201, 359, code=14),
                 receipt(201, 359, release=False), receipt(201, 359, action='AccessClose'),
                 receipt(201, 359).replace('scancode=15', 'scancode=14'),
                 receipt(201, 359).replace('ok=1 reason=ok', 'ok=0 reason=denied')]
        for trace in cases:
            with self.subTest(trace=trace): self.assertFalse(_completed(trace, 15, 'AccessFocusNext'))
        self.assertTrue(_completed(receipt(201, 359), 15, 'AccessFocusNext'))

    def test_missing_ack_fails_at_deadline_without_sending_more_keys(self):
        stream = '[shell.focus.set] id=201\n'; keys = []
        def key(*args):
            nonlocal stream
            keys.append(args); stream += '[shell.focus.set] id=359\n'
        with patch('desktop_focus.time.monotonic', side_effect=[0, 6]):
            with self.assertRaisesRegex(RuntimeError, 'missing completed keyboard'):
                focus_surface(359, lambda: stream, key)
        self.assertEqual(len(keys), 1)

    def test_normal_redacted_edges_require_exact_semantic_action(self):
        normal = ('[input.keyboard.keydown.ok] source=ps2 payload=redacted\n'
                  '[shell.kbd.ui.action] payload=redacted\n'
                  '[shell.focus.set] id=359\n'
                  '[shell.kbd.ui.result] action=AccessFocusNext ok=1 reason=ok frame=3 sid=359\n'
                  '[input.keyboard.keyup.ok] source=ps2 payload=redacted\n')
        self.assertTrue(_completed(normal, 15, 'AccessFocusNext'))
        self.assertFalse(_completed(normal, 15, 'AccessFocusNext', proof=True))
        self.assertFalse(_completed(normal.replace('action=AccessFocusNext', 'action=AccessClose'), 15, 'AccessFocusNext'))
        self.assertFalse(_completed(normal.replace('[input.keyboard.keyup.ok]', '[input.keyboard.other]'), 15, 'AccessFocusNext'))

    def test_input_pair_requires_make_before_release_on_same_device(self):
        for raw in (True, False):
            trace = receipt(201, 359)
            if not raw:
                trace = re.sub(r'code=15 source=ps2', 'source=ps2 payload=redacted', trace)
                trace = re.sub(r'\[shell.kbd.ui.action\][^\n]+', '[shell.kbd.ui.action] payload=redacted', trace)
            lines = trace.splitlines(keepends=True)
            with self.subTest(raw=raw):
                self.assertTrue(_completed(trace, 15, 'AccessFocusNext'))
                self.assertFalse(_completed(trace.replace('keyup.ok] '+('code=15 source=ps2' if raw else 'source=ps2'), 'keyup.ok] '+('code=15 source=usb' if raw else 'source=usb')), 15, 'AccessFocusNext'))
                self.assertFalse(_completed(lines[-1]+''.join(lines[1:-1])+lines[0], 15, 'AccessFocusNext'))
                # Same Input order remains valid when Shell handles it later.
                self.assertTrue(_completed(lines[0]+lines[-1]+''.join(lines[1:-1]), 15, 'AccessFocusNext'))

    def test_explicit_alt_tab_path_requires_fresh_make_and_release(self):
        trace = ('[input.keyboard.keydown.ok] code=15 source=ps2\n'
                 '[shell.app.switch] old=300 new=201 live=1 shortcut=alt_tab\n'
                 '[input.keyboard.keyup.ok] code=15 source=ps2\n')
        self.assertTrue(_completed(trace, 15, 'AccessFocusNext'))
        self.assertFalse(_completed(trace.replace('shortcut=alt_tab', 'shortcut=other'), 15, 'AccessFocusNext'))
        self.assertFalse(_completed(trace.splitlines()[1]+'\n', 15, 'AccessFocusNext'))


if __name__ == '__main__': unittest.main()
