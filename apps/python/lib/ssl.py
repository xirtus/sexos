# SPDX-License-Identifier: GPL-3.0-only
"""TLS for SexOS's Python: the ssl module's everyday surface on _sextls.

SexOS's network service holds each TLS session (sex-tls, on rustls) and checks the
server's certificate against the system's trusted roots; a socket asks for it once
connected and then carries plain bytes. So a client context always verifies and
always uses TLS 1.2 or 1.3: options that would weaken it are accepted and ignored,
and servers (wrapping an accepted socket) are not offered. Enough for http.client,
urllib.request and the like.
"""
import socket as _socket
import _sextls

__all__ = ['SSLContext', 'SSLSocket', 'SSLError', 'SSLCertVerificationError', 'CertificateError',
           'create_default_context', 'wrap_socket', 'PROTOCOL_TLS_CLIENT', 'PROTOCOL_TLS',
           'CERT_NONE', 'CERT_OPTIONAL', 'CERT_REQUIRED', 'Purpose']

OPENSSL_VERSION = 'sex-tls (rustls) via SexOS sexnet'
OPENSSL_VERSION_INFO = (0, 0, 0, 0, 0)
OPENSSL_VERSION_NUMBER = 0
HAS_SNI = True
HAS_ALPN = False
HAS_ECDH = True
HAS_NPN = False
HAS_NEVER_CHECK_COMMON_NAME = True
HAS_TLSv1 = HAS_TLSv1_1 = HAS_SSLv2 = HAS_SSLv3 = False
HAS_TLSv1_2 = HAS_TLSv1_3 = True
CHANNEL_BINDING_TYPES = []

PROTOCOL_TLS = PROTOCOL_SSLv23 = 2
PROTOCOL_TLS_CLIENT = 16
PROTOCOL_TLS_SERVER = 17
CERT_NONE, CERT_OPTIONAL, CERT_REQUIRED = 0, 1, 2
OP_ALL = OP_NO_SSLv2 = OP_NO_SSLv3 = OP_NO_TLSv1 = OP_NO_TLSv1_1 = OP_NO_COMPRESSION = 0
OP_NO_TLSv1_2 = OP_NO_TLSv1_3 = OP_CIPHER_SERVER_PREFERENCE = OP_SINGLE_DH_USE = 0
OP_SINGLE_ECDH_USE = OP_NO_TICKET = OP_NO_RENEGOTIATION = OP_ENABLE_MIDDLEBOX_COMPAT = 0
VERIFY_DEFAULT = VERIFY_X509_STRICT = VERIFY_X509_PARTIAL_CHAIN = VERIFY_X509_TRUSTED_FIRST = 0
SSL_ERROR_WANT_READ, SSL_ERROR_WANT_WRITE, SSL_ERROR_EOF = 2, 3, 8


class TLSVersion:
    MINIMUM_SUPPORTED = TLSv1_2 = 0x0303
    MAXIMUM_SUPPORTED = TLSv1_3 = 0x0304
    SSLv3, TLSv1, TLSv1_1 = 0x0300, 0x0301, 0x0302


class Purpose:
    SERVER_AUTH = 'server_auth'
    CLIENT_AUTH = 'client_auth'


class SSLError(OSError):
    library = reason = None


class SSLZeroReturnError(SSLError): pass
class SSLWantReadError(SSLError): pass
class SSLWantWriteError(SSLError): pass
class SSLSyscallError(SSLError): pass
class SSLEOFError(SSLError): pass


class SSLCertVerificationError(SSLError, ValueError):
    verify_code = 1
    verify_message = 'certificate verify failed'


CertificateError = SSLCertVerificationError


class SSLContext:
    """A client context. Verification is the system's and always on."""

    def __init__(self, protocol=PROTOCOL_TLS_CLIENT, *args, **kwargs):
        if protocol == PROTOCOL_TLS_SERVER:
            raise NotImplementedError('TLS servers are not offered on SexOS yet')
        self.protocol = protocol
        self.check_hostname = True
        self.verify_mode = CERT_REQUIRED
        self.verify_flags = VERIFY_DEFAULT
        self.options = OP_ALL
        self.minimum_version = TLSVersion.TLSv1_2
        self.maximum_version = TLSVersion.TLSv1_3
        self.post_handshake_auth = None
        self.keylog_filename = None
        self.hostname_checks_common_name = False
        self.security_level = 2
        self.sni_callback = None

    # The system's trusted roots are the service's; these change nothing.
    def load_default_certs(self, purpose=Purpose.SERVER_AUTH): pass
    def load_verify_locations(self, cafile=None, capath=None, cadata=None): pass
    def set_default_verify_paths(self): pass
    def load_cert_chain(self, certfile, keyfile=None, password=None):
        raise NotImplementedError('client certificates are not offered on SexOS yet')
    def set_ciphers(self, ciphers): pass
    def set_alpn_protocols(self, protocols): pass
    def set_npn_protocols(self, protocols): pass
    def set_ecdh_curve(self, name): pass
    def get_ca_certs(self, binary_form=False): return []
    def cert_store_stats(self): return {'x509': 0, 'crl': 0, 'x509_ca': 0}
    def session_stats(self): return {}

    def wrap_socket(self, sock, server_side=False, do_handshake_on_connect=True,
                    suppress_ragged_eofs=True, server_hostname=None, session=None):
        if server_side:
            raise NotImplementedError('TLS servers are not offered on SexOS yet')
        if not server_hostname:
            raise ValueError('server_hostname is required: SexOS checks the certificate against it')
        return SSLSocket._wrap(sock, self, server_hostname, do_handshake_on_connect, suppress_ragged_eofs)

    def wrap_bio(self, *args, **kwargs):
        raise NotImplementedError('memory BIOs are not offered on SexOS yet')


class SSLSocket(_socket.socket):
    """A socket whose bytes travel over TLS (the network service does the TLS)."""

    @classmethod
    def _wrap(cls, sock, context, hostname, handshake, ragged):
        timeout = sock.gettimeout()
        self = cls(sock.family, sock.type, sock.proto, fileno=sock.detach())
        self.settimeout(timeout)
        self._context, self.server_hostname, self.server_side = context, hostname, False
        self.suppress_ragged_eofs = ragged
        self._tls = False
        try:
            self.getpeername()
            connected = True
        except OSError:
            connected = False
        if connected and handshake:
            self.do_handshake()
        else:
            self._handshake_on_connect = handshake
        return self

    @property
    def context(self): return self._context

    def connect(self, address):
        super().connect(address)
        if getattr(self, '_handshake_on_connect', False):
            self.do_handshake()

    def connect_ex(self, address):
        r = super().connect_ex(address)
        if r == 0 and getattr(self, '_handshake_on_connect', False):
            self.do_handshake()
        return r

    def do_handshake(self, block=False):
        if self._tls:
            return
        try:
            _sextls.start(self.fileno(), self.server_hostname)
        except _sextls.CertificateError as e:
            raise SSLCertVerificationError(1, str(e)) from None
        except OSError as e:
            raise SSLError(e.errno, f'TLS handshake with {self.server_hostname} failed: {e.strerror}') from None
        self._tls = True

    def read(self, len=1024, buffer=None):
        if buffer is not None:
            return self.recv_into(buffer, len)
        return self.recv(len)

    def write(self, data):
        return self.send(data)

    def pending(self): return 0
    def getpeercert(self, binary_form=False): return b'' if binary_form else {}
    def selected_alpn_protocol(self): return None
    def selected_npn_protocol(self): return None
    def cipher(self): return ('TLS_AES_128_GCM_SHA256', 'TLSv1.3', 128) if self._tls else None
    def shared_ciphers(self): return None
    def compression(self): return None
    def version(self): return 'TLSv1.3' if self._tls else None
    def get_channel_binding(self, cb_type='tls-unique'): return None
    def verify_client_post_handshake(self): raise NotImplementedError

    def unwrap(self):
        raise NotImplementedError('a TLS connection cannot go back to plain on SexOS')

    def dup(self):
        raise NotImplementedError("can't dup() an SSLSocket")


def create_default_context(purpose=Purpose.SERVER_AUTH, *, cafile=None, capath=None, cadata=None):
    if purpose != Purpose.SERVER_AUTH:
        raise NotImplementedError('TLS servers are not offered on SexOS yet')
    return SSLContext(PROTOCOL_TLS_CLIENT)


_create_default_https_context = create_default_context


def _create_unverified_context(*args, **kwargs):
    # The service always verifies: an "unverified" context is a verified one.
    return create_default_context()


_create_stdlib_context = _create_unverified_context


def wrap_socket(sock, keyfile=None, certfile=None, server_side=False, cert_reqs=CERT_REQUIRED,
                ssl_version=PROTOCOL_TLS_CLIENT, ca_certs=None, do_handshake_on_connect=True,
                suppress_ragged_eofs=True, ciphers=None, server_hostname=None):
    return create_default_context().wrap_socket(sock, server_side, do_handshake_on_connect,
                                                suppress_ragged_eofs, server_hostname)


def get_default_verify_paths():
    from collections import namedtuple
    Paths = namedtuple('DefaultVerifyPaths', 'cafile capath openssl_cafile_env openssl_cafile openssl_capath_env openssl_capath')
    return Paths(None, None, None, None, None, None)


def match_hostname(cert, hostname):
    # The service checked the certificate's names during the handshake.
    return None


def DER_cert_to_PEM_cert(der_cert_bytes):
    import base64, textwrap
    f = base64.standard_b64encode(der_cert_bytes).decode('ascii')
    return '-----BEGIN CERTIFICATE-----\n' + '\n'.join(textwrap.wrap(f, 64)) + '\n-----END CERTIFICATE-----\n'


def PEM_cert_to_DER_cert(pem_cert_string):
    import base64
    d = pem_cert_string.strip()[len('-----BEGIN CERTIFICATE-----'):-len('-----END CERTIFICATE-----')]
    return base64.decodebytes(d.encode('ascii', 'strict'))
