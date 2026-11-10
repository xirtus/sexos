# SexOS changes to virtio-drivers 0.13.0 (MIT)

- `device/sound.rs`: capture. `pcm_capture_nb` queues a request for one period of an
  input stream on the RX queue; `pcm_capture_take` returns the oldest finished one.
  The upstream crate supports output streams only.
- Removed the upstream `.github`, `Cargo.lock` and `rust-toolchain.toml`.
