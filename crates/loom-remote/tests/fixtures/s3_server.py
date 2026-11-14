# SPDX-License-Identifier: GPL-3.0-only
"""A mock S3 endpoint for loom-remote's tests (Python standard library only).

Usage: s3_server.py PORT DIR [PAGE_SIZE]

One bucket, "loom-bucket", path-style, in memory. Every request must carry a valid
AWS Signature Version 4 (checked here independently, with region us-east-1 and the
key pair below) and a body matching x-amz-content-sha256. Supports ListObjectsV2
(prefix, delimiter, continuation tokens, max-keys; PAGE_SIZE keys per page by
default, to exercise paging), HEAD/GET with Range, PUT, CopyObject, DeleteObject and
multipart uploads (parts of at least 5 MiB but the last). GET /__stats (unsigned)
reports counters as JSON. DIR is unused (kept for the common launcher).
"""
import base64, email.utils, hashlib, hmac, json, sys, time, urllib.parse, uuid
import xml.etree.ElementTree as ET
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
PAGE = int(sys.argv[3]) if len(sys.argv) > 3 else 3
BUCKET, REGION = "loom-bucket", "us-east-1"
KEYS = {"LOOMTESTKEY": "loom/test+secret/KEY"}
OBJECTS = {}  # key -> (bytes, mtime, etag)
UPLOADS = {}  # upload id -> (key, {part number: (bytes, etag)})
STATS = {"put": 0, "multipart_complete": 0, "parts": 0, "aborted": 0, "copy": 0, "bad_signature": 0}


def h(b):
    return hashlib.sha256(b).hexdigest()


def sign(key, msg):
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def q(s):
    return urllib.parse.quote(s, safe="-_.~")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, code, body=b"", headers=()):
        if isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        for k, v in headers:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def error(self, code, s3code, msg=""):
        self.reply(code, f'<?xml version="1.0" encoding="UTF-8"?><Error><Code>{s3code}</Code><Message>{msg}</Message></Error>', [("Content-Type", "application/xml")])

    def check_signature(self, body):
        auth = self.headers.get("Authorization", "")
        if not auth.startswith("AWS4-HMAC-SHA256 "):
            return "no SigV4 Authorization"
        fields = dict(p.strip().split("=", 1) for p in auth[len("AWS4-HMAC-SHA256 "):].split(","))
        ak, day, region, service, term = fields["Credential"].split("/")
        signed = fields["SignedHeaders"].split(";")
        if ak not in KEYS or region != REGION or service != "s3" or term != "aws4_request":
            return "bad credential scope"
        for need in ("host", "x-amz-date", "x-amz-content-sha256"):
            if need not in signed:
                return "unsigned " + need
        amz = self.headers["x-amz-date"]
        if amz[:8] != day or abs(time.mktime(time.strptime(amz, "%Y%m%dT%H%M%SZ")) - time.mktime(time.gmtime())) > 900:
            return "bad date"
        payload = self.headers["x-amz-content-sha256"]
        if payload != "UNSIGNED-PAYLOAD" and payload != h(body):
            return "payload hash mismatch"
        u = urllib.parse.urlsplit(self.path)
        curi = urllib.parse.quote(urllib.parse.unquote(u.path), safe="/-_.~")
        cq = "&".join(f"{q(k)}={q(v)}" for k, v in sorted(urllib.parse.parse_qsl(u.query, keep_blank_values=True)))
        ch = "".join(f"{n}:{' '.join(self.headers.get(n, '').split())}\n" for n in signed)
        creq = "\n".join([self.command, curi, cq, ch, ";".join(signed), payload])
        sts = "\n".join(["AWS4-HMAC-SHA256", amz, f"{day}/{region}/{service}/aws4_request", h(creq.encode())])
        k = sign(("AWS4" + KEYS[ak]).encode(), day)
        for part in (region, service, "aws4_request"):
            k = sign(k, part)
        if hmac.new(k, sts.encode(), hashlib.sha256).hexdigest() != fields["Signature"]:
            return "signature mismatch"
        return None

    def handle_any(self):
        n = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(n)
        u = urllib.parse.urlsplit(self.path)
        if u.path == "/__stats":
            return self.reply(200, json.dumps({**STATS, "uploads_open": len(UPLOADS), "keys": sorted(OBJECTS)}))
        why = self.check_signature(body)
        if why:
            STATS["bad_signature"] += 1
            return self.error(403, "SignatureDoesNotMatch", why)
        path = urllib.parse.unquote(u.path)
        query = dict(urllib.parse.parse_qsl(u.query, keep_blank_values=True))
        parts = path.split("/", 2)
        if len(parts) < 2 or parts[1] != BUCKET:
            return self.error(404, "NoSuchBucket")
        key = parts[2] if len(parts) > 2 else ""
        m = self.command
        if key == "":
            if m == "GET" and query.get("list-type") == "2":
                return self.list(query)
            return self.error(400, "InvalidRequest")
        if m in ("GET", "HEAD"):
            return self.get(key)
        if m == "PUT" and "uploadId" in query:
            up = UPLOADS.get(query["uploadId"])
            if not up:
                return self.error(404, "NoSuchUpload")
            etag = '"%s"' % hashlib.md5(body).hexdigest()
            up[1][int(query["partNumber"])] = (body, etag)
            STATS["parts"] += 1
            return self.reply(200, b"", [("ETag", etag)])
        if m == "PUT" and self.headers.get("x-amz-copy-source"):
            src = urllib.parse.unquote(self.headers["x-amz-copy-source"]).lstrip("/")
            b, _, k = src.partition("/")
            if b != BUCKET or k not in OBJECTS:
                return self.error(404, "NoSuchKey")
            data, _, etag = OBJECTS[k]
            OBJECTS[key] = (data, time.time(), etag)
            STATS["copy"] += 1
            return self.reply(200, f'<?xml version="1.0" encoding="UTF-8"?><CopyObjectResult><ETag>{etag}</ETag></CopyObjectResult>')
        if m == "PUT":
            etag = '"%s"' % hashlib.md5(body).hexdigest()
            OBJECTS[key] = (body, time.time(), etag)
            STATS["put"] += 1
            return self.reply(200, b"", [("ETag", etag)])
        if m == "POST" and "uploads" in query:
            uid = uuid.uuid4().hex
            UPLOADS[uid] = (key, {})
            return self.reply(200, f'<?xml version="1.0" encoding="UTF-8"?><InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>{key}</Key><UploadId>{uid}</UploadId></InitiateMultipartUploadResult>')
        if m == "POST" and "uploadId" in query:
            up = UPLOADS.get(query["uploadId"])
            if not up or up[0] != key:
                return self.error(404, "NoSuchUpload")
            ns = {"s": "http://s3.amazonaws.com/doc/2006-03-01/"}
            listed = [(int(p.find("s:PartNumber", ns).text), p.find("s:ETag", ns).text) for p in ET.fromstring(body).findall("s:Part", ns)]
            if [n for n, _ in listed] != list(range(1, len(listed) + 1)):
                return self.error(400, "InvalidPartOrder")
            data = b""
            for i, (n, etag) in enumerate(listed):
                got = up[1].get(n)
                if not got or got[1] != etag:
                    return self.error(400, "InvalidPart")
                if i + 1 < len(listed) and len(got[0]) < 5 * 1024 * 1024:
                    return self.error(400, "EntityTooSmall")
                data += got[0]
            del UPLOADS[query["uploadId"]]
            OBJECTS[key] = (data, time.time(), '"multi-%d"' % len(listed))
            STATS["multipart_complete"] += 1
            # Real S3 sends 200 at once and may still fail later in the body; mirror the success form.
            return self.reply(200, f'<?xml version="1.0" encoding="UTF-8"?><CompleteMultipartUploadResult><Key>{key}</Key></CompleteMultipartUploadResult>')
        if m == "DELETE" and "uploadId" in query:
            UPLOADS.pop(query["uploadId"], None)
            STATS["aborted"] += 1
            return self.reply(204)
        if m == "DELETE":
            OBJECTS.pop(key, None)
            return self.reply(204)
        return self.error(405, "MethodNotAllowed")

    do_GET = do_HEAD = do_PUT = do_POST = do_DELETE = handle_any

    def get(self, key):
        if key not in OBJECTS:
            return self.error(404, "NoSuchKey")
        data, mtime, etag = OBJECTS[key]
        hd = [("ETag", etag), ("Last-Modified", email.utils.formatdate(mtime, usegmt=True))]
        rng = self.headers.get("Range")
        if rng and rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            a, b = int(a), min(int(b) if b else len(data) - 1, len(data) - 1)
            if a >= len(data):
                return self.error(416, "InvalidRange")
            return self.reply(206, data[a:b + 1], hd + [("Content-Range", f"bytes {a}-{b}/{len(data)}")])
        if self.command == "HEAD":
            self.send_response(200)
            for k, v in hd:
                self.send_header(k, v)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            return
        return self.reply(200, data, hd)

    def list(self, query):
        prefix, delim = query.get("prefix", ""), query.get("delimiter", "")
        maxk = min(int(query.get("max-keys", "1000")), PAGE)
        after = base64.urlsafe_b64decode(query["continuation-token"]).decode() if "continuation-token" in query else None
        items = []  # (sort key, kind, value)
        seen = set()
        for k in sorted(OBJECTS):
            if not k.startswith(prefix):
                continue
            rest = k[len(prefix):]
            if delim and delim in rest:
                cp = prefix + rest[:rest.index(delim) + 1]
                if cp not in seen:
                    seen.add(cp)
                    items.append((cp, "p", cp))
            else:
                items.append((k, "k", k))
        items.sort()
        if after is not None:
            items = [i for i in items if i[0] > after]
        page, more = items[:maxk], len(items) > maxk
        x = [f'<?xml version="1.0" encoding="UTF-8"?>\n<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>{BUCKET}</Name><Prefix>{prefix}</Prefix><KeyCount>{len(page)}</KeyCount><MaxKeys>{maxk}</MaxKeys><IsTruncated>{"true" if more else "false"}</IsTruncated>']
        if more:
            x.append(f"<NextContinuationToken>{base64.urlsafe_b64encode(page[-1][0].encode()).decode()}</NextContinuationToken>")
        esc = lambda s: s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        for _, kind, v in page:
            if kind == "k":
                data, mtime, etag = OBJECTS[v]
                lm = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(mtime))
                x.append(f"<Contents><Key>{esc(v)}</Key><LastModified>{lm}</LastModified><ETag>{esc(etag)}</ETag><Size>{len(data)}</Size><StorageClass>STANDARD</StorageClass></Contents>")
            else:
                x.append(f"<CommonPrefixes><Prefix>{esc(v)}</Prefix></CommonPrefixes>")
        x.append("</ListBucketResult>")
        self.reply(200, "".join(x), [("Content-Type", "application/xml")])


ThreadingHTTPServer.allow_reuse_address = True
ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
