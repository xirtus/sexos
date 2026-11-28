// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The Intel audio DSP lease Bell gets beside a Tiger Lake HD Audio lease:
//! `cargo test -p sex-pdx --test audio_dsp`.
use sex_pdx::audio_dsp::*;

#[test]
fn only_tiger_lake_cavs_gets_a_dsp() {
    assert_eq!(firmware(0x8086, 0x43c8), Some("sof-tgl-h.ri"));
    assert_eq!(firmware(0x8086, 0xa0c8), Some("sof-tgl.ri"));
    assert!(cavs(0x8086, 0x43c8) && cavs(0x8086, 0xa0c8));
    // QEMU's ICH6/ICH9 HD Audio, NVIDIA HDMI audio and other vendors keep the plain lease.
    for (v, d) in [(0x8086, 0x2668), (0x8086, 0x293e), (0x10de, 0x228b), (0x1022, 0x43c8)] {
        assert!(!cavs(v, d));
        assert_eq!(firmware(v, d), None);
    }
}

#[test]
fn dsp_view_spans_the_sram_windows() {
    // Linux sound/soc/sof/intel/hda.h: SRAM_WINDOW_OFFSET(x) = 0x80000 + x * 0x20000.
    assert_eq!(REGISTER_BYTES, 1 << 20);
    assert_eq!(BAR, 4);
    assert!(HDA_BYTES >= 0xc00 + 0x100);
    // The firmware copy (sof-tgl.ri is 525056 bytes) fits the pool with room for capture.
    assert!(DMA_BYTES >= 525_056 + 256 * 1024);
}

#[test]
fn slots_and_regions_do_not_collide_with_bells_others() {
    let slots = [SLOT_REGISTERS, SLOT_DMA, SLOT_FIRMWARE];
    let bell = [
        sex_pdx::usb_audio::SLOT, sex_pdx::bt_audio::SLOT, sex_pdx::virtio::SLOT_SOUND,
        sex_pdx::SLOT_DMA_REGION, sex_pdx::paging::SLOT_DEMAND_REGION, sex_pdx::SLOT_SEXT, sex_pdx::SLOT_SHELL,
    ];
    let rings = sex_pdx::sound_ring::bell_slot(0)..=sex_pdx::sound_ring::bell_slot(sex_pdx::app_catalog::INCARNATIONS.len());
    let extra = sex_pdx::device::EXTRA_SLOT_BASE..sex_pdx::device::EXTRA_SLOT_BASE + sex_pdx::device::EXTRA_SLOTS;
    for s in slots {
        assert!(s <= 1024 && !bell.contains(&s) && !rings.contains(&s) && !extra.contains(&s), "slot {s}");
    }
    assert!(slots[0] != slots[1] && slots[1] != slots[2] && slots[0] != slots[2]);
    let regions = [REGISTERS_REGION_ID, DMA_REGION_ID, FIRMWARE_REGION_ID, sex_pdx::pci_command::HDA_DMA_REGION_ID,
        sex_pdx::usb_audio::REGION_ID, sex_pdx::bt_audio::REGION_ID];
    for (i, a) in regions.iter().enumerate() {
        assert!(!regions[i + 1..].contains(a), "region {a:#x}");
    }
    assert_eq!(FIRMWARE_MODULE, "sof.pack");
}
