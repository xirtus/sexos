# SPDX-License-Identifier: GPL-3.0-only
"""A small WebDAV server for loom-remote's tests (Python standard library only).

Usage: webdav_server.py PORT ROOT [USER PASSWORD]

Serves ROOT under /remote.php/dav/files/USER (Nextcloud's layout) with PROPFIND
(depth 0 and 1), GET/HEAD with Range, PUT (Content-Length or chunked; committed
only when the whole body came), MKCOL, MOVE (Destination, Overwrite) and DELETE.
HTTP/1.1 keep-alive; PROPFIND replies are sent chunked.
"""
import base64, email.utils, os, shutil, sys, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT, ROOT = int(sys.argv[1]), os.path.abspath(sys.argv[2])
USER = sys.argv[3] if len(sys.argv) > 3 else "alice"
PASSWORD = sys.argv[4] if len(sys.argv) > 4 else "s3cret pass"
BASE = "/remote.php/dav/files/" + USER


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, code, body=b"", headers=()):
        self.send_response(code)
        for k, v in headers:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def authorized(self):
        want = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()
        if self.headers.get("Authorization") == want:
            return True
        self.discard_body()
        self.reply(401, b"login", [("WWW-Authenticate", 'Basic realm="loom"')])
        return False

    def fs(self, url_path):
        """Local path for a URL path under BASE, or None."""
        p = urllib.parse.unquote(urllib.parse.urlsplit(url_path).path)
        if not (p == BASE or p.startswith(BASE + "/")):
            return None
        parts = [c for c in p[len(BASE):].split("/") if c not in ("", ".")]
        if ".." in parts:
            return None
        return os.path.join(ROOT, *parts)

    def href(self, local):
        rel = os.path.relpath(local, ROOT)
        rel = "" if rel == "." else rel
        h = urllib.parse.quote(BASE + "/" + rel)
        return h + "/" if os.path.isdir(local) and not h.endswith("/") else h

    def read_body(self):
        """The request body, or None when it was cut short."""
        if "chunked" in self.headers.get("Transfer-Encoding", "").lower():
            out = b""
            while True:
                line = self.rfile.readline()
                if not line:
                    return None
                n = int(line.split(b";")[0].strip() or b"x", 16)
                if n == 0:
                    while self.rfile.readline() not in (b"\r\n", b"\n", b""):
                        pass
                    return out
                chunk = self.rfile.read(n)
                if len(chunk) < n:
                    return None
                out += chunk
                self.rfile.readline()
        n = int(self.headers.get("Content-Length", "0"))
        data = self.rfile.read(n)
        return data if len(data) == n else None

    def discard_body(self):
        if self.headers.get("Content-Length") or self.headers.get("Transfer-Encoding"):
            self.read_body()

    def do_PROPFIND(self):
        if not self.authorized():
            return
        self.discard_body()
        local = self.fs(self.path)
        depth = self.headers.get("Depth", "infinity")
        if local is None or not os.path.lexists(local):
            return self.reply(404)
        if depth not in ("0", "1"):
            return self.reply(403, b"depth infinity is not allowed")
        items = [local]
        if depth == "1" and os.path.isdir(local):
            items += [os.path.join(local, n) for n in sorted(os.listdir(local))]
        parts = ['<?xml version="1.0" encoding="utf-8"?>\n<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">']
        for it in items:
            st = os.stat(it)
            date = email.utils.formatdate(st.st_mtime, usegmt=True)
            if os.path.isdir(it):
                props = f"<d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>{date}</d:getlastmodified>"
                missing = "<d:getcontentlength/>"
            else:
                props = f"<d:resourcetype/><d:getcontentlength>{st.st_size}</d:getcontentlength><d:getlastmodified>{date}</d:getlastmodified>"
                missing = ""
            href = self.href(it).replace("&", "&amp;")
            parts.append(f"<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>")
            if missing:
                parts.append(f"<d:propstat><d:prop>{missing}</d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>")
            parts.append("</d:response>")
        parts.append("</d:multistatus>")
        body = "".join(parts).encode()
        self.send_response(207)
        self.send_header("Content-Type", "application/xml; charset=utf-8")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        for i in range(0, len(body), 700):
            c = body[i:i + 700]
            self.wfile.write(b"%x\r\n%s\r\n" % (len(c), c))
        self.wfile.write(b"0\r\n\r\n")

    def do_GET(self):
        if not self.authorized():
            return
        local = self.fs(self.path)
        if local is None or not os.path.isfile(local):
            return self.reply(404)
        size = os.path.getsize(local)
        rng = self.headers.get("Range")
        with open(local, "rb") as f:
            if rng and rng.startswith("bytes="):
                a, _, b = rng[6:].partition("-")
                a = int(a)
                b = min(int(b) if b else size - 1, size - 1)
                if a >= size:
                    return self.reply(416, b"", [("Content-Range", f"bytes */{size}")])
                f.seek(a)
                return self.reply(206, f.read(b - a + 1), [("Content-Range", f"bytes {a}-{b}/{size}")])
            return self.reply(200, f.read())

    do_HEAD = do_GET

    def do_PUT(self):
        if not self.authorized():
            return
        local = self.fs(self.path)
        if local is None or os.path.isdir(local):
            self.discard_body()
            return self.reply(405)
        if not os.path.isdir(os.path.dirname(local)):
            self.discard_body()
            return self.reply(409)
        data = self.read_body()
        if data is None:
            self.close_connection = True
            return
        existed = os.path.exists(local)
        tmp = local + ".srv-upload"
        with open(tmp, "wb") as f:
            f.write(data)
        os.replace(tmp, local)
        self.reply(204 if existed else 201)

    def do_MKCOL(self):
        if not self.authorized():
            return
        self.discard_body()
        local = self.fs(self.path)
        if local is None:
            return self.reply(403)
        if os.path.lexists(local):
            return self.reply(405)
        if not os.path.isdir(os.path.dirname(local.rstrip("/"))):
            return self.reply(409)
        os.mkdir(local)
        self.reply(201)

    def do_MOVE(self):
        if not self.authorized():
            return
        self.discard_body()
        src, dst = self.fs(self.path), self.fs(self.headers.get("Destination", ""))
        if src is None or dst is None:
            return self.reply(403)
        if not os.path.lexists(src):
            return self.reply(404)
        if not os.path.isdir(os.path.dirname(dst.rstrip("/"))):
            return self.reply(409)
        existed = os.path.lexists(dst)
        if existed:
            if self.headers.get("Overwrite", "T") == "F":
                return self.reply(412)
            shutil.rmtree(dst) if os.path.isdir(dst) else os.unlink(dst)
        os.rename(src, dst)
        self.reply(204 if existed else 201)

    def do_DELETE(self):
        if not self.authorized():
            return
        self.discard_body()
        local = self.fs(self.path)
        if local is None or local == ROOT:
            return self.reply(403)
        if not os.path.lexists(local):
            return self.reply(404)
        shutil.rmtree(local) if os.path.isdir(local) else os.unlink(local)
        self.reply(204)


ThreadingHTTPServer.allow_reuse_address = True
ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
