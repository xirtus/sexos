// SPDX-License-Identifier: GPL-3.0-only
use shuttle::{
    script_mask, FontCatalog, FontFilter, FontPicker, PickerKey, PRESET_SIZES,
};

#[test]
fn test_font_catalog_metadata_and_filtering() {
    let mut catalog = FontCatalog::new();
    let entries = catalog.entries();
    assert!(entries.len() >= 9, "Standard embedded fonts must be present");

    // Monospace filter
    let mono_filter = FontFilter {
        monospace_only: true,
        ..Default::default()
    };
    let monos = catalog.query(&mono_filter);
    assert!(!monos.is_empty());
    assert!(monos.iter().all(|e| e.is_monospaced));

    // Script filter: Arabic
    let arabic_filter = FontFilter {
        required_script: Some(script_mask::ARABIC),
        ..Default::default()
    };
    let arabics = catalog.query(&arabic_filter);
    assert!(!arabics.is_empty());
    assert!(arabics.iter().any(|e| e.name == "Noto Sans Arabic"));

    // Case-insensitive query filter
    let query_filter = FontFilter {
        query: Some(String::from("GeIsT")),
        ..Default::default()
    };
    let geists = catalog.query(&query_filter);
    assert_eq!(geists.len(), 2, "Geist Sans and Geist Mono must match");

    // Dynamic registration
    let dynamic_id = catalog.register(sex_font::GEIST_SANS);
    assert!(dynamic_id.is_some());
}

#[test]
fn test_font_picker_navigation_and_workflow() {
    let mut picker = FontPicker::new();
    assert!(!picker.filtered_entries().is_empty());

    let initial = picker.selected_entry().unwrap().clone();
    assert_eq!(picker.selected_size(), 16.0);

    // Keyboard navigation: Down then Up
    assert!(picker.handle_key(PickerKey::Down));
    let next = picker.selected_entry().unwrap().clone();
    assert_ne!(initial.id, next.id);

    assert!(picker.handle_key(PickerKey::Up));
    assert_eq!(picker.selected_entry().unwrap().id, initial.id);

    // PageDown and Home
    picker.handle_key(PickerKey::PageDown);
    picker.handle_key(PickerKey::Home);
    assert_eq!(picker.selected_entry().unwrap().id, initial.id);

    // Filter by typing "Mono"
    for c in "Mono".chars() {
        picker.handle_key(PickerKey::Char(c));
    }
    let filtered = picker.filtered_entries();
    assert!(!filtered.is_empty());
    assert!(filtered.iter().all(|e| e.name.contains("Mono")));

    // Backspace
    picker.handle_key(PickerKey::Backspace);

    // Click preset size
    picker.set_selected_size(PRESET_SIZES[4]); // 14.0
    assert_eq!(picker.selected_size(), 14.0);

    // Mouse click OK button
    assert!(!picker.is_confirmed());
    picker.handle_click(400, 330);
    assert!(picker.is_confirmed());

    let sel = picker.selection().expect("Selection must be valid");
    assert_eq!(sel.size, 14.0);
    assert!(!sel.font_data.is_empty());

    // Reset and click Cancel button
    picker.reset_actions();
    assert!(!picker.is_cancelled());
    picker.handle_click(300, 330);
    assert!(picker.is_cancelled());
}

#[test]
fn test_font_picker_software_rendering() {
    let picker = FontPicker::new();
    let width = 480;
    let height = 360;
    let mut buffer = vec![0u32; width * height];

    picker.render(&mut buffer, width, height);

    // Check that buffer was rendered into
    let non_zero_pixels = buffer.iter().filter(|&&p| p != 0).count();
    assert!(
        non_zero_pixels > 20000,
        "Rendered buffer should have substantial content, got {}",
        non_zero_pixels
    );

    // Check specific UI coordinates are non-zero:
    // Background pixel (0, 0)
    assert_ne!(buffer[0], 0);
    // OK button area
    assert_ne!(buffer[330 * width + 420], 0);
    // Cancel button area
    assert_ne!(buffer[330 * width + 320], 0);
}
