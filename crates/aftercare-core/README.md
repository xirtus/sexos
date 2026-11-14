# aftercare-core: repository format, version 1

Aftercare's backup engine stores snapshots in a *repository*: a flat store of named,
write-once files of six kinds. Path-based stores (a directory on a USB disk, an SFTP
server) use this layout:

```text
config                  sealed Config
keys/<hex>              key files (one per password)
data/<xx>/<hex>         packs; <xx> = first two hex digits of the name
index/<hex>             sealed index files
snapshots/<hex>         sealed snapshot records
locks/<hex>             sealed lock records
```

All names except `config` are 64 lowercase hex digits: the SHA-256 of the file's
stored bytes (key files, packs, index files, snapshots, locks). Integers are little
endian. `str` and `bytes` are a `u32` length then the bytes (strings are UTF-8);
`strs` is a `u32` count then that many `str`. `id` is 32 raw bytes. Every structure
starts with a 4-byte magic and a `u16` format version (currently 1); readers refuse
versions they do not know.

## Cryptography

* **Master key**: 64 random bytes from the caller's RNG: a 32-byte XChaCha20-Poly1305
  key (`enc`) and a 32-byte HMAC-SHA256 key (`mac`). It never changes; passwords only
  wrap it.
* **Sealing**: `seal(aad, m) = nonce(24, random) || XChaCha20-Poly1305(enc, nonce, m, aad)`
  (ciphertext followed by the 16-byte tag). The AAD names what the message is, so a
  file of one kind cannot be passed off as another:

  | message | AAD |
  | --- | --- |
  | config | `aftercare/config` |
  | index file | `aftercare/index` |
  | snapshot | `aftercare/snapshot` |
  | lock | `aftercare/lock` |
  | pack header | `aftercare/pack-header` |
  | blob | blob id (32 bytes) `\|\|` blob kind (1 byte: 0 data, 1 tree) |

* **Blob ids**: `HMAC-SHA256(mac, plaintext)` of the uncompressed blob. Keyed, so the
  store does not learn which well-known files you have. Restore and `check --read-data`
  recompute it for every blob.

### Key file (`keys/<hex>`, not sealed)

```text
"ACKY" u16 version=1
u8   kdf = 1 (Argon2id, version 0x13)
u32  m_cost (KiB)   u32 t_cost   u32 p_cost
32   salt
...  XChaCha20-Poly1305(kek, nonce, enc || mac, aad = all bytes above), sealed as above
```

`kek = Argon2id(password, salt, m, t, p, 32 bytes)`. Defaults: 64 MiB, t=3, p=1. If the
tag fails the password is wrong ([`Error::WrongPassword`]). Several key files may wrap
the same master key (one per password).

## Config (`config`, sealed)

```text
"ACCF" u16 version=1
id   repository id (random)
u32  chunk min   u32 chunk avg   u32 chunk max      (bytes)
u64  gear seed   (secret; seeds the FastCDC gear table)
u32  target pack size (bytes)
u8   compression (1 = zstd allowed)
```

## Chunking

FastCDC (2016) with normalized chunking, level 2. A 256-entry `u64` gear table comes
from SplitMix64 seeded with the config's gear seed. With `bits = floor(log2(avg))`,
scanning starts at `min`; the hash is `h = (h << 1) + gear[byte]`; a cut is made after
the byte where `h & mask == 0`, with `mask` = the top `bits + 2` bits before `avg` and
the top `bits - 2` bits after it, and at `max` at the latest. Defaults: min 256 KiB,
avg 1 MiB, max 4 MiB.

## Blobs and packs (`data/<xx>/<hex>`)

A blob is a file chunk (*data*) or an encoded directory (*tree*). Each is optionally
compressed with zstd (one frame, kept only when it saves more than 64 bytes) and then
sealed with the blob AAD. Data and tree blobs go to separate packs.

```text
pack   = blob_0 || blob_1 || ... || blob_n-1 || seal(header) || u32 len(seal(header))
header = "ACPH" u16 version=1
         u32 count
         count x { u8 kind, u8 flags (bit 0 = zstd), id, u32 stored length, u32 plain length }
```

Blob offsets are the running sum of stored lengths and must end where the header
starts. Packs close when they reach the target size (16 MiB by default).

## Index (`index/<hex>`, sealed)

```text
"ACIX" u16 version=1
u32 pack count
per pack: id pack, u32 n, n x { u8 kind, u8 flags, id blob, u32 offset, u32 stored length, u32 plain length }
```

The repository index is the union of all index files; the first location of a blob
wins. A backup writes one new index file covering the packs it wrote; prune replaces
all index files by one.

## Trees (tree blobs)

```text
"ACTR" u16 version=1
u32 count
per node (sorted by name, names unique, no '/'):
  str name, u8 kind (1 file, 2 dir, 3 symlink), u64 size, i64 mtime secs, u32 mtime nanos, u32 mode
  file:    u32 n, n x id (data blob ids, in order)
  dir:     id (subtree)
  symlink: str target
```

The root tree mirrors absolute paths: backing up `/home/ann` gives a root tree with
`home`, whose tree holds `ann`. Directories made up only to reach a requested path take
their metadata from the source when it can stat them.

## Snapshots (`snapshots/<hex>`, sealed)

```text
"ACSN" u16 version=1
i64 secs, u32 nanos      (time, UTC; supplied by the caller)
id  root tree
u8  has parent, [id parent snapshot]
str host
strs paths (normalized, sorted)
strs tags
10 x u64 summary: files new, changed, unchanged, dirs, bytes total, bytes read,
                  blobs added, bytes added, bytes stored, errors
```

A backup with a parent reuses a file's chunk list without reading it when kind, size
and mtime match the parent's node and all its chunks are still indexed.

## Locks (`locks/<hex>`, sealed)

```text
"ACLK" u16 version=1
u8 exclusive, str host, i64 secs, u32 nanos
```

Backup takes a shared lock; forget and prune an exclusive one. A conflicting lock makes
the operation fail with [`Error::Locked`]; stale locks are removed explicitly
(`remove_locks`).

## Prune and crash safety

Prune walks every snapshot to find referenced blobs, deletes packs with nothing
referenced, and repacks packs with more than `max_unused_percent` unreferenced bytes by
copying their live sealed blobs unchanged into new packs. Order: write new packs, write
the new index, delete old index files, delete old packs. An interruption at any point
leaves every snapshot restorable; the leftovers are unindexed packs, which `check`
reports as warnings and the next prune deletes. A cancelled backup writes its open packs
and an index for them, so the next backup reuses that data.

## Why not restic's format (rustic_core)

rustic_core 0.13 depends on the `zstd` crate (C libzstd through zstd-sys), rayon,
crossbeam-channel, pariter (threads), nix, ignore, walkdir, dirs, gethostname,
filetime and more: no `no_std`, threads everywhere, C code. restic's format also needs
scrypt and AES-256-CTR + Poly1305-AES and zstd for its v2 format. This engine keeps
the same ideas (content-defined chunks, packs, index, trees, snapshots) in a small
`no_std` crate that uses only RustCrypto, rand_core and ruzstd.
