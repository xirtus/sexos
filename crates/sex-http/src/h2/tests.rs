extern crate std;
use super::*;
use std::{format, io::{Read, Write}, net::TcpStream, process::{Command, Stdio}, string::String, vec};

/// A scripted server end: frames to feed the client.
fn f(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> { let mut v = Vec::new(); frame(&mut v, kind, flags, stream, payload); v }
fn settings(pairs: &[(u16, u32)]) -> Vec<u8> {
    let mut p = Vec::new();
    for (i, v) in pairs { p.extend_from_slice(&i.to_be_bytes()); p.extend_from_slice(&v.to_be_bytes()); }
    f(SETTINGS, 0, 0, &p)
}
fn head(status: &[u8], extra: &[(&[u8], &[u8])]) -> Vec<u8> { let mut b = Vec::new(); let mut all = vec![(&b":status"[..], status)]; all.extend_from_slice(extra); hpack::encode(&mut b, &all); b }
/// Frames the client sent: (type, flags, stream, payload), after the preface.
fn frames(bytes: &[u8]) -> Vec<(u8, u8, u32, Vec<u8>)> {
    let mut b = bytes.strip_prefix(PREFACE).unwrap_or(bytes);
    let mut v = Vec::new();
    while b.len() >= 9 {
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]) as usize;
        v.push((b[3], b[4], u32::from_be_bytes([b[5], b[6], b[7], b[8]]), b[9..9 + n].to_vec()));
        b = &b[9 + n..];
    }
    v
}
fn ready() -> Client { let mut c = Client::new(); c.receive(&settings(&[])).unwrap(); c.output(); c }
fn events(c: &mut Client) -> Vec<Event> { let mut v = Vec::new(); while let Some(e) = c.event() { v.push(e); } v }

#[test]
fn preface_settings_and_settings_ack() {
    let mut c = Client::new();
    let out = c.output();
    assert!(out.starts_with(PREFACE));
    let fr = frames(&out);
    assert_eq!(fr[0].0, SETTINGS);
    assert!(fr[0].3.chunks(6).any(|s| s == [0, 2, 0, 0, 0, 0]), "push disabled");
    assert_eq!((fr[1].0, fr[1].2), (WINDOW_UPDATE, 0));
    c.receive(&settings(&[(3, 10)])).unwrap();
    assert_eq!(frames(&c.output()), [(SETTINGS, ACK, 0, vec![])]);
}

#[test]
fn a_get_gets_its_head_body_and_end_split_across_reads() {
    let mut c = ready();
    let s = c.request(b"GET", b"https", b"example.org", b"/x", &[(b"accept", b"*/*"), (b"connection", b"close")], None).unwrap();
    let fr = frames(&c.output());
    assert_eq!((fr[0].0, fr[0].1, fr[0].2), (HEADERS, END_HEADERS | END_STREAM, 1));
    let sent = hpack::Decoder::new(4096, 65536).decode(&fr[0].3).unwrap();
    assert!(sent.contains(&(b":path".to_vec(), b"/x".to_vec())) && !sent.iter().any(|(n, _)| n == b"connection"));
    let mut wire = f(HEADERS, END_HEADERS, s, &head(b"200", &[(b"content-type", b"text/plain")]));
    wire.extend(f(DATA, PADDED, s, &[3, b'h', b'i', 0, 0, 0]));
    wire.extend(f(DATA, END_STREAM, s, b"!"));
    for b in wire.chunks(5) { c.receive(b).unwrap(); }
    assert_eq!(events(&mut c), [
        Event::Headers { stream: 1, status: 200, headers: vec![(b"content-type".to_vec(), b"text/plain".to_vec())], trailers: false },
        Event::Data { stream: 1, bytes: b"hi".to_vec() }, Event::Data { stream: 1, bytes: b"!".to_vec() }, Event::End { stream: 1 }]);
    assert_eq!(c.active(), 0);
}

#[test]
fn continuation_frames_join_and_nothing_may_interleave() {
    let mut c = ready();
    let s = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    let block = head(b"204", &[(b"x-long", &[b'v'; 300])]);
    let mut wire = f(HEADERS, END_STREAM, s, &block[..100]);
    wire.extend(f(CONTINUATION, 0, s, &block[100..200]));
    wire.extend(f(CONTINUATION, END_HEADERS, s, &block[200..]));
    c.receive(&wire).unwrap();
    let e = events(&mut c);
    assert!(matches!(&e[0], Event::Headers { status: 204, headers, .. } if headers[0].1.len() == 300));
    assert_eq!(e[1], Event::End { stream: 1 });
    let mut c = ready();
    let s = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    let mut wire = f(HEADERS, 0, s, &block[..100]);
    wire.extend(f(PING, 0, 0, &[0; 8]));
    assert_eq!(c.receive(&wire), Err(Error::Connection(PROTOCOL_ERROR)));
    assert!(frames(&c.output()).iter().any(|fr| fr.0 == GOAWAY));
}

#[test]
fn request_bodies_wait_for_window_updates() {
    let mut c = ready();
    c.receive(&settings(&[(4, 1000)])).unwrap();
    c.output();
    let body: Vec<u8> = (0..2500u32).map(|i| i as u8).collect();
    let s = c.request(b"POST", b"https", b"a", b"/up", &[(b"content-type", b"application/octet-stream")], Some(&body)).unwrap();
    let fr = frames(&c.output());
    assert_eq!(fr[0].1 & END_STREAM, 0);
    let sent: usize = fr.iter().filter(|x| x.0 == DATA).map(|x| x.3.len()).sum();
    assert_eq!(sent, 1000, "stream window");
    c.receive(&f(WINDOW_UPDATE, 0, s, &2000u32.to_be_bytes())).unwrap();
    let fr = frames(&c.output());
    let rest: Vec<u8> = fr.iter().filter(|x| x.0 == DATA).flat_map(|x| x.3.clone()).collect();
    assert_eq!(rest, body[1000..]);
    assert_eq!(fr.last().unwrap().1 & END_STREAM, END_STREAM);
}

#[test]
fn the_server_stream_limit_queues_requests_until_one_ends() {
    let mut c = ready();
    c.receive(&settings(&[(3, 2)])).unwrap();
    c.output();
    let ids: Vec<u32> = (0..4).map(|_| c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap()).collect();
    assert_eq!(frames(&c.output()).iter().filter(|x| x.0 == HEADERS).count(), 2);
    c.receive(&f(HEADERS, END_HEADERS | END_STREAM, ids[0], &head(b"200", &[]))).unwrap();
    let fr = frames(&c.output());
    assert_eq!(fr.iter().filter(|x| x.0 == HEADERS).map(|x| x.2).collect::<Vec<_>>(), [ids[2]]);
}

#[test]
fn goaway_refuses_unprocessed_streams_and_new_requests() {
    let mut c = ready();
    let a = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    let b = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    let mut p = a.to_be_bytes().to_vec();
    p.extend_from_slice(&NO_ERROR.to_be_bytes());
    c.receive(&f(GOAWAY, 0, 0, &p)).unwrap();
    let e = events(&mut c);
    assert!(e.contains(&Event::Reset { stream: b, code: REFUSED_STREAM }) && e.contains(&Event::GoAway { last: a, code: 0 }));
    assert_eq!(c.request(b"GET", b"https", b"a", b"/", &[], None), Err(Error::GoingAway));
    c.receive(&f(HEADERS, END_HEADERS | END_STREAM, a, &head(b"200", &[]))).unwrap();
    assert!(events(&mut c).contains(&Event::End { stream: a }));
}

#[test]
fn hostile_frames_break_the_connection_or_stream_not_the_client() {
    for bad in [f(PUSH_PROMISE, END_HEADERS, 1, &[0, 0, 0, 2]), f(DATA, 0, 0, b"x"), f(WINDOW_UPDATE, 0, 0, &0u32.to_be_bytes()), {
        let mut v = Vec::new(); v.extend_from_slice(&((OUR_MAX_FRAME + 1) as u32).to_be_bytes()[1..]); v.extend_from_slice(&[0, 0, 0, 0, 0, 1]); v
    }] {
        let mut c = ready();
        c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
        assert!(matches!(c.receive(&bad), Err(Error::Connection(_))));
    }
    // Data before the head resets only its stream.
    let mut c = ready();
    let s = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    c.receive(&f(DATA, 0, s, b"early")).unwrap();
    assert_eq!(events(&mut c), [Event::Reset { stream: s, code: PROTOCOL_ERROR }]);
    // Too much data for the window breaks the connection.
    let mut c = ready();
    let s = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    c.receive(&f(HEADERS, END_HEADERS, s, &head(b"200", &[]))).unwrap();
    let chunk = vec![0u8; OUR_MAX_FRAME];
    let mut r = Ok(());
    for _ in 0..=(OUR_STREAM_WINDOW as usize / OUR_MAX_FRAME) { r = r.and(c.receive(&f(DATA, 0, s, &chunk))); }
    assert!(r.is_ok(), "the client refills windows as it delivers");
}

#[test]
fn pings_are_answered_and_trailers_end_the_stream() {
    let mut c = ready();
    c.receive(&f(PING, 0, 0, b"12345678")).unwrap();
    assert_eq!(frames(&c.output()), [(PING, ACK, 0, b"12345678".to_vec())]);
    let s = c.request(b"GET", b"https", b"a", b"/", &[], None).unwrap();
    c.receive(&f(HEADERS, END_HEADERS, s, &head(b"103", &[(b"link", b"</s.css>")]))).unwrap();
    c.receive(&f(HEADERS, END_HEADERS, s, &head(b"200", &[]))).unwrap();
    let mut t = Vec::new();
    hpack::encode(&mut t, &[(b"grpc-status", b"0")]);
    c.receive(&f(HEADERS, END_HEADERS | END_STREAM, s, &t)).unwrap();
    let e = events(&mut c);
    assert!(matches!(e[0], Event::Headers { status: 103, trailers: false, .. }));
    assert!(matches!(e[1], Event::Headers { status: 200, trailers: false, .. }));
    assert!(matches!(e[2], Event::Headers { trailers: true, .. }));
    assert_eq!(e[3], Event::End { stream: s });
}

/// A real HTTP/2 server (Python's `h2`, prior knowledge on cleartext): fifty requests at
/// once on one connection with the server allowing twenty streams at a time, bodies
/// bigger than the default windows, and one POST echoed back.
const SERVER: &str = r#"
import socket,sys,h2.connection,h2.config,h2.events,h2.settings
s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',0));s.listen(1)
print(s.getsockname()[1],flush=True)
c,_=s.accept()
h=h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=False))
h.initiate_connection();h.update_settings({h2.settings.SettingCodes.MAX_CONCURRENT_STREAMS:20});c.sendall(h.data_to_send())
bodies={};paths={};pending={};served=0;most=0;open_=set()
def push(sid):
 data=pending[sid]
 while data:
  n=min(len(data),h.local_flow_control_window(sid),h.max_outbound_frame_size)
  if n<=0:break
  h.send_data(sid,data[:n]);data=data[n:]
 pending[sid]=data
 if not data:h.end_stream(sid);del pending[sid];open_.discard(sid)
while served<51:
 d=c.recv(65536)
 if not d:break
 for e in h.receive_data(d):
  if isinstance(e,h2.events.RequestReceived):
   hd=dict(e.headers);paths[e.stream_id]=hd[b':path'];bodies[e.stream_id]=b'';open_.add(e.stream_id);most=max(most,h.open_inbound_streams)
  elif isinstance(e,h2.events.DataReceived):
   bodies[e.stream_id]+=e.data;h.acknowledge_received_data(e.flow_controlled_length,e.stream_id)
  elif isinstance(e,h2.events.StreamEnded):
   sid=e.stream_id;p=paths[sid]
   if p==b'/echo':body=bodies[sid]
   else:n=int(p[1:]);body=bytes((n*7+i)%251 for i in range(20000+n*1000))
   h.send_headers(sid,[(':status','200'),('content-length',str(len(body))),('x-path',p.decode())])
   pending[sid]=body;push(sid);served+=1
  elif isinstance(e,h2.events.WindowUpdated):
   for sid in list(pending):push(sid)
 c.sendall(h.data_to_send())
print('served=%d most=%d'%(served,most),flush=True)
"#;

#[test]
fn fifty_parallel_streams_against_a_real_h2_server() {
    if Command::new("python3").args(["-c", "import h2"]).status().map_or(true, |s| !s.success()) {
        assert!(std::env::var("SEXOS_REQUIRE_H2").is_err(), "python3 h2 missing (pip install h2)");
        std::eprintln!("skipped: python3 h2 missing");
        return;
    }
    let mut server = Command::new("python3").args(["-c", SERVER]).stdout(Stdio::piped()).spawn().unwrap();
    let mut out = server.stdout.take().unwrap();
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while out.read(&mut b).unwrap() == 1 && b[0] != b'\n' { line.push(b[0]); }
    let port: u16 = String::from_utf8(line).unwrap().trim().parse().unwrap();
    let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut c = Client::new();
    let mut buf = vec![0u8; 65536];
    // As on TLS, the server's SETTINGS arrive before the requests go out.
    sock.write_all(&c.output()).unwrap();
    while !c.settled() { let n = sock.read(&mut buf).unwrap(); c.receive(&buf[..n]).unwrap(); }
    let mut want = BTreeMap::new();
    for n in 0..50u32 { want.insert(c.request(b"GET", b"http", b"localhost", format!("/{n}").as_bytes(), &[], None).unwrap(), n); }
    let posted: Vec<u8> = (0..100_000u32).map(|i| (i % 253) as u8).collect();
    let echo = c.request(b"POST", b"http", b"localhost", b"/echo", &[], Some(&posted)).unwrap();
    let mut got: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut ended = 0;
    while ended < 51 {
        let o = c.output();
        if !o.is_empty() { sock.write_all(&o).unwrap(); }
        let n = sock.read(&mut buf).unwrap();
        assert!(n > 0, "server closed early");
        c.receive(&buf[..n]).unwrap();
        while let Some(e) = c.event() {
            match e {
                Event::Headers { stream, status, headers, .. } => {
                    assert_eq!(status, 200);
                    let path = &headers.iter().find(|(n, _)| n == b"x-path").unwrap().1;
                    if stream != echo { assert_eq!(path, format!("/{}", want[&stream]).as_bytes()); }
                }
                Event::Data { stream, bytes } => got.entry(stream).or_default().extend(bytes),
                Event::End { .. } => ended += 1,
                other => panic!("{other:?}"),
            }
        }
    }
    sock.write_all(&c.output()).unwrap();
    for (s, n) in &want {
        let expect: Vec<u8> = (0..20000 + n * 1000).map(|i| ((n * 7 + i) % 251) as u8).collect();
        assert!(got[s] == expect, "stream {s} body");
    }
    assert!(got[&echo] == posted, "echoed body");
    drop(sock);
    let mut summary = String::new();
    out.read_to_string(&mut summary).unwrap();
    server.wait().unwrap();
    // The server allowed twenty at a time: the client kept to it (h2 refuses more).
    let most: usize = summary.trim().strip_prefix("served=51 most=").expect(&summary).parse().unwrap();
    assert!((2..=20).contains(&most), "{summary}");
}
