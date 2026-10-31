"""Validate the complete global q35 UTC source stream before consumer joins.

No guest authority is inferred from a filtered subset. Browser and SilkBar reads
share one kernel ordinal; their exact immutable BOOT grants remain distinct.
"""
import re

SOURCE = '[clock.source.snapshot]'
BROWSER_GRANT = '[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0'
SILKBAR_GRANT = '[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0'
_SOURCE = re.compile(
    r'\[clock.source.snapshot\] pd=(?P<pd>[0-9]+) task=(?P<task>[0-9]+) slot=61 '
    r'seconds=(?P<seconds>[0-9]+) attempts=(?P<attempts>[1-4]) precision=1 '
    r'profile=q35_cmos_utc ordinal=(?P<ordinal>[0-9]+)\r?$', re.MULTILINE)
_BROWSER = re.compile(
    r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=([0-9]+) '
    r'attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=([0-9]+)\r?$', re.MULTILINE)


def validate_clock_sources(trace):
    """Return named regex matches, or raise AssertionError on any bad evidence.

    Atomic kernel records may interrupt an older non-atomic user print, so a
    marker may start mid-line. Its complete payload must still end the line;
    prefix counts reject truncated, duplicate, or suffix-corrupted records.
    Matches retain original trace offsets. Empty streams are allowed: callers
    enforce their own required sample count (including unavailable profiles).
    """
    records = list(_SOURCE.finditer(trace))
    assert len(records) == trace.count(SOURCE), 'clock source: malformed snapshot marker'
    for ordinal, record in enumerate(records, 1):
        pair = (record['pd'], record['task'])
        assert pair in (('13', '13'), ('6', '6')), 'clock source: foreign holder/task pair'
        assert 1577836800 <= int(record['seconds']) < 4102444800, 'clock source: UTC outside 2020..2099'
        assert int(record['ordinal']) == ordinal, 'clock source: global ordinal skipped/reused/reordered'
    for holder, grant in (('13', BROWSER_GRANT), ('6', SILKBAR_GRANT)):
        prefix = grant.split(']')[0] + ']'
        held = [record for record in records if record['pd'] == holder]
        grants = list(re.finditer('^' + re.escape(grant) + r'\r?$', trace, re.MULTILINE))
        if held or prefix in trace:
            assert len(grants) == trace.count(prefix) == 1, 'clock source: missing/foreign/duplicate BOOT grant for holder ' + holder
            assert not held or grants[0].start() < held[0].start(), 'clock source: snapshot precedes BOOT grant'
    return records


def browser_clock_sources(trace):
    """Validated Browser matches: [1] seconds, [2] attempts, [3] global ordinal."""
    validate_clock_sources(trace)
    return list(_BROWSER.finditer(trace))
