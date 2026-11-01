"""Synthetic source evidence only, never native BOOT or authority proof."""
import re,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import browser_source_evidence as e

def line(tag,**fields):return '['+tag+'] '+' '.join(k+'='+str(v)for k,v in fields.items())+'\n'

def inventory(plans=e.CURRENT_PLANS,observed=False):
    text='';tasks={15:150,30:300,31:310}
    for i,p in enumerate(plans):
        close,route=((43,43),(155,157),(159,161))[i]
        text+=line('boot.app.catalog',target=p.target,task=tasks[p.target],surface=p.surface,logical=15,incarnation=i+1,predecessor=0 if i==0 else plans[i-1].target,lifecycle=p.lifecycle,close=close,route=route,dormant=int(i!=0),grants_verified=1,data_only=1)
        base=dict(holder=13,task=130,consumer=p.target,consumer_task=tasks[p.target],runtime_mint=0)
        text+=line('boot.entropy.source',**base,slot=p.entropy,source='rdseed',attempts=8)
        text+=line('boot.clock.source',**base,slot=p.clock,source='q35_cmos_utc',attempts=4,precision=1)
        text+=line('boot.trust.policy',**base,slot=p.trust,profile=1,root_bytes=376)
    text+=line('boot.silkbar.clock.source',holder=6,task=6,slot=61,consumer=6,consumer_task=6,source='q35_cmos_utc',attempts=4,precision=1,runtime_mint=0)
    extra=dict(trust_profile=1,trust_root_bytes=376,trust_version=1)if observed else{}
    for p in plans:
        text+=line('boot.browser.sources.readback',holder=13,target=p.target,entropy=p.entropy,clock=p.clock,trust=p.trust,late_before_seal=1,typed_info=1,runtime_context=0,**extra)
    for p in plans:
        text+=line('net.source.binding.held',target=p.target,task=tasks[p.target],surface=p.surface,lifecycle_slot=p.lifecycle,entropy_slot=p.entropy,clock_slot=p.clock,trust_slot=p.trust,before_dispatch=1,**extra)
    return text

def clock(pd,task,slot,seconds,ordinal):
    return line('clock.source.snapshot',pd=pd,task=task,slot=slot,seconds=seconds,attempts=1,precision=1,profile='q35_cmos_utc',ordinal=ordinal)
def success(pd,task,seq,seconds):
    return line('certificate.time.snapshot',owner=pd,task=task,seq=seq,seconds=seconds,source='q35_cmos_utc',precision=1,receipt='retained')
def rollback(pd,task,seq):
    return line('certificate.time.rollback',owner=pd,task=task,seq=seq,blocked_until_boot=1,last_good_fallback=0)
def seed(slot,count):return line('entropy.source.sample',pd=13,task=130,slot=slot,source='rdseed',attempts=8,count=count,private=1)

class Evidence(unittest.TestCase):
    def test_current_and_future_exact_unequal_tasks(self):
        for plans in (e.CURRENT_PLANS,e.FUTURE_PLANS):
            for observed in(False,True):
                r=e.validate_inventory(inventory(plans,observed),plans,require_version=observed)
                self.assertEqual(r['holder'],(13,130));self.assertEqual(r['roster'][15]['task'],150)
                self.assertEqual(r['trust_version_observed'],observed)
                self.assertEqual(len(r['clock_allowed']),len(plans)+1)
                self.assertIn((6,6,61),r['clock_allowed'])
    def test_optional_network_tls_requires_every_exact_planned_tuple(self):
        for plans in(e.CURRENT_PLANS,e.FUTURE_PLANS):
            base=inventory(plans,True)
            tasks={15:150,30:300,31:310}
            rows=[line('boot.network.tls',holder=13,task=130,consumer=p.target,
                       consumer_task=tasks[p.target],profile=1,peer_port=46080,
                       origin='immutable_boot_profile',runtime_mint=0)for p in plans]
            self.assertEqual(e.validate_inventory(base,plans,require_version=True)['holder'],(13,130))
            text=base+''.join(rows)
            self.assertEqual(set(e.validate_inventory(text,plans,require_version=True)['roster']),{p.target for p in plans})
            for index,row in enumerate(rows):
                # Absence of the whole optional family is allowed. Once any row
                # exists, every planned consumer must be present exactly once.
                if len(rows)>1:
                    with self.subTest(missing=index),self.assertRaises(ValueError):
                        e.validate_inventory(base+''.join(rows[:index]+rows[index+1:]),plans)
                with self.subTest(duplicate=index),self.assertRaises(ValueError):
                    e.validate_inventory(text+row,plans)
                for old,new in [('holder=13','holder=6'),('task=130','task=13'),
                                ('consumer='+str(plans[index].target),'consumer=99'),
                                ('consumer_task='+str(tasks[plans[index].target]),'consumer_task='+str(plans[index].target)),
                                ('profile=1','profile=2'),('peer_port=46080','peer_port=46081'),
                                ('origin=immutable_boot_profile','origin=runtime'),('runtime_mint=0','runtime_mint=1')]:
                    self.assertIn(old,row)
                    changed=rows[:index]+[row.replace(old,new,1)]+rows[index+1:]
                    with self.subTest(consumer=plans[index].target,old=old),self.assertRaises(ValueError):
                        e.validate_inventory(base+''.join(changed),plans)
            extra=rows[0].replace('consumer=15','consumer=99').replace('consumer_task=150','consumer_task=990')
            with self.assertRaises(ValueError):e.validate_inventory(text+extra,plans)

    def test_version_observation_is_not_inferred_from_expected_constant(self):
        with self.assertRaises(ValueError):e.validate_inventory(inventory(),require_version=True)
        text=inventory(observed=True)
        for old,new in [('trust_version=1','trust_version=2'),('trust_profile=1','trust_profile=2'),('trust_root_bytes=376','trust_root_bytes=377')]:
            with self.subTest(old=old),self.assertRaises(ValueError):e.validate_inventory(text.replace(old,new,1),require_version=True)
        for tag in ('boot.browser.sources.readback','net.source.binding.held'):
            rows=text.splitlines(True);idx=next(i for i,r in enumerate(rows)if '['+tag+']'in r)
            rows[idx]=rows[idx].replace(' trust_version=1','')
            with self.assertRaises(ValueError):e.validate_inventory(''.join(rows),require_version=True)
    def test_omission_and_duplicate_every_required_record(self):
        text=inventory(e.FUTURE_PLANS,True);rows=text.splitlines(True)
        for i,row in enumerate(rows):
            with self.subTest(row=row):
                with self.assertRaises(ValueError):e.validate_inventory(''.join(rows[:i]+rows[i+1:]),e.FUTURE_PLANS,require_version=True)
                with self.assertRaises(ValueError):e.validate_inventory(text+row,e.FUTURE_PLANS,require_version=True)
    def test_wrong_tasks_slots_profiles_aliases_and_catalogs(self):
        text=inventory(e.FUTURE_PLANS,True)
        for old,new in [('task=130','task=13'),('consumer_task=300','consumer_task=30'),('slot=70','slot=60'),('slot=71','slot=4294967367'),('profile=1 root_bytes','profile=2 root_bytes'),('surface=343','surface=344'),('lifecycle=154','lifecycle=158'),('clock_slot=74','clock_slot=71'),('holder=13','holder=6'),('attempts=8','attempts=9'),('precision=1','precision=2'),('runtime_mint=0','runtime_mint=1'),('incarnation=2','incarnation=1'),('slot=60','slot=060')]:
            self.assertIn(old,text)
            with self.subTest(old=old),self.assertRaises(ValueError):e.validate_inventory(text.replace(old,new,1),e.FUTURE_PLANS,require_version=True)
        with self.assertRaises(ValueError):e.validate_inventory(text,e.CURRENT_PLANS)
    def test_malformed_suffix_duplicate_fields_and_binding_before_grants(self):
        text=inventory()
        for changed in [text.replace('attempts=8','attempts=8 secret=99',1),text.replace('attempts=8','attempts=8 attempts=8',1),text.replace('slot=60','slot=60.0',1),text+'[boot.clock.source]\n']:
            with self.assertRaises(ValueError):e.validate_inventory(changed)
        rows=text.splitlines(True)
        with self.assertRaises(ValueError):e.validate_inventory(rows[-1]+''.join(rows[:-1]))
    def test_full_clock_stream_then_filter_all_four_held_clock_tuples(self):
        text=inventory(e.FUTURE_PLANS,True);t=e.MIN_SECONDS+100
        text+=clock(13,130,61,t,1)+success(15,150,1,t)
        text+=clock(6,6,61,t-50,2) # Separate service, no Net high-water update.
        text+=clock(13,130,71,t,3)+success(30,300,1,t)
        text+=clock(13,130,74,t+1,4)+success(31,310,1,t+1)
        inv=e.validate_inventory(text,e.FUTURE_PLANS,require_version=True)
        all_rows=e.validate_clock_stream(text,inv);self.assertEqual([r['ordinal']for r in all_rows],[1,2,3,4])
        selected=e.consumer_clock_sources(text,inv,30,300);self.assertEqual([r['ordinal']for r in selected],[3])
        for old,new in [('pd=6 task=6 slot=61 seconds','pd=6 task=6 slot=74 seconds'),('ordinal=2','ordinal=3'),('ordinal=4','ordinal=5'),('pd=13 task=130 slot=74 seconds','pd=13 task=13 slot=74 seconds')]:
            with self.subTest(old=old),self.assertRaises(ValueError):e.consumer_clock_sources(text.replace(old,new,1),inv,15,150)
    def test_global_net_highwater_poison_spans_retirement_and_consumers(self):
        t=e.MIN_SECONDS+100;text=inventory(e.FUTURE_PLANS)
        text+=clock(13,130,61,t,1)+success(15,150,1,t)
        text+=line('certificate.time.retired',target=15,task=150,before_cleanup_ack=1,high_water_retained=1)
        text+=clock(13,130,71,t-1,2)+rollback(30,300,1)
        text+=clock(6,6,61,t+5,3)
        inv=e.validate_inventory(text,e.FUTURE_PLANS);self.assertEqual(len(e.validate_clock_stream(text,inv)),3)
        with self.assertRaises(ValueError):e.validate_clock_stream(text+clock(13,130,74,t+5,4)+success(31,310,1,t+5),inv)
        with self.assertRaises(ValueError):e.validate_clock_stream(text.replace(rollback(30,300,1),success(30,300,1,t-1)),inv)
        with self.assertRaises(ValueError):e.validate_clock_stream(text.replace('owner=30 task=300','owner=31 task=310'),inv)
    def test_source_service_omission_reordering_and_foreign_effect_fail(self):
        t=e.MIN_SECONDS+1;base=inventory();inv=e.validate_inventory(base)
        for extra in [clock(13,130,61,t,1),success(15,150,1,t),clock(13,130,61,t,1)+rollback(15,150,1),clock(13,130,61,t,1)+success(15,15,1,t),clock(99,99,61,t,1),clock(13,130,61,t,0)]:
            with self.subTest(extra=extra),self.assertRaises(ValueError):e.validate_clock_stream(base+extra,inv)
    def test_entropy_sparse_global_counter_not_per_consumer_or_per_word(self):
        base=inventory(e.FUTURE_PLANS);inv=e.validate_inventory(base,e.FUTURE_PLANS)
        text=base+''.join(seed(slot,count)for slot,count in zip([60,70,73,60,73,70],[1,2,3,4,8,16]))
        self.assertEqual([r['count']for r in e.validate_entropy_stream(text,inv)],[1,2,3,4,8,16])
        for old,new in [('count=8','count=5'),('count=16','count=32'),('slot=73 source','slot=72 source'),('task=130 slot=70 source','task=13 slot=70 source')]:
            with self.subTest(old=old),self.assertRaises(ValueError):e.validate_entropy_stream(base+text[len(base):].replace(old,new,1),inv)
        with self.assertRaises(ValueError):e.validate_entropy_stream(base+seed(60,2),inv)
        with self.assertRaises(ValueError):e.validate_entropy_stream(text.replace(' private=1',' private=1 value=99',1),inv)
    def test_empty_runtime_streams_and_atomic_marker_midline(self):
        text=inventory();inv=e.validate_inventory(text)
        self.assertEqual(e.validate_clock_stream(text,inv),[]);self.assertEqual(e.validate_entropy_stream(text,inv),[])
        t=e.MIN_SECONDS;text+='older print interrupted '+clock(6,6,61,t,1)
        self.assertEqual(e.validate_clock_stream(text,inv)[0]['seconds'],t)

class BoundSourcePlans(unittest.TestCase):
    def setUp(self):
        self.repo=Path(__file__).resolve().parents[2]
        self.catalog=(self.repo/'crates/sex-pdx/src/app_catalog.rs').read_text()
        self.source=(self.repo/'crates/sex-pdx/src/browser_sources.rs').read_text()
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.directory=self.root/'crates/sex-pdx/src'
        self.directory.mkdir(parents=True)

    def read(self,catalog=None,source=None):
        (self.directory/'app_catalog.rs').write_text(self.catalog if catalog is None else catalog)
        (self.directory/'browser_sources.rs').write_text(self.source if source is None else source)
        return e.plans_from_catalog(self.root)

    def test_actual_three_and_original_only_archival_source_without_trace(self):
        self.assertEqual(e.plans_from_catalog(self.repo),e.FUTURE_PLANS)
        self.assertEqual(self.read(),e.FUTURE_PLANS)
        archived=''.join(row for row in self.catalog.splitlines(keepends=True)
                         if not re.search(r'Incarnation \{ app: App::Browser, ordinal: [12],',row))
        self.assertNotEqual(archived,self.catalog)
        self.assertEqual(self.read(catalog=archived),e.CURRENT_PLANS)

    def test_duplicate_inferred_or_unknown_source_selector_rejected_without_trace(self):
        cases=[('1=>[70,71,72],','1=>[70,71,72],\n        1=>[70,71,72],'),
               ('0=>[60,61,62],','0=>[60,60+1,62],'),
               ('2=>[73,74,75],','2=>[73,74,75],\n        3=>[76,77,78],'),
               ('0=>[60,61,62],','0=>[60,61,63],'),
               ('1=>[70,71,72],','1=>[60,71,72],'),
               ('2=>[73,74,75],','2=>[73,74,4294967371],')]
        for old,new in cases:
            self.assertIn(old,self.source)
            with self.subTest(old=old,new=new),self.assertRaises(ValueError):
                self.read(source=self.source.replace(old,new,1))

    def test_catalog_duplicate_inference_unknown_row_and_omission_rejected_without_trace(self):
        browser_rows=[row for row in self.catalog.splitlines(keepends=True)if 'Incarnation { app: App::Browser,'in row]
        self.assertEqual(len(browser_rows),3)
        changes=[self.catalog+browser_rows[0],
                 self.catalog.replace('surface: 343,','surface: 300+43,',1),
                 self.catalog.replace('ordinal: 2, target: 31,','ordinal: 3, target: 31,',1),
                 self.catalog.replace('target: 31, surface: 344,','target: 32, surface: 344,',1),
                 self.catalog.replace(browser_rows[1],'',1),
                 self.catalog.replace('lifecycle_slot: 158,','lifecycle_slot: 154,',1)]
        for changed in changes:
            self.assertNotEqual(changed,self.catalog)
            with self.subTest(catalog=changed[-80:]),self.assertRaises(ValueError):self.read(catalog=changed)

if __name__=='__main__':unittest.main()
