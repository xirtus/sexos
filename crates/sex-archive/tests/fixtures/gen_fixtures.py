# Regenerates the 7z and tar.zst fixtures: needs `pip install py7zr zstandard`.
# Usage: python3 gen_fixtures.py <out-dir>   (fixtures were made with py7zr 1.1.4)
import py7zr, zstandard, io, tarfile, sys, os
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
files = {
  "docs/readme.txt": b"hello from 7z\n" * 50,
  "docs/deep/nested/data.bin": bytes(range(256)) * 40,
  "empty.txt": b"",
  "unicode-éè.txt": "café ☃\n".encode(),
}
def mk7z(name, filters):
    with py7zr.SevenZipFile(os.path.join(out, name), "w", filters=filters) as z:
        for p, d in files.items():
            z.writestr(d, p)
mk7z("lzma2.7z", [{"id": py7zr.FILTER_LZMA2, "preset": 6}])
mk7z("lzma.7z", [{"id": py7zr.FILTER_LZMA}])
mk7z("copy.7z", [{"id": py7zr.FILTER_COPY}])
mk7z("bcj.7z", [{"id": py7zr.FILTER_X86}, {"id": py7zr.FILTER_LZMA2, "preset": 6}])
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as t:
    for p, d in files.items():
        ti = tarfile.TarInfo(p); ti.size = len(d); ti.mtime = 1700000000
        t.addfile(ti, io.BytesIO(d))
raw = buf.getvalue()
c = zstandard.ZstdCompressor(level=19, write_checksum=True)
half = len(raw)//2
open(os.path.join(out, "ref.tar.zst"), "wb").write(c.compress(raw[:half]) + c.compress(raw[half:]))
for f in ['lzma2','lzma','copy','bcj']:
    z = py7zr.SevenZipFile(os.path.join(out, f + '.7z'))
    print(f, [(i.filename, i.uncompressed, i.is_directory) for i in z.list()])
    z.close()
