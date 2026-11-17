// SPDX-License-Identifier: GPL-3.0-only
//! Filigree font manager engine (`OmaFont` parity for SexOS).
//!
//! Pure `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod catalog;
pub mod cli;
pub mod specimen;
pub mod validator;

pub use catalog::{
    canonical_family_name, find_duplicates, parse_font_version, unify_families, ActiveManifest,
    Collection, DuplicateGroup, DuplicateKind, FontCatalog, FontEntry, FontVersion, ManifestError,
    SmartFilter, TidyAction, TidyVerb, UnifiedFamily, DEFAULT_MANIFEST_PATH,
};
pub use cli::{CliOutput, FiligreeCli};
pub use sex_font::{
    font_collection_count, inspect_font, FontClass, FontMetadata, Panose, ScriptRange,
    UnicodeCoverage,
};
pub use specimen::{
    build_comparison, calculate_waterfall, codepoint_page, inspect_codepoint_page,
    inspect_repertoire, nonempty_pages, spindle_font_config, spindle_font_config_entry,
    CodepointPage, CompareError, CompareSlot, ComparisonBoard, NormalizedMetrics, Repertoire,
    ScalarRange, SpindleFontConfig, SpindleHandoffError, WaterfallLayout, WaterfallRow,
    DEFAULT_SETTINGS_PATH, DEFAULT_WATERFALL_SIZES, MAX_COMPARE_SLOTS,
};
pub use validator::{
    sha256_digest, sha256_hex, validate_bundle, validate_bundle_with_limit, validate_font_bytes,
    validate_input, InputFormat, InstallPreview, ValidatedFace, ValidatedFontAsset,
    ValidationError, DEFAULT_MAX_ARCHIVE_MEMBER_BYTES,
};
