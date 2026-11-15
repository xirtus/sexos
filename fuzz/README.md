# SexOS File & Parser Fuzzing Suite

Standalone `cargo-fuzz` targets for host-side fuzz testing of SexOS untrusted file, protocol, and format parsers.

Uses `libfuzzer-sys` with AddressSanitizer (`-Zsanitizer=address`) and LLVM coverage counters (`-Cpasses=sancov-module`).

## Targets

| Target | Target Crate(s) | Format / Functionality Covered |
|---|---|---|
| `fuzz_archive` | `crates/sex-archive` | ZIP (deflate/zstd/xz/lzma/stored, zip64), TAR (ustar/pax/gnu), 7z, `.tar.gz`, `.tar.zst`, `.tar.xz`, decompression limits, path sanitation |
| `fuzz_docx_xlsx_ooxml` | `crates/sex-archive`<br>`crates/sex-html` | OOXML container structure (`.docx`, `.xlsx`), `[Content_Types].xml`, `_rels`, document/sheet XML fragments |
| `fuzz_pdf` | `crates/vellum-edit` | PDF lexer, cross-reference tables (`xref`), object streams, COS types, stream filters, AcroForm fields, signatures, incremental save |
| `fuzz_volumes` | `crates/sex-volumes` | MBR and GPT partition tables, FAT12/FAT16/FAT32, exFAT, ext4, and NTFS filesystems over simulated block devices |
| `fuzz_image` | `crates/sex-image`<br>`crates/gaze-codecs` | PNG, JPEG, GIF (including animated multi-frame), BMP, TIFF, EXIF metadata, Camera RAW embedded previews |
| `fuzz_html_css` | `crates/sex-html` | HTML5 DOM tokenizer and tree builder, CSS stylesheet parser and cascade, URL resolver and fragment parser |
| `fuzz_http_hpack` | `crates/sex-http` | HTTP/1.1 wire framing and headers, HTTP/2 HPACK header block decompression, HTTP/2 connection and stream frames |
| `fuzz_mail` | `crates/sex-mail` | RFC 5322 email headers, MIME multipart boundaries and parts, RFC 2047 encoded words, base64 decoding |
| `fuzz_font` | `crates/sex-font` | TrueType and OpenType font SFNT tables (head, hhea, hmtx, cmap, glyf, loca, GPOS), metrics, and outline rasterization |
| `fuzz_torrent` | `sex-torrent` (PR #236) | Bencode decoding, integer overflow bounds, list/dict recursion limits, Magnet URI parsing, wire protocol packet framing |
| `fuzz_kodi_xml` | `sex-kodi` (PR #251) | Kodi addon XML unescaping, element/attribute parsing, Debian/Kodi version comparator, zip addon package unpack |
| `fuzz_docx_office` | `crates/sex-office`<br>`wordcraft-engine` (PR #224) | DOCX OOXML ZIP part extraction, Office Pen/Canvas rendering coordinates, integer overflow checks (`x * 256`, `cx + cw`), alpha blending |
| `fuzz_xlsx_office` | `crates/sex-office`<br>`apps/spreadeagle`<br>`gridcraft-engine` (PR #219) | XLSX OOXML ZIP part extraction, spreadsheet column name arithmetic (`col + 1` overflow checks), formula tokenization |
| `fuzz_spreadeagle_xlsx_ods` | `apps/spreadeagle`<br>`gridcraft-engine` | Spreadeagle XLSX and ODS import engines, raw archive bytes and structured uncompressed ZIP wrapping sheets, formulas, cells, and manifest |
| `fuzz_assuit_docx` | `apps/assuit`<br>`wordcraft-engine` | Assuit DOCX document import and layout engine, raw bytes and structured ZIP wrapping document.xml, styles, and numbering |
| `fuzz_bodice_epub` | `crates/sex-ebook`<br>`apps/bodice` | Bodice EPUB 2/3 container, OPF package, NCX/nav TOC, XHTML chapter text reflow and page pagination |
| `fuzz_gossip_feed` | `crates/sex-feed`<br>`apps/gossip` | Gossip RSS 2.0 / Atom feed parser, podcast enclosures, OPML import/export, subscription list and read state serialization |
| `fuzz_swingers_bencode` | `crates/sex-torrent`<br>`apps/swingers` (PR #236) | Bencode decoding (`decode`, `decode_prefix`), integer overflow bounds, canonical round-trip verification, and torrent metainfo decoding |
| `fuzz_kaleidoscope_ext_package` | `crates/sex-extensions`<br>`crates/sex-shields`<br>`apps/web` | Extension packages: `.crx` (v2/v3) and ZIP archives, uncompressed/deflate inflation, top-folder strip, manifest/locale/ruleset loading, path sanitation invariants |
| `fuzz_kaleidoscope_ext_manifest` | `crates/sex-extensions` | Manifest V2/V3 JSON parsing, BOM and comment stripping, `__MSG_` localization, match patterns, content scripts, clean path invariants |
| `fuzz_kaleidoscope_ext_dnr` | `crates/sex-extensions`<br>`crates/sex-shields` | declarativeNetRequest ruleset conversion to Shields filters, filter option injection & cosmetic smuggling verification, Shields engine check/hidden/css/exceptions |
| `fuzz_burlesque_pptx` | `crates/burlesque-core` | Burlesque OOXML PPTX deck import, structured slide/theme/notes XML, slide software rendering and dual-pane presenter view |
| `fuzz_batik_psd` | `crates/batik-core` | Photoshop PSD multi-layer parser, raw/PackBits channel decoding, limits admission, canvas layer composition and flattening |

## Usage

### Prerequisites
- Nightly Rust toolchain: `rustup toolchain install nightly`
- `cargo-fuzz`: `cargo install cargo-fuzz`

### Running Fuzz Targets
Run a specific target for 60 seconds with a 2 GB RSS limit:
```bash
cargo +nightly fuzz run fuzz_archive -- -max_total_time=60 -rss_limit_mb=2048 -timeout=10
```

Run a short smoke test across all targets:
```bash
./scripts/fuzz_smoke_host_gate.sh
```

### Reproducing Crashes
When libFuzzer finds a crash or hang, it saves the input in `fuzz/artifacts/<target>/crash-<hash>`.
Minimize the reproducer:
```bash
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/crash-<hash>
```

Run the target against the reproducer:
```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/minimized-<hash>
```

## Adding Fuzz Targets for Workstream PRs
For parsers introduced in open feature branches (such as `sex-torrent` in PR #236, `sex-ebook` and `sex-xml` in PR #230, and `gridcraft-engine` in PR #219), once merged into `master`, add the crate dependency to `fuzz/Cargo.toml` and create corresponding `fuzz/fuzz_targets/fuzz_<name>.rs` targets following the existing patterns.
