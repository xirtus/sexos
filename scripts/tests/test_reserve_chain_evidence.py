import copy
import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from alpha_evidence import verify_third_captures
from check_document_disk import fnv
from quil_reserve_catalog import quil_catalog,verify_boot_quil

ROOT=Path(__file__).resolve().parents[2]


def fixture():
 rows=quil_catalog(ROOT);text='';frames=[];steps=[]
 for i,r in enumerate(rows):
  text+=f'[boot.app.catalog] target={r["target"]} task={r["target"]} surface={r["surface"]} logical=9 incarnation={i+1} predecessor={rows[i-1]["target"]if i else 0} lifecycle={r["lifecycle"]} close={r["close"]} route={r["route"]} dormant={int(i!=0)} grants_verified=1 data_only=1\n'
 first=dict(object=3,bytes=5,hash=hex(fnv(b'first')));body='second'
 for i,r in enumerate(rows[2:]):
  target=r['target'];a=len(text);pre=rows[r['ordinal']-1]['target']
  text+=f'[app.activation.committed] logical=9 incarnation={r["ordinal"]+1} predecessor={pre} successor={target} task={target} surface={r["surface"]} pages=70 runtime_mint=0\n[app.surface.presented] target={target} surface={r["surface"]} sequence=1 ok=1\n'
  second=dict(object=4,bytes=len(body),hash=hex(fnv(body.encode())))
  for load in [first,second]:text+=f'[quil.persist.load.ok] bytes={load["bytes"]} hash={load["hash"]} doc={load["object"]}\n'
  e=len(text);text+='[quil.document.edit] revision=2\n'
  frame=dict(name='third-quil-edit'if i==0 else f'reserve-quil-{target}-edit',kind='quil',target=target,expected='t#',serial_end=len(text));frames.append(frame)
  b=len(text);body+='t';saved=dict(object=4,content=body,bytes=len(body),hash=hex(fnv(body.encode())))
  text+=f'[quil.persist.save.ok] bytes={len(body)} hash={saved["hash"]} doc=4\n';d=len(text)
  text+=f'[task.finish] pd={target} task={target} reason=0 terminal=1\n[sext.app.pool.ready] target={target} task={target} ok=1\n'
  steps.append(dict(target=target,surface=r['surface'],logical=9,incarnation=r['ordinal']+1,predecessor=pre,serial_start=a,edit_start=e,save_start=b,close_start=d,serial_end=len(text),loaded=[dict(first),second],saved=saved,frame=frame))
 selection=len(text);text+='[shell.palette.select] old=0 new=1\n[launcher.text.view] visible=1\n';row=len(text);text+='[launcher.text.row] index=1 row=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED\n'
 end=dict(name='third-quil-exhausted-launcher',kind='launcher',target=9,expected='Open Quil',serial_end=len(text));frames.append(end)
 return dict(frame=frames[0],chain=steps,catalog=rows,final_target=rows[-1]['target'],append_count=len(steps),exhaustion_frame=end,exhaustion_selection_offset=selection,exhaustion_row_offset=row),frames,text,rows


class ReserveChainEvidence(unittest.TestCase):
 def test_all_eight_source_subjects_and_six_later_edits_join(self):
  third,frames,text,rows=fixture();self.assertEqual([r['target']for r in rows],[9,16,18,19,20,21,22,23]);verify_boot_quil(text,rows);verify_third_captures(third,frames,text,rows)
 def test_missing_middle_wrong_identity_recovery_frame_or_final_cleanup_fails(self):
  original=fixture()
  for mutation in range(7):
   third,frames,text,rows=copy.deepcopy(original)
   if mutation==0:third['chain'].pop(2)
   elif mutation==1:third['chain'][-1]['target']=22
   elif mutation==2:third['chain'][1]['loaded'][1]['hash']='0x1'
   elif mutation==3:frames[2]['target']=18
   elif mutation==4:text=text.replace('pd=23 task=23 reason=0','pd=22 task=23 reason=0')
   elif mutation==5:third['chain'][-1]['saved']['content']+='z'
   else:third['final_target']=18
   with self.subTest(mutation=mutation),self.assertRaises(ValueError):verify_third_captures(third,frames,text,rows)
 def test_duplicate_boot_audit_or_forged_slot_fails(self):
  third,frames,text,rows=fixture()
  for raw in [text+text.splitlines()[0]+'\n',text.replace('lifecycle=126','lifecycle=54')]:
   with self.assertRaises(ValueError):verify_boot_quil(raw,rows)


if __name__=='__main__':unittest.main()
