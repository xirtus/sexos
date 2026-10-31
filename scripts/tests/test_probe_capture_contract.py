"""Exercise helper control flow against the shared immutable-capture contract."""
import re
import sys
import unittest
from pathlib import Path
from unittest.mock import Mock, call, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import workspace_refresh_probe as workspace
import quil_third_probe as third


class CaptureContract(unittest.TestCase):
    def workspace_context(self, capture):
        return dict(wait=Mock(), text=lambda: '[shell.focus.set] id=201\n',
                    focus=Mock(), key=Mock(), capture=capture)

    def test_automatic_list_uses_linen_capture_and_preserves_frame_identity(self):
        frame = dict(name='auto-list-uialpha-0001', sha256='immutable-image',
                     serial_sha256='immutable-prefix', kind='linen')
        capture = Mock(return_value=frame)
        context = self.workspace_context(capture)
        with patch.object(workspace, 'final_view', return_value=({'selected': 5}, [(5, 0, 'uialpha')])):
            result = workspace.after_save(context, 17, 2, 'uialpha')
        capture.assert_called_once_with('auto-list-uialpha', kind='linen')
        self.assertIs(result['frame'], frame)
        self.assertEqual([c.args for c in context['focus'].call_args_list], [(200,), (201,)])
        context['key'].assert_not_called()
        self.assertFalse(result['manual_rescan'])
        self.assertTrue(all(call.args[1] == 17 for call in context['wait'].call_args_list))

    def test_failed_linen_capture_cannot_return_success_or_restore_focus_early(self):
        context = self.workspace_context(Mock(side_effect=RuntimeError('unverified frame')))
        with patch.object(workspace, 'final_view', return_value=({'selected': 5}, [(5, 0, 'uialpha')])):
            with self.assertRaisesRegex(RuntimeError, 'unverified frame'):
                workspace.after_save(context, 0, 2, 'uialpha')
        context['focus'].assert_called_once_with(200)

    def third_context(self, capture, current_row=True):
        events = []
        row='[launcher.text.row] index=1 row=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED\n'
        state=dict(log=row, saved=False)
        def wait(pattern, start=0):
            events.append(('wait', pattern))
            if 'persist.save.ok' in pattern:
                return re.search(pattern, '[quil.persist.save.ok] bytes=13 hash=0xabcd doc=6')
            if 'launcher.text.row' in pattern or 'shell.palette.select' in pattern:
                match=re.search(pattern,state['log'][start:])
                if match is None:raise RuntimeError('required current launcher receipt absent')
                return match
        def ctrl(*args):
            events.append(('save', args))
            state['saved']=True
        def key(name, pattern):
            events.append(('key', name))
            if state['saved'] and name=='grave_accent':
                # A prior render and positive non-atomic diagnostic cannot
                # prove the subsequent physical selection's current row.
                state['log']+=row+'[shell.palette.item] idx=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED\n'
            if state['saved'] and name=='j':
                state['log']+='[shell.palette.select] old=0 new=1\n'
                state['log']+=row if current_row else '[shell.palette.status] CLOSED / RESERVE EXHAUSTED\n'
        context = dict(key=Mock(side_effect=key), wait=wait, text=lambda: state['log'], focus=Mock(),
                       capture=capture, ctrl=ctrl,
                       first=dict(object=5, bytes=11, hash='0x111', content='first'),
                       second=dict(object=6, bytes=12, hash='0x222', content='second'))
        return context, events

    def test_third_capture_binds_subject18_and_precedes_save_and_close(self):
        frame = dict(name='third-quil-edit-0002', target=18, sha256='image', serial_sha256='prefix')
        exhaustion_frame=dict(name='third-quil-exhausted-launcher-0003',target=9,kind='launcher',sha256='launcher-image')
        capture = Mock()
        context, events = self.third_context(capture)
        def captured(*args, **kwargs):
            if kwargs.get('kind')=='launcher':
                self.assertTrue(any(e[0]=='close' for e in events))
                return exhaustion_frame
            self.assertFalse(any(e[0] == 'save' for e in events))
            events.append(('capture', frame))
            return frame
        capture.side_effect = captured
        refresh, close = Mock(), Mock(side_effect=lambda *args: events.append(('close', args)))
        with patch.object(third, 'open_selected', side_effect=[dict(context['first']), dict(context['second'])]):
            result = third.run(context, refresh, close)
        self.assertEqual(capture.call_args_list,[call('third-quil-edit','t#',target=18),
            call('third-quil-exhausted-launcher','Open Quil',target=9,kind='launcher')])
        self.assertIs(result['frame'], frame)
        self.assertIs(result['exhaustion_frame'],exhaustion_frame)
        self.assertGreater(result['exhaustion_row_offset'],result['exhaustion_selection_offset'])
        self.assertTrue(context['text']()[result['exhaustion_row_offset']:].startswith('[launcher.text.row]'))
        self.assertEqual(result['documents'], [5, 6])
        self.assertEqual(context['second']['content'], 'secondt')
        self.assertEqual((context['second']['bytes'], context['second']['hash']), (13, '0xabcd'))
        refresh.assert_called_once_with()
        close.assert_called_once_with(18, 307)
        self.assertLess(next(i for i,e in enumerate(events) if e[0]=='capture'),
                        next(i for i,e in enumerate(events) if e[0]=='save'))

    def test_failed_third_capture_stops_before_persist_or_cleanup_success(self):
        context, events = self.third_context(Mock(side_effect=RuntimeError('wrong subject pixels')))
        close = Mock()
        with patch.object(third, 'open_selected', side_effect=[dict(context['first']), dict(context['second'])]):
            with self.assertRaisesRegex(RuntimeError, 'wrong subject pixels'):
                third.run(context, Mock(), close)
        self.assertFalse(any(e[0] == 'save' for e in events))
        self.assertEqual(context['second']['content'], 'second')
        close.assert_not_called()

    def test_old_atomic_row_and_legacy_status_do_not_satisfy_new_selection(self):
        capture=Mock(return_value={'name':'edit-frame'})
        context,events=self.third_context(capture,current_row=False)
        with patch.object(third,'open_selected',side_effect=[dict(context['first']),dict(context['second'])]):
            with self.assertRaisesRegex(RuntimeError,'required current launcher receipt absent'):
                third.run(context,Mock(),Mock())
        capture.assert_called_once_with('third-quil-edit','t#',target=18)
        self.assertEqual([e for e in events if e==('key','esc')],[])

    def test_failed_exhaustion_pixels_cannot_return_success(self):
        capture=Mock(side_effect=[{'name':'edit-frame'},RuntimeError('launcher pixels differ')])
        context,events=self.third_context(capture)
        with patch.object(third,'open_selected',side_effect=[dict(context['first']),dict(context['second'])]):
            with self.assertRaisesRegex(RuntimeError,'launcher pixels differ'):
                third.run(context,Mock(),Mock())
        self.assertEqual(capture.call_args_list[-1],call('third-quil-exhausted-launcher','Open Quil',target=9,kind='launcher'))
        self.assertEqual([e for e in events if e==('key','esc')],[])


if __name__ == '__main__':
    unittest.main()
