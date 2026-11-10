"""Source-bound palette navigation callbacks; no guest or native input proof."""
import re
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from quil_workflow_gate import palette_commands,palette_select

ROOT=Path(__file__).resolve().parents[2]
LEGACY=(('FocusSpindle','Open Spindle'),('FocusQuil','Open Quil'),('FocusLinen','Open Linen'),
        ('FocusAtlas','Open Atlas'),('FocusBell','Open Bell'),('FocusCollar','Open Collar'),
        ('FocusMesh','Open Mesh'),('RestoreMinimized','Restore Minimized'),
        ('ZoomToggle','Zoom Toggle'),('MinimizeFocused','Minimize Focused'))
CURRENT=LEGACY+(('FocusBrowser','Open Browser'),('FocusProgram','Run Hello'))
FUTURE=CURRENT+(('HostFixtureOnly','Host fixture only'),)
# The live catalog: CURRENT plus the power rows at the end.
ACTUAL=CURRENT+(('ShutDown','Shut Down'),('Restart','Restart'))

def catalog(commands):
    return 'const COMMAND_LIST: [CommandDef; '+str(len(commands))+'] = [\n'+''.join(
        '    CommandDef { command: Command::'+name+', name: "'+label+'" },\n'for name,label in commands)+'];\n'

class PaletteCommands(unittest.TestCase):
    def read(self,source):
        with tempfile.TemporaryDirectory()as directory:
            root=Path(directory);path=root/'servers/silk-shell/src/main.rs'
            path.parent.mkdir(parents=True);path.write_text(source)
            return palette_commands(root)

    def test_actual_and_archival_and_future_source_catalogs(self):
        self.assertEqual(palette_commands(ROOT),ACTUAL)
        for commands in(LEGACY,CURRENT,FUTURE):self.assertEqual(self.read(catalog(commands)),commands)

    def test_zoom_paths_are_independent_literal_goldens(self):
        # Appending commands changes wrap distance, without moving Zoom's row8.
        paths=[(LEGACY,[(0,9),(9,8)]),(CURRENT,[(0,11),(11,10),(10,9),(9,8)]),
               (FUTURE,[(0,12),(12,11),(11,10),(10,9),(9,8)])]
        for commands,path in paths:
            receipts=[('grave_accent',f'[shell.palette.open] ok=1 selected=0 count={len(commands)}\n')]
            receipts += [('k',f'[shell.palette.select] old={old} new={new}\n')for old,new in path]
            observed=[]
            def key(code,pattern):
                wanted,line=receipts[len(observed)];self.assertEqual(code,wanted)
                self.assertIsNotNone(re.search(pattern,line));observed.append(code)
            self.assertEqual(palette_select(key,'ZoomToggle',self.read(catalog(commands))),8)
            self.assertEqual(observed,['grave_accent']+['k']*len(path))
            self.assertNotIn('ret',observed) # Caller owns Enter/action observation.

    def test_forward_origin_and_browser_targets_have_exact_receipts(self):
        for command,target,path in [('FocusSpindle',0,[]),('FocusLinen',2,[('j',0,1),('j',1,2)]),
                                    ('FocusBrowser',10,[('k',0,11),('k',11,10)])]:
            receipts=[('grave_accent','[shell.palette.open] ok=1 selected=0 count=12\n')]+[
                (code,f'[shell.palette.select] old={old} new={new}\n')for code,old,new in path]
            sent=[]
            def key(code,pattern):
                want,text=receipts[len(sent)];self.assertEqual(code,want)
                self.assertIsNotNone(re.search(pattern,text));sent.append(code)
            self.assertEqual(palette_select(key,command,CURRENT),target)
            self.assertEqual(len(sent),len(receipts))

    def test_unknown_or_duplicate_requested_command_sends_nothing(self):
        for command,commands in [('Absent',CURRENT),('ZoomToggle',CURRENT+(('ZoomToggle','Other label'),)),('ZoomToggle',())]:
            sent=[]
            with self.assertRaises(ValueError):palette_select(lambda *args:sent.append(args),command,commands)
            self.assertEqual(sent,[])

    def test_source_count_duplicate_inference_and_legacy_changes_fail(self):
        source=catalog(CURRENT)
        cases=[source.replace('CommandDef; 12','CommandDef; 11'),source+source,
               source.replace('Command::FocusBrowser','Command::ZoomToggle'),
               source.replace('name: "Open Browser"','name: "Zoom Toggle"'),
               source.replace('Command::ZoomToggle','pick_command()'),
               source.replace('name: "Zoom Toggle"','name: ZOOM_LABEL'),
               source.replace('name: "Open Spindle"','name: "Renamed Spindle"'),
               source.replace('    CommandDef { command: Command::FocusBrowser, name: "Open Browser" },\n','')]
        for changed in cases:
            self.assertNotEqual(changed,source)
            with self.subTest(changed=changed[-100:]),self.assertRaises(ValueError):self.read(changed)

    def test_stale_truncated_wrong_count_or_transition_prevents_enter(self):
        good=['[shell.palette.open] ok=1 selected=0 count=12\n',
              '[shell.palette.select] old=0 new=11\n','[shell.palette.select] old=11 new=10\n',
              '[shell.palette.select] old=10 new=9\n','[shell.palette.select] old=9 new=8\n']
        mutations=[(0,good[0].replace('count=12','count=11')),(0,good[0].replace('selected=0','selected=8')),
                   (0,'[shell.palette.item] idx=0\n'),(1,good[1].replace('new=11','new=9')),
                   (1,good[1].rstrip('\n')),(2,good[2].replace('new=10','new=90')),(4,'')]
        for index,bad in mutations:
            sent=[];current=list(good);current[index]=bad
            def key(code,pattern):
                offset=len(sent);sent.append(code)
                # An old matching line is outside this key's fresh receipt window.
                stale=good[offset];self.assertIsNotNone(re.search(pattern,stale))
                if re.search(pattern,current[offset])is None:raise TimeoutError('no exact fresh transition')
            with self.subTest(index=index,bad=bad),self.assertRaises(TimeoutError):
                palette_select(key,'ZoomToggle',CURRENT)
                sent.append('ret')
            self.assertNotIn('ret',sent)
            self.assertEqual(len(sent),index+1)

if __name__=='__main__':unittest.main()
