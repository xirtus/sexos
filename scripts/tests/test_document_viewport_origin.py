"""Run the actual Shell receipt expression against actual native child layouts."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def run_receipt(expression):
    program = 'extern crate self as sex_pdx;\npub const ERR_SERVICE_NOT_READY:u64=0xFFFF_FFFF_FFFF_FFFE;\n'
    for module, relative in (
            ('app_catalog', 'crates/sex-pdx/src/app_catalog.rs'),
            ('desktop_style', 'crates/sex-pdx/src/desktop_style.rs'),
            ('spindle_view', 'crates/sex-pdx/src/spindle_view.rs'),
            ('desktop_content', 'servers/silk-shell/src/desktop_content.rs')):
        program += f'#[path={json.dumps(str(ROOT/relative))}] pub mod {module};\n'
    program += '''fn main() {
      let mut checked=0;
      for scale in [1,2,3] {
        desktop_style::set_ui_scale(scale);
        for incarnation in app_catalog::INCARNATIONS.iter().filter(|r|r.app==app_catalog::App::Quil && r.shell_surface.is_some()) {
          let frame=incarnation.frame(); let inset=true;
          for (x,y,w,h) in [(8i32,36i32,692u32,756u32),(-20,100,256,144),(0,0,1,1)] {
            let get_surface_bounds=|_:u64|Some((x,y,w,h));
    ''' + expression + '''
            let expected=if w<=16*scale||h<=42*scale {None}
              else {Some((x+8*scale as i32,y+34*scale as i32))};
            assert_eq!(origin,expected);
            assert_eq!(desktop_content::child_origin(frame,incarnation.surface,x,y,w,h),expected);
            assert_eq!(desktop_content::child_origin(frame,u64::MAX,x,y,w,h),None);
            checked+=1;
          }
        }
      }
      assert!(checked>=9);
    }'''
    with tempfile.TemporaryDirectory(prefix='sexos-viewport-receipt-') as directory:
        source = Path(directory)/'probe.rs'
        executable = Path(directory)/'probe'
        source.write_text(program)
        subprocess.run(['rustc','--edition=2021','-A','dead_code',str(source),'-o',str(executable)],
                       check=True,capture_output=True,timeout=60)
        return subprocess.run([str(executable)],capture_output=True,timeout=10)


class DocumentViewportOrigin(unittest.TestCase):
    def test_actual_receipt_expression_matches_owned_child_at_every_ui_scale(self):
        source=(ROOT/'servers/silk-shell/src/main.rs').read_text()
        expression=next(line for line in source.splitlines() if 'let origin=get_surface_bounds(frame)' in line)
        self.assertEqual(run_receipt(expression).returncode,0)

    def test_original_hardcoded_receipt_is_rejected_by_production_layout(self):
        old='let origin=get_surface_bounds(frame).map(|(x,y,_,_)|if inset{(x.saturating_add(8),y.saturating_add(36))}else{(x,y)});'
        self.assertNotEqual(run_receipt(old).returncode,0)


if __name__ == '__main__':
    unittest.main()
