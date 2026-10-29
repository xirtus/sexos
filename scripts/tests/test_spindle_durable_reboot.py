import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from spindle_durable_reboot_gate import snapshot,fnv

def bank(disk,index,generation=1,source=17,command=b'echo spindle17'):
    payload=bytearray(26+len(command));payload[:4]=b'SHI1';payload[4:8]=source.to_bytes(4,'little');payload[8:10]=(1).to_bytes(2,'little');payload[10:12]=len(payload).to_bytes(2,'little');payload[24:26]=len(command).to_bytes(2,'little');payload[26:]=command;payload[16:24]=fnv(payload).to_bytes(8,'little')
    h=bytearray(512);h[:8]=b'SEXHST01';h[8:12]=(12).to_bytes(4,'little');h[12:16]=(1).to_bytes(4,'little');h[16:24]=generation.to_bytes(8,'little');h[24:28]=source.to_bytes(4,'little');h[28:32]=source.to_bytes(4,'little');h[32:36]=len(payload).to_bytes(4,'little');h[40:48]=fnv(payload).to_bytes(8,'little');h[48:56]=fnv(h).to_bytes(8,'little');offset=(1808+index*9)*512;disk[offset:offset+512]=h;disk[offset+512:offset+512+len(payload)]=payload
    return offset
class Check(unittest.TestCase):
    def test_actual_layout_selects_latest_complete_snapshot(self):
        disk=bytearray(2048*512);bank(disk,0,1,12);bank(disk,1,2);r=snapshot(disk);self.assertEqual((r['generation'],r['source'],r['count']),(2,17,1));self.assertEqual(r['data_hash'],hex(fnv(b'\x0e\x00echo spindle17')))
    def test_damaged_new_bank_uses_previous(self):
        disk=bytearray(2048*512);bank(disk,0,1,12);offset=bank(disk,1,2);disk[offset+512+26]^=1;self.assertEqual(snapshot(disk)['generation'],1)
    def test_blank_truncated_unknown_and_unbound_task_rejected(self):
        for kind in ['blank','truncated','version','task','padding','payload']:
            disk=bytearray(2048*512);offset=bank(disk,0)
            if kind=='blank':disk=bytes(len(disk))
            elif kind=='truncated':disk=disk[:offset+520]
            elif kind=='padding':disk[offset+512+80]=1
            elif kind in ('version','task'):
                field=12 if kind=='version'else 28;disk[offset+field:offset+field+4]=(99).to_bytes(4,'little');disk[offset+48:offset+56]=bytes(8);disk[offset+48:offset+56]=fnv(disk[offset:offset+512]).to_bytes(8,'little')
            else:disk[offset+512+26]=0
            with self.assertRaises(RuntimeError,msg=kind):snapshot(disk)
    def test_conflicting_equal_generations_are_not_arbitrarily_accepted(self):
        disk=bytearray(2048*512);bank(disk,0,1,12);bank(disk,1,1,17)
        with self.assertRaises(RuntimeError):snapshot(disk)
if __name__=='__main__':unittest.main()
