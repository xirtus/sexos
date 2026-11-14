#!/usr/bin/env python3
"""Writes the byte-exact goldens for tests/goldens.rs with an IPP and DNS encoder that
shares no code with sex-print (scripts/needle_fakes.py, the gate's fake printer).
Run from the repository root: python3 crates/sex-print/tests/fixtures/make_fixtures.py"""
import struct,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[3]/'scripts'))
from needle_fakes import ipp_message,FakePrinter,CAPS,dnssd_answer,dns_name

URI='ipp://10.0.2.2:8631/ipp/print'
OP=lambda *extra:(1,[(0x47,'attributes-charset',['utf-8']),(0x48,'attributes-natural-language',['en']),(0x45,'printer-uri',[URI]),(0x42,'requesting-user-name',['needle'])]+list(extra))
def write(name,data):(HERE/name).write_bytes(data if isinstance(data,bytes) else data.encode())

write('ipp_gpa_request.bin',ipp_message((2,0),0x0B,1,[OP((0x44,'requested-attributes',['printer-name','document-format-supported']))]))
write('ipp_print_job_request.bin',ipp_message((2,0),0x02,1,[OP((0x42,'job-name',['hello.txt']),(0x49,'document-format',['image/pwg-raster'])),
 (2,[(0x21,'copies',[2]),(0x44,'sides',['two-sided-long-edge']),(0x44,'print-color-mode',['monochrome']),(0x44,'media',['iso_a4_210x297mm']),(0x32,'printer-resolution',[(300,300,3)])])],b'RaS2'))
write('ipp_cancel_job_request.bin',ipp_message((1,1),0x08,1,[OP((0x21,'job-id',[42]))]))
p=FakePrinter();p.uri=URI
write('ipp_gpa_response.bin',p.handle(ipp_message((2,0),0x0B,7,[OP()]),'/ipp/print'))
p.handle(ipp_message((2,0),0x02,8,[OP((0x42,'job-name',['one']),(0x49,'document-format',['application/pdf']))],b'%PDF-1.4'),'/')
p.handle(ipp_message((2,0),0x02,9,[OP((0x42,'job-name',['two']),(0x49,'document-format',['image/pwg-raster']))],b'RaS2'),'/')
p.jobs[1]['state']=5
write('ipp_get_jobs_response.bin',p.handle(ipp_message((2,0),0x0A,10,[OP((0x44,'which-jobs',['all']))]),'/'))
p.stop()

question=b''.join(dns_name(n)+struct.pack('>HH',12,0x8001) for n in('_ipp._tcp.local','_ipps._tcp.local','_uscan._tcp.local','_uscans._tcp.local'))
write('dnssd_browse_query.bin',struct.pack('>HHHHHH',0x4e64,0,4,0,0,0)+question)
r=dnssd_answer(0x4e64,question,8631,8080);write('dnssd_response.bin',r[:4]+struct.pack('>H',4)+r[6:])
write('escl_capabilities.xml',CAPS)
