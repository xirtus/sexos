// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_edit::Editor;

fuzz_target!(|data: &[u8]| {
    if let Ok(editor) = Editor::open(data.to_vec()) {
        let pages = editor.page_count();
        let _ = editor.original();
        let _ = editor.xref_style();
        let _ = editor.is_modified();

        for i in 0..pages.min(8) {
            let _ = editor.page_info(i);
            let _ = editor.annotations(i);
        }

        let _ = editor.fields();
        let _ = editor.signatures();

        // Exercise incremental save serialization if parsed
        let _ = editor.save_incremental();
    }
});
