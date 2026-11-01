"""Full synthetic source inventories and existing joins; no native proof."""
import re
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import browser_source_evidence as evidence
from clock_source_evidence import BROWSER_GRANT,SILKBAR_GRANT,browser_clock_sources,validate_clock_sources
from test_browser_source_evidence import inventory,clock,success,rollback
from source_boot_fixture import complete


def source(pd=13,task=None,ordinal=1,seconds=1793404800,slot=61):
    task=pd if task is None else task
    return clock(pd,task,slot,seconds,ordinal).rstrip('\n')


def interleave(trace):
    """Complete current inventory; interleave only runtime clock records."""
    lines=[];ordinal=0
    for line in complete(trace).splitlines():
        if line.startswith('[clock.source.snapshot]'):
            seconds=int(re.search(r'seconds=(\d+)',line)[1]);ordinal+=1
            lines.append(source(6,ordinal=ordinal,seconds=seconds));ordinal+=1
            line=re.sub(r'ordinal=\d+',f'ordinal={ordinal}',line)
        lines.append(line)
    return '\n'.join(lines)+'\n'


class GlobalClockEvidence(unittest.TestCase):
    def current(self,extra=''):
        return inventory()+extra
    def test_preserves_offsets_and_three_numbered_groups_with_unequal_task(self):
        text=self.current(clock(6,6,61,1793404800,1)+clock(13,130,61,1793404800,2)+success(15,150,1,1793404800))
        rows=browser_clock_sources(text,plans=evidence.CURRENT_PLANS)
        self.assertEqual(rows[0].groups(),('1793404800','1','2'))
        self.assertEqual(text[rows[0].start():rows[0].end()],rows[0][0])
        self.assertEqual([m['pd']for m in validate_clock_sources(text,plans=evidence.CURRENT_PLANS)],['6','13'])
        with self.assertRaises(AssertionError):browser_clock_sources(text,15,15,plans=evidence.CURRENT_PLANS)
    def test_future_consumers_validate_global_before_selection(self):
        text=inventory(evidence.FUTURE_PLANS,True)
        for ordinal,(pd,task,slot)in enumerate([(15,150,61),(30,300,71),(31,310,74)],1):
            text+=clock(13,130,slot,1793404800,ordinal)+success(pd,task,1,1793404800)
        for pd,task,ordinal in[(15,150,1),(30,300,2),(31,310,3)]:
            self.assertEqual([int(m[3])for m in browser_clock_sources(text,pd,task,plans=evidence.FUTURE_PLANS,require_version=True)],[ordinal])
        for old,new in [('slot=74 seconds','slot=61 seconds'),('ordinal=3','ordinal=4'),('pd=13 task=130 slot=74 seconds','pd=99 task=130 slot=74 seconds')]:
            with self.subTest(old=old),self.assertRaises(AssertionError):browser_clock_sources(text.replace(old,new),plans=evidence.FUTURE_PLANS)
    def test_default_roster_comes_from_source_not_trace(self):
        root=Path(__file__).resolve().parents[2]
        plans=evidence.plans_from_catalog(root)
        self.assertEqual([p.target for p in plans],[15,30,31])
        text=inventory(plans)
        self.assertEqual(browser_clock_sources(text),[])
        # A complete but obsolete one-row trace cannot select its own roster.
        with self.assertRaises(AssertionError):browser_clock_sources(inventory(evidence.CURRENT_PLANS))
        frozen=Path('/explicit/frozen/source')
        with patch('clock_source_evidence.evidence.plans_from_catalog',return_value=evidence.CURRENT_PLANS)as reader:
            self.assertEqual(browser_clock_sources(inventory(evidence.CURRENT_PLANS),root=frozen),[])
            reader.assert_called_once_with(frozen)
        with patch('clock_source_evidence.evidence.plans_from_catalog',return_value=evidence.CURRENT_PLANS):
            with self.assertRaises(AssertionError):browser_clock_sources(text,root=frozen)
    def test_empty_stream_requires_complete_inventory(self):
        self.assertEqual(browser_clock_sources(inventory(),plans=evidence.CURRENT_PLANS),[])
        with self.assertRaises(AssertionError):browser_clock_sources('unrelated\n',plans=evidence.CURRENT_PLANS)
    def test_every_missing_duplicate_or_mutated_grant_fails(self):
        text=inventory();lines=text.splitlines(True)
        for i,line in enumerate(lines):
            if '[boot.browser.sources.readback]'in line:continue # archival optional
            with self.subTest(line=line):
                for bad in [''.join(lines[:i]+lines[i+1:]),text+line]:
                    with self.assertRaises(AssertionError):validate_clock_sources(bad,plans=evidence.CURRENT_PLANS)
        for old,new in [('consumer_task=150','consumer_task=15'),('slot=60','slot=61'),('profile=1 root_bytes','profile=2 root_bytes')]:
            with self.assertRaises(AssertionError):validate_clock_sources(text.replace(old,new,1),plans=evidence.CURRENT_PLANS)
    def test_atomic_midline_crlf_year_endpoints_and_malformed_nonselected(self):
        for seconds in(1577836800,4102444799):
            text=self.current('old print '+clock(6,6,61,seconds,1)).replace('\n','\r\n')
            self.assertEqual(len(validate_clock_sources(text,plans=evidence.CURRENT_PLANS)),1)
        text=self.current('old print '+clock(6,6,61,1793404800,1))
        for old,new in [('seconds=1793404800','seconds=-1'),('seconds=1793404800','seconds=١٧٩٣٤٠٤٨٠٠'),('attempts=1','attempts=5'),('ordinal=1','ordinal=1 extra=1'),('pd=6 task=6 slot=61 seconds','pd=6 task=13 slot=61 seconds'),('ordinal=1','ordinal=2')]:
            with self.subTest(old=old),self.assertRaises(AssertionError):browser_clock_sources(text.replace(old,new,1),plans=evidence.CURRENT_PLANS)
    def test_net_highwater_spans_consumer_and_survives_retirement(self):
        text=inventory(evidence.FUTURE_PLANS)+clock(13,130,61,1793404800,1)+success(15,150,1,1793404800)
        text+='[certificate.time.retired] target=15 task=150 before_cleanup_ack=1 high_water_retained=1\n'
        text+=clock(6,6,61,1793404700,2)+clock(13,130,71,1793404799,3)+rollback(30,300,1)
        self.assertEqual(len(validate_clock_sources(text,plans=evidence.FUTURE_PLANS)),3)
        bad=text+clock(13,130,74,1793404900,4)+success(31,310,1,1793404900)
        with self.assertRaises(AssertionError):browser_clock_sources(bad,15,plans=evidence.FUTURE_PLANS)


class ExistingCheckerJoins(unittest.TestCase):
    def setUp(self):
        self.plan_patch=patch('clock_source_evidence.evidence.plans_from_catalog',return_value=evidence.FUTURE_PLANS)
        self.plan_patch.start();self.addCleanup(self.plan_patch.stop)

    def test_prepare_exchange_document_import_asset_accept_interleaving(self):
        import test_tls_prepare_gate as prepare
        import test_tls_exchange_gate as exchange
        import test_https_document_gate as document
        import test_https_import_gate as imported
        import test_https_asset_gate as asset
        checks = [
            (prepare.TlsPreparationOracle().inspect, prepare.metadata()),
            (exchange.ControlledTlsOracle().inspect, exchange.metadata(exchange.capture())),
            (document.HTTPSAuthorityReceipts().check, document.metadata()),
            (imported.FullImportJoins().check, imported.metadata()[0]),
            (asset.AssetTrace().check, asset.metadata()[0]),
        ]
        for check, trace in checks:
            with self.subTest(check=check.__qualname__):
                result = check(interleave(trace))
                self.assertTrue(result['ok'], result)
                for changed in (interleave(trace).replace('pd=6 task=6', 'pd=6 task=13', 1),
                                re.sub(r'^.*\[boot.silkbar.clock.source\].*\n','',interleave(trace),count=1,flags=re.M)):
                    try:
                        result = check(changed)
                    except ValueError:  # Asset checker exposes failures as exceptions.
                        continue
                    self.assertFalse(result['ok'], result)

    def test_cold_import_recovery_validates_silkbar_stream_too(self):
        import test_https_import_gate as imported
        trace = imported.metadata(recovery=True)[0]
        mixed = interleave(trace+'\n'+source(6))
        check = imported.FullImportJoins().check
        self.assertTrue(check(mixed, recovery=True)['ok'])
        self.assertFalse(check(mixed.replace('pd=6 task=6', 'pd=6 task=13'), recovery=True)['ok'])

    def test_lifecycle_and_responsiveness_accept_interleaving(self):
        import test_tls_lifecycle_gate as lifecycle
        import test_tls_responsiveness_gate as responsive
        result = lifecycle.LifecycleOwnershipOracle().inspect(interleave(lifecycle.metadata()))
        self.assertTrue(result['ok'], result)
        trace, _ = responsive.source_trace()
        # source_trace is a fragment; its real caller supplies the BOOT grant.
        rows = responsive.gate.source_cycles(interleave(BROWSER_GRANT+'\n'+trace),
            {'pipe_bytes': 8248}, responsive.UTC-5, responsive.UTC+5)
        self.assertEqual(len(rows), 3)


if __name__ == '__main__':
    unittest.main()
