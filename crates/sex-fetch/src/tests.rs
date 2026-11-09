// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use alloc::vec;

const GZ: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 203, 72, 205, 201, 201, 87, 200, 64, 34, 43, 243, 75, 75, 74, 147, 82, 81, 196, 70, 101, 134, 187, 12, 0, 1, 190, 135, 65, 8, 2, 0, 0];
const ZL: &[u8] = &[120, 156, 203, 72, 205, 201, 201, 87, 200, 64, 34, 43, 243, 75, 75, 74, 147, 82, 81, 196, 70, 101, 134, 187, 12, 0, 74, 157, 195, 181];
const RAW: &[u8] = &[203, 72, 205, 201, 201, 87, 200, 64, 34, 43, 243, 75, 75, 74, 147, 82, 81, 196, 70, 101, 134, 187, 12, 0];
fn plain() -> Vec<u8> { b"hello hello hello youtube ".repeat(20) }

#[test]
fn urls() {
    let u = Url::parse("HTTPS://WWW.YouTube.com/watch?v=abc#t=3").unwrap();
    assert_eq!((u.secure, u.host.as_str(), u.port, u.target.as_str()), (true, "www.youtube.com", 443, "/watch?v=abc"));
    assert_eq!(u.path(), "/watch");
    let u = Url::parse("http://10.0.2.2:8080?q=a b").unwrap();
    assert_eq!((u.port, u.target.as_str(), u.authority().as_str()), (8080, "/?q=a%20b", "10.0.2.2:8080"));
    assert_eq!(Url::parse("http://user:pw@host.example/").unwrap().host, "host.example");
    for bad in ["ftp://x", "https://", "https://a b/", "http://h:99999/", "http://h:0/", "http://h\r\nX: y/"] { assert!(Url::parse(bad).is_err(), "{}", bad); }
}

#[test]
fn requests() {
    let u = Url::parse("https://www.youtube.com/youtubei/v1/next?prettyPrint=false").unwrap();
    let body = br#"{"videoId":"x"}"#;
    let r = Request { method: "post", url: "", headers: &[("Content-Type", "application/json"), ("Host", "evil"), ("X-Goog-Visitor-Id", "v1")], body };
    let bytes = encode(&r, &u, Some("YSC=a; PREF=b")).unwrap();
    let text = core::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with("POST /youtubei/v1/next?prettyPrint=false HTTP/1.1\r\nHost: www.youtube.com\r\n"), "{}", text);
    assert!(text.contains("\r\nContent-Type: application/json\r\n") && text.contains("\r\nX-Goog-Visitor-Id: v1\r\n"));
    assert!(text.contains("\r\nCookie: YSC=a; PREF=b\r\n") && text.contains("\r\nContent-Length: 15\r\n\r\n{\"videoId\""));
    assert!(!text.contains("evil"), "scripts can't set Host");
    assert_eq!(encode(&Request { headers: &[("X", "a\r\nInjected: 1")], ..Request::get("") }, &u, None), Err(Error::Header));
    assert_eq!(encode(&Request { method: "GET X", ..Request::get("") }, &u, None), Err(Error::Header));
    let g = encode(&Request { headers: &[("Range", "bytes=0-99")], ..Request::get("") }, &u, None).unwrap();
    let g = core::str::from_utf8(&g).unwrap();
    assert!(g.contains("Range: bytes=0-99\r\n") && !g.contains("Content-Length"));
}

#[test]
fn heads() {
    assert_eq!(parse_head(b"HTTP/1.1 200 OK\r\nContent-Le"), Ok(None));
    let raw = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nSet-Cookie: a=1\r\nset-cookie: b=2\r\nContent-Length: 4\r\n\r\nabcdEXTRA";
    let (h, at) = parse_head(raw).unwrap().unwrap();
    assert_eq!((h.status, h.reason.as_str(), h.content_type()), (206, "Partial Content", "video/mp4"));
    assert_eq!(h.all("set-cookie").collect::<Vec<_>>(), vec!["a=1", "b=2"]);
    assert_eq!(h.framing("GET"), Ok(Framing::Length(4)));
    assert_eq!(decode(&h, "GET", &raw[at..], 100).unwrap(), b"abcd");
    assert_eq!(complete(Framing::Length(4), b"abc"), Ok(false));
    assert!(parse_head(b"HTTP/1.1 200 OK\r\n folded\r\n\r\n").is_err());
    assert!(parse_head(b"SSH-2.0\r\n\r\n").is_err());
    let (h, _) = parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\n").unwrap().unwrap();
    assert_eq!(h.framing("GET"), Err(Error::Malformed));
    let big = vec![b'a'; HEAD_MAX + 10];
    assert_eq!(parse_head(&big), Err(Error::HeadTooLarge));
    let (h, _) = parse_head(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 50\r\n\r\n").unwrap().unwrap();
    assert_eq!(h.framing("GET"), Ok(Framing::Empty));
}

#[test]
fn chunked_and_gzip() {
    let (h, _) = parse_head(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Encoding: gzip\r\n\r\n").unwrap().unwrap();
    assert_eq!(h.framing("GET"), Ok(Framing::Chunked));
    let mut body = Vec::new();
    for piece in GZ.chunks(10) { body.extend_from_slice(alloc::format!("{:x};ext=1\r\n", piece.len()).as_bytes()); body.extend_from_slice(piece); body.extend_from_slice(b"\r\n"); }
    let whole_len = body.len();
    body.extend_from_slice(b"0\r\nX-Trailer: 1\r\n\r\n");
    assert_eq!(chunked_end(&body[..whole_len]), Ok(None));
    assert_eq!(chunked_end(&body), Ok(Some(body.len())));
    let mut scan = ChunkScan::new();
    for n in (0..body.len()).step_by(7) { assert_eq!(scan.advance(&body[..n]), Ok(None), "at {}", n); }
    assert_eq!(scan.advance(&body), Ok(Some(body.len())));
    assert_eq!(decode(&h, "GET", &body, 1 << 20).unwrap(), plain());
    assert_eq!(decode(&h, "GET", &body, 100), Err(Error::TooLarge), "a gzip bomb stops at the limit");
    assert_eq!(decode(&h, "GET", &body[..whole_len], 1 << 20), Err(Error::Truncated));
    assert_eq!(chunked_end(b"zz\r\n"), Err(Error::Malformed));
    assert_eq!(chunked_end(b"3\r\nabcX\r\n"), Err(Error::Malformed));
}

#[test]
fn deflate_both_ways() {
    for data in [ZL, RAW] {
        let head = alloc::format!("HTTP/1.1 200 OK\r\nContent-Encoding: deflate\r\nContent-Length: {}\r\n\r\n", data.len());
        let (h, _) = parse_head(head.as_bytes()).unwrap().unwrap();
        assert_eq!(decode(&h, "GET", data, 1 << 20).unwrap(), plain());
    }
    let (h, _) = parse_head(b"HTTP/1.1 200 OK\r\nContent-Encoding: br\r\n\r\n").unwrap().unwrap();
    assert_eq!(decode(&h, "GET", b"x", 10), Err(Error::Encoding));
    let (h, _) = parse_head(b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n").unwrap().unwrap();
    assert_eq!(decode(&h, "GET", b"not gzip", 10), Err(Error::Corrupt));
}

#[test]
fn joins() {
    let b = "https://www.youtube.com/watch/a/b?v=1";
    assert_eq!(join(b, "/embed/x").unwrap(), "https://www.youtube.com/embed/x");
    assert_eq!(join(b, "//i.ytimg.com/vi/x.jpg").unwrap(), "https://i.ytimg.com/vi/x.jpg");
    assert_eq!(join(b, "c.js").unwrap(), "https://www.youtube.com/watch/a/c.js");
    assert_eq!(join(b, "../c.js?x=1").unwrap(), "https://www.youtube.com/watch/c.js?x=1");
    assert_eq!(join(b, "?v=2").unwrap(), "https://www.youtube.com/watch/a/b?v=2");
    assert_eq!(join(b, "http://other/").unwrap(), "http://other/");
    assert_eq!(join(b, "javascript:alert(1)"), None);
}

#[test]
fn dates() {
    assert_eq!(parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1_445_412_480));
    assert_eq!(parse_http_date("Thu, 01-Jan-1970 00:00:01 GMT"), Some(1));
    assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), Some(784_111_777));
    assert_eq!(parse_http_date("nonsense"), None);
}

#[test]
fn cookies() {
    let now = 1_800_000_000;
    let yt = Url::parse("https://www.youtube.com/watch?v=1").unwrap();
    let mut jar = Jar::new();
    jar.store(&yt, "YSC=abc; Domain=.youtube.com; Path=/; Secure; HttpOnly; SameSite=None", now);
    jar.store(&yt, "VISITOR_INFO1_LIVE=v; Domain=youtube.com; Path=/; Max-Age=15552000; Secure", now);
    jar.store(&yt, "PREF=f6=40; Expires=Wed, 21 Oct 2015 07:28:00 GMT", now); // already expired
    jar.store(&yt, "x=1; Domain=com", now); // a whole top-level domain
    jar.store(&yt, "y=1; Domain=google.com", now); // someone else
    assert_eq!(jar.len(), 2);
    let m = Url::parse("https://m.youtube.com/").unwrap();
    assert_eq!(jar.header(&m, now).unwrap(), "YSC=abc; VISITOR_INFO1_LIVE=v");
    assert_eq!(jar.script_view(&m, now), "VISITOR_INFO1_LIVE=v", "HttpOnly stays out of scripts");
    let plain_http = Url::parse("http://www.youtube.com/").unwrap();
    assert_eq!(jar.header(&plain_http, now), None, "Secure needs TLS");
    assert_eq!(jar.header(&m, now + 15_552_001).unwrap(), "YSC=abc", "Max-Age runs out");
    // Scripts can't replace an HttpOnly cookie or set one.
    jar.store_from_script(&yt, "YSC=evil; domain=youtube.com; path=/", now);
    jar.store_from_script(&yt, "s=1; HttpOnly", now);
    assert_eq!(jar.header(&m, now).unwrap(), "YSC=abc; VISITOR_INFO1_LIVE=v");
    // Host-only cookies and paths.
    jar.store_from_script(&yt, "h=1", now);
    assert!(jar.script_view(&m, now).find("h=1").is_none());
    assert!(jar.script_view(&Url::parse("https://www.youtube.com/watch").unwrap(), now).contains("h=1"));
    jar.store(&yt, "p=1; Path=/embed", now);
    assert!(!jar.script_view(&Url::parse("https://www.youtube.com/embedded").unwrap(), now).contains("p=1"));
    assert!(jar.script_view(&Url::parse("https://www.youtube.com/embed/x").unwrap(), now).starts_with("p=1"), "longest path first");
    // Deleting by expiry.
    jar.store(&yt, "VISITOR_INFO1_LIVE=; Domain=youtube.com; Path=/; Max-Age=0", now);
    assert_eq!(jar.header(&m, now).unwrap(), "YSC=abc");
    jar.store(&yt, "__Host-a=1; Path=/; Secure", now);
    jar.store(&yt, "__Host-b=1; Path=/; Secure; Domain=youtube.com", now);
    assert!(jar.header(&yt, now).unwrap().contains("__Host-a=1") && !jar.header(&yt, now).unwrap().contains("__Host-b"));
    // Counts are capped.
    for i in 0..400 { jar.store(&yt, &alloc::format!("c{}=1", i), now); }
    assert!(jar.iter().filter(|c| c.domain == "www.youtube.com").count() <= 180);
}
