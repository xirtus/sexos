import pathlib
import sys
import unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from desktop_focus import focus_surface

class FocusTests(unittest.TestCase):
    def test_entering_spindle_during_traversal_uses_global_control(self):
        state={'sid':200,'shown':True};keys=[]
        def text():return '[shell.focus.set] id='+str(state['sid'])
        def key(code,pattern):
            keys.append(code)
            if code=='scroll_lock':state.update(shown=False,sid=200)
            elif state['sid']==200:state['sid']=300
            elif state['sid']==300:state['sid']=153 if state['shown'] else 201
            else:self.fail('Tab was sent to app-local completion')
        focus_surface(201,text,key)
        self.assertEqual(keys,['tab','tab','scroll_lock','tab','tab'])
    def test_shown_unfocused_spindle_can_be_minimized_then_restored(self):
        state={'sid':200,'shown':True};keys=[]
        def key(code,pattern):
            self.assertEqual(code,'scroll_lock');keys.append(code)
            state['shown']=not state['shown'];state['sid']=153 if state['shown'] else 200
        focus_surface(153,lambda:'[shell.focus.set] id='+str(state['sid']),key)
        self.assertEqual(len(keys),2)
    def test_current_successor_needs_no_input(self):
        focus_surface(306,lambda:'[shell.focus.set] id=306',lambda *args:self.fail())
    def test_nonprogressing_guest_is_bounded(self):
        keys=[]
        with self.assertRaises(RuntimeError):focus_surface(201,lambda:'[shell.focus.set] id=200',lambda *args:keys.append(args))
        self.assertEqual(len(keys),24)
if __name__=='__main__':unittest.main()
