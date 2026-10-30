"""Pure redirect fixture/order oracle checks, never native guest evidence."""
import copy,re,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import http_redirect_gate as gate

def receipt_fixture(mode):
 lines=[];admissions=[];fixtures=gate.plan(mode);positive=mode in('chain','self-loop','two-node-loop','hop-limit','cancel-retry','close')
 lines.append('[browser.http.released] ticket=1 view=Idle private_job=0\n')
 for index,(ticket,generation)in enumerate(gate.expected_admissions(mode)):
  item=fixtures[index]
  if mode=='cancel-retry'and index==1:
   lines.append('[browser.dns.released] ticket=2 view=Cancelled private_job=0\n')
  lines.append(f'[net.dns.egress.submit] ticket={generation} sequence=1 length=64 hash=0x1 scope=dma_admission_not_response\n')
  offset=sum(map(len,lines));lines.append(f'[net.job.resolved.admitted] owner=15 task=15 ticket={ticket} dns_generation={generation} observed=1 expires=60001 now=2 fixed=1 at_begin=1\n')
  admissions.append(dict(ticket=ticket,dns_generation=generation,admission_log_offset=offset))
  if item['status']==200:
   lines.extend([f'[browser.http.body] ticket={ticket} status=200 length=12288 hash=0x1 controlled_peer=1 tls=0\n',f'[browser.http.released] ticket={ticket} view=Ready private_job=0\n',f"[browser.http.document] ticket={ticket} path={item['path'].decode()} status=200 private_bytes=12288 positive_release=1\n",'[browser.view.applied] bytes=512 http=2 body_bytes=12288 pixels=unconfirmed\n'])
  elif positive:
   lines.extend([f'[browser.http.released] ticket={ticket} view=Pending private_job=0\n',f"[browser.http.redirect.released] ticket={ticket} status={item['status']} path={item['next'].decode()} positive_release=1 document_bytes=0\n"])
   follow=(mode in('chain','hop-limit')and index<2)or(mode in('two-node-loop','cancel-retry','close')and index==0)
   if follow:lines.append(f"[browser.http.redirect.follow] hop={index+1} path={item['next'].decode()} same_endpoint=1 fresh_dns=1 positive_release=1\n")
   elif mode in('self-loop','two-node-loop','hop-limit'):lines.append('[browser.http.redirect.denied] loop_or_limit_or_superseded=1 effects=0\n')
  else:lines.append(f'[browser.http.released] ticket={ticket} view=Unavailable private_job=0\n')
 return ''.join(lines),admissions

class RedirectOracle(unittest.TestCase):
 def test_independent_fixed_host_request_bytes_for_each_actual_path(self):
  for path in(b'/welcome',b'/guide',b'/guide/text'):
   self.assertEqual(gate.request(path),b'GET '+path+b' HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n')
  self.assertLessEqual(len(gate.request(b'/'+b'a'*47)),128)
 def test_path_selector_rejects_authority_aliases_controls_and_bound_overflow(self):
  for value in(b'',b'http://outside.test/',b'//outside.test/',b'/a//b',b'/a/../b',b'/./b',b'/x?q=1',b'/x#part',b'/x%2f',b'/x:80',b'/x\\y',b'/\xff',b'/x\r\nHost: other',b'/'+b'a'*48):
   with self.subTest(value=value),self.assertRaises(ValueError):gate.request(value)
 def test_chain_has_two_complete_bounded_redirects_and_final_full_document(self):
  rows=gate.plan('chain');self.assertEqual([(r['path'],r['status'],r['next'])for r in rows],[(b'/welcome',302,b'/guide'),(b'/guide',307,b'/guide/text'),(b'/guide/text',200,None)])
  for row in rows[:2]:
   header,body=row['response'].split(b'\r\n\r\n',1);self.assertEqual(int(re.search(br'Content-Length: (\d+)',header)[1]),len(body));self.assertLessEqual(len(body),1024);self.assertFalse(row['aborted'])
  self.assertTrue(rows[-1]['response'].endswith(gate.STREAM_BODY));self.assertEqual(len(gate.STREAM_BODY),12288)
 def test_negative_plans_have_exact_admission_and_distinct_malformed_body_evidence(self):
  self.assertEqual(gate.expected_admissions('two-node-loop'),[(2,1),(3,2)]);self.assertEqual(gate.expected_admissions('hop-limit'),[(2,1),(3,2),(4,3)])
  for mode in('external','duplicate-location','missing-location','short-body','trailing-body'):
   row=gate.plan(mode)[0];self.assertTrue(row['aborted']);self.assertEqual(gate.expected_admissions(mode),[(2,1)])
  short=gate.plan('short-body')[0]['response'];trailing=gate.plan('trailing-body')[0]['response']
  self.assertEqual(int(re.search(br'Content-Length: (\d+)',short)[1]),len(short.split(b'\r\n\r\n',1)[1])+1)
  self.assertEqual(int(re.search(br'Content-Length: (\d+)',trailing)[1])+1,len(trailing.split(b'\r\n\r\n',1)[1]))
 def test_fixture_builder_rejects_unbounded_or_injected_responses(self):
  for args in[(301,b'/guide'),(302,b'/guide\r\nHost: other')]:
   with self.assertRaises(ValueError):gate.redirect(*args)
  with self.assertRaises(ValueError):gate.redirect(302,b'/guide',b'x'*1025)
  with self.assertRaises(ValueError):gate.redirect(302,b'/guide',b'',length_delta=-1)
 def test_order_oracle_accepts_each_explicit_mode(self):
  for mode in gate.MODES:
   with self.subTest(mode=mode):
    trace,admissions=receipt_fixture(mode);result=gate.verify_navigation(trace,mode,admissions);self.assertTrue(result['ok'],result)
 def test_positive_release_is_required_before_next_fresh_dns(self):
  trace,rows=receipt_fixture('chain')
  for bad in[trace.replace('[browser.http.released] ticket=2 view=Pending','[browser.http.released] ticket=2 view=Unavailable'),trace.replace('positive_release=1 document_bytes=0','positive_release=0 document_bytes=0',1)]:
   self.assertFalse(gate.verify_navigation(bad,'chain',rows)['ok'])
  bad=copy.deepcopy(rows);bad[1]['admission_log_offset']=0;self.assertFalse(gate.verify_navigation(trace,'chain',bad)['ok'])
 def test_wrong_generation_missing_hop_and_stale_continuation_rejected(self):
  trace,rows=receipt_fixture('chain');bad=copy.deepcopy(rows);bad[1]['dns_generation']=1;self.assertFalse(gate.verify_navigation(trace,'chain',bad)['ok'])
  self.assertFalse(gate.verify_navigation(trace.replace('hop=2 path=/guide/text','hop=3 path=/guide/text'),'chain',rows)['ok'])
  trace,rows=receipt_fixture('cancel-retry');self.assertEqual(gate.plan('cancel-retry')[1]['path'],b'/welcome');self.assertEqual(gate.expected_admissions('cancel-retry'),[(2,1),(3,3)])
  self.assertFalse(gate.verify_navigation(trace.replace('ticket=2 view=Cancelled','ticket=2 view=Unavailable'),'cancel-retry',rows)['ok'])
 def test_redirect_body_or_early_ready_cannot_qualify_document(self):
  trace,rows=receipt_fixture('chain')
  for extra in['[browser.http.body] ticket=2 status=200 length=1 hash=0x1 controlled_peer=1 tls=0\n','[browser.http.document] ticket=2 path=/welcome status=200 private_bytes=1 positive_release=1\n','[browser.view.applied] bytes=512 http=2 body_bytes=1 pixels=unconfirmed\n']:
   self.assertFalse(gate.verify_navigation(extra+trace,'chain',rows)['ok'])
 def test_final_document_requires_exact_path_length_and_positive_ready_release(self):
  trace,rows=receipt_fixture('chain')
  for bad in[trace.replace('path=/guide/text status=200','path=/welcome status=200'),trace.replace('private_bytes=12288','private_bytes=64'),trace.replace('ticket=4 view=Ready','ticket=4 view=Pending')]:self.assertFalse(gate.verify_navigation(bad,'chain',rows)['ok'])
 def test_unknown_mode_never_falls_back_to_success(self):
  with self.assertRaises(ValueError):gate.plan('other')

if __name__=='__main__':unittest.main()
