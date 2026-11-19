#!/usr/bin/env python3
"""A KDE Connect test peer, playing the phone for Hookup's tests (written from the
protocol's public description, plain TCP like Hookup's links today).

It waits for Hookup's identity datagram, opens a link to the TCP port the identity
names, says who it is, asks to pair and waits for the answer. Once paired it sends a
notification, its clipboard, a media player and then shares a file over a payload
connection. With --wait-media it waits for a media control request; with --wait-file
it waits for Hookup to share a file, fetches it and reports its size and hashes.

Every step is printed as one JSON object per line. Used as a script by the host test
(crates/sex-kdeconnect/tests/python_peer.rs) and as a module by the QEMU gate
(scripts/program_hookup_gate.py), where QEMU forwards Hookup's ports (--forward).
"""
import argparse, hashlib, json, os, socket, sys, threading, time

DEVICE_ID = 'peer0phone0test0device0id0000001'

def fnv(data):
    h = 0xcbf29ce484222325
    for b in data: h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return '%016x' % h

def packet(kind, body, **extra):
    p = {'id': int(time.time() * 1000), 'type': kind, 'body': body}
    p.update(extra)
    return (json.dumps(p) + '\n').encode()

class Peer:
    def __init__(self, udp_port=0, forward=None, log=None, name='Test Phone'):
        self.udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.udp.bind(('127.0.0.1', udp_port))
        self.udp_port = self.udp.getsockname()[1]
        self.forward = forward or {}
        self.name = name
        self.events = []
        self.log = log or (lambda e: None)
        self.sock = None
        self.buf = b''

    def say(self, **e):
        self.events.append(e); self.log(e)

    def where(self, port):
        """Where a port of Hookup's is reached from this host."""
        return ('127.0.0.1', self.forward.get(port, port))

    def identity(self):
        return packet('kdeconnect.identity', {'deviceId': DEVICE_ID, 'deviceName': self.name, 'deviceType': 'phone', 'protocolVersion': 7,
            'incomingCapabilities': ['kdeconnect.ping', 'kdeconnect.clipboard', 'kdeconnect.share.request', 'kdeconnect.mpris.request', 'kdeconnect.notification.request'],
            'outgoingCapabilities': ['kdeconnect.ping', 'kdeconnect.notification', 'kdeconnect.clipboard', 'kdeconnect.share.request', 'kdeconnect.mpris']})

    def read_packet(self, timeout):
        self.sock.settimeout(timeout)
        while b'\n' not in self.buf:
            b = self.sock.recv(65536)
            if not b: raise EOFError('Hookup closed the link')
            self.buf += b
        line, self.buf = self.buf.split(b'\n', 1)
        return json.loads(line)

    def expect(self, kind, timeout, pred=lambda p: True):
        end = time.time() + timeout
        while True:
            p = self.read_packet(max(0.1, end - time.time()))
            self.say(event='packet', type=p['type'], body=p.get('body'))
            if p['type'] == kind and pred(p): return p
            if time.time() > end: raise TimeoutError(kind)

    def link(self, timeout=300):
        self.udp.settimeout(timeout)
        while True:
            data, src = self.udp.recvfrom(65536)
            p = json.loads(data)
            if p.get('type') == 'kdeconnect.identity': break
        hook = p["body"]; self.buf = b""
        self.say(event='heard', device=hook['deviceId'], name=hook['deviceName'], tcp_port=hook.get('tcpPort'), from_addr=src[0],
                 incoming=hook.get('incomingCapabilities', []))
        self.sock = socket.create_connection(self.where(hook.get('tcpPort', 1716)), timeout=30)
        self.sock.sendall(self.identity())
        p = self.expect('kdeconnect.identity', 60)
        assert p['body']['deviceId'] == hook['deviceId'], 'a different device answered'
        self.say(event='linked', device=hook['deviceId'])
        return hook

    def pair(self, timeout=180):
        self.sock.sendall(packet('kdeconnect.pair', {'pair': True}))
        p = self.expect('kdeconnect.pair', timeout)
        self.say(event='pair-answer', pair=p['body'].get('pair'))
        return p['body'].get('pair') is True

    def send_extras(self):
        self.sock.sendall(packet('kdeconnect.notification', {'id': '0|org.chat|42', 'appName': 'Chat', 'title': 'Ana', 'text': 'Meet at the fountain at 8', 'isClearable': True}))
        self.sock.sendall(packet('kdeconnect.clipboard', {'content': 'copied on the phone'}))
        self.sock.sendall(packet('kdeconnect.mpris', {'playerList': ['Radio'], 'supportAlbumArtPayload': False}))
        self.sock.sendall(packet('kdeconnect.mpris', {'player': 'Radio', 'title': 'Night Drive', 'artist': 'The Hookups', 'isPlaying': True, 'pos': 1000, 'length': 200000}))
        self.say(event='extras-sent')

    def share(self, name, data):
        srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(('127.0.0.1', 0)); srv.listen(1); port = srv.getsockname()[1]
        done = threading.Event()
        def serve():
            srv.settimeout(120)
            c, _ = srv.accept()
            c.sendall(data); c.shutdown(socket.SHUT_WR)
            try:
                c.settimeout(10)
                while c.recv(4096): pass
            except OSError: pass
            c.close(); srv.close(); done.set()
        threading.Thread(target=serve, daemon=True).start()
        self.sock.sendall(packet('kdeconnect.share.request', {'filename': name, 'numberOfFiles': 1, 'totalPayloadSize': len(data)},
                                 payloadSize=len(data), payloadTransferInfo={'port': port}))
        self.say(event='shared', name=name, bytes=len(data), fnv=fnv(data), port=port)
        return done

    def wait_media(self, timeout=180):
        p = self.expect('kdeconnect.mpris.request', timeout, lambda p: 'action' in p['body'])
        self.say(event='media', action=p['body']['action'], player=p['body'].get('player'))
        return p['body']

    def wait_file(self, timeout=180):
        p = self.expect('kdeconnect.share.request', timeout, lambda p: 'filename' in p['body'])
        size = p['payloadSize']; port = p['payloadTransferInfo']['port']
        c = socket.create_connection(self.where(port), timeout=30)
        data = b''
        while len(data) < size:
            b = c.recv(65536)
            if not b: break
            data += b
        c.close()
        self.say(event='fetched', name=p['body']['filename'], bytes=len(data), declared=size, fnv=fnv(data), sha256=hashlib.sha256(data).hexdigest())
        return p['body']['filename'], data

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--udp', type=int, default=0, help='port to hear identity datagrams on')
    ap.add_argument('--forward', action='append', default=[], help='GUEST=HOST: where a port of Hookup is reached')
    ap.add_argument('--file', help='a file to share once paired')
    ap.add_argument('--wait-media', action='store_true')
    ap.add_argument('--wait-file', action='store_true')
    a = ap.parse_args()
    out = lambda e: (print(json.dumps(e)), sys.stdout.flush())
    peer = Peer(a.udp, {int(k): int(v) for k, v in (f.split('=') for f in a.forward)}, out)
    out({'event': 'listening', 'udp': peer.udp_port})
    try:
        peer.link()
        if not peer.pair(): out({'event': 'refused'}); return 1
        peer.send_extras()
        if a.file:
            with open(a.file, 'rb') as f: data = f.read()
            done = peer.share(os.path.basename(a.file), data)
            if not done.wait(120): out({'event': 'error', 'what': 'nobody fetched the file'}); return 1
            out({'event': 'payload-taken'})
        if a.wait_media: peer.wait_media()
        if a.wait_file: peer.wait_file()
        out({'event': 'done'})
        time.sleep(0.5)
        return 0
    except Exception as e:
        out({'event': 'error', 'what': repr(e)})
        return 1

if __name__ == '__main__':
    sys.exit(main())
