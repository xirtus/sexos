# Signed updates: cutting a release

`update` on an installed SexOS takes a release only when its index is signed by the
release key the running system was built with. The key pair belongs to the project owner.
The secret half never goes into the repository, a chat or a CI log.

## Once: the release key

```sh
cargo run --release --manifest-path tools/sexos-release/Cargo.toml -- keygen ~/sexos-release-key
cp ~/sexos-release-key/release.pub keys/release.pub   # commit this file (.gitignore lets this one .pub through)
```

Keep `~/sexos-release-key/release.key` offline and backed up. If it is lost, installed
systems can't take updates until they are reinstalled from a build with a new key. For
CI, store its hex line as a repository secret (for example `SEXOS_RELEASE_SECRET`) and
sign with `sign - ...`.

Builds made after `keys/release.pub` is committed trust it. Older installs (and any build
without a key) refuse every update, so the first installs meant to be updated must come
from a build that has the key.

## Each release

```sh
./scripts/entrypoint_build.sh                        # trusts keys/release.pub
python3 scripts/make_boot_image.py --out out/sexos-boot.img
cargo run --release --manifest-path tools/sexos-release/Cargo.toml -- \
    sign ~/sexos-release-key/release.key VERSION out/sexos-boot.img out/
cargo run --release --manifest-path tools/sexos-release/Cargo.toml -- verify keys/release.pub out/
```

VERSION is a whole number, higher than every earlier release: systems refuse anything not
newer than what they run. Attach the three files to a GitHub release, named exactly
`sexos-update.txt`, `sexos-update.sig` and `sexos-boot.img`. `update` with no URL reads
`https://github.com/xirtus/sexos/releases/latest/download/`, so the newest non-prerelease
release is the one offered. `update URL` reads another folder with the same three names.

## What the person sees

`update` prints what it found, downloads the image (256 MiB), and says which slot it wrote.
After a restart the new system runs. If it never reaches the desktop, the next start goes
back to the previous one. "SexOS previous system" is always in the boot menu.

## Gates

`scripts/update_gate.py` with an ISO built with
`SEXOS_RELEASE_KEY=$(cargo run -q --release --manifest-path tools/sexos-release/Cargo.toml -- public test)`.
The test key (`sign test ...`) is public on purpose: only builds that set that variable trust it.
