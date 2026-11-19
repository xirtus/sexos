use sex_hotplug::pcie::{self, SlotCaps};
use sex_hotplug::plan::{self, Bar, Node, Placed, Window, Error};
use sex_hotplug::slot::{self, Look, Slot, State};

fn look(present: bool, link: bool) -> Look { Look { present, link, button: false, powered: true } }

#[test]
fn slot_caps_decode_qemu_root_port() {
    // QEMU pcie-root-port: button, power controller, attention + power LEDs, hot-plug, slot 1.
    let caps = SlotCaps::decode(1 | 1 << 1 | 1 << 3 | 1 << 4 | 1 << 6 | 1 << 19);
    assert!(caps.button && caps.power_controller && caps.attention_led && caps.power_led && caps.hotplug);
    assert!(!caps.surprise && !caps.mrl);
    assert_eq!(caps.number, 1);
    // Root port (type 4) with a slot: hot-plug port. Endpoint (type 0): not.
    assert!(pcie::is_hotplug_port(0x0142, 1 << 6));
    assert!(!pcie::is_hotplug_port(0x0102 & !0x100, 1 << 6));
    assert!(!pcie::is_hotplug_port(0x0002, 1 << 6));
    assert!(!pcie::is_hotplug_port(0x0142, 0));
}

#[test]
fn control_sets_power_and_leds_in_one_write() {
    use pcie::control::*;
    let v = pcie::control(0x02c0, Some(false), Some(LED_OFF), None);
    assert_eq!(v & POWER_OFF, POWER_OFF);
    assert_eq!(v & POWER_LED_MASK, LED_OFF << 8);
    assert_eq!(v & ATTENTION_MASK, 0x00c0);
    let on = pcie::control(v, Some(true), Some(LED_ON), None);
    assert_eq!(on & POWER_OFF, 0);
    assert_eq!(on & POWER_LED_MASK, LED_ON << 8);
    assert_eq!(pcie::quiet_control(0x1fff) & (HOTPLUG_IRQ_ENABLE | PRESENCE_ENABLE | LINK_CHANGE_ENABLE), 0);
}

#[test]
fn card_arrives_after_settle_and_link() {
    let mut s = Slot::at_boot(look(false, false), true);
    assert_eq!(s.state, State::Empty);
    let unpowered = Look { present: true, link: false, button: false, powered: false };
    assert_eq!(s.step(0, unpowered), Default::default());
    assert!(matches!(s.state, State::Settling { .. }));
    // A bounce shorter than the settle time is ignored.
    s.step(50, look(false, false));
    assert_eq!(s.state, State::Empty);
    s.step(100, unpowered);
    let st = s.step(100 + slot::SETTLE_MS, unpowered);
    assert_eq!(st.power, Some(true));
    assert!(!st.arrived);
    s.step(400, look(true, true));
    assert!(matches!(s.state, State::LinkUp { .. }));
    assert!(!s.step(450, look(true, true)).arrived);
    let st = s.step(400 + slot::LINK_MS, look(true, true));
    assert!(st.arrived);
    assert_eq!(s.state, State::Up);
}

#[test]
fn surprise_removal_leaves_at_once() {
    let mut s = Slot::at_boot(look(true, true), true);
    assert_eq!(s.state, State::Up);
    let st = s.step(10, look(false, false));
    assert!(st.left);
    assert_eq!(st.power, Some(false));
    assert_eq!(s.state, State::Empty);
    // Link down with the card still in (a Thunderbolt cable pulled at the dock) counts too.
    let mut s = Slot::at_boot(look(true, true), false);
    let st = s.step(10, look(true, false));
    assert!(st.left);
    assert_eq!(st.power, None);
}

#[test]
fn attention_button_waits_five_seconds_and_can_be_cancelled() {
    let pressed = Look { present: true, link: true, button: true, powered: true };
    let mut s = Slot::at_boot(look(true, true), true);
    s.step(0, pressed);
    assert!(matches!(s.state, State::Button { .. }));
    assert!(!s.step(1000, look(true, true)).left);
    // Pressed again: cancelled.
    s.step(2000, pressed);
    assert_eq!(s.state, State::Up);
    s.step(3000, pressed);
    let st = s.step(3000 + slot::BUTTON_MS, look(true, true));
    assert!(st.left);
    assert_eq!(st.power, Some(false));
    assert_eq!(st.led, Some(pcie::control::LED_OFF));
    assert_eq!(s.state, State::Off);
    // The card is taken out: empty. Put back: settles again.
    s.step(9000, Look { present: false, link: false, button: false, powered: false });
    assert_eq!(s.state, State::Empty);
}

#[test]
fn link_that_never_comes_up_fails_once() {
    let mut s = Slot::at_boot(look(false, false), true);
    s.step(0, look(true, false));
    s.step(slot::SETTLE_MS, look(true, false));
    let st = s.step(slot::SETTLE_MS + slot::LINK_TIMEOUT_MS, look(true, false));
    assert!(st.failed);
    assert_eq!(s.state, State::Failed);
    assert_eq!(s.step(99_999, look(true, false)), Default::default());
    s.step(100_000, look(false, false));
    assert_eq!(s.state, State::Empty);
}

fn mem(size: u64) -> Bar { Bar { size, prefetch: false, wide: false } }
fn pref(size: u64) -> Bar { Bar { size, prefetch: true, wide: true } }

#[test]
fn nvme_below_root_port_fits_window() {
    // QEMU NVMe: BAR0 16 KiB (64-bit), MSI-X in BAR0.
    let nodes = [Node { parent: None, bridge: false, bars: [Bar { size: 16 << 10, prefetch: false, wide: true }, Bar::default(), Bar::default(), Bar::default(), Bar::default(), Bar::default()] }];
    let mut out = [Placed::default(); 1];
    plan::plan(&nodes, Window { base: 0xfe00_0000, size: 2 << 20 }, Window::default(), &mut out).unwrap();
    assert_eq!(out[0].bars[0], 0xfe00_0000);
}

#[test]
fn egpu_enclosure_gets_nested_windows() {
    // Thunderbolt enclosure: upstream port -> downstream port -> GPU (+ its audio function).
    let mut gpu = [Bar::default(); 6];
    gpu[0] = mem(16 << 20);
    gpu[1] = pref(256 << 20);
    gpu[3] = pref(32 << 20);
    let mut audio = [Bar::default(); 6];
    audio[0] = mem(16 << 10);
    let nodes = [
        Node { parent: None, bridge: true, bars: [Bar::default(); 6] },
        Node { parent: Some(0), bridge: true, bars: [Bar::default(); 6] },
        Node { parent: Some(1), bridge: false, bars: gpu },
        Node { parent: Some(1), bridge: false, bars: audio },
    ];
    let mut out = [Placed::default(); 4];
    let memw = Window { base: 0x8000_0000, size: 64 << 20 };
    let prefw = Window { base: 0x40_0000_0000, size: 1 << 30 };
    plan::plan(&nodes, memw, prefw, &mut out).unwrap();
    // GPU registers aligned to their size, inside both bridges' windows.
    assert_eq!(out[2].bars[0] % (16 << 20), 0);
    assert_eq!(out[2].bars[1] % (256 << 20), 0);
    for (node, bar, size) in [(2, 0, 16u64 << 20), (3, 0, 16 << 10)] {
        let a = out[node].bars[bar];
        for bridge in [0, 1] {
            let w = out[bridge].mem;
            assert!(a >= w.base && a + size <= w.base + w.size, "node {node} outside bridge {bridge}");
        }
    }
    for bridge in [0, 1] {
        let w = out[bridge].pref;
        assert!(out[2].bars[1] >= w.base && out[2].bars[3] + (32 << 20) <= w.base + w.size);
        assert_eq!(w.base % plan::BRIDGE_ALIGN, 0);
    }
    // 256 + 32 MiB rounded to a MiB: 288 MiB.
    assert_eq!(out[1].pref.size, 288 << 20);
    // No two BARs overlap.
    let mut spans = vec![(out[2].bars[0], 16u64 << 20), (out[2].bars[1], 256 << 20), (out[2].bars[3], 32 << 20), (out[3].bars[0], 16 << 10)];
    spans.sort();
    for w in spans.windows(2) { assert!(w[0].0 + w[0].1 <= w[1].0); }
}

#[test]
fn prefetchable_bars_use_memory_window_without_pref_window() {
    let mut bars = [Bar::default(); 6];
    bars[0] = pref(1 << 20);
    bars[2] = mem(4 << 10);
    let nodes = [Node { parent: None, bridge: false, bars }];
    let mut out = [Placed::default(); 1];
    plan::plan(&nodes, Window { base: 0xc000_0000, size: 2 << 20 }, Window::default(), &mut out).unwrap();
    assert_eq!(out[0].bars[0], 0xc000_0000);
    assert_eq!(out[0].bars[2], 0xc010_0000);
}

#[test]
fn too_small_window_is_refused() {
    let mut bars = [Bar::default(); 6];
    bars[0] = mem(4 << 20);
    let nodes = [Node { parent: None, bridge: false, bars }];
    let mut out = [Placed::default(); 1];
    let e = plan::plan(&nodes, Window { base: 0xc000_0000, size: 2 << 20 }, Window::default(), &mut out).unwrap_err();
    assert_eq!(e, Error::NoRoom { prefetch: false, need: 4 << 20, have: 2 << 20 });
    // Children before parents only.
    let nodes = [Node { parent: Some(1), bridge: false, bars: [Bar::default(); 6] }, Node { parent: None, bridge: true, bars: [Bar::default(); 6] }];
    let mut out = [Placed::default(); 2];
    assert_eq!(plan::plan(&nodes, Window { base: 0, size: 1 << 20 }, Window::default(), &mut out), Err(Error::Malformed));
}

#[test]
fn bar_sizing_and_bridge_registers_round_trip() {
    // 32-bit 16 KiB BAR.
    assert_eq!(plan::bar_size(0, 0xffff_c000, 0), Some(Bar { size: 16 << 10, prefetch: false, wide: false }));
    // 64-bit prefetchable 256 MiB BAR.
    assert_eq!(plan::bar_size(0xc, 0xf000_000c, 0xffff_ffff), Some(Bar { size: 256 << 20, prefetch: true, wide: true }));
    // 64-bit 8 GiB BAR (upper word holds the size).
    assert_eq!(plan::bar_size(0xc, 0x0000_000c, 0xffff_fffe), Some(Bar { size: 8 << 30, prefetch: true, wide: true }));
    // I/O and unimplemented.
    assert_eq!(plan::bar_size(1, 0xffff_ffe1, 0), None);
    assert_eq!(plan::bar_size(0, 0, 0), None);
    let m = Window { base: 0xfe00_0000, size: 2 << 20 };
    let p = Window { base: 0x40_0000_0000, size: 1 << 30 };
    let (lo, bh, lh) = plan::pref_registers(p);
    assert_eq!(plan::windows(plan::mem_register(m), lo, bh, lh), (m, p));
    let (lo, bh, lh) = plan::pref_registers(Window::default());
    assert_eq!(plan::windows(plan::mem_register(Window::default()), lo, bh, lh), (Window::default(), Window::default()));
    assert_eq!(plan::windows(0, 0, 0, 0), (Window::default(), Window::default()));
}

#[test]
fn narrow_prefetchable_bar_stays_below_4g() {
    let mut bars = [Bar::default(); 6];
    bars[0] = Bar { size: 1 << 20, prefetch: true, wide: false };
    bars[2] = pref(1 << 20);
    let nodes = [Node { parent: None, bridge: false, bars }];
    let mut out = [Placed::default(); 1];
    plan::plan(&nodes, Window { base: 0xc000_0000, size: 2 << 20 }, Window { base: 0x40_0000_0000, size: 1 << 30 }, &mut out).unwrap();
    assert_eq!(out[0].bars[0], 0xc000_0000);
    assert_eq!(out[0].bars[2], 0x40_0000_0000);
}
