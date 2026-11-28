// SPDX-License-Identifier: GPL-3.0-only
//! sex-rtp on the host: packets, extensions, RTCP, payload formats, jitter buffer, BWE.
use sex_rtp::codec::{annexb_nals, vp8_descriptor, Opus, Vp8, Vp9, H264};
use sex_rtp::packet::{abs_send_time, abs_send_time_us, twcc_seq};
use sex_rtp::rtcp::{nack_pairs, nack_seqs, ReceiveStats};
use sex_rtp::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn rtp_header_with_one_byte_extensions() {
    // A Chrome-shaped VP8 packet: abs-send-time (id 2), transport-cc (id 3), mid (id 4).
    let b = hex("90e0 1234 00000064 deadbeef bede0003 22 0a0b0c 31 0102 40 30 000000 aabbcc");
    let p = Packet::parse(&b).unwrap();
    assert!(p.marker);
    assert_eq!((p.payload_type, p.seq, p.timestamp, p.ssrc), (96, 0x1234, 100, 0xdeadbeef));
    assert_eq!(p.extension(2), Some(&[0x0a, 0x0b, 0x0c][..]));
    assert_eq!(twcc_seq(p.extension(3).unwrap()), Some(0x0102));
    assert_eq!(p.extension(4), Some(&b"0"[..]));
    assert_eq!(p.payload, hex("aabbcc"));
    let again = Packet::parse(&p.write()).unwrap();
    assert_eq!(again, p);
}

#[test]
fn two_byte_extensions_and_padding_and_csrc() {
    let mut p = Packet { payload_type: 111, seq: 9, timestamp: 960, ssrc: 1, csrc: vec![7, 8], padding: 4, ..Default::default() };
    p.set_extension(1, b"a-very-long-rid-value-over-16".to_vec());
    p.set_extension(20, vec![1]);
    p.payload = vec![1, 2, 3];
    let w = p.write();
    assert_eq!(&w[12 + 8..12 + 10], &[0x10, 0x00]);
    assert_eq!(Packet::parse(&w).unwrap(), p);
}

#[test]
fn abs_send_time_fixed_point() {
    let b = abs_send_time(1_500_000);
    // 1.5 s in 6.18 is 0x060000.
    assert_eq!(b, [0x06, 0x00, 0x00]);
    assert_eq!(abs_send_time_us(&b), Some(1_500_000));
}

#[test]
fn seq_unwrap_across_wrap() {
    let mut u = SeqUnwrapper::default();
    let a = u.unwrap(65534);
    let b = u.unwrap(1);
    let c = u.unwrap(65535);
    assert_eq!(b - a, 3);
    assert_eq!(c - a, 1);
}

#[test]
fn rtcp_compound_roundtrip() {
    let rb = ReportBlock { ssrc: 5, fraction_lost: 25, cumulative_lost: -3, highest_seq: 70000, jitter: 12, last_sr: 0x1234_5678, delay_since_last_sr: 65536 };
    let pkts = vec![
        Rtcp::SenderReport { ssrc: 1, ntp: 0xe000_0000_8000_0000, rtp_ts: 90000, packets: 10, octets: 12000, reports: vec![rb] },
        Rtcp::ReceiverReport { ssrc: 2, reports: vec![rb, rb] },
        Rtcp::Sdes(vec![(1, "sexos-cname".into()), (3, "x".into())]),
        Rtcp::Nack { sender_ssrc: 1, media_ssrc: 2, pairs: vec![(100, 0b101)] },
        Rtcp::Pli { sender_ssrc: 1, media_ssrc: 2 },
        Rtcp::Fir { sender_ssrc: 1, entries: vec![(2, 7)] },
        Rtcp::Remb { sender_ssrc: 1, bitrate: 1_500_000 >> 3 << 3, ssrcs: vec![2, 3] },
        Rtcp::Bye { ssrcs: vec![1], reason: Some("bye".into()) },
    ];
    let b = Rtcp::compound(&pkts);
    assert_eq!(Rtcp::parse_compound(&b).unwrap(), pkts);
    // The SR header: V=2 RC=1 PT=200, length 12 words.
    assert_eq!(&b[..4], &[0x81, 200, 0, 12]);
}

#[test]
fn known_rtcp_bytes() {
    // PLI from Chrome: 81 ce 00 02, sender, media.
    let pli = hex("81ce0002 00000001 4d2a6e5f");
    assert_eq!(Rtcp::parse_compound(&pli).unwrap(), vec![Rtcp::Pli { sender_ssrc: 1, media_ssrc: 0x4d2a6e5f }]);
    // REMB 'REMB' n=1 exp=2 mantissa=0x3d090 (1 Mbps = 250000 << 2).
    let remb = hex("8fce0005 00000001 00000000 52454d42 010bd090 4d2a6e5f");
    assert_eq!(
        Rtcp::parse_compound(&remb).unwrap(),
        vec![Rtcp::Remb { sender_ssrc: 1, bitrate: 1_000_000, ssrcs: vec![0x4d2a6e5f] }]
    );
    assert_eq!(Rtcp::compound(&[Rtcp::Remb { sender_ssrc: 1, bitrate: 1_000_000, ssrcs: vec![0x4d2a6e5f] }]), remb);
}

#[test]
fn nack_pairs_both_ways() {
    let seqs = vec![65530, 65531, 65535, 2, 40];
    let pairs = nack_pairs(&seqs);
    assert_eq!(pairs, vec![(65530, 0b1001_0001), (40, 0)]);
    assert_eq!(nack_seqs(&pairs), seqs);
}

#[test]
fn transport_cc_roundtrip_and_known_bytes() {
    // Draft example shape: base 1, 3 packets, run chunk "small delta" x3, deltas 1,2,3.
    let b = hex("af cd 0006 00000001 00000002 0001 0003 000010 00 2003 010203 000003");
    let p = Rtcp::parse_compound(&b).unwrap();
    let Rtcp::TransportCc(t) = &p[0] else { panic!() };
    assert_eq!((t.base_seq, t.reference_time, t.fb_count), (1, 16, 0));
    assert_eq!(t.deltas, vec![Some(1), Some(2), Some(3)]);
    assert_eq!(Rtcp::compound(&p), b);
    let a = t.arrivals();
    assert_eq!(a[2], (3, Some(16 * 64_000 + 6 * 250)));

    // Losses, large and negative deltas go through status vectors.
    let mut r = TwccRecorder::default();
    let mut t_us = 10_000_000u64;
    for s in 0u16..40 {
        if s % 7 == 3 {
            continue;
        }
        t_us += if s == 20 { 100_000 } else { 1000 };
        r.record(s.wrapping_add(65520), if s == 30 { t_us - 3000 } else { t_us });
    }
    let fb = r.feedback(1, 2).unwrap();
    assert_eq!(fb.base_seq, 65520);
    let w = Rtcp::compound(&[Rtcp::TransportCc(fb.clone())]);
    assert_eq!(w.len() % 4, 0);
    let back = Rtcp::parse_compound(&w).unwrap();
    assert_eq!(back, vec![Rtcp::TransportCc(fb.clone())]);
    let arr = fb.arrivals();
    assert_eq!(arr.len(), 40);
    assert!(arr[3].1.is_none() && arr[4].1.is_some());
    // Next feedback starts after the last one.
    r.record(65520u16.wrapping_add(40), t_us + 1000);
    assert_eq!(r.feedback(1, 2).unwrap().base_seq, 65520u16.wrapping_add(40));
}

#[test]
fn receive_stats_loss_and_jitter() {
    let mut st = ReceiveStats::default();
    for s in 0u16..100 {
        if s % 10 == 0 && s > 0 {
            continue;
        }
        st.packet(s, s as u32 * 3000, s as u32 * 3000 + if s % 2 == 0 { 0 } else { 900 });
    }
    let r = st.report(9, 0);
    assert_eq!(r.cumulative_lost, 9);
    assert_eq!(r.fraction_lost, (9 * 256 / 100) as u8);
    assert_eq!(r.highest_seq, 99);
    assert!(r.jitter > 500 && r.jitter < 900, "{}", r.jitter);
}

fn roundtrip(p: &mut dyn Packetizer, d: &mut dyn Depacketizer, frame: &[u8], mtu: usize) -> (Vec<Vec<u8>>, Frame) {
    let pk = p.packetize(frame, mtu);
    assert!(pk.iter().all(|x| x.len() <= mtu), "payload over mtu");
    let mut out = None;
    for (i, x) in pk.iter().enumerate() {
        let f = d.push(x, i + 1 == pk.len());
        assert!(f.is_none() || i + 1 == pk.len());
        out = out.or(f);
    }
    (pk, out.unwrap())
}

#[test]
fn opus_vp8_vp9_payloads() {
    let (pk, f) = roundtrip(&mut Opus, &mut Opus, &[0xfc, 1, 2, 3], 1200);
    assert_eq!((pk.len(), f.data), (1, vec![0xfc, 1, 2, 3]));

    let key: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8 & 0xfe).collect();
    let mut tx = Vp8::default();
    tx.picture_id = 0x7ffe;
    let mut rx = Vp8::default();
    let (pk, f) = roundtrip(&mut tx, &mut rx, &key, 1000);
    assert_eq!(pk.len(), 6);
    assert_eq!(f, Frame { data: key.clone(), keyframe: true });
    assert_eq!(&pk[0][..4], &[0x90, 0x80, 0xff, 0xfe]);
    assert_eq!(vp8_descriptor(&pk[1]), Some((4, false, 0)));
    let (pk, f) = roundtrip(&mut tx, &mut rx, &[1, 2, 3], 1000);
    assert_eq!(&pk[0][2..4], &[0xff, 0xff]);
    assert_eq!(tx.picture_id, 0);
    assert!(!f.keyframe);
    // Short descriptor from another sender: X=0.
    let mut rx = Vp8::default();
    assert_eq!(rx.push(&[0x10, 0x00, 9, 9], true).unwrap().data, vec![0, 9, 9]);

    let mut tx = Vp9::default();
    (tx.keyframe, tx.width, tx.height) = (true, 1280, 720);
    let mut rx = Vp9::default();
    let (pk, f) = roundtrip(&mut tx, &mut rx, &key, 1100);
    assert_eq!(f, Frame { data: key.clone(), keyframe: true });
    assert_eq!(pk[0][0], 0x8a);
    assert_eq!(&pk[0][4..8], &[0x05, 0x00, 0x02, 0xd0]);
    tx.keyframe = false;
    let (pk, f) = roundtrip(&mut tx, &mut rx, &key[..2000], 1100);
    assert_eq!(pk[0][0], 0xc8);
    assert_eq!(f, Frame { data: key[..2000].to_vec(), keyframe: false });
}

#[test]
fn h264_single_stap_fua() {
    let sps = [0x67, 0x42, 0xc0, 0x1f, 0xda];
    let pps = [0x68, 0xce, 0x3c, 0x80];
    let idr: Vec<u8> = std::iter::once(0x65).chain((0..3000u32).map(|i| (i % 251) as u8 + 1)).collect();
    let mut frame = vec![0, 0, 0, 1];
    frame.extend_from_slice(&sps);
    frame.extend_from_slice(&[0, 0, 1]);
    frame.extend_from_slice(&pps);
    frame.extend_from_slice(&[0, 0, 0, 1]);
    frame.extend_from_slice(&idr);
    assert_eq!(annexb_nals(&frame).len(), 3);
    let (pk, f) = roundtrip(&mut H264::default(), &mut H264::default(), &frame, 1200);
    // STAP-A of SPS+PPS, then FU-A start, middle, end.
    assert_eq!(pk[0][0] & 0x1f, 24);
    assert_eq!(&pk[0][1..3], &[0, 5]);
    assert_eq!((pk[1][0], pk[1][1]), (0x7c, 0x85));
    assert_eq!(pk.last().unwrap()[1], 0x45);
    assert!(f.keyframe);
    let mut want = vec![0, 0, 0, 1];
    want.extend_from_slice(&sps);
    want.extend_from_slice(&[0, 0, 0, 1]);
    want.extend_from_slice(&pps);
    want.extend_from_slice(&[0, 0, 0, 1]);
    want.extend_from_slice(&idr);
    assert_eq!(f.data, want);
    // A small P slice goes as one single NAL packet.
    let (pk, f) = roundtrip(&mut H264::default(), &mut H264::default(), &[0, 0, 0, 1, 0x41, 9, 9], 1200);
    assert_eq!(pk, vec![vec![0x41, 9, 9]]);
    assert!(!f.keyframe);
}

fn video_packets(stream: &mut RtpStream, frames: usize, per: usize) -> Vec<Packet> {
    let mut v = Vec::new();
    for f in 0..frames {
        let payloads = (0..per).map(|i| vec![f as u8, i as u8]).collect();
        v.extend(stream.packets(payloads, f as u32 * 3000));
    }
    v
}

#[test]
fn jitter_buffer_orders_frames_and_nacks() {
    let mut s = RtpStream::new(1, 96, 65530);
    let pkts = video_packets(&mut s, 4, 3);
    let mut jb = JitterBuffer::new(JitterConfig::video());
    jb.rtt_ms = 50;
    // Frame 1's middle packet (index 4) is lost at first; frame 2 arrives reordered.
    let order = [0, 1, 2, 3, 5, 8, 7, 6, 9, 10, 11];
    let mut out = Vec::new();
    for &i in &order {
        jb.push(pkts[i].clone(), 0);
        while let Some(f) = jb.pop(0) {
            out.push(f);
        }
    }
    assert_eq!(out.len(), 1);
    let n = jb.nacks(0);
    assert_eq!(n, vec![pkts[4].seq]);
    assert!(jb.nacks(10).is_empty());
    assert_eq!(jb.nacks(60), vec![pkts[4].seq]);
    // The retransmission from the stream's history fills the gap.
    jb.push(s.resend(pkts[4].seq).unwrap().clone(), 70);
    while let Some(f) = jb.pop(70) {
        out.push(f);
    }
    assert_eq!(out.len(), 4);
    for (k, f) in out.iter().enumerate() {
        assert_eq!(f.iter().map(|p| p.payload.clone()).collect::<Vec<_>>(), (0..3).map(|i| vec![k as u8, i]).collect::<Vec<_>>());
    }
    assert!(jb.nacks(1000).is_empty());
}

#[test]
fn jitter_buffer_gives_up_on_a_lost_frame() {
    let mut s = RtpStream::new(1, 96, 100);
    let pkts = video_packets(&mut s, 3, 2);
    let mut jb = JitterBuffer::new(JitterConfig::video());
    for (i, p) in pkts.iter().enumerate() {
        if i != 2 {
            jb.push(p.clone(), 0);
        }
    }
    assert_eq!(jb.pop(0).unwrap()[0].payload, vec![0, 0]);
    assert!(jb.pop(100).is_none());
    // After max_delay the broken frame 1 is dropped and frame 2 comes out.
    assert_eq!(jb.pop(600).unwrap()[0].payload, vec![2, 0]);
    assert_eq!(jb.lost_frames, 1);

    let mut jb = JitterBuffer::new(JitterConfig::audio());
    let mut a = RtpStream::new(2, 111, 0);
    let ap: Vec<Packet> = (0..3).flat_map(|i| a.packets(vec![vec![i]], i as u32 * 960)).collect();
    jb.push(ap[0].clone(), 0);
    jb.push(ap[2].clone(), 0);
    assert_eq!(jb.pop(0).unwrap()[0].payload, vec![0]);
    assert!(jb.pop(50).is_none());
    assert_eq!(jb.pop(200).unwrap()[0].payload, vec![2]);
}

/// A bottleneck link: `cap_bps`, 20 ms one way, drop-tail queue of 200 ms.
struct Link {
    cap_bps: f64,
    free_at: f64,
}

impl Link {
    fn send(&mut self, t_us: f64, size: usize) -> Option<f64> {
        let start = self.free_at.max(t_us);
        if start - t_us > 200_000.0 {
            return None;
        }
        self.free_at = start + size as f64 * 8.0 * 1e6 / self.cap_bps;
        Some(self.free_at + 20_000.0)
    }
}

fn simulate(cap_bps: f64, seconds: u64, random_loss: u64) -> (Vec<u32>, BandwidthEstimator) {
    let mut bwe = BandwidthEstimator::new(300_000, 50_000, 5_000_000);
    let mut link = Link { cap_bps, free_at: 0.0 };
    let mut rec = TwccRecorder::default();
    let mut seq = 0u16;
    let mut pending: Vec<(f64, u16)> = Vec::new();
    let mut targets = Vec::new();
    let mut rng = 12345u64;
    let mut t = 0u64;
    let mut next_fb = 50_000u64;
    let mut budget = 0.0f64;
    while t < seconds * 1_000_000 {
        // Paced sender: 1200-byte packets at the current target, every 5 ms.
        budget += bwe.target_bps() as f64 * 0.005 / 8.0;
        while budget >= 1200.0 {
            budget -= 1200.0;
            bwe.on_sent(seq, 1200, t as i64);
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let lost = random_loss > 0 && (rng >> 33) % 100 < random_loss;
            if let (false, Some(a)) = (lost, link.send(t as f64, 1200)) {
                pending.push((a, seq));
            }
            seq = seq.wrapping_add(1);
        }
        pending.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let now = t as f64;
        let arrived: Vec<_> = pending.iter().filter(|p| p.0 <= now).copied().collect();
        pending.retain(|p| p.0 > now);
        for (a, s) in arrived {
            rec.record(s, a as u64);
        }
        if t >= next_fb {
            next_fb += 50_000;
            if let Some(fb) = rec.feedback(1, 2) {
                // Feedback takes the 20 ms back path.
                bwe.on_feedback(&fb, t as i64 + 20_000);
            }
            targets.push(bwe.target_bps());
        }
        t += 5_000;
    }
    (targets, bwe)
}

#[test]
fn bwe_finds_a_bottleneck() {
    let (targets, _) = simulate(1_000_000.0, 40, 0);
    let tail = &targets[targets.len() - 200..];
    eprintln!("1 Mbps link: {:?}", targets.iter().step_by(40).collect::<Vec<_>>());
    let avg = tail.iter().map(|&x| x as f64).sum::<f64>() / tail.len() as f64;
    assert!(avg > 550_000.0 && avg < 1_150_000.0, "avg {avg} {:?}", &tail[..20]);
    // It ramps up from the start rate on a fast link.
    let (targets, _) = simulate(4_000_000.0, 30, 0);
    assert!(*targets.last().unwrap() > 1_500_000, "{:?}", &targets[targets.len() - 10..]);
}

#[test]
fn bwe_backs_off_on_loss() {
    let (targets, bwe) = simulate(10_000_000.0, 20, 20);
    eprintln!("20% loss: {:?} loss {}", targets.iter().step_by(40).collect::<Vec<_>>(), bwe.last_loss);
    assert!(bwe.last_loss > 0.05 || bwe.target_bps() <= 60_000, "{}", bwe.last_loss);
    assert!(*targets.last().unwrap() < 300_000, "{:?}", &targets[targets.len() - 10..]);
}
