"""Generate original, fictional credentials with independent pykeepass 4.1.1.
Run only in a developer venv with pykeepass installed. These deliberately cheap
KDF parameters are test fixtures, never Chastity's production save policy.
"""
from pathlib import Path
from pykeepass import create_database
root = Path(__file__).parent
p = root / 'keepassxc-compatible.kdbx'
k = create_database(str(p), password='fixture-password-only')
k.add_entry(k.root_group, 'Fixture account', 'alice', 'fixture-secret',
            url='https://example.org', notes='original test note')
params = k.kdbx.header.value.dynamic_header.kdf_parameters.data.dict
params['I'].value = 3
params['M'].value = 65536
params['P'].value = 1
k.save()

# Exercise the second supported cipher/KDF and escaped XML text independently.
import os
k.kdbx.header.value.dynamic_header.cipher_id.data = 'chacha20'
k.kdbx.header.value.dynamic_header.encryption_iv.data = os.urandom(12)
params['$UUID'].value = bytes.fromhex('9E298B1956DB4773B23DFC3EC6F0A1E6')
k.entries[0].username = 'alice&bob'
k.entries[0].password = 'fixture<&>secret'
k.save(str(root / 'argon2id-chacha20.kdbx'))
