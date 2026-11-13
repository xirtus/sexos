# HTTP/2 client (workstream K3)

- `crates/sex-http/src/h2/`: sans-I/O HTTP/2 client (RFC 9113) with HPACK (RFC 7541: static + dynamic table, Huffman both ways, size updates, list limits). Streams multiplex on one connection; the server's MAX_CONCURRENT_STREAMS queues extra requests; request bodies follow stream and connection windows; our windows (4 MiB per stream, 16 MiB connection) refill as data is delivered. SETTINGS/PING/GOAWAY/RST_STREAM/CONTINUATION/padding/1xx/trailers handled; push refused (ENABLE_PUSH=0); protocol violations send GOAWAY.
- `sex_tls::open::Session::start_offering(host, seed, seconds, &[b"h2", b"http/1.1"])` and `Session::alpn()`. `Session::start` still offers only http/1.1, so nothing changes for sexnet until it opts in.
- Tests: RFC 7541 C.4/C.6 vectors; 50 parallel streams + a 100 KB POST echo against Python's `h2` server (limit 20 streams); 50 streams over TLS with ALPN h2. CI installs `h2==4.4.1`.
- Next: sexnet TLS_START flag for h2 and `sex_app::net` using one h2 connection per origin (after the x17 networking thread merges sexnet).
