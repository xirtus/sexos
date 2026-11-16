# Local no_std port

vst3 0.3.0, https://github.com/coupler-rs/vst3-rs, MIT OR Apache-2.0;
original licenses retained. Published generated bindings and COM support use
core instead of std, with no ABI/vtable/UID changes. com-scrape-types0.1.1 uses
alloc::sync::Arc in place of std::sync::Arc. The unchanged upstream Gain example
is the source for fixtures/moan-vst-gain; its separate freestanding adapter uses
core/alloc, activates the Linux-style module entry exports for target_os=none,
and delegates allocation to explicit host callbacks. DSP is unchanged.
Official format license reference: https://github.com/steinbergmedia/vst3sdk
(current SDK MIT; no proprietary SDK code is vendored by this port).
