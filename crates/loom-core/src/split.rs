// SPDX-License-Identifier: GPL-3.0-only
//! A bounded text row for Loom's two-folder list view.
use alloc::format;
use alloc::string::String;

pub fn columns(left: &str, right: &str, width: usize) -> String {
    let mut clipped: String = left.chars().take(width).collect();
    if left.chars().count() > width && width > 0 {
        clipped.pop();
        clipped.push('…');
    }
    format!("{clipped:<width$} │ {right}")
}

#[cfg(test)]
mod tests {
    use super::columns;

    #[test]
    fn right_folder_stays_visible_when_left_name_is_long() {
        let line = columns("> a-very-long-folder-name", "  report.pdf", 12);
        assert_eq!(line, "> a-very-lo… │   report.pdf");
        assert_eq!(columns("* docs", "> notes", 12), "* docs       │ > notes");
    }
}
