"""Validate all BOOT source identities and global UTC records before selection.

Plans come from the frozen source tree (or an explicit host fixture), never from
observed logs. Net high-water is service-global; SilkBar shares kernel ordinals
but has its own service. Returned regex matches retain original trace offsets.
"""
import re
from pathlib import Path
import browser_source_evidence as evidence

SOURCE = '[clock.source.snapshot]'
# Archival record constants; these do not select current authority or task IDs.
BROWSER_GRANT = '[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0'
SILKBAR_GRANT = '[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0'
_SOURCE = re.compile(
    r'\[clock.source.snapshot\] pd=(?P<pd>[0-9]+) task=(?P<task>[0-9]+) slot=(?P<slot>[0-9]+) '
    r'seconds=(?P<seconds>[0-9]+) attempts=(?P<attempts>[1-4]) precision=1 '
    r'profile=q35_cmos_utc ordinal=(?P<ordinal>[0-9]+)\r?$', re.MULTILINE)


def _validated(trace,root,plans,require_version):
    try:
        selected=tuple(plans)if plans is not None else evidence.plans_from_catalog(
            Path(root)if root is not None else Path(__file__).resolve().parents[1])
        inventory=evidence.validate_inventory(trace,selected,require_version=require_version)
        evidence.validate_clock_stream(trace,inventory)
        matches=list(_SOURCE.finditer(trace))
        if len(matches)!=trace.count(SOURCE):raise ValueError('malformed snapshot marker')
        return inventory,matches
    except (ValueError,KeyError)as exc:
        raise AssertionError('clock source: '+str(exc))from exc


def validate_clock_sources(trace,*,root=None,plans=None,require_version=False):
    """Named matches after complete inventory/global ordinal/service checks.

    root identifies the frozen image's source tree. Explicit plans are only for
    source-bound callers and synthetic tests. Empty sample streams still require
    a complete BOOT inventory. Old diagnostics can leave trust version unobserved.
    """
    return _validated(trace,root,plans,require_version)[1]


def browser_clock_sources(trace,consumer_pd=15,consumer_task=None,*,root=None,plans=None,require_version=False):
    """Selected consumer matches: [1] seconds, [2] attempts, [3] global ordinal.

    Selection happens only after validating every holder/slot and service event.
    Omitted task resolves from exact validated BOOT identity, never task==PD.
    """
    inventory,_=_validated(trace,root,plans,require_version)
    row=inventory['roster'].get(consumer_pd)
    if row is None:raise AssertionError('clock source: consumer absent from source-bound roster')
    task=row['task']
    if consumer_task is not None and consumer_task!=task:
        raise AssertionError('clock source: selected task differs from held identity')
    holder,holder_task=inventory['holder'];slot=row['plan'].clock
    pattern=re.compile(
        rf'\[clock.source.snapshot\] pd={holder} task={holder_task} slot={slot} seconds=([0-9]+) '
        r'attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=([0-9]+)\r?$',re.MULTILINE)
    return list(pattern.finditer(trace))
