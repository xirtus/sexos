// SPDX-License-Identifier: GPL-3.0-only
//! Browser Media & YouTube Self-Test core (no_std, host-testable).
//!
//! Shared between Kaleidoscope (`apps/web`, invoked via `--selftest` / `--selftest-net`
//! or `about:selftest`) and host unit tests (`cargo test -p sex-web`).
//!
//! All synthetic media fixtures (fMP4 with valid H.264 Baseline `I_PCM` IDR frames,
//! WebM EBML clusters, and 48 kHz stereo PCM WAV clips) are synthesized on the heap
//! on demand — zero large static buffers.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_js::{Document, Limits, Script};

use crate::mse::{self, Format, Stream};

/// Outcome of one self-test case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaseStatus {
    Pass,
    Fail,
    Skip,
}

impl CaseStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            CaseStatus::Pass => "PASS",
            CaseStatus::Fail => "FAIL",
            CaseStatus::Skip => "SKIP",
        }
    }
}

/// Result record for one self-test case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseResult {
    pub case: String,
    pub status: CaseStatus,
    pub ms: u64,
    pub detail: String,
}

impl CaseResult {
    pub fn pass(case: impl Into<String>, ms: u64, detail: impl Into<String>) -> Self {
        Self { case: case.into(), status: CaseStatus::Pass, ms, detail: detail.into() }
    }

    pub fn fail(case: impl Into<String>, ms: u64, detail: impl Into<String>) -> Self {
        Self { case: case.into(), status: CaseStatus::Fail, ms, detail: detail.into() }
    }

    pub fn skip(case: impl Into<String>, ms: u64, detail: impl Into<String>) -> Self {
        Self { case: case.into(), status: CaseStatus::Skip, ms, detail: detail.into() }
    }

    /// Machine-readable and human-readable log line for serial/tether output.
    pub fn format_line(&self) -> String {
        format!(
            "[web.selftest] case={} status={} ms={} detail={}",
            self.case,
            self.status.as_str(),
            self.ms,
            self.detail
        )
    }
}

/// Aggregate summary across all executed self-test cases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl Summary {
    pub fn from_results(results: &[CaseResult]) -> Self {
        let mut s = Self::default();
        for r in results {
            match r.status {
                CaseStatus::Pass => s.passed += 1,
                CaseStatus::Fail => s.failed += 1,
                CaseStatus::Skip => s.skipped += 1,
            }
        }
        s
    }

    /// Machine-readable summary line (`[web.selftest.summary] passed=N failed=M`).
    pub fn format_line(&self) -> String {
        if self.skipped > 0 {
            format!(
                "[web.selftest.summary] passed={} failed={} skipped={}",
                self.passed, self.failed, self.skipped
            )
        } else {
            format!(
                "[web.selftest.summary] passed={} failed={}",
                self.passed, self.failed
            )
        }
    }
}

/// Renders a crisp HTML report page for Kaleidoscope's window.
pub fn render_report_html(mode: &str, results: &[CaseResult], summary: &Summary) -> String {
    let mut rows = String::new();
    for r in results {
        let color = match r.status {
            CaseStatus::Pass => "#22c55e",
            CaseStatus::Fail => "#ef4444",
            CaseStatus::Skip => "#eab308",
        };
        rows.push_str(&format!(
            "<tr><td style=\"padding:8px 12px;font-family:monospace;\">{}</td>\
             <td style=\"padding:8px 12px;font-weight:bold;color:{};\">{}</td>\
             <td style=\"padding:8px 12px;font-family:monospace;\">{} ms</td>\
             <td style=\"padding:8px 12px;font-family:monospace;\">{}</td></tr>\n",
            escape_html(&r.case),
            color,
            r.status.as_str(),
            r.ms,
            escape_html(&r.detail),
        ));
    }
    let banner_color = if summary.failed == 0 { "#14532d" } else { "#7f1d1d" };
    let banner_text = if summary.failed == 0 { "ALL CHECKS PASSED" } else { "SELF-TEST FAILURES DETECTED" };
    format!(
        "<!doctype html><html><head><title>Kaleidoscope Media Self-Test</title></head>\
         <body style=\"background:#0f172a;color:#e2e8f0;font-family:sans-serif;margin:24px;\">\
         <h1 style=\"margin:0 0 8px 0;\">Kaleidoscope Browser Media &amp; YouTube Self-Test</h1>\
         <p style=\"color:#94a3b8;margin:0 0 16px 0;\">mode: <code>{}</code></p>\
         <div style=\"background:{};padding:12px 16px;border-radius:8px;margin-bottom:18px;font-weight:bold;\">\
         {} &mdash; <code>{}</code></div>\
         <table style=\"width:100%;border-collapse:collapse;background:#1e293b;border-radius:8px;\">\
         <thead><tr style=\"border-bottom:1px solid #334155;text-align:left;\">\
         <th style=\"padding:8px 12px;\">Case</th>\
         <th style=\"padding:8px 12px;\">Status</th>\
         <th style=\"padding:8px 12px;\">Time</th>\
         <th style=\"padding:8px 12px;\">Detail</th></tr></thead>\
         <tbody>{}</tbody></table>\
         </body></html>",
        escape_html(mode),
        banner_color,
        banner_text,
        escape_html(&summary.format_line()),
        rows,
    )
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

// ── MSE Codec Support Matrix ────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct CodecProbe {
    pub label: &'static str,
    pub mime: &'static str,
    pub expected: bool,
}

/// Core codec matrix covering YouTube DASH/fMP4/WebM formats plus unsupported checks.
pub const CODEC_MATRIX: &[CodecProbe] = &[
    CodecProbe { label: "avc1", mime: r#"video/mp4; codecs="avc1.640028""#, expected: true },
    CodecProbe { label: "vp9", mime: r#"video/webm; codecs="vp9""#, expected: true },
    CodecProbe { label: "vp09", mime: r#"video/webm; codecs="vp09.00.10.08""#, expected: true },
    CodecProbe { label: "av01_mp4", mime: r#"video/mp4; codecs="av01.0.04M.08""#, expected: true },
    CodecProbe { label: "av01_webm", mime: r#"video/webm; codecs="av01.0.04M.08""#, expected: true },
    CodecProbe { label: "mp4a", mime: r#"audio/mp4; codecs="mp4a.40.2""#, expected: true },
    CodecProbe { label: "opus", mime: r#"audio/webm; codecs="opus""#, expected: true },
    CodecProbe { label: "unsupported_hevc", mime: r#"video/mp4; codecs="hev1.1.6.L93.B0""#, expected: false },
    CodecProbe { label: "unsupported_ogg", mime: r#"video/ogg; codecs="theora""#, expected: false },
];

/// Evaluates [`CODEC_MATRIX`] against [`mse::supported`].
/// Returns `(matched, total, mismatched_labels)`.
pub fn check_codec_matrix() -> (usize, usize, Vec<&'static str>) {
    let mut matched = 0usize;
    let mut failed = Vec::new();
    for p in CODEC_MATRIX {
        if mse::supported(p.mime) == p.expected {
            matched += 1;
        } else {
            failed.push(p.label);
        }
    }
    (matched, CODEC_MATRIX.len(), failed)
}

// ── Test 1: DOM + QuickJS-NG + Timers + onclick ─────────────────────────────

/// Runs Test 1 (DOM + JS + Timers + `onclick`) using `sex-js` (`Script<Document>`).
///
/// `clock` returns monotonic nanoseconds (matching `Document::new`).
/// `now_ms` returns elapsed milliseconds for timing the case.
pub fn run_dom_js_timers_test(clock: fn() -> u64, now_ms: impl Fn() -> u64) -> CaseResult {
    let t0 = now_ms();
    let html = r#"<!doctype html><html><body><button id="btn">Go</button><div id="out"></div></body></html>"#;
    let dom = sex_html::parse::parse(html);
    let btn_node = dom
        .descendants(sex_html::dom::Dom::ROOT)
        .into_iter()
        .find(|&n| dom.attr(n, "id") == Some("btn"));
    let Some(btn_node) = btn_node else {
        return CaseResult::fail("dom_js_timers", now_ms().saturating_sub(t0), "missing #btn node");
    };

    let doc = Document::new(dom, "about:selftest", (800, 600), clock);
    let mut script = match Script::new(doc, Limits::default()) {
        Ok(s) => s,
        Err(e) => {
            return CaseResult::fail(
                "dom_js_timers",
                now_ms().saturating_sub(t0),
                format!("Script::new failed: {:?}", e),
            );
        }
    };

    let js = r#"
        (function() {
            var v = document.createElement('video');
            v.setAttribute('width', '320');
            v.setAttribute('height', '180');
            document.body.appendChild(v);

            var probes = [
                'video/mp4; codecs="avc1.640028"',
                'video/webm; codecs="vp9"',
                'video/mp4; codecs="av01.0.04M.08"',
                'audio/mp4; codecs="mp4a.40.2"',
                'audio/webm; codecs="opus"'
            ];
            var mseOk = 0;
            for (var i = 0; i < probes.length; i++) {
                if (typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported(probes[i])) {
                    mseOk++;
                }
            }

            var timerFired = 0;
            var rafFired = 0;
            var clickFired = 0;

            function syncOut() {
                var el = document.getElementById('out');
                if (el) {
                    el.textContent = 'video=' + (v.tagName === 'VIDEO' ? 1 : 0)
                        + ' mse=' + mseOk + '/' + probes.length
                        + ' timer=' + timerFired
                        + ' raf=' + rafFired
                        + ' click=' + clickFired;
                }
            }

            setTimeout(function() { timerFired = 1; syncOut(); }, 0);
            requestAnimationFrame(function() { rafFired = 1; syncOut(); });
            var btn = document.getElementById('btn');
            if (btn) {
                btn.onclick = function() { clickFired = 1; syncOut(); };
            }
            syncOut();
        })();
    "#;

    if let Err(e) = script.eval(js, "selftest_dom.js") {
        return CaseResult::fail(
            "dom_js_timers",
            now_ms().saturating_sub(t0),
            format!("script.eval error: {:?}", e),
        );
    }

    let _ = script.dispatch("ready");
    // Drive due setTimeout(0) and requestAnimationFrame callbacks.
    for _ in 0..4 {
        let _ = script.dispatch("tick");
    }
    // Dispatch click on #btn to verify onclick handler.
    let _ = script.dispatch(&format!("click\t{}", btn_node));
    let _ = script.dispatch("tick");

    let dom_ref = &script.page().dom;
    let out_text = dom_ref
        .descendants(sex_html::dom::Dom::ROOT)
        .into_iter()
        .find(|&n| dom_ref.attr(n, "id") == Some("out"))
        .map(|n| dom_ref.text_content(n))
        .unwrap_or_default();

    let video_in_dom = dom_ref
        .descendants(sex_html::dom::Dom::ROOT)
        .into_iter()
        .any(|n| dom_ref.tag(n) == Some("video"));

    let elapsed = now_ms().saturating_sub(t0);
    let expected = "video=1 mse=5/5 timer=1 raf=1 click=1";
    if video_in_dom && out_text == expected {
        CaseResult::pass("dom_js_timers", elapsed, out_text)
    } else {
        CaseResult::fail(
            "dom_js_timers",
            elapsed,
            format!("video_in_dom={} out={:?}", video_in_dom as u8, out_text),
        )
    }
}

// ── Test 2 Core: Synthetic fMP4 (H.264 I_PCM) & WebM MSE Pipeline ──────────

/// Minimal MSB-first bitstream writer for spec-compliant H.264 Baseline NAL units.
struct BitWriter {
    bytes: Vec<u8>,
    cur: u8,
    bits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self { bytes: Vec::new(), cur: 0, bits: 0 }
    }

    fn bit(&mut self, b: u8) {
        self.cur = (self.cur << 1) | (b & 1);
        self.bits += 1;
        if self.bits == 8 {
            self.bytes.push(self.cur);
            self.cur = 0;
            self.bits = 0;
        }
    }

    fn u(&mut self, n: u8, val: u32) {
        for i in (0..n).rev() {
            self.bit(((val >> i) & 1) as u8);
        }
    }

    /// Unsigned Exp-Golomb code `ue(v)`.
    fn ue(&mut self, v: u32) {
        let code_num = v + 1;
        let leading_zeros = (31 - code_num.leading_zeros()) as u8;
        for _ in 0..leading_zeros {
            self.bit(0);
        }
        self.u(leading_zeros + 1, code_num);
    }

    /// Signed Exp-Golomb code `se(v)`.
    fn se(&mut self, v: i32) {
        let mapped = if v <= 0 { (-2 * v) as u32 } else { (2 * v - 1) as u32 };
        self.ue(mapped);
    }

    fn align_zero(&mut self) {
        while self.bits != 0 {
            self.bit(0);
        }
    }

    fn push_raw_bytes(&mut self, data: &[u8]) {
        debug_assert_eq!(self.bits, 0);
        self.bytes.extend_from_slice(data);
    }

    fn rbsp_trailing_bits(&mut self) {
        self.bit(1);
        self.align_zero();
    }

    /// Emits NAL header + EBSP (with 0x00 0x00 0x03 emulation prevention).
    fn finish_nal(mut self, nal_header: u8) -> Vec<u8> {
        self.rbsp_trailing_bits();
        let mut out = Vec::with_capacity(self.bytes.len() + 8);
        out.push(nal_header);
        let mut zeros = 0u8;
        for b in self.bytes {
            if zeros == 2 && b <= 3 {
                out.push(0x03);
                zeros = 0;
            }
            out.push(b);
            if b == 0 {
                zeros = zeros.saturating_add(1);
            } else {
                zeros = 0;
            }
        }
        out
    }
}

/// Generates a valid H.264 Baseline Profile SPS NAL for `(mb_w * 16) x (mb_h * 16)`.
pub fn synth_h264_sps(mb_w: u32, mb_h: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(8, 66);   // profile_idc = Baseline (66)
    w.u(8, 0xC0); // constraint_set0_flag=1, constraint_set1_flag=1
    w.u(8, 30);   // level_idc = 3.0
    w.ue(0);      // seq_parameter_set_id
    w.ue(0);      // log2_max_frame_num_minus4 (frame_num = 4 bits)
    w.ue(2);      // pic_order_cnt_type = 2 (no POC fields in slice header)
    w.ue(0);      // max_num_ref_frames = 0 (IDR-only)
    w.bit(0);     // gaps_in_frame_num_value_allowed_flag
    w.ue(mb_w.saturating_sub(1)); // pic_width_in_mbs_minus1
    w.ue(mb_h.saturating_sub(1)); // pic_height_in_map_units_minus1
    w.bit(1);     // frame_mbs_only_flag = 1
    w.bit(0);     // direct_8x8_inference_flag
    w.bit(0);     // frame_cropping_flag
    w.bit(0);     // vui_parameters_present_flag
    w.finish_nal(0x67)
}

/// Generates a valid H.264 Baseline Profile PPS NAL (CAVLC).
pub fn synth_h264_pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ue(0);    // pic_parameter_set_id
    w.ue(0);    // seq_parameter_set_id
    w.bit(0);   // entropy_coding_mode_flag = 0 (CAVLC)
    w.bit(0);   // bottom_field_pic_order_in_frame_present_flag
    w.ue(0);    // num_slice_groups_minus1
    w.ue(0);    // num_ref_idx_l0_default_active_minus1
    w.ue(0);    // num_ref_idx_l1_default_active_minus1
    w.bit(0);   // weighted_pred_flag
    w.u(2, 0);  // weighted_bipred_idc
    w.se(0);    // pic_init_qp_minus26
    w.se(0);    // pic_init_qs_minus26
    w.se(0);    // chroma_qp_index_offset
    w.bit(0);   // deblocking_filter_control_present_flag
    w.bit(0);   // constrained_intra_pred_flag
    w.bit(0);   // redundant_pic_cnt_present_flag
    w.finish_nal(0x68)
}

/// Generates a valid H.264 IDR Slice NAL using `I_PCM` macroblocks (exact YUV 4:2:0 pixels).
pub fn synth_h264_idr_ipcm(mb_w: u32, mb_h: u32, idr_pic_id: u32, y_val: u8, u_val: u8, v_val: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ue(0);          // first_mb_in_slice
    w.ue(7);          // slice_type = 7 (all I-slices)
    w.ue(0);          // pic_parameter_set_id
    w.u(4, 0);        // frame_num = 0 (4 bits)
    w.ue(idr_pic_id); // idr_pic_id
    w.bit(0);         // no_output_of_prior_pics_flag
    w.bit(0);         // long_term_reference_flag
    w.se(0);          // slice_qp_delta

    let y_block = [y_val; 256];
    let u_block = [u_val; 64];
    let v_block = [v_val; 64];
    let total_mbs = mb_w * mb_h;
    for _ in 0..total_mbs {
        w.ue(25); // mb_type = 25 (I_PCM)
        w.align_zero();
        w.push_raw_bytes(&y_block);
        w.push_raw_bytes(&u_block);
        w.push_raw_bytes(&v_block);
    }
    w.finish_nal(0x65)
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn mp4_fullbox(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(4 + body.len());
    let vf = ((version as u32) << 24) | (flags & 0x00FF_FFFF);
    inner.extend_from_slice(&vf.to_be_bytes());
    inner.extend_from_slice(body);
    mp4_box(kind, &inner)
}

/// Synthesizes a valid fragmented MP4 (ISO BMFF) initialization segment (`ftyp` + `moov`)
/// with a real H.264 Baseline `avc1`/`avcC` track (16*mb_w x 16*mb_h).
pub fn synth_fmp4_h264_init(mb_w: u32, mb_h: u32, timescale: u32, default_dur: u32) -> Vec<u8> {
    let width = (mb_w * 16) as u16;
    let height = (mb_h * 16) as u16;
    let sps = synth_h264_sps(mb_w, mb_h);
    let pps = synth_h264_pps();

    // ftyp: major=isom, minor=512, compatible=[isom, iso6, mp41]
    let mut ftyp_body = Vec::new();
    ftyp_body.extend_from_slice(b"isom");
    ftyp_body.extend_from_slice(&512u32.to_be_bytes());
    ftyp_body.extend_from_slice(b"isomiso6mp41");
    let ftyp = mp4_box(b"ftyp", &ftyp_body);

    // mvhd v0
    let mut mvhd_body = Vec::new();
    mvhd_body.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    mvhd_body.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    mvhd_body.extend_from_slice(&timescale.to_be_bytes());
    mvhd_body.extend_from_slice(&0u32.to_be_bytes()); // duration = 0 (fragmented)
    mvhd_body.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    mvhd_body.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    mvhd_body.extend_from_slice(&[0u8; 10]); // reserved
    // unity matrix (9 x u32)
    for m in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        mvhd_body.extend_from_slice(&m.to_be_bytes());
    }
    mvhd_body.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd_body.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
    let mvhd = mp4_fullbox(b"mvhd", 0, 0, &mvhd_body);

    // tkhd v0 (flags=3: enabled | in_movie)
    let mut tkhd_body = Vec::new();
    tkhd_body.extend_from_slice(&0u32.to_be_bytes()); // creation
    tkhd_body.extend_from_slice(&0u32.to_be_bytes()); // mod
    tkhd_body.extend_from_slice(&1u32.to_be_bytes()); // track_ID = 1
    tkhd_body.extend_from_slice(&0u32.to_be_bytes()); // reserved
    tkhd_body.extend_from_slice(&0u32.to_be_bytes()); // duration = 0
    tkhd_body.extend_from_slice(&[0u8; 8]);           // reserved
    tkhd_body.extend_from_slice(&[0u8; 8]);           // layer, alt_group, volume, reserved
    for m in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        tkhd_body.extend_from_slice(&m.to_be_bytes());
    }
    tkhd_body.extend_from_slice(&((width as u32) << 16).to_be_bytes());
    tkhd_body.extend_from_slice(&((height as u32) << 16).to_be_bytes());
    let tkhd = mp4_fullbox(b"tkhd", 0, 3, &tkhd_body);

    // mdhd v0
    let mut mdhd_body = Vec::new();
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&timescale.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0x55C4u16.to_be_bytes()); // language = und
    mdhd_body.extend_from_slice(&0u16.to_be_bytes());
    let mdhd = mp4_fullbox(b"mdhd", 0, 0, &mdhd_body);

    // hdlr
    let mut hdlr_body = Vec::new();
    hdlr_body.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    hdlr_body.extend_from_slice(b"vide");
    hdlr_body.extend_from_slice(&[0u8; 12]);
    hdlr_body.extend_from_slice(b"VideoHandler\0");
    let hdlr = mp4_fullbox(b"hdlr", 0, 0, &hdlr_body);

    // vmhd
    let vmhd = mp4_fullbox(b"vmhd", 0, 1, &[0u8; 8]);

    // dinf -> dref -> url
    let url_box = mp4_fullbox(b"url ", 0, 1, &[]);
    let mut dref_body = Vec::new();
    dref_body.extend_from_slice(&1u32.to_be_bytes());
    dref_body.extend_from_slice(&url_box);
    let dinf = mp4_box(b"dinf", &mp4_fullbox(b"dref", 0, 0, &dref_body));

    // avcC (ISO/IEC 14496-15)
    let mut avcc_body = Vec::new();
    avcc_body.push(1); // configurationVersion
    avcc_body.push(sps[1]); // AVCProfileIndication
    avcc_body.push(sps[2]); // profile_compatibility
    avcc_body.push(sps[3]); // AVCLevelIndication
    avcc_body.push(0xFF);   // lengthSizeMinusOne = 3 (4-byte NAL length prefix)
    avcc_body.push(0xE1);   // numOfSequenceParameterSets = 1
    avcc_body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(&sps);
    avcc_body.push(1);      // numOfPictureParameterSets = 1
    avcc_body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(&pps);
    let avcc = mp4_box(b"avcC", &avcc_body);

    // avc1 visual sample entry
    let mut avc1_body = Vec::new();
    avc1_body.extend_from_slice(&[0u8; 6]);            // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes());  // data_reference_index
    avc1_body.extend_from_slice(&[0u8; 16]);           // pre_defined + reserved
    avc1_body.extend_from_slice(&width.to_be_bytes());
    avc1_body.extend_from_slice(&height.to_be_bytes());
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // horizresolution 72 dpi
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // vertresolution 72 dpi
    avc1_body.extend_from_slice(&0u32.to_be_bytes());  // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes());  // frame_count
    avc1_body.extend_from_slice(&[0u8; 32]);           // compressorname
    avc1_body.extend_from_slice(&0x0018u16.to_be_bytes()); // depth = 24
    avc1_body.extend_from_slice(&(-1i16).to_be_bytes());   // pre_defined = -1
    avc1_body.extend_from_slice(&avcc);
    let avc1 = mp4_box(b"avc1", &avc1_body);

    // stsd
    let mut stsd_body = Vec::new();
    stsd_body.extend_from_slice(&1u32.to_be_bytes());
    stsd_body.extend_from_slice(&avc1);
    let stsd = mp4_fullbox(b"stsd", 0, 0, &stsd_body);

    // empty required sample tables for fragmented MP4
    let stts = mp4_fullbox(b"stts", 0, 0, &0u32.to_be_bytes());
    let stsc = mp4_fullbox(b"stsc", 0, 0, &0u32.to_be_bytes());
    let stsz = mp4_fullbox(b"stsz", 0, 0, &[0u8; 8]);
    let stco = mp4_fullbox(b"stco", 0, 0, &0u32.to_be_bytes());

    let mut stbl_body = Vec::new();
    stbl_body.extend_from_slice(&stsd);
    stbl_body.extend_from_slice(&stts);
    stbl_body.extend_from_slice(&stsc);
    stbl_body.extend_from_slice(&stsz);
    stbl_body.extend_from_slice(&stco);
    let stbl = mp4_box(b"stbl", &stbl_body);

    let mut minf_body = Vec::new();
    minf_body.extend_from_slice(&vmhd);
    minf_body.extend_from_slice(&dinf);
    minf_body.extend_from_slice(&stbl);
    let minf = mp4_box(b"minf", &minf_body);

    let mut mdia_body = Vec::new();
    mdia_body.extend_from_slice(&mdhd);
    mdia_body.extend_from_slice(&hdlr);
    mdia_body.extend_from_slice(&minf);
    let mdia = mp4_box(b"mdia", &mdia_body);

    let mut trak_body = Vec::new();
    trak_body.extend_from_slice(&tkhd);
    trak_body.extend_from_slice(&mdia);
    let trak = mp4_box(b"trak", &trak_body);

    // mvex -> trex
    let mut trex_body = Vec::new();
    trex_body.extend_from_slice(&1u32.to_be_bytes());           // track_ID = 1
    trex_body.extend_from_slice(&1u32.to_be_bytes());           // default_sample_description_index
    trex_body.extend_from_slice(&default_dur.to_be_bytes());    // default_sample_duration
    trex_body.extend_from_slice(&0u32.to_be_bytes());           // default_sample_size
    trex_body.extend_from_slice(&0x0200_0000u32.to_be_bytes()); // default_sample_flags (sync)
    let mvex = mp4_box(b"mvex", &mp4_fullbox(b"trex", 0, 0, &trex_body));

    let mut moov_body = Vec::new();
    moov_body.extend_from_slice(&mvhd);
    moov_body.extend_from_slice(&trak);
    moov_body.extend_from_slice(&mvex);
    let moov = mp4_box(b"moov", &moov_body);

    let mut out = Vec::with_capacity(ftyp.len() + moov.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    out
}

/// Synthesizes a valid fragmented MP4 media segment (`moof` + `mdat`) containing
/// `frame_count` real H.264 Baseline `I_PCM` IDR frames.
pub fn synth_fmp4_h264_segment(
    seq: u32,
    base_decode_time: u64,
    frame_count: u32,
    frame_dur: u32,
    mb_w: u32,
    mb_h: u32,
    y_base: u8,
    u_val: u8,
    v_val: u8,
) -> Vec<u8> {
    let mut mdat_payload = Vec::new();
    let mut sample_sizes = Vec::with_capacity(frame_count as usize);

    for i in 0..frame_count {
        let y = y_base.wrapping_add((i as u8).wrapping_mul(7)).clamp(24, 230);
        let nal = synth_h264_idr_ipcm(mb_w, mb_h, (seq + i) & 1, y, u_val, v_val);
        let avcc_len = nal.len() as u32;
        mdat_payload.extend_from_slice(&avcc_len.to_be_bytes());
        mdat_payload.extend_from_slice(&nal);
        sample_sizes.push(4 + avcc_len);
    }

    let mfhd = mp4_fullbox(b"mfhd", 0, 0, &seq.to_be_bytes());

    // tfhd: flags = 0x020008 (default-base-is-moof | default-sample-duration-present)
    let mut tfhd_body = Vec::new();
    tfhd_body.extend_from_slice(&1u32.to_be_bytes());         // track_ID = 1
    tfhd_body.extend_from_slice(&frame_dur.to_be_bytes());    // default_sample_duration
    let tfhd = mp4_fullbox(b"tfhd", 0, 0x020008, &tfhd_body);

    // tfdt v1 (64-bit baseMediaDecodeTime)
    let tfdt = mp4_fullbox(b"tfdt", 1, 0, &base_decode_time.to_be_bytes());

    // trun: flags = 0x000205 (data-offset-present | first-sample-flags-present | sample-size-present)
    let build_trun = |data_offset: i32| -> Vec<u8> {
        let mut trun_body = Vec::new();
        trun_body.extend_from_slice(&frame_count.to_be_bytes());
        trun_body.extend_from_slice(&data_offset.to_be_bytes());
        trun_body.extend_from_slice(&0x0200_0000u32.to_be_bytes()); // first_sample_flags = sync
        for &sz in &sample_sizes {
            trun_body.extend_from_slice(&sz.to_be_bytes());
        }
        mp4_fullbox(b"trun", 0, 0x000205, &trun_body)
    };

    // Compute exact moof size first so data_offset points to first byte of mdat payload.
    let trun_dummy = build_trun(0);
    let moof_size = 8 + mfhd.len() + 8 + tfhd.len() + tfdt.len() + trun_dummy.len();
    let data_offset = (moof_size + 8) as i32; // +8 for mdat box header
    let trun = build_trun(data_offset);

    let mut traf_body = Vec::new();
    traf_body.extend_from_slice(&tfhd);
    traf_body.extend_from_slice(&tfdt);
    traf_body.extend_from_slice(&trun);
    let traf = mp4_box(b"traf", &traf_body);

    let mut moof_body = Vec::new();
    moof_body.extend_from_slice(&mfhd);
    moof_body.extend_from_slice(&traf);
    let moof = mp4_box(b"moof", &moof_body);
    debug_assert_eq!(moof.len(), moof_size);

    let mdat = mp4_box(b"mdat", &mdat_payload);

    let mut out = Vec::with_capacity(moof.len() + mdat.len());
    out.extend_from_slice(&moof);
    out.extend_from_slice(&mdat);
    out
}

/// Synthesizes a compact EBML element (id + 8-byte VINT size + body).
fn ebml_el(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let id_bytes = id.to_be_bytes();
    let start = id_bytes.iter().position(|&b| b != 0).unwrap_or(3);
    out.extend_from_slice(&id_bytes[start..]);
    let len = body.len() as u64;
    // 4-byte VINT size (supports up to 256 MiB cleanly)
    let v = (len & 0x0FFF_FFFF) | 0x1000_0000;
    out.extend_from_slice(&(v as u32).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Synthesizes a compact WebM initialization segment for MSE pipeline verification.
pub fn synth_webm_init(track_type: u8, default_dur_ns: u32) -> Vec<u8> {
    let mut v = ebml_el(0x1A45_DFA3, &ebml_el(0x4282, b"webm"));
    // Unknown-size Segment header
    v.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    // Info (TimecodeScale = 1_000_000 ns = 1 ms)
    v.extend_from_slice(&ebml_el(0x1549_A966, &ebml_el(0x2A_D7B1, &1_000_000u32.to_be_bytes())));
    // Tracks -> TrackEntry
    let mut entry = Vec::new();
    entry.extend_from_slice(&ebml_el(0xD7, &[1])); // TrackNumber = 1
    entry.extend_from_slice(&ebml_el(0x83, &[track_type])); // 1=video, 2=audio
    entry.extend_from_slice(&ebml_el(0x23_E383, &default_dur_ns.to_be_bytes()));
    v.extend_from_slice(&ebml_el(0x1654_AE6B, &ebml_el(0xAE, &entry)));
    v
}

/// Synthesizes a compact WebM Cluster covering `frames` blocks of `step_ms` each.
pub fn synth_webm_cluster(timecode_ms: u64, frames: i16, step_ms: i16) -> Vec<u8> {
    let mut body = ebml_el(0xE7, &timecode_ms.to_be_bytes());
    for i in 0..frames {
        let mut blk = vec![0x81]; // track 1 VINT
        blk.extend_from_slice(&(i * step_ms).to_be_bytes());
        blk.push(0x80); // keyframe flag
        blk.extend_from_slice(&[0x42; 64]);
        body.extend_from_slice(&ebml_el(0xA3, &blk));
    }
    ebml_el(0x1F43_B675, &body)
}

/// Synthesizes a valid RIFF/WAVE 16-bit stereo PCM audio clip on the heap.
///
/// Generates a gentle 440 Hz (L) / 554 Hz (R) triangle wave so `sex-media::Decoder`
/// and `sexbell` receive real non-silent PCM audio without any static buffer.
pub fn synth_pcm_wav(duration_ms: u32, sample_rate: u32) -> Vec<u8> {
    let total_frames = ((sample_rate as u64) * (duration_ms as u64) / 1000) as usize;
    let data_bytes = total_frames * 4; // 2 channels * 2 bytes (i16)
    let riff_size = (36 + data_bytes) as u32;
    let byte_rate = sample_rate * 4;

    let mut out = Vec::with_capacity(44 + data_bytes);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes());  // AudioFormat = 1 (PCM)
    out.extend_from_slice(&2u16.to_le_bytes());  // NumChannels = 2 (stereo)
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());  // BlockAlign = 4
    out.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample = 16
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    let period_l = (sample_rate / 440).max(2) as usize;
    let period_r = (sample_rate / 554).max(2) as usize;
    let tri = |idx: usize, period: usize| -> i16 {
        let half = period / 2;
        let phase = idx % period;
        let v = if phase < half {
            (phase as i32 * 2) - half as i32
        } else {
            ((period - phase) as i32 * 2) - half as i32
        };
        ((v * 6000) / (half as i32).max(1)) as i16
    };

    for i in 0..total_frames {
        out.extend_from_slice(&tri(i, period_l).to_le_bytes());
        out.extend_from_slice(&tri(i, period_r).to_le_bytes());
    }
    out
}

/// Deterministic FNV-1a 64-bit hash over decoded 0x00RRGGBB pixel buffers.
pub fn fnv1a_pixels(pixels: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &p in pixels {
        for b in p.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Runs Test 2 core (MSE fMP4 + WebM segmenting, buffered ranges, seek lookup,
/// removal, timestampOffset, and codec matrix) in pure no_std Rust.
pub fn run_mse_pipeline_core_test(now_ms: impl Fn() -> u64) -> CaseResult {
    let t0 = now_ms();

    // 1. Codec support matrix check
    let (matched, total, bad_codecs) = check_codec_matrix();
    if matched != total {
        return CaseResult::fail(
            "mse_pipeline",
            now_ms().saturating_sub(t0),
            format!("codec_matrix={}/{} failed={:?}", matched, total, bad_codecs),
        );
    }

    // 2. Fragmented MP4 (H.264 I_PCM) pipeline: init + 2 contiguous 1s segments + 1 gap segment
    let init = synth_fmp4_h264_init(1, 1, 90_000, 3_000);
    let seg0 = synth_fmp4_h264_segment(1, 0, 30, 3_000, 1, 1, 120, 96, 160);
    let seg1 = synth_fmp4_h264_segment(2, 90_000, 30, 3_000, 1, 1, 160, 140, 100);
    let seg_gap = synth_fmp4_h264_segment(3, 360_000, 30, 3_000, 1, 1, 200, 80, 180);

    let mut mp4 = Stream::new(Format::Mp4);
    let mut all_mp4 = Vec::with_capacity(init.len() + seg0.len() + seg1.len() + seg_gap.len());
    all_mp4.extend_from_slice(&init);
    all_mp4.extend_from_slice(&seg0);
    all_mp4.extend_from_slice(&seg1);
    all_mp4.extend_from_slice(&seg_gap);
    for chunk in all_mp4.chunks(517) {
        if !mp4.append(chunk) {
            return CaseResult::fail(
                "mse_pipeline",
                now_ms().saturating_sub(t0),
                "fMP4 append rejected",
            );
        }
    }
    mp4.finish();

    let mp4_ranges = mp4.buffered();
    let mp4_ok = mp4.inited
        && mp4.has_video()
        && mp4_ranges.len() == 2
        && (mp4_ranges[0].0 - 0.0).abs() < 0.01
        && (mp4_ranges[0].1 - 2.0).abs() < 0.01
        && (mp4_ranges[1].0 - 4.0).abs() < 0.01
        && (mp4_ranges[1].1 - 5.0).abs() < 0.01
        && mp4.seg_at(1.5) == Some(1)
        && mp4.seg_at(4.5) == Some(2);
    if !mp4_ok {
        return CaseResult::fail(
            "mse_pipeline",
            now_ms().saturating_sub(t0),
            format!("fMP4 ranges={:?} segs={}", mp4_ranges, mp4.segments),
        );
    }

    // 3. WebM pipeline: init + 2 clusters (0..2s) + timestampOffset cluster (5..6s)
    let mut webm = Stream::new(Format::WebM);
    let mut all_webm = synth_webm_init(1, 40_000_000);
    all_webm.extend_from_slice(&synth_webm_cluster(0, 25, 40));
    all_webm.extend_from_slice(&synth_webm_cluster(1000, 25, 40));
    for chunk in all_webm.chunks(97) {
        if !webm.append(chunk) {
            return CaseResult::fail(
                "mse_pipeline",
                now_ms().saturating_sub(t0),
                "WebM append rejected",
            );
        }
    }
    webm.offset = 5.0;
    if !webm.append(&synth_webm_cluster(0, 25, 40)) {
        return CaseResult::fail(
            "mse_pipeline",
            now_ms().saturating_sub(t0),
            "WebM offset cluster rejected",
        );
    }
    webm.finish();

    let webm_ranges = webm.buffered();
    let webm_ok = webm.inited
        && webm.has_video()
        && webm_ranges.len() == 2
        && (webm_ranges[0].0 - 0.0).abs() < 0.01
        && (webm_ranges[0].1 - 2.0).abs() < 0.01
        && (webm_ranges[1].0 - 5.0).abs() < 0.01
        && (webm_ranges[1].1 - 6.0).abs() < 0.01
        && webm.seg_at(5.5) == Some(2);

    let elapsed = now_ms().saturating_sub(t0);
    if webm_ok {
        CaseResult::pass(
            "mse_pipeline",
            elapsed,
            format!(
                "codecs={}/{} fmp4_segs={} fmp4_ranges=0..2,4..5 webm_segs={} webm_ranges=0..2,5..6",
                matched, total, mp4.segments, webm.segments
            ),
        )
    } else {
        CaseResult::fail(
            "mse_pipeline",
            elapsed,
            format!("WebM ranges={:?} segs={}", webm_ranges, webm.segments),
        )
    }
}

// ── Test 4 Parsers: YouTube /oembed & Watch Page DASH/MSE Probe ────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OembedInfo {
    pub title: String,
    pub author: String,
    pub provider: String,
}

/// Extracts `title`, `author_name`, and `provider_name` from a YouTube `/oembed` JSON payload.
pub fn parse_oembed_json(body: &[u8]) -> Option<OembedInfo> {
    let text = core::str::from_utf8(body).ok()?;
    let title = json_string_field(text, "title")?;
    let author = json_string_field(text, "author_name").unwrap_or_default();
    let provider = json_string_field(text, "provider_name").unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    Some(OembedInfo { title, author, provider })
}

/// Result of analyzing a YouTube watch page for `ytInitialPlayerResponse` DASH formats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct YtWatchProbe {
    pub has_player_response: bool,
    pub total_formats: usize,
    pub mse_supported: usize,
    pub video_supported: usize,
    pub audio_supported: usize,
    pub sample_codecs: Vec<String>,
}

/// Inspects a YouTube watch HTML document for `ytInitialPlayerResponse` and checks every
/// discovered `mimeType` against [`mse::supported`].
pub fn analyze_yt_watch_html(html: &str) -> YtWatchProbe {
    let mut probe = YtWatchProbe::default();
    let Some(pos) = html.find("ytInitialPlayerResponse") else {
        return probe;
    };
    probe.has_player_response = true;
    let tail = &html[pos..];
    let search_slice = tail.find("streamingData").map(|s| &tail[s..]).unwrap_or(tail);

    let mut cur = search_slice;
    while let Some(idx) = cur.find("\"mimeType\"") {
        let after = &cur[idx + "\"mimeType\"".len()..];
        let Some(colon) = after.find(':') else { break };
        let val_part = after[colon + 1..].trim_start();
        if let Some((raw, consumed)) = parse_json_string(val_part) {
            probe.total_formats += 1;
            let supported = mse::supported(&raw);
            if supported {
                probe.mse_supported += 1;
                if raw.starts_with("video/") {
                    probe.video_supported += 1;
                } else if raw.starts_with("audio/") {
                    probe.audio_supported += 1;
                }
            }
            if probe.sample_codecs.len() < 4 && !probe.sample_codecs.contains(&raw) {
                probe.sample_codecs.push(raw);
            }
            cur = &val_part[consumed..];
        } else {
            cur = &after[colon + 1..];
        }
        // Bound scanning to the streamingData region so huge pages stay fast.
        if probe.total_formats >= 64 {
            break;
        }
    }
    probe
}

fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let idx = json.find(&needle)?;
    let after = &json[idx + needle.len()..];
    let colon = after.find(':')?;
    let val = after[colon + 1..].trim_start();
    parse_json_string(val).map(|(s, _)| s)
}

fn parse_json_string(s: &str) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return Some((out, i + 1));
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' if i + 4 < bytes.len() => {
                    if let Ok(hex) = core::str::from_utf8(&bytes[i + 1..i + 5]) {
                        if let Ok(code) = u32::from_str_radix(hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                    }
                    i += 4;
                }
                other => out.push(other as char),
            }
            i += 1;
            continue;
        }
        // Copy valid UTF-8 scalar
        let rest = &s[i..];
        let ch = rest.chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};

    static FAKE_NS: AtomicU64 = AtomicU64::new(1_000_000_000);
    fn fake_clock() -> u64 {
        FAKE_NS.fetch_add(16_000_000, Ordering::Relaxed)
    }

    #[test]
    fn codec_matrix_matches_expected_youtube_codecs() {
        let (matched, total, failed) = check_codec_matrix();
        assert_eq!(matched, total, "mismatched codecs: {:?}", failed);
        assert!(mse::supported(r#"video/mp4; codecs="avc1.640028""#));
        assert!(mse::supported(r#"video/webm; codecs="vp9""#));
        assert!(mse::supported(r#"video/mp4; codecs="av01.0.04M.08""#));
        assert!(mse::supported(r#"audio/mp4; codecs="mp4a.40.2""#));
        assert!(mse::supported(r#"audio/webm; codecs="opus""#));
        assert!(!mse::supported(r#"video/ogg; codecs="theora""#));
    }

    #[test]
    fn dom_js_timers_and_onclick_selftest_passes() {
        let res = run_dom_js_timers_test(fake_clock, || 0);
        assert_eq!(res.status, CaseStatus::Pass, "{}", res.format_line());
        assert!(res.detail.contains("video=1"));
        assert!(res.detail.contains("mse=5/5"));
        assert!(res.detail.contains("timer=1"));
        assert!(res.detail.contains("raf=1"));
        assert!(res.detail.contains("click=1"));
    }

    #[test]
    fn mse_pipeline_core_selftest_passes() {
        let res = run_mse_pipeline_core_test(|| 0);
        assert_eq!(res.status, CaseStatus::Pass, "{}", res.format_line());
        assert!(res.detail.contains("fmp4_segs=3"));
        assert!(res.detail.contains("webm_segs=3"));
    }

    #[test]
    fn synthetic_wav_and_pixel_hash_are_valid() {
        let wav = synth_pcm_wav(250, 48_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // 250 ms at 48 kHz stereo i16 = 12000 frames * 4 bytes = 48000 bytes + 44 header
        assert_eq!(wav.len(), 44 + 48_000);
        // Non-silent audio samples
        assert!(wav[44..].iter().any(|&b| b != 0));

        let px = vec![0x00FF_4488u32; 16 * 16];
        let h1 = fnv1a_pixels(&px);
        let h2 = fnv1a_pixels(&vec![0x0011_2233u32; 16 * 16]);
        assert_ne!(h1, 0);
        assert_ne!(h1, h2);
    }

    #[test]
    fn youtube_oembed_and_watch_probe_parsers() {
        let oembed = br#"{"title":"Test Clip","author_name":"SexOS Channel","provider_name":"YouTube"}"#;
        let info = parse_oembed_json(oembed).expect("valid oembed");
        assert_eq!(info.title, "Test Clip");
        assert_eq!(info.author, "SexOS Channel");
        assert_eq!(info.provider, "YouTube");

        let watch_html = r#"
            <html><script>
            var ytInitialPlayerResponse = {"streamingData":{"adaptiveFormats":[
                {"itag":137,"mimeType":"video/mp4; codecs=\"avc1.640028\""},
                {"itag":248,"mimeType":"video/webm; codecs=\"vp9\""},
                {"itag":399,"mimeType":"video/mp4; codecs=\"av01.0.04M.08\""},
                {"itag":140,"mimeType":"audio/mp4; codecs=\"mp4a.40.2\""},
                {"itag":251,"mimeType":"audio/webm; codecs=\"opus\""},
                {"itag":999,"mimeType":"video/ogg; codecs=\"theora\""}
            ]}};
            </script></html>
        "#;
        let probe = analyze_yt_watch_html(watch_html);
        assert!(probe.has_player_response);
        assert_eq!(probe.total_formats, 6);
        assert_eq!(probe.mse_supported, 5);
        assert_eq!(probe.video_supported, 3);
        assert_eq!(probe.audio_supported, 2);
    }

    #[test]
    fn summary_and_report_formatting() {
        let results = vec![
            CaseResult::pass("dom_js_timers", 12, "video=1 mse=5/5"),
            CaseResult::fail("yt_net_probe", 45, "no +net grant"),
        ];
        assert_eq!(
            results[0].format_line(),
            "[web.selftest] case=dom_js_timers status=PASS ms=12 detail=video=1 mse=5/5"
        );
        assert_eq!(
            results[1].format_line(),
            "[web.selftest] case=yt_net_probe status=FAIL ms=45 detail=no +net grant"
        );
        let summary = Summary::from_results(&results);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.format_line(), "[web.selftest.summary] passed=1 failed=1");

        let html = render_report_html("selftest-net", &results, &summary);
        assert!(html.contains("Kaleidoscope Browser Media"));
        assert!(html.contains("dom_js_timers"));
        assert!(html.contains("yt_net_probe"));
        assert!(html.contains("[web.selftest.summary] passed=1 failed=1"));
    }
}
