"""Synthetic global-source and real checker joins; no native guest proof."""
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from clock_source_evidence import (
    BROWSER_GRANT, SILKBAR_GRANT, browser_clock_sources, validate_clock_sources,
)


def source(pd=13, task=None, ordinal=1, seconds=1793404800):
    task = pd if task is None else task
    return (f'[clock.source.snapshot] pd={pd} task={task} slot=61 seconds={seconds} '
            f'attempts=1 precision=1 profile=q35_cmos_utc ordinal={ordinal}')


def interleave(trace):
    """Add real-shaped SilkBar samples without altering Browser service data."""
    lines = [SILKBAR_GRANT]
    ordinal = 0
    for line in trace.splitlines():
        if line.startswith('[clock.source.snapshot]'):
            seconds = int(re.search(r'seconds=(\d+)', line)[1])
            ordinal += 1
            lines.append(source(6, ordinal=ordinal, seconds=seconds))
            ordinal += 1
            line = re.sub(r'ordinal=\d+', f'ordinal={ordinal}', line)
        lines.append(line)
    return '\n'.join(lines) + '\n'


class GlobalClockEvidence(unittest.TestCase):
    def test_browser_only_and_interleaved_offsets(self):
        text = '\n'.join([BROWSER_GRANT, source(), 'service', source(ordinal=2)])
        self.assertEqual([int(m[3]) for m in browser_clock_sources(text)], [1, 2])
        mixed = interleave(text)
        self.assertEqual([m['pd'] for m in validate_clock_sources(mixed)], ['6', '13', '6', '13'])
        rows = browser_clock_sources(mixed)
        self.assertEqual([int(m[3]) for m in rows], [2, 4])
        self.assertEqual(mixed[rows[0].start():rows[0].end()], rows[0][0])

    def test_allowed_year_endpoints_and_crlf(self):
        for seconds in (1577836800, 4102444799):
            self.assertEqual(len(validate_clock_sources(BROWSER_GRANT + '\r\n' + source(seconds=seconds) + '\r\n')), 1)

    def test_unknown_holder_cross_pair_and_foreign_slot_fail(self):
        for row in (source(7), source(6, 13), source(13, 6), source().replace('slot=61', 'slot=60')):
            with self.subTest(row=row), self.assertRaises(AssertionError):
                validate_clock_sources(BROWSER_GRANT + '\n' + SILKBAR_GRANT + '\n' + row)

    def test_global_gaps_duplicates_and_reordering_fail(self):
        for ordinals in ((2,), (1, 1), (1, 3), (2, 1)):
            text = BROWSER_GRANT + '\n' + SILKBAR_GRANT + '\n' + '\n'.join(
                source(13 if i % 2 else 6, ordinal=n) for i, n in enumerate(ordinals))
            with self.subTest(ordinals=ordinals), self.assertRaises(AssertionError):
                browser_clock_sources(text)

    def test_missing_duplicate_wrong_and_late_grants_fail(self):
        for grant, row in ((BROWSER_GRANT, source()), (SILKBAR_GRANT, source(6))):
            for text in (row, grant+'\n'+grant+'\n'+row, grant.replace('runtime_mint=0','runtime_mint=1')+'\n'+row,
                         row+'\n'+grant, grant+' extra=1\n'+row):
                with self.subTest(text=text), self.assertRaises(AssertionError):
                    validate_clock_sources(text)

    def test_every_malformed_sample_is_rejected_even_when_not_browser(self):
        row = source(6)
        mutations = [('seconds=1793404800', 'seconds=-1'), ('seconds=1793404800', 'seconds=1577836799'),
                     ('seconds=1793404800', 'seconds=4102444800'), ('attempts=1', 'attempts=0'),
                     ('attempts=1', 'attempts=5'), ('precision=1', 'precision=2'),
                     ('profile=q35_cmos_utc', 'profile=host'), ('ordinal=1', 'ordinal=1 extra=1'),
                     ('ordinal=1', 'ordinal=NaN'), ('seconds=1793404800', 'seconds=١٧٩٣٤٠٤٨٠٠')]
        for old, new in mutations:
            with self.subTest(new=new), self.assertRaises(AssertionError):
                browser_clock_sources(SILKBAR_GRANT+'\n'+row.replace(old, new))

    def test_empty_stream_has_no_synthetic_samples(self):
        self.assertEqual(browser_clock_sources('unrelated\n'), [])

    def test_complete_atomic_record_can_interrupt_legacy_user_print(self):
        # Observed guest shape: an atomic kernel record interrupts an unfinished
        # sexdrive lba= print, then terminates its own record with a newline.
        prefix = '[sexdrive.nvme.submit.detail] path=typed op=WRITE cid=1297 lba='
        text = SILKBAR_GRANT+'\n'+BROWSER_GRANT+'\n'+prefix+source(6)+'\n'+prefix+source(ordinal=2)+'\n'
        self.assertEqual([m['pd'] for m in validate_clock_sources(text)], ['6', '13'])
        browser = browser_clock_sources(text)
        self.assertEqual(len(browser), 1)
        self.assertEqual(browser[0].start(), text.index('[clock.source.snapshot] pd=13'))
        for bad in (text.replace('ordinal=1\n', 'ordinal=1 trailing\n'),
                    text.replace('ordinal=1\n', '\n'),
                    text.replace('pd=6 task=6', 'pd=7 task=7'),
                    text.replace('ordinal=2', 'ordinal=1')):
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                browser_clock_sources(bad)


class ExistingCheckerJoins(unittest.TestCase):
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
                                interleave(trace).replace(SILKBAR_GRANT+'\n', '', 1)):
                    try:
                        result = check(changed)
                    except ValueError:  # Asset checker exposes failures as exceptions.
                        continue
                    self.assertFalse(result['ok'], result)

    def test_cold_import_recovery_validates_silkbar_stream_too(self):
        import test_https_import_gate as imported
        trace = imported.metadata(recovery=True)[0]
        mixed = SILKBAR_GRANT+'\n'+trace+'\n'+source(6)
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
