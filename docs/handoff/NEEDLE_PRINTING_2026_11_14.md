# Needle: driverless printing and scanning (W4, W5)

Date: 2026-11-14. Branch `claude/project-thread-bcyshp-needle`.
Plan rows: W4, W5 in `APP_PARITY_PLAN_2026_11_14.md`. Later users: M11 (craft-print shim), N9
(Kaleidoscope print), Q11 (Vellum print), T5 (Brocade print).

SexOS prints and scans the way phones do: no drivers, no CUPS, no SANE. Printers that speak
IPP Everywhere or AirPrint get PWG raster, Apple URF or PDF. Scanners that speak eSCL
(AirScan, Mopria) send pages over HTTP. Nothing runs in the kernel. Needle is an ordinary
program that holds `+net`.

## What is done

| Piece | Where | Notes |
|---|---|---|
| IPP/1.1 and 2.0 binary encode and decode | `crates/sex-print/src/ipp.rs` | Every group and value tag, collections (`media-col`) nested both ways, text/name with language, out-of-band values. Requests: Get-Printer-Attributes, Validate-Job, Print-Job, Get-Jobs, Get-Job-Attributes, Cancel-Job. |
| IPP Everywhere capabilities | `caps.rs` | document-format, media and PWG media sizes, sides, colour modes, printer and pwg-raster resolutions, raster types, sheet-back, urf-supported, copies, operations. Chooses the raster format, resolution and media. |
| PWG Raster (`RaS2`) and Apple URF writer | `raster.rs` | 1796-byte PWG header at the offsets in PWG 5102.4. Shared line-repeat and PackBits-style run-length coding, sGray 8 or sRGB 8. Includes a decoder for tests. |
| PDF | `pdf.rs` | Sniffs a PDF to pass it through unchanged. Writes plain text as a small Courier PDF for printers that take PDF but not raster. |
| DNS-SD / mDNS | `dnssd.rs` | Builds queries (QU bit). Parses responses with name compression and refuses pointer loops. Reads PTR/SRV/TXT/A/AAAA into services for `_ipp`, `_ipps`, `_uscan` and `_uscans`, with TXT keys (`rp`, `pdl`, `URF`, `Color`, `Duplex`, `UUID`, `rs`, ...). |
| eSCL client | `escl.rs`, `xml.rs` | Parses ScannerCapabilities (Platen and Adf). Chooses settings and writes the ScanSettings XML. Paths for ScanJobs, Location and NextDocument. Uses a small hand-written XML reader with no dependencies. |
| HTTP/1.1 | `http.rs`, `url.rs` | One request per connection. Bodies by Content-Length, chunked encoding or connection close, and it skips 100 Continue. Handles ipp/ipps/http/https URLs, IPv6 literals and Location joins. |
| `needle` program | `apps/needle` | Built on sex-std (`run +net needle ...`). It also builds for the host, where `scripts/needle_host_test.py` drives it. |

The crate is `no_std` + `alloc` with no dependencies and does no I/O.

## CLI

```
run +net needle discover [HOST[:PORT]]
run +net needle printers [URI...]
run +net needle print URI FILE|--test-page [--format MIME] [--copies N] [--sides KW] [--color] [--media NAME]
run +net needle jobs URI
run +net needle cancel URI JOB-ID
run +net needle scan ESCL-URL OUT.png|.jpg|.pdf|.pgm [--color] [--dpi N]
```

Machine-checkable lines (window and serial record):
`[needle.printer] name=… uri=… formats=…`, `[needle.scanner] …`, `[needle.discover] found=N`,
`[needle.sent] id=… format=… bytes=… pages=…`, `[needle.job] id=… state=completed`,
`[needle.jobs] count=N`, `[needle.cancel] id=… status=…`, `[needle.scan] pages=1 bytes=…`,
`[needle.failed] reason`.

How `print` picks a format:
1. A PDF goes to `application/pdf`, else `application/octet-stream`.
2. A JPEG goes as is if the printer takes `image/jpeg`.
3. Other pictures (PNG, JPEG, GIF, WebP via sex-image, PGM/PPM) are fitted onto a raster page.
4. Text is drawn in Geist Mono (sex-font) onto PWG raster pages, or URF when the printer only
   does AirPrint. Without raster it goes as a generated PDF, then `text/plain`.

The raster resolution is the lowest the printer lists. Every job is checked with Validate-Job
first, then Get-Job-Attributes is polled until the job reaches a final state.

`discover` saves the printers it finds in the program's folder (`printers`), so `needle printers`
with no arguments lists them.

### Discovery without multicast

sex-std UDP has no multicast group join yet, and sexnet does not route 224.0.0.251. So
`needle discover HOST[:PORT]` sends the DNS-SD browse as a legacy unicast query (RFC 6762 §6.7)
to one host, and that host's mDNS responder answers. Avahi and Bonjour responders, and most
printers, answer unicast queries on port 5353. With no host given, Needle tries 224.0.0.251:5353
and says so if it can't send. **Real multicast browsing is a follow-up.** It needs an IGMP join
and multicast receive in sexnet and `UdpSocket::join_multicast_v4` in sex-std (workstream K or
the networking thread).

## Tests

- `cargo test -p sex-print` runs 8 unit tests and 11 golden tests. The golden `.bin` files come
  from an independent Python encoder that shares no code with the crate
  (`crates/sex-print/tests/fixtures/make_fixtures.py` over `scripts/needle_fakes.py`):
  - IPP request encode is byte-exact (Get-Printer-Attributes, Print-Job with document,
    Cancel-Job at 1.1).
  - IPP response decode and re-encode are byte-exact, and the capabilities and Get-Jobs it
    reads are checked.
  - PWG and URF bytes are worked out by hand.
  - The mDNS query is byte-exact. The response is parsed into services, plus a hand-built
    packet with compression, an escaped dot and AAAA, and a pointer loop that must be refused.
  - Two eSCL XML fixtures: platen, and feeder only.
  - The text PDF is checked down to its xref offsets.
- `cargo build -p needle && python3 scripts/needle_host_test.py` runs the whole CLI on the host
  against the fakes (discover, printers, print the test page, scan, print the scan, print a PDF,
  jobs, cancel). It is part of the CI host job.
- `scripts/needle_gate.py` runs the same flow in QEMU (CI step after the ssh gate). The fakes are
  reached at 10.0.2.2 through user networking. The gate decodes the received PWG raster itself
  and checks the A4 75 dpi page geometry and the ink, the PGM size, and the eSCL settings sent.

## How other apps will use Needle

Two layers, so apps don't each carry printing code.

**1. Library (now).** Any program can link `sex-print`: build a `raster::Page` or a PDF, then use
`ipp::Client` with its own `std::net::TcpStream`. This works today but needs `+net` and puts the
whole job in the app.

**2. A print service over PDX (proposed, not built).** Needle becomes `servers/needle` (the plan's
write set) and holds the network grant. Apps hold only a "print" capability. The proposed
protocol is one new file, `crates/sex-pdx/src/print.rs`, with no new syscall:

| Op | Request | Reply |
|---|---|---|
| `PRINTERS` | — | list of {id, name, uri, formats, media, duplex, color} |
| `SUBMIT` | printer id (0 = default), a read grant on the document (a file the app holds, or a shared region), format MIME, options {copies, sides, color, media, page range}, job name | job id or refusal (format unsupported, no printer, ...) |
| `STATUS` | job id | state, reasons, pages done |
| `CANCEL` | job id | ok / not possible |
| `SCAN` | scanner id, {format, color, dpi, source} | a write grant back on the page file(s) |

Callers:
- **N9 Kaleidoscope**: lays out with `@page`/`@media print`, makes a PDF (it already writes one
  for "save as PDF"), then `SUBMIT` with `application/pdf`. Needle passes it through, or renders
  it to raster once a PDF renderer is linked (below).
- **Q11 Vellum**: already has hayro. It `SUBMIT`s the original PDF. For printers without PDF it
  can rasterize pages itself and `SUBMIT` `image/pwg-raster` made with `sex_print::raster`.
- **T5 Brocade**: paginates, writes PDF, then `SUBMIT`.
- **M11 craft-print shim**: maps craft's print calls to `SUBMIT` (PDF), with its print dialog
  filled from `PRINTERS`.

Until the server exists, these apps can launch the program through `sex_pdx::launch` (the
program-launch protocol), e.g. `needle print URI @/path/doc.pdf`, and read `[needle.job]` from its
exit status. That is clumsy but needs no new protocol.

## Next steps

1. **Queue UI.** A window listing printers, jobs (with cancel) and scanner preview. This needs the
   Weft toolkit (workstream M). It is out of scope here; the CLI is the whole interface for now.
2. **IPPS/eSCL over TLS** (`ipps://`, `https://`). Wrap the stream in `crates/sex-tls`. Printers
   use self-signed certificates, so store the certificate on first use like ssh's `known_hosts`.
   Needle currently refuses `ipps://` with a clear message.
3. **Multicast mDNS**: see above. Also announce nothing, since Needle is only a client.
4. **PDF to raster** for printers without PDF: link hayro (as in apps/pdf / Vellum) into the
   print service.
5. **Real printer receipts** on the x17 network: a printer that does IPP Everywhere (HP, Brother,
   Canon, Epson) and an eSCL scanner. Record `[needle.*]` lines and photos of the pages.
   Expected quirks: `which-jobs=all` refused (Needle retries with `completed`), `Location`
   without a host (handled by `Url::join`), 503 while the scanner warms up (retried 60 times at
   0.5 s), and chunked responses (handled).
6. Duplex raster needs `pwg-raster-document-sheet-back` (flipped/rotated back sides). It is read
   but not yet applied.
7. Colour management: sRGB pages go as they are. ICC is out of scope.
