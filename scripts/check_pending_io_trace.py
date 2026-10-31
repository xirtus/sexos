#!/usr/bin/env python3
"""Read-only proof of one Quil9 Close overlapping a real journal WRITE.

Profile prerequisites, checked by the owning native runner: one NVMe controller
with namespace1/512-byte LBAs and SQ1->CQ1, one active UART already configured
with DLAB clear, and a complete critical-phase log with exactly these four
events enabled. Extent arguments are evidence geometry, never guest authority.

Formats: installed QEMU11.0.2 share/qemu/trace-events-all. Versioned QEMU
hw/nvme/ctrl.c emits actual block count/mapped bytes in pci_nvme_write, and
traces enqueue_req_completion BEFORE enqueue/publication. File order supplies
the conservative completion bound; host timestamps are not globally sorted.
The observed -msg timestamp=on ISO8601Z format and the documented TID@epoch
format are distinct supported profiles; mixing them in one stream fails.
The matching completion must precede the first byte of the exact zeroed-pool
receipt. This does not prove durability, rollback, or all five cleanup roles.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import pathlib
import re

MAX_TRACE_BYTES = 16 * 1024 * 1024
MAX_EVENTS = 200_000
MAX_UART_BYTES = 128 * 1024
MAX_LINE_BYTES = 512
MAX_EXTENTS = 8
SECTOR_BYTES = 512
WRITE_NAME = 'NVME_NVM_CMD_WRITE'

PREFIX = re.compile(
    rb'(?P<tid>[0-9]{1,20})@(?P<seconds>[0-9]{1,20})\.'
    rb'(?P<microseconds>[0-9]{6}):(?P<name>[a-z_]{1,64}) (?P<payload>[^\r\n]*)\n')
ISO_PREFIX = re.compile(
    rb'(?P<timestamp>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z) '
    rb'(?P<name>[a-z_]{1,64}) (?P<payload>[^\r\n]*)\n')
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
FORMATS = {
    'serial_write': re.compile(rb'\[0x([0-9a-f]{2})\] <- 0x([0-9a-f]{2})'),
    'pci_nvme_io_cmd': re.compile(
        rb"cid ([0-9]+) nsid 0x([0-9a-f]+) sqid ([0-9]+) opc 0x([0-9a-f]+) opname '([A-Z0-9_]{1,64})'"),
    'pci_nvme_write': re.compile(
        rb"cid ([0-9]+) opname '([A-Z0-9_]{1,64})' nsid ([0-9]+) nlb ([0-9]+) count ([0-9]+) lba 0x([0-9a-f]+)"),
    'pci_nvme_enqueue_req_completion': re.compile(
        rb'cid ([0-9]+) cqid ([0-9]+) dw0 0x([0-9a-f]+) dw1 0x([0-9a-f]+) status 0x([0-9a-f]+)'),
}


def _number(raw, bits, base=10):
    value = int(raw, base)
    if not 0 <= value < 1 << bits:
        raise ValueError('trace integer exceeds u' + str(bits))
    return value


def _ranges(values):
    if not isinstance(values, (list, tuple)) or not 1 <= len(values) <= MAX_EXTENTS:
        raise ValueError('one to eight expected journal extent ranges required')
    ranges = []
    for value in values:
        if (not isinstance(value, (list, tuple)) or len(value) != 2
                or any(type(v) is not int for v in value)
                or not 0 <= value[0] < value[1] <= 1 << 64):
            raise ValueError('extent must be a nonempty half-open u64 LBA range')
        ranges.append(tuple(value))
    ranges.sort()
    if any(a[1] > b[0] for a, b in zip(ranges, ranges[1:])):
        raise ValueError('expected journal extent ranges overlap')
    return ranges


def _events(raw):
    if not isinstance(raw, bytes) or not 0 < len(raw) <= MAX_TRACE_BYTES:
        raise ValueError('trace must be nonempty bounded bytes')
    if not raw.endswith(b'\n'):
        raise ValueError('truncated final trace record')
    offset = 0
    events = []
    stream_format = None
    for line in raw.splitlines(keepends=True):
        if len(line) > MAX_LINE_BYTES or len(events) >= MAX_EVENTS:
            raise ValueError('trace line/event bound exceeded')
        match = PREFIX.fullmatch(line)
        timestamp_format = 'tid_epoch'
        if not match:
            match = ISO_PREFIX.fullmatch(line)
            timestamp_format = 'iso8601z'
        if not match:
            raise ValueError('malformed timestamped trace record at byte ' + str(offset))
        if stream_format is not None and timestamp_format != stream_format:
            raise ValueError('mixed trace timestamp formats at byte ' + str(offset))
        stream_format = timestamp_format
        if timestamp_format == 'tid_epoch':
            metadata = dict(tid=_number(match['tid'], 64), seconds=_number(match['seconds'], 64),
                            microseconds=int(match['microseconds']),
                            timestamp=(match['seconds'] + b'.' + match['microseconds']).decode('ascii'))
        else:
            stamp = match['timestamp']
            try:
                moment = datetime(int(stamp[:4]), int(stamp[5:7]), int(stamp[8:10]),
                                  int(stamp[11:13]), int(stamp[14:16]), int(stamp[17:19]),
                                  tzinfo=timezone.utc)
            except ValueError:
                raise ValueError('invalid ISO8601Z timestamp at byte ' + str(offset)) from None
            elapsed = moment - EPOCH
            metadata = dict(tid=None, seconds=elapsed.days * 86400 + elapsed.seconds,
                            microseconds=int(stamp[20:26]), timestamp=stamp.decode('ascii'))
        name = match['name'].decode('ascii')
        pattern = FORMATS.get(name)
        fields = pattern.fullmatch(match['payload']) if pattern else None
        if not fields:
            raise ValueError('unexpected/malformed trace event at byte ' + str(offset))
        event = dict(index=len(events), offset=offset, end=offset + len(line), name=name,
                     timestamp_format=timestamp_format, fields=fields.groups(), **metadata)
        events.append(event)
        offset += len(line)
    return events


def _point(event):
    return {k: event[k] for k in ('index', 'offset', 'end', 'tid', 'seconds', 'microseconds',
                                'timestamp_format', 'timestamp')}


def _state(raw):
    """Strict complete EVENT records; UART edges and commands may be pending."""
    events = _events(raw)
    commands = {}
    uart = bytearray()
    uart_events = []
    for event in events:
        fields = event['fields']
        if event['name'] == 'serial_write':
            address, value = (int(v, 16) for v in fields)
            if address > 7 or address == 3 and value & 0x80:
                raise ValueError('UART address/DLAB configuration invalid for this profile')
            if address == 0:
                if len(uart) >= MAX_UART_BYTES:
                    raise ValueError('reconstructed UART byte bound exceeded')
                uart.append(value)
                uart_events.append(event)
        elif event['name'] == 'pci_nvme_io_cmd':
            cid = _number(fields[0], 16)
            nsid, sqid = _number(fields[1], 32, 16), _number(fields[2], 16)
            opcode = _number(fields[3], 8, 16)
            opname = fields[4].decode('ascii')
            names = {0: 'NVME_NVM_CMD_FLUSH', 1: WRITE_NAME, 2: 'NVME_NVM_CMD_READ'}
            if sqid != 1 or nsid != 1 or names.get(opcode) != opname:
                raise ValueError('unsupported namespace/queue/opcode in single-controller profile')
            if cid in commands:
                raise ValueError('ambiguous/reused command CID ' + str(cid))
            commands[cid] = dict(cid=cid, nsid=nsid, sqid=sqid, opcode=opcode,
                                 command=event, write=None, completion=None)
        elif event['name'] == 'pci_nvme_write':
            cid = _number(fields[0], 16)
            name = fields[1].decode('ascii')
            nsid, blocks = _number(fields[2], 32), _number(fields[3], 32)
            count, lba = _number(fields[4], 64), _number(fields[5], 64, 16)
            command = commands.get(cid)
            if (not command or command['opcode'] != 1 or nsid != command['nsid']
                    or name != WRITE_NAME or command['write'] is not None
                    or command['completion'] is not None):
                raise ValueError('WRITE lacks one exact preceding live command')
            if not 1 <= blocks <= 65536 or count != blocks * SECTOR_BYTES or lba + blocks > 1 << 64:
                raise ValueError('WRITE block/byte count or LBA interval invalid')
            command['write'] = dict(event=event, lba=lba, end_lba=lba + blocks,
                                    blocks=blocks, count=count)
        else:
            cid, cqid = _number(fields[0], 16), _number(fields[1], 16)
            dw0, dw1 = _number(fields[2], 32, 16), _number(fields[3], 32, 16)
            status = _number(fields[4], 16, 16)
            command = commands.get(cid)
            if not command or command['completion'] is not None or cqid != 1:
                raise ValueError('CQE lacks one exact live SQ1/CQ1 command')
            if status != 0 or command['opcode'] == 1 and command['write'] is None:
                raise ValueError('unsuccessful CQE or missing WRITE geometry')
            command['completion'] = dict(event=event, cqid=cqid, dw0=dw0, dw1=dw1, status=status)
    return events, commands, bytes(uart), uart_events


def outstanding_target_writes(raw_complete_event_prefix, journal_ranges):
    """Provisional live observations only, NEVER qualified final evidence.

    The caller supplies a complete-event prefix from one continuous enable
    interval. A positive observation can become stale immediately; only the
    eventual check_trace result proves ordering through the terminal receipt.
    Commands/UART may be incomplete here, but malformed event records fail.
    """
    ranges = _ranges(journal_ranges)
    if isinstance(raw_complete_event_prefix, bytes) and not raw_complete_event_prefix:
        return []
    _, commands, _, _ = _state(raw_complete_event_prefix)
    candidates = []
    for command in commands.values():
        write = command['write']
        if write is None or command['completion'] is not None:
            continue
        owned = [r for r in ranges if r[0] <= write['lba'] and write['end_lba'] <= r[1]]
        if owned:
            candidates.append(dict(provisional=True, qualified=False, cid=command['cid'],
                nsid=command['nsid'], sqid=command['sqid'], command=_point(command['command']),
                write=dict(event=_point(write['event']), lba=write['lba'], end_lba=write['end_lba'],
                           blocks=write['blocks'], count=write['count'], matched_range=list(owned[0])),
                trace_prefix_bytes=len(raw_complete_event_prefix),
                trace_prefix_sha256=hashlib.sha256(raw_complete_event_prefix).hexdigest()))
    return candidates


def check_trace(raw, journal_ranges):
    """Return an exact bounded proof or raise ValueError; never start a guest.

    Ranges are half-open LBAs. Every command must have one completion within
    the supplied stream. Any CID reuse is rejected, even after completion.
    Peripheral UART text may start/end mid-record (trace toggled outside the
    decisive interval); no critical receipt may be partial or spliced.
    The owning runner must seal one continuous critical enable interval.
    """
    ranges = _ranges(journal_ranges)
    events, commands, uart, uart_events = _state(raw)
    if not commands or any(c['completion'] is None for c in commands.values()):
        raise ValueError('missing/truncated command lifecycle')
    if not uart:
        raise ValueError('missing UART records')

    def records(tag, pattern):
        found = list(re.finditer(pattern, uart, re.MULTILINE))
        if len(found) != uart.count(tag):
            raise ValueError('malformed or incomplete UART ' + tag.decode('ascii'))
        return found

    closes = records(b'[quil.document.close]', rb'^\[quil.document.close\] pending=([01])\r?\n')
    finishes = records(b'[task.finish]', rb'^\[task.finish\] pd=([0-9]+) task=([0-9]+) reason=([0-9]+) terminal=([01])\r?\n')
    pools = records(b'[sext.app.pool.ready]',
                    rb'^\[sext.app.pool.ready\] target=([0-9]+) task=([0-9]+) bytes=([0-9]+) zeroed=([0-9]+) workspace_records=([0-9]+) reused=([01]) zero_owner=([a-z]{1,32}) ok=([01])\r?\n')
    identities = [(m, _number(m[1], 64), _number(m[2], 64), _number(m[3], 64)) for m in finishes]
    exact = [m for m, pd, task, reason in identities if pd == 9 and task == 9 and reason == 0 and m[4] == b'1']
    if len(closes) != 1 or closes[0][1] != b'1' or len(exact) != 1:
        raise ValueError('one exact pending Quil Close and terminal task9 receipt required')
    close, finish = closes[0], exact[0]
    if not close.end() <= finish.start():
        raise ValueError('Quil Close precedes no exact task9 finish')
    if any(m is not finish and (m[1] == b'9' or m[2] == b'9') for m in finishes):
        raise ValueError('conflicting task9 terminal identity')
    pool_subjects = [(m, *(_number(m[i], 64)for i in range(1, 6)))for m in pools]
    exact_pools = [(m, size, zeroed, workspace)for m, pd, task, size, zeroed, workspace in pool_subjects
                   if pd == 9 and task == 9]
    if len(exact_pools) != 1 or any((pd == 9 or task == 9) and (pd, task) != (9, 9)
                                  for _, pd, task, _, _, _ in pool_subjects):
        raise ValueError('one exact nonconflicting task9 pool-ready receipt required')
    pool, size, zeroed, workspace = exact_pools[0]
    if size == 0 or size % 4096 or size != zeroed or pool[6] != b'1' or pool[7] != b'sext' or pool[8] != b'1':
        raise ValueError('pool-ready bytes must be nonzero page-aligned, fully zeroed, and reusable by sext')
    if not finish.end() <= pool.start():
        raise ValueError('pool reuse precedes the exact terminal task9 receipt')

    def receipt(match):
        first, last = uart_events[match.start()], uart_events[match.end() - 1]
        return dict(uart_start=match.start(), uart_end=match.end(),
                    first=_point(first), last=_point(last))

    close_at, finish_at, pool_at = receipt(close), receipt(finish), receipt(pool)
    candidates = []
    for command in commands.values():
        write = command['write']
        if write is None:
            continue
        owned = [r for r in ranges if r[0] <= write['lba'] and write['end_lba'] <= r[1]]
        if (owned and command['command']['index'] < write['event']['index']
                < close_at['first']['index'] <= close_at['last']['index']
                < finish_at['first']['index'] <= finish_at['last']['index']
                < command['completion']['event']['index'] < pool_at['first']['index']):
            candidates.append((command, owned[0]))
    if len(candidates) != 1:
        raise ValueError('exactly one owned WRITE must span task9 termination and complete before pool reuse')
    command, owned = candidates[0]
    write, completion = command['write'], command['completion']
    excerpt = uart[close.start():pool.end()]
    try:
        excerpt_text = excerpt.decode('utf8')
    except UnicodeDecodeError as error:
        raise ValueError('invalid UTF8 in terminal UART excerpt at byte ' + str(error.start)) from None
    excerpt_events = uart_events[close.start():pool.end()]
    char_offsets = []
    byte = 0
    for char in excerpt_text:
        char_offsets.append(excerpt_events[byte]['offset'])
        byte += len(char.encode('utf8'))
    return dict(ok=True, evidence_version=2, format='QEMU11.0.2/log/' + events[0]['timestamp_format'],
                order='exact file/event order; timestamps retained without sorting',
                trace_sha256=hashlib.sha256(raw).hexdigest(), trace_bytes=len(raw),
                events=len(events), uart_bytes=len(uart), journal_ranges=[list(r)for r in ranges],
                cid=command['cid'], nsid=1, sqid=1, command=_point(command['command']),
                write=dict(event=_point(write['event']), lba=write['lba'], end_lba=write['end_lba'],
                           blocks=write['blocks'], count=write['count'], matched_range=list(owned)),
                close=close_at, finish=finish_at,
                pool_ready=dict(**pool_at, target=9, task=9, bytes=size, zeroed=zeroed,
                                workspace_records=workspace, reused=1, zero_owner='sext', ok=1),
                completion=dict(event=_point(completion['event']), cqid=1, dw0=completion['dw0'],
                                dw1=completion['dw1'], status=0),
                uart=dict(text=excerpt_text, raw_hex=excerpt.hex(),
                          byte_trace_offsets=[e['offset']for e in excerpt_events],
                          character_trace_offsets=char_offsets),
                scope='real journal WRITE spans Quil9 termination, completes before zeroed pool reuse; no durability, rollback, or full cleanup claim')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=pathlib.Path)
    parser.add_argument('--range', dest='ranges', action='append', required=True,
                        help='half-open 512-byte-sector LBA START:END (decimal or0xhex)')
    args = parser.parse_args()
    try:
        ranges = [tuple(int(v, 0)for v in item.split(':'))for item in args.ranges]
        with args.trace.open('rb')as stream:
            raw = stream.read(MAX_TRACE_BYTES + 1)
        result = check_trace(raw, ranges)
    except (OSError, ValueError) as error:
        result = dict(ok=False, errors=[str(error)])
    print(json.dumps(result, indent=2))
    return not result['ok']


if __name__ == '__main__':
    raise SystemExit(main())
