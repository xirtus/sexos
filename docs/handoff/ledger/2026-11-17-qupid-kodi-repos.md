# Qupid Q13b: Kodi repositories, browse and install

`sex_kodi::manager::Manager` keeps Kodi's addon folder layout (`addons/<id>/`,
`installed.txt`) over a caller's `Net` and `Store`: install a repository addon zip by
address, refresh every repository's addons.xml (gz handled, MD5/SHA checked), install an
addon with its `<requires>` first, each zip checked against its published hash and by
`package::unpack`. Host tests `crates/sex-kodi/tests/manager.rs`. Qupid wires it in
`apps/play/src/kodi.rs` (sex_app::net with redirects; files in its own `kodi/` folder):
`--add-repo URL`, `--addons [WORDS]` (Enter installs, R reloads), `--install ID`. QEMU gate
`scripts/qupid_kodi_gate.py` (CI media job) serves a real-layout repository from the host.
Next: the Python helper runs installed plugins (Q13c).
