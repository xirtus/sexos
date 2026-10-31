"""Actual QEMU11 log-format fixtures; no guest or shell execution."""
import pathlib
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import check_pending_io_trace as oracle

RANGES = [(0x1000, 0x1080)]
# Exact retained prototype-A UART lines. A stopped before any NVMe event;
# this fixture must parse, but can never become positive device evidence.
NATIVE_A_UART = (
    b'2026-10-31T17:37:53.107345Z serial_write [0x00] <- 0x30\n'
    b'2026-10-31T17:37:53.107757Z serial_write [0x00] <- 0x78\n'
    b'2026-10-31T17:37:53.108043Z serial_write [0x00] <- 0x31\n'
    b'2026-10-31T17:37:53.108055Z serial_write [0x00] <- 0x35\n')


def iso_trace(raw):
    # Preserve exact event payloads/order; use the observed A timestamp shape.
    return b''.join(b'2026-10-31T17:37:53.' + line.split(b'.', 1)[1].split(b':', 1)[0]
                    + b'Z ' + line.split(b':', 1)[1] for line in raw.splitlines(keepends=True))


class Trace:
    def __init__(self):
        self.lines = []

    def event(self, name, body, *, tid=17, seconds=1761955200):
        self.lines.append(f'{tid}@{seconds}.{len(self.lines):06d}:{name} {body}\n'.encode('ascii'))

    def uart(self, text):
        for value in text.encode('utf8'):
            self.event('serial_write', f'[0x00] <- 0x{value:02x}', tid=23)

    def command(self, cid=7, *, opcode=1, name='NVME_NVM_CMD_WRITE', nsid=1, sqid=1):
        self.event('pci_nvme_io_cmd', f"cid {cid} nsid 0x{nsid:x} sqid {sqid} opc 0x{opcode:x} opname '{name}'")

    def write(self, cid=7, *, lba=0x1020, blocks=8, count=None, name='NVME_NVM_CMD_WRITE', nsid=1):
        if count is None:
            count = blocks * 512
        self.event('pci_nvme_write', f"cid {cid} opname '{name}' nsid {nsid} nlb {blocks} count {count} lba 0x{lba:x}")

    def completion(self, cid=7, *, cqid=1, status=0):
        self.event('pci_nvme_enqueue_req_completion', f'cid {cid} cqid {cqid} dw0 0x0 dw1 0x0 status 0x{status:x}', tid=29)

    def close(self, pending=1):
        self.uart(f'[quil.document.close] pending={pending}\r\n')

    def finish(self, pd=9, task=9, reason=0, terminal=1):
        self.uart(f'[task.finish] pd={pd} task={task} reason={reason} terminal={terminal}\r\n')

    def pool(self, target=9, task=9, size=286720, zeroed=None, workspace=2, reused=1, owner='sext', ok=1):
        if zeroed is None:zeroed=size
        self.uart(f'[sext.app.pool.ready] target={target} task={task} bytes={size} zeroed={zeroed} workspace_records={workspace} reused={reused} zero_owner={owner} ok={ok}\r\n')

    def raw(self):
        return b''.join(self.lines)


def fixture(*, pending=1, lba=0x1020, blocks=8, with_pool=True, pool_kwargs=None):
    trace = Trace()
    trace.uart('[fixture.begin] normal_uart=1\r\n')
    trace.command()
    trace.write(lba=lba, blocks=blocks)
    trace.close(pending)
    trace.uart('[quil.clipboard.closed] pending=0 payload_erased=1 selection_cleared=1\r\n')
    trace.finish()
    trace.completion()
    if with_pool:trace.pool(**(pool_kwargs or {}))
    return trace


class PendingIoTraceTests(unittest.TestCase):
    def reject(self, trace, ranges=RANGES):
        raw = trace.raw() if isinstance(trace, Trace) else trace
        with self.assertRaises(ValueError):
            oracle.check_trace(raw, ranges)

    def test_real_write_spans_close_and_exact_terminal_receipt(self):
        trace = fixture()
        result = oracle.check_trace(trace.raw(), RANGES)
        self.assertTrue(result['ok'])
        self.assertEqual((result['cid'], result['nsid'], result['sqid']), (7, 1, 1))
        self.assertEqual((result['write']['lba'], result['write']['blocks'], result['write']['count']), (0x1020, 8, 4096))
        self.assertEqual(result['write']['matched_range'], [0x1000, 0x1080])
        self.assertEqual(result['completion']['status'], 0)
        self.assertEqual(result['evidence_version'], 2)
        self.assertEqual(result['pool_ready']['bytes'], 286720)
        self.assertEqual(result['pool_ready']['zeroed'], 286720)
        self.assertIn('[sext.app.pool.ready]', result['uart']['text'])
        self.assertLess(result['completion']['event']['index'], result['pool_ready']['first']['index'])
        self.assertLess(result['write']['event']['index'], result['close']['first']['index'])
        self.assertLess(result['finish']['last']['index'], result['completion']['event']['index'])
        for offset in result['uart']['byte_trace_offsets']:
            self.assertIn(b':serial_write [0x00] <- ', trace.raw()[offset:trace.raw().find(b'\n', offset)])
        self.assertEqual(bytes.fromhex(result['uart']['raw_hex']).decode(), result['uart']['text'])
        self.assertEqual(len(result['uart']['byte_trace_offsets']), result['pool_ready']['uart_end'] - result['close']['uart_start'])

    def test_pool_reuse_requires_exact_nonzero_page_aligned_zeroed_private_bytes(self):
        for kwargs in [dict(size=0), dict(size=286721), dict(zeroed=282624),
                       dict(size=1 << 64), dict(zeroed=1 << 64), dict(workspace=1 << 64),
                       dict(reused=0), dict(owner='other'), dict(ok=0)]:
            self.reject(fixture(pool_kwargs=kwargs))

    def test_missing_duplicate_conflicting_or_partial_pool_receipts_reject(self):
        self.reject(fixture(with_pool=False))
        trace = fixture();trace.pool()
        self.reject(trace)
        for kwargs in [dict(target=16), dict(task=16)]:
            self.reject(fixture(pool_kwargs=kwargs))
            trace = fixture();trace.pool(**kwargs)
            self.reject(trace)
        trace = fixture(with_pool=False);trace.uart('[sext.app.pool.ready] target=9 task=9 bytes=286720')
        self.reject(trace)

    def test_completion_must_precede_first_pool_character_not_merely_end(self):
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.finish();trace.pool();trace.completion()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.finish()
        trace.uart('[sext.app.pool.ready] target=9 ');trace.completion()
        trace.uart('task=9 bytes=286720 zeroed=286720 workspace_records=2 reused=1 zero_owner=sext ok=1\n')
        self.reject(trace)

    def test_foreign_pool_receipt_cannot_substitute_for_target9(self):
        self.reject(fixture(pool_kwargs=dict(target=16, task=16)))
        trace = fixture();trace.pool(target=16, task=16)
        self.assertEqual(oracle.check_trace(trace.raw(), RANGES)['pool_ready']['target'], 9)

    def test_app_pending_only_has_no_device_proof(self):
        trace = Trace();trace.uart('[fixture.begin]\n');trace.close();trace.finish()
        self.reject(trace)

    def test_normal_journal_reads_and_flushes_can_surround_owned_write(self):
        trace = Trace();trace.uart('[fixture.begin]\n')
        trace.command(6, opcode=2, name='NVME_NVM_CMD_READ');trace.completion(6)
        trace.command();trace.write();trace.close();trace.finish();trace.completion()
        trace.command(8, opcode=0, name='NVME_NVM_CMD_FLUSH');trace.completion(8);trace.pool()
        self.assertEqual(oracle.check_trace(trace.raw(), RANGES)['cid'], 7)

    def test_completed_save_close_does_not_become_pending_teardown(self):
        self.reject(fixture(pending=0))

    def test_wrong_object_or_cross_extent_boundary_is_not_owned(self):
        self.reject(fixture(lba=0x2000))
        self.reject(fixture(lba=0x107f, blocks=2))
        self.reject(fixture(lba=0x1020, blocks=8), [(0x1020, 0x1024), (0x1024, 0x1028)])
        self.assertTrue(oracle.check_trace(fixture(lba=0x107f, blocks=1).raw(), RANGES)['ok'])

    def test_count_is_bytes_and_nlb_is_actual_blocks(self):
        for blocks, count in [(0, 0), (8, 8), (7, 4096), (65537, 65537 * 512)]:
            trace = fixture();trace.lines = [line.replace(b'nlb 8 count 4096', f'nlb {blocks} count {count}'.encode())for line in trace.lines]
            self.reject(trace)

    def test_wrong_opcode_name_namespace_queue_and_raw_overflow_reject(self):
        raw = fixture().raw()
        for old, new in [(b'opc 0x1', b'opc 0x2'), (b'NVME_NVM_CMD_WRITE', b'WRITE'),
                         (b'nsid 0x1', b'nsid 0x2'), (b'sqid 1 ', b'sqid 2 '),
                         (b'cid 7 ', b'cid 65536 '), (b'opc 0x1', b'opc 0x101'),
                         (b'count 4096', b'count 18446744073709551616')]:
            self.reject(raw.replace(old, new))

    def test_missing_command_write_or_completion_reject(self):
        for name in [b'pci_nvme_io_cmd', b'pci_nvme_write', b'pci_nvme_enqueue_req_completion']:
            self.reject(b''.join(line for line in fixture().lines if b':' + name + b' ' not in line))

    def test_wrong_completion_identity_queue_status_and_duplicate_reject(self):
        trace = fixture()
        position = next(i for i,line in enumerate(trace.lines)if b':pci_nvme_enqueue_req_completion 'in line)
        last = trace.lines[position]
        for old, new in [(b'cid 7 ', b'cid 8 '), (b'cqid 1', b'cqid 0'),
                         (b'status 0x0', b'status 0x1'), (b'dw0 0x0', b'dw0 0x100000000')]:
            self.reject(b''.join(trace.lines[:position]) + last.replace(old,new) + b''.join(trace.lines[position+1:]))
        self.reject(trace.raw() + last)

    def test_cid_reuse_rejects_even_a_completed_prior_command(self):
        trace = fixture();trace.command();trace.write();trace.completion()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.command();trace.write();trace.close();trace.finish();trace.completion()
        self.reject(trace)

    def test_duplicate_write_and_ambiguous_overlapping_candidates_reject(self):
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.write();trace.close();trace.finish();trace.completion()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.command(8);trace.write(8);trace.close();trace.finish();trace.completion();trace.completion(8);trace.pool()
        self.reject(trace)

    def test_completion_before_finish_or_during_terminal_line_reject(self):
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.completion();trace.finish()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.uart('[task.finish] pd=9 ');trace.completion();trace.uart('task=9 reason=0 terminal=1\n')
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.close();trace.command();trace.write();trace.finish();trace.completion()
        self.reject(trace)

    def test_exact_task_identity_normal_reason_and_unique_records_required(self):
        for kwargs in [dict(pd=16), dict(task=16), dict(reason=1), dict(terminal=0)]:
            trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.finish(**kwargs);trace.completion()
            self.reject(trace)
        trace = fixture();trace.finish()
        self.reject(trace)
        trace = fixture();trace.close()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.completion()
        self.reject(trace)
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write();trace.close();trace.uart('[task.finish] pd=9 task=9 reason=0 terminal=1');trace.completion()
        self.reject(trace)

    def test_uart_boundaries_missing_registers_and_dlab_do_not_fabricate_text(self):
        raw = fixture().raw()
        self.reject(raw.replace(b':serial_write [0x00]', b':serial_write [0x01]'))
        self.reject(raw.replace(b'[0x00] <- 0x5b', b'[0x08] <- 0x5b', 1))
        trace = fixture();trace.event('serial_write', '[0x03] <- 0x83')
        self.reject(trace)
        trace = Trace();trace.command();trace.write();trace.close();trace.finish();trace.completion();trace.pool()
        self.assertTrue(oracle.check_trace(trace.raw(), RANGES)['ok'])  # Complete marker at stream start.
        trace = Trace();trace.uart('[fixture.begin]\nnoise');trace.command();trace.write();trace.close();trace.finish();trace.completion()
        self.reject(trace)

    def test_uart_utf8_and_records_split_across_interleaved_events(self):
        trace = Trace();trace.uart('[fixture.begin]\n');trace.command();trace.write()
        trace.uart('[quil.document.');trace.event('serial_write', '[0x01] <- 0x00');trace.uart('close] pending=1\r\n')
        trace.uart('[note] ')
        for byte in 'é→'.encode():
            trace.event('serial_write', f'[0x00] <- 0x{byte:02x}')
            trace.event('serial_write', '[0x01] <- 0x00')
        trace.uart('\n');trace.finish();trace.completion();trace.pool()
        result = oracle.check_trace(trace.raw(), RANGES)
        self.assertIn('é→', result['uart']['text'])
        self.assertGreater(len(result['uart']['byte_trace_offsets']), len(result['uart']['character_trace_offsets']))

    def test_unrelated_partial_uart_edges_do_not_invalidate_complete_critical_lines(self):
        trace = Trace()
        # Trace enabled halfway through a UTF8 scalar/ordinary diagnostic.
        trace.event('serial_write', '[0x00] <- 0xa9')
        trace.uart('unrelated record tail\r\n')
        trace.command();trace.write();trace.close();trace.finish();trace.completion();trace.pool()
        trace.uart('[unrelated record head')
        trace.event('serial_write', '[0x00] <- 0xc3')  # Partial trailing UTF8.
        result = oracle.check_trace(trace.raw(), RANGES)
        self.assertTrue(result['ok'])
        self.assertNotIn('unrelated', result['uart']['text'])

    def test_critical_partial_records_are_not_spliced_around_unrelated_text(self):
        trace = Trace();trace.uart('unrelated tail\n');trace.command();trace.write()
        trace.uart('[quil.document.clo\nunrelated\nse] pending=1\n')
        trace.finish();trace.completion()
        self.reject(trace)
        trace = fixture();trace.uart('[quil.document.close] pending=')
        self.reject(trace)
        trace = fixture();trace.uart('[task.finish] pd=9 task=9')
        self.reject(trace)

    def test_file_order_is_decisive_and_timestamps_are_not_sorted(self):
        trace = fixture()
        position = next(i for i,line in enumerate(trace.lines)if b':pci_nvme_enqueue_req_completion 'in line)
        trace.lines[position] = trace.lines[position].replace(b'1761955200.', b'1761955199.')
        result = oracle.check_trace(trace.raw(), RANGES)
        self.assertTrue(result['ok'])
        self.assertLess(result['completion']['event']['seconds'], result['finish']['last']['seconds'])

    def test_retained_native_a_iso_uart_parses_but_cannot_prove_io(self):
        events = oracle._events(NATIVE_A_UART)
        self.assertEqual(len(events), 4)
        self.assertEqual(events[0]['timestamp'], '2026-10-31T17:37:53.107345Z')
        self.assertIsNone(events[0]['tid'])
        self.assertEqual(events[0]['microseconds'], 107345)
        self.assertEqual(events[0]['timestamp_format'], 'iso8601z')
        self.assertEqual(oracle.outstanding_target_writes(NATIVE_A_UART, RANGES), [])
        self.reject(NATIVE_A_UART)

    def test_full_iso_device_fixture_preserves_timestamp_and_absent_tid(self):
        raw = iso_trace(fixture().raw())
        proof = oracle.check_trace(raw, RANGES)
        self.assertEqual(proof['format'], 'QEMU11.0.2/log/iso8601z')
        for point in [proof['command'], proof['write']['event'], proof['close']['first'],
                      proof['finish']['last'], proof['completion']['event']]:
            self.assertIsNone(point['tid'])
            self.assertEqual(point['timestamp_format'], 'iso8601z')
            self.assertTrue(point['timestamp'].endswith('Z'))
        prefix = raw[:proof['completion']['event']['offset']]
        self.assertEqual(oracle.outstanding_target_writes(prefix, RANGES)[0]['cid'], 7)
        earlier_completion = raw[proof['completion']['event']['offset']:].replace(b'2026-10-31', b'2026-10-30')
        self.assertTrue(oracle.check_trace(prefix + earlier_completion, RANGES)['ok'])

    def test_iso_calendar_precision_timezone_and_mixed_format_reject(self):
        raw = iso_trace(fixture().raw())
        for stamp in [b'2026-02-29T17:37:53.000000Z', b'2026-13-31T17:37:53.000000Z',
                      b'2026-10-31T24:37:53.000000Z', b'2026-10-31T17:60:53.000000Z',
                      b'2026-10-31T17:37:60.000000Z', b'0000-10-31T17:37:53.000000Z',
                      b'2026-10-31T17:37:53.00000Z', b'2026-10-31T17:37:53.000000z',
                      b'2026-10-31T17:37:53.000000+00:00']:
            self.reject(raw.replace(raw.split(b' ', 1)[0], stamp, 1))
        self.reject(raw + fixture().lines[0])
        self.reject(fixture().raw() + NATIVE_A_UART)

    def test_unknown_malformed_truncated_nonascii_and_oversized_records_reject(self):
        raw = fixture().raw()
        for broken in [b'', raw[:-1], raw + b'17@1761955200.000001:serial_write [0x00] <-',
                       raw.replace(b'serial_write', b'serial_read', 1),
                       raw.replace(b'1761955200.000000:', b'1761955200.00000:', 1),
                       raw + b'\xff\n', raw + b'x' * 513 + b'\n']:
            self.reject(broken)
        with patch.object(oracle, 'MAX_TRACE_BYTES', len(raw) - 1):self.reject(raw)
        with patch.object(oracle, 'MAX_EVENTS', len(fixture().lines) - 1):self.reject(raw)
        with patch.object(oracle, 'MAX_UART_BYTES', 10):self.reject(raw)

    def test_invalid_empty_overlapping_boolean_and_overflow_ranges_reject(self):
        for ranges in [[], [(1, 1)], [(5, 2)], [(True, 20)], [(0, 1 << 65)],
                       [(0, 20), (19, 21)], [(0, 1)] * 9, [(0, 1, 2)]]:
            self.reject(fixture(), ranges)
        trace = fixture(lba=(1 << 64) - 1, blocks=2)
        self.reject(trace, [((1 << 64) - 1, 1 << 64)])


class ProvisionalWriteTests(unittest.TestCase):
    def test_outstanding_observation_is_provisional_and_never_final_success(self):
        trace = Trace();trace.uart('partial peripheral text');trace.command();trace.write()
        candidates = oracle.outstanding_target_writes(trace.raw(), RANGES)
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertTrue(candidate['provisional'])
        self.assertIs(candidate['qualified'], False)
        self.assertNotIn('ok', candidate)
        self.assertEqual((candidate['cid'], candidate['write']['lba']), (7, 0x1020))
        self.assertEqual(candidate['trace_prefix_bytes'], len(trace.raw()))
        with self.assertRaises(ValueError):oracle.check_trace(trace.raw(), RANGES)
        trace.completion()
        self.assertEqual(oracle.outstanding_target_writes(trace.raw(), RANGES), [])

    def test_live_observer_needs_complete_write_geometry_inside_target(self):
        self.assertEqual(oracle.outstanding_target_writes(b'', RANGES), [])
        trace = Trace();trace.command()
        self.assertEqual(oracle.outstanding_target_writes(trace.raw(), RANGES), [])
        trace.write(lba=0x2000)
        self.assertEqual(oracle.outstanding_target_writes(trace.raw(), RANGES), [])
        trace = Trace();trace.command(6, opcode=2, name='NVME_NVM_CMD_READ')
        self.assertEqual(oracle.outstanding_target_writes(trace.raw(), RANGES), [])

    def test_provisional_observer_does_not_swallow_malformed_or_partial_events(self):
        trace = Trace();trace.command();trace.write()
        for raw in [trace.raw()[:-1], trace.raw().replace(b'count 4096', b'count 4095'),
                    trace.raw() + b'broken\n', bytearray()]:
            with self.assertRaises(ValueError):oracle.outstanding_target_writes(raw, RANGES)
        trace.command()
        with self.assertRaises(ValueError):oracle.outstanding_target_writes(trace.raw(), RANGES)

    def test_multiple_outstanding_candidates_are_explicit_not_arbitrarily_selected(self):
        trace = Trace();trace.command();trace.write();trace.command(8);trace.write(8)
        self.assertEqual([c['cid'] for c in oracle.outstanding_target_writes(trace.raw(), RANGES)], [7, 8])


if __name__ == '__main__':
    unittest.main()
