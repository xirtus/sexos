// SPDX-License-Identifier: GPL-3.0-only
//! `gaze-core`: Gaze Photo Library catalog, user albums, faces-free smart search by
//! date & place, offline reverse-geocoding gazetteer, and non-destructive edits
//! (crop, linear-light exposure, and shared `batik-core` filters).
//!
//! Privacy architecture: `FACES_FREE_POLICY = true`. Zero face detection, zero facial
//! landmarking, zero person clustering, and zero network reverse-geocoding.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod contact_sheet;
pub mod develop;
pub mod edits;
pub mod gazetteer;
pub mod library;
pub mod metadata;
pub mod search;
pub mod sidecar;

pub use contact_sheet::{
    analyze_scope_stats, export_contact_sheet_jpeg, export_contact_sheet_png,
    render_contact_sheet, render_contact_sheet_pages, render_scope_image, render_scope_into,
    render_scope_linear, ContactSheetEntry, ContactSheetSpec, HistogramData, ScopeInput,
    ScopeKind, ScopeStandard, ScopeStats, ScopeSurface, WaveformMode,
};
pub use develop::{
    apply_full_develop, builtin_lut_preset, denoise_bilateral, denoise_bilateral_image,
    develop_frame, develop_image, film_stock_from_slug, film_stock_slug, find_best_heal_source,
    heal_region, heal_region_image, next_film_stock, prev_film_stock, BrushDab, BrushMask,
    ColorGrade, ColorPrimaries, ColorRangeMask, Curve1D, DenoiseParams, DevelopRecipe, FilmStock,
    HealPatch, InterpMode, LinearFrame, LinearGradientMask, LocalMask, LocalMaskKind,
    LuminanceRangeMask, Lut3D, LutError, MaskAdjustments, RadialMask, RgbaF32,
};
pub use edits::{
    apply_edits, apply_filter_stack, build_tone_luts, CropAspect, CropRect, CurvesAdjustment,
    CurvesPreset, EditRecipe, FilterNode, FilterSpec, FilterStack, HueSatAdjustment,
    SharedFilterPreset,
};
pub use gazetteer::{haversine_km, resolve_place, CityEntry, ResolvedPlace, WORLD_CITIES};
pub use library::{basename, fnv1a_64, Album, PhotoLibrary, PhotoRecord, SidebarCollection};
pub use metadata::{
    build_jpeg_exif_app1, extract_metadata, inject_jpeg_exif, PhotoDate, PhotoMeta,
};
pub use search::{QueryToken, Season, SmartQuery};
pub use sidecar::{
    is_sidecar_filename, sidecar_path_for, ColorLabel, CullFlag, PhotoSidecar, SidecarError,
    SmartAlbumRule,
};

/// Compile-time and runtime marker confirming Gaze performs zero facial biometrics.
pub const FACES_FREE_POLICY: bool = true;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;
    use sex_image::Image;

    fn sample_image(w: usize, h: usize) -> Image {
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let r = ((x * 255) / w.max(1)) as u8;
                let g = ((y * 255) / h.max(1)) as u8;
                let b = (((x + y) * 127) / (w + h).max(1)) as u8;
                pixels.push(0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
            }
        }
        Image { width: w, height: h, pixels }
    }

    #[test]
    fn offline_gazetteer_resolves_cities_across_continents_and_remote_coords() {
        assert!(FACES_FREE_POLICY);
        let paris = resolve_place(48.8566, 2.3522).expect("paris");
        assert_eq!(paris.city, "Paris");
        assert_eq!(paris.country, "France");
        assert_eq!(paris.display, "Paris, France");
        assert!(paris.distance_km < 1.0);

        let tokyo = resolve_place(35.6895, 139.6917).expect("tokyo");
        assert_eq!(tokyo.city, "Tokyo");
        assert_eq!(tokyo.country, "Japan");
        assert_eq!(tokyo.display, "Tokyo, Japan");

        let reykjavik = resolve_place(64.1466, -21.9426).expect("reykjavik");
        assert_eq!(reykjavik.display, "Reykjavik, Iceland");

        let nyc = resolve_place(40.7128, -74.0060).expect("nyc");
        assert_eq!(nyc.display, "New York, USA");

        let sydney = resolve_place(-33.8688, 151.2093).expect("sydney");
        assert_eq!(sydney.display, "Sydney, Australia");

        let capetown = resolve_place(-33.9249, 18.4241).expect("capetown");
        assert_eq!(capetown.display, "Cape Town, South Africa");

        let saopaulo = resolve_place(-23.5505, -46.6333).expect("saopaulo");
        assert_eq!(saopaulo.display, "Sao Paulo, Brazil");

        // Point Nemo in South Pacific (> 250 km from any city) falls back to hemisphere coordinates
        let remote = resolve_place(-48.8767, -123.3933).expect("remote");
        assert_eq!(remote.city, "");
        assert_eq!(remote.display, "48.88°S, 123.39°W");

        // Invalid coordinates rejected
        assert!(resolve_place(95.0, 0.0).is_none());
        assert!(resolve_place(0.0, -185.0).is_none());
        assert!(resolve_place(f64::NAN, 2.35).is_none());
    }

    #[test]
    fn jpeg_exif_builder_and_png_text_and_mtime_extract_date_and_place() {
        let img = sample_image(32, 24);
        let raw_jpg = batik_core::encode_jpeg(&img.pixels, img.width, img.height, 90).expect("jpeg");
        let tagged_jpg = inject_jpeg_exif(
            &raw_jpg,
            "2026:05:14 16:30:00",
            48.8566,
            2.3522,
            Some("SexOS Leica"),
        );

        let meta = extract_metadata(&tagged_jpg, None);
        assert_eq!(meta.width, 32);
        assert_eq!(meta.height, 24);
        assert_eq!(meta.format_name, "JPEG");
        assert_eq!(
            meta.date,
            PhotoDate::new(2026, 5, 14, 16, 30, 0)
        );
        assert_eq!(meta.place.as_deref(), Some("Paris, France"));
        assert_eq!(meta.camera.as_deref(), Some("SexOS Leica"));
        let (lat, lon) = meta.gps.expect("gps");
        assert!((lat - 48.8566).abs() < 0.001);
        assert!((lon - 2.3522).abs() < 0.001);

        // West/South hemisphere EXIF check (Reykjavik: 64.1466 N, -21.9426 W)
        let tagged_isl = inject_jpeg_exif(
            &raw_jpg,
            "2025:08:21 11:00:00",
            64.1466,
            -21.9426,
            None,
        );
        let meta_isl = extract_metadata(&tagged_isl, None);
        assert_eq!(meta_isl.place.as_deref(), Some("Reykjavik, Iceland"));
        assert_eq!(meta_isl.date, PhotoDate::new(2025, 8, 21, 11, 0, 0));

        // PNG fallback to mtime (2026-11-15 00:00:00 UTC = 1794700800)
        let png_bytes = batik_core::encode_rgba_png(&img.pixels, img.width, img.height).expect("png");
        let meta_png = extract_metadata(&png_bytes, Some(1_794_700_800));
        assert_eq!(meta_png.format_name, "PNG");
        assert_eq!(meta_png.date, PhotoDate::new(2026, 11, 15, 0, 0, 0));
    }

    #[test]
    fn smart_search_matches_dates_ranges_seasons_places_albums_and_refuses_faces() {
        let mut lib = PhotoLibrary::new();
        lib.upsert_photo(
            "photos/paris.jpg",
            PhotoMeta {
                width: 400,
                height: 300,
                format_name: "JPEG".into(),
                date: PhotoDate::new(2026, 5, 14, 16, 30, 0),
                gps: Some((48.8566, 2.3522)),
                place: Some("Paris, France".into()),
                place_region: Some("Ile-de-France Europe".into()),
                camera: Some("SexOS Cam".into()),
                ..Default::default()
            },
        );
        lib.upsert_photo(
            "photos/tokyo.jpg",
            PhotoMeta {
                width: 400,
                height: 300,
                format_name: "JPEG".into(),
                date: PhotoDate::new(2026, 11, 2, 20, 15, 0),
                gps: Some((35.6762, 139.6503)),
                place: Some("Tokyo, Japan".into()),
                place_region: Some("Kanto Asia".into()),
                camera: Some("SexOS Cam".into()),
                ..Default::default()
            },
        );
        lib.upsert_photo(
            "photos/reykjavik.jpg",
            PhotoMeta {
                width: 400,
                height: 300,
                format_name: "JPEG".into(),
                date: PhotoDate::new(2025, 8, 21, 11, 0, 0),
                gps: Some((64.1466, -21.9426)),
                place: Some("Reykjavik, Iceland".into()),
                place_region: Some("Capital Region Europe".into()),
                camera: Some("SexOS Cam".into()),
                ..Default::default()
            },
        );
        lib.upsert_photo(
            "photos/new_york.jpg",
            PhotoMeta {
                width: 400,
                height: 300,
                format_name: "JPEG".into(),
                date: PhotoDate::new(2026, 5, 28, 14, 45, 0),
                gps: Some((40.7128, -74.0060)),
                place: Some("New York, USA".into()),
                place_region: Some("New York North America".into()),
                camera: Some("SexOS Cam".into()),
                ..Default::default()
            },
        );

        let trips = lib.create_album("Europe");
        lib.toggle_photo_in_album(trips, "photos/paris.jpg");
        lib.toggle_photo_in_album(trips, "photos/reykjavik.jpg");
        lib.toggle_favorite("photos/tokyo.jpg");

        let all = SidebarCollection::AllPhotos;
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("2026-05")).len(), 2);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("May 2026")).len(), 2);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("2026-05 Paris")).len(), 1);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("autumn Japan")).len(), 1);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("summer 2025")).len(), 1);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("Europe")).len(), 2);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("album:europe")).len(), 2);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("2025..2026")).len(), 4);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("2026-01..2026-06")).len(), 2);
        assert_eq!(lib.visible_indices(&all, &SmartQuery::parse("fav")).len(), 1);

        // Biometric face/person queries are strictly refused -> 0 matches + rejected_biometric flag
        let bio_q = SmartQuery::parse("person:alice");
        assert!(bio_q.rejected_biometric);
        assert_eq!(lib.visible_indices(&all, &bio_q).len(), 0);

        let face_q = SmartQuery::parse("face:smile 2026");
        assert!(face_q.rejected_biometric);
        assert_eq!(lib.visible_indices(&all, &face_q).len(), 0);
    }

    #[test]
    fn shared_batik_filters_match_batik_core_bit_for_bit_and_keep_original_safe() {
        let src = sample_image(48, 36);
        let src_backup = src.clone();

        for preset in SharedFilterPreset::ALL {
            let mut recipe = EditRecipe {
                preset,
                ..EditRecipe::default()
            };
            recipe.filter_stack.push(FilterNode::new(FilterSpec::Sharpen { amount_pct: 60 }));

            let gaze_out = apply_edits(&src, &recipe).expect("gaze edit");

            // Original source image is strictly unmutated
            assert_eq!(src, src_backup);

            // Direct batik_core::filters::apply_filter_stack reference check
            let mut batik_pixels = src.pixels.clone();
            let mut ref_stack = Vec::new();
            if let Some(node) = preset.to_filter_node() {
                ref_stack.push(node);
            }
            ref_stack.push(FilterNode::new(FilterSpec::Sharpen { amount_pct: 60 }));
            batik_core::filters::apply_filter_stack(&ref_stack, &mut batik_pixels, src.width, src.height);

            assert_eq!(gaze_out.pixels, batik_pixels, "mismatch on preset {:?}", preset);
        }

        // Verify 16:9 non-destructive crop + +0.75 EV exposure increases midtone luminance smoothly
        let crop_recipe = EditRecipe {
            crop: CropRect::from_aspect(CropAspect::SixteenNine, 48, 36, 0),
            aspect: CropAspect::SixteenNine,
            exposure_ev_x100: 75,
            ..EditRecipe::default()
        };
        let cropped = apply_edits(&src, &crop_recipe).expect("cropped");
        assert_eq!(cropped.width, 48);
        assert_eq!(cropped.height, 27);
        let orig_mid = (src.pixels[18 * 48 + 24] >> 16) & 0xFF;
        let bright_mid = (cropped.pixels[13 * 48 + 24] >> 16) & 0xFF;
        assert!(bright_mid > orig_mid);
    }

    #[test]
    fn library_catalog_albums_and_recipes_roundtrip_deterministically() {
        let mut lib = PhotoLibrary::new();
        lib.upsert_photo(
            "@/data/spindle/photos/paris.jpg",
            PhotoMeta {
                width: 320,
                height: 240,
                format_name: "JPEG".into(),
                date: PhotoDate::new(2026, 5, 14, 16, 30, 0),
                gps: Some((48.8566, 2.3522)),
                place: Some("Paris, France".into()),
                place_region: Some("Ile-de-France Europe".into()),
                camera: Some("SexOS Cam".into()),
                lens: Some("Summicron 35\tf/2".into()),
                exposure: Some("1/250 s \u{b7} f/2.8 \u{b7} ISO 400 \u{b7} 35 mm".into()),
            },
        );
        lib.toggle_favorite("@/data/spindle/photos/paris.jpg");
        let album_idx = lib.create_album("Summer Trip");
        lib.toggle_photo_in_album(album_idx, "@/data/spindle/photos/paris.jpg");

        let mut recipe = EditRecipe {
            crop: CropRect::from_aspect(CropAspect::SixteenNine, 320, 240, 50),
            aspect: CropAspect::SixteenNine,
            quarter_turns: 0,
            flip_h: false,
            exposure_ev_x100: 50,
            contrast_pct: 15,
            warmth_pct: 15,
            saturation_pct: 10,
            preset: SharedFilterPreset::WarmVintage,
            filter_stack: vec![FilterNode::new(FilterSpec::Sharpen { amount_pct: 75 })],
        };
        lib.set_recipe("@/data/spindle/photos/paris.jpg", recipe.clone());

        let text = lib.serialize();
        let reloaded = PhotoLibrary::deserialize(&text);
        assert_eq!(reloaded.albums.len(), 1);
        assert_eq!(reloaded.albums[0].name, "Summer Trip");
        assert!(reloaded.albums[0].contains("@/data/spindle/photos/paris.jpg"));

        let meta_copy = {
            let rec = reloaded
                .photo_by_path("@/data/spindle/photos/paris.jpg")
                .expect("photo reloaded");
            assert!(rec.favorite);
            assert_eq!(rec.meta.place.as_deref(), Some("Paris, France"));
            assert_eq!(rec.meta.date, PhotoDate::new(2026, 5, 14, 16, 30, 0));
            assert_eq!(rec.recipe, recipe);
            assert_eq!(rec.meta.lens.as_deref(), Some("Summicron 35\tf/2"));
            assert_eq!(rec.meta.exposure.as_deref(), Some("1/250 s \u{b7} f/2.8 \u{b7} ISO 400 \u{b7} 35 mm"));
            rec.meta.clone()
        };

        // Re-upserting metadata preserves favorite & non-destructive recipe!
        let mut reloaded = reloaded;
        reloaded.upsert_photo("@/data/spindle/photos/paris.jpg", meta_copy);
        let rec2 = reloaded.photo_by_path("@/data/spindle/photos/paris.jpg").unwrap();
        assert!(rec2.favorite);
        assert_eq!(rec2.recipe, recipe);
        recipe.exposure_ev_x100 = 0;
        assert_ne!(rec2.recipe, recipe);
    }

    #[test]
    fn exposure_line_reads_like_a_camera_and_old_catalogs_still_load() {
        use gaze_codecs::Rational;
        let r = |num, den| Some(Rational { num, den });
        let exif = gaze_codecs::Exif { exposure_time: r(1, 250), f_number: r(28, 10), iso: Some(400), focal_length: r(35, 1), ..Default::default() };
        assert_eq!(metadata::exposure_line(&exif).as_deref(), Some("1/250 s \u{b7} f/2.8 \u{b7} ISO 400 \u{b7} 35 mm"));
        let long = gaze_codecs::Exif { exposure_time: r(5, 2), f_number: r(8, 1), ..Default::default() };
        assert_eq!(metadata::exposure_line(&long).as_deref(), Some("2.5 s \u{b7} f/8"));
        let odd = gaze_codecs::Exif { exposure_time: r(10, 1250), focal_length: r(0, 0), ..Default::default() };
        assert_eq!(metadata::exposure_line(&odd).as_deref(), Some("1/125 s"));
        // Slow but under a second: seconds, not a rounded 1/N (4/5 s is not 1/1 s).
        for ((num, den), want) in [((4, 5), "0.8 s"), ((3, 5), "0.6 s"), ((1, 3), "0.3 s"), ((1, 4), "1/4 s")] {
            let e = gaze_codecs::Exif { exposure_time: r(num, den), ..Default::default() };
            assert_eq!(metadata::exposure_line(&e).as_deref(), Some(want));
        }
        assert_eq!(metadata::exposure_line(&gaze_codecs::Exif::default()), None);

        // A catalog line written before lens and exposure were kept (13 columns).
        let mut fresh = PhotoLibrary::new();
        fresh.upsert_photo("photos/a.jpg", PhotoMeta { width: 10, height: 10, camera: Some("Cam".into()), lens: Some("L".into()), ..Default::default() });
        let text = fresh.serialize();
        let old: String = text.lines().map(|l| {
            let mut line = if l.starts_with("PHOTO\t") { l.split('\t').take(13).collect::<Vec<_>>().join("\t") } else { l.to_string() };
            line.push('\n');
            line
        }).collect();
        assert!(old.lines().any(|l| l.starts_with("PHOTO\t") && l.split('\t').count() == 13));
        let lib = PhotoLibrary::deserialize(&old);
        let rec = lib.photo_by_path("photos/a.jpg").expect("old line loads");
        assert_eq!(rec.meta.camera.as_deref(), Some("Cam"));
        assert_eq!((rec.meta.lens.as_deref(), rec.meta.exposure.as_deref()), (None, None));
    }

    #[test]
    fn png_text_chunks_and_corrupt_catalog_resilience() {
        let img = sample_image(16, 16);
        let mut png = batik_core::encode_rgba_png(&img.pixels, img.width, img.height).expect("png");
        // Insert a valid tEXt chunk ("Place\0Kyoto, Japan") and ("Date\02026-04-08 09:15:00") before IEND
        let iend_pos = png.len() - 12;
        let tail = png[iend_pos..].to_vec();
        png.truncate(iend_pos);
        for payload in [&b"Place\0Kyoto, Japan"[..], &b"Date\02026-04-08 09:15:00"[..]] {
            png.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            png.extend_from_slice(b"tEXt");
            png.extend_from_slice(payload);
            png.extend_from_slice(&0u32.to_be_bytes());
        }
        png.extend_from_slice(&tail);

        let meta = extract_metadata(&png, None);
        assert_eq!(meta.place.as_deref(), Some("Kyoto, Japan"));
        assert_eq!(meta.date, PhotoDate::new(2026, 4, 8, 9, 15, 0));

        // Corrupt / malformed library.gaze inputs never panic
        assert!(PhotoLibrary::deserialize("").photos.is_empty());
        assert!(PhotoLibrary::deserialize("NOT_GAZE\nPHOTO\tx").photos.is_empty());
        let weird = PhotoLibrary::deserialize(
            "GAZE_LIBRARY_V1\nALBUM\tTrips\\t2026\nALBUM_ITEM\t99\tmissing\nPHOTO\tbad\n",
        );
        assert_eq!(weird.albums.len(), 1);
        assert_eq!(weird.albums[0].name, "Trips\t2026");
        assert!(weird.photos.is_empty());
    }

    fn channel_rgb(px: u32) -> (u8, u8, u8) {
        (
            ((px >> 16) & 0xFF) as u8,
            ((px >> 8) & 0xFF) as u8,
            (px & 0xFF) as u8,
        )
    }

    fn luma_u8(px: u32) -> f32 {
        let (r, g, b) = channel_rgb(px);
        0.2126 * (r as f32) + 0.7152 * (g as f32) + 0.0722 * (b as f32)
    }

    #[test]
    fn sidecar_toml_roundtrip_cull_edit_and_full_develop() {
        let mut sc = PhotoSidecar::new("DSC0042.ARW");
        sc.set_stars(5);
        sc.flag = CullFlag::Pick;
        sc.color_label = ColorLabel::Green;
        sc.add_keyword("Kyoto");
        sc.add_keyword("Golden Hour");
        sc.add_keyword("kyoto"); // case-insensitive deduplication

        sc.edit = EditRecipe {
            crop: CropRect::from_aspect(CropAspect::SixteenNine, 600, 400, 20),
            aspect: CropAspect::SixteenNine,
            quarter_turns: 1,
            flip_h: true,
            exposure_ev_x100: 50,
            contrast_pct: 10,
            warmth_pct: -5,
            saturation_pct: 15,
            preset: SharedFilterPreset::WarmVintage,
            filter_stack: vec![FilterNode::new(FilterSpec::Sharpen { amount_pct: 60 })],
        };

        let mut dev = DevelopRecipe::default();
        dev.grade.exposure_ev = 0.75;
        dev.grade.temperature = 0.25;
        dev.grade.tint = -0.10;
        dev.grade.contrast = 1.18;
        dev.grade.vibrance = 0.35;
        dev.grade.lift = [0.01, 0.0, -0.02];
        dev.grade.gamma = [1.05, 1.0, 0.98];
        dev.grade.gain = [1.08, 1.02, 0.94];
        dev.grade.offset = [0.0, 0.005, -0.005];
        dev.grade.master = Curve1D::from_points(&[[0.0, 0.02], [0.5, 0.54], [1.0, 0.98]]);
        dev.highlights = -0.30;
        dev.shadows = 0.40;
        dev.film_stock = Some(FilmStock::Portra400);
        dev.film_strength = 0.85;
        let (_, lut) = builtin_lut_preset("teal_amber").expect("builtin lut");
        dev.lut_name = Some("Cine Teal & Amber".to_string());
        dev.lut = Some(lut);
        dev.lut_strength = 0.65;
        dev.denoise = DenoiseParams::high_iso();
        dev.heal_spots.push(HealPatch::auto(0.42, 0.31, 0.03).with_source(0.49, 0.31));

        // Attach all 5 local mask types
        let adj = MaskAdjustments {
            exposure_ev: -0.5,
            contrast: 0.15,
            highlights: -0.2,
            shadows: 0.1,
            temperature: -0.25,
            tint: 0.05,
            saturation: 0.2,
            sharpness: 0.3,
        };
        dev.masks.push(LocalMask::brush(
            BrushMask::new(vec![BrushDab::new(0.25, 0.5, 0.12, 0.5, 0.8)]),
            adj,
        ));
        dev.masks.push(LocalMask::linear_gradient(
            LinearGradientMask { x0: 0.5, y0: 0.0, x1: 0.5, y1: 0.5 },
            adj,
        ));
        dev.masks.push(LocalMask::radial(
            RadialMask { cx: 0.5, cy: 0.5, rx: 0.3, ry: 0.2, feather: 0.6, rotation_deg: 15.0 },
            adj,
        ));
        dev.masks.push(LocalMask::luminance_range(
            LuminanceRangeMask { min_luma: 0.6, max_luma: 1.0, feather: 0.1 },
            adj,
        ));
        dev.masks.push(LocalMask::color_range(
            ColorRangeMask::from_srgb([0.1, 0.4, 0.9], 0.2, 0.25, 0.4, 0.2),
            adj,
        ));
        sc.develop = dev;

        let toml = sc.to_toml();
        assert!(toml.contains("[cull]"));
        assert!(toml.contains("[develop.denoise]"));
        assert!(toml.contains("[[develop.mask]]"));

        let parsed = PhotoSidecar::from_toml(&toml).expect("parse sidecar");
        assert_eq!(parsed.stars, 5);
        assert_eq!(parsed.flag, CullFlag::Pick);
        assert_eq!(parsed.color_label, ColorLabel::Green);
        assert_eq!(parsed.keywords, vec!["Kyoto", "Golden Hour"]);
        assert_eq!(parsed.edit, sc.edit);
        assert_eq!(parsed.develop.film_stock, Some(FilmStock::Portra400));
        assert_eq!(parsed.develop.masks.len(), 5);
        assert_eq!(parsed.develop.heal_spots.len(), 1);
        assert_eq!(parsed.develop.denoise, sc.develop.denoise);
        assert!(parsed.develop.lut.is_some());
        // Deterministic re-serialization matches byte-for-byte
        assert_eq!(parsed.to_toml(), toml);
    }

    #[test]
    fn sidecar_path_and_filename_helpers_and_corrupt_resilience() {
        assert_eq!(
            sidecar_path_for("photos/IMG_001.DNG"),
            "photos/IMG_001.DNG.gaze.toml"
        );
        assert!(is_sidecar_filename("IMG_001.DNG.gaze.toml"));
        assert!(!is_sidecar_filename("IMG_001.DNG"));

        assert_eq!(PhotoSidecar::from_toml("   \n# comment only\n"), Err(SidecarError::EmptyInput));
        assert_eq!(PhotoSidecar::from_toml("invalid_no_equals"), Err(SidecarError::ParseError));
        assert_eq!(PhotoSidecar::from_toml("unknown_only = 42"), Err(SidecarError::ParseError));

        // Clamping stars > 5 and ignoring unknown keys safely
        let partial = "[cull]\nstars = 99\nflag = \"pick\"\nfuture_field = \"ok\"\n";
        let loaded = PhotoSidecar::from_toml(partial).expect("partial sidecar");
        assert_eq!(loaded.stars, 5);
        assert_eq!(loaded.flag, CullFlag::Pick);
    }

    #[test]
    fn smart_album_rule_filters_stars_flags_labels_keywords_camera_date_film() {
        let mut rec = PhotoRecord::new(
            "photos/fuji.dng",
            PhotoMeta {
                width: 4000,
                height: 3000,
                format_name: "RAW".into(),
                date: PhotoDate::new(2026, 10, 12, 17, 20, 0),
                place: Some("Kyoto, Japan".into()),
                camera: Some("Fujifilm X-T5".into()),
                lens: Some("XF 35mm f/1.4".into()),
                ..Default::default()
            },
        );
        rec.stars = 4;
        rec.flag = CullFlag::Pick;
        rec.color_label = ColorLabel::Blue;
        rec.keywords = vec!["Autumn".into(), "Temple".into()];
        rec.develop.film_stock = Some(FilmStock::Fuji3510Print);

        let mut rule = SmartAlbumRule::keepers("Kyoto Fuji Picks");
        rule.min_stars = Some(4);
        rule.required_flag = Some(CullFlag::Pick);
        rule.required_label = Some(ColorLabel::Blue);
        rule.required_keywords = vec!["autumn".into()];
        rule.camera_contains = Some("x-t5".into());
        rule.year_range = Some((2025, 2026));
        rule.film_stock = Some(FilmStock::Fuji3510Print);
        rule.only_edited = true;
        assert!(rule.matches(&rec));

        // Reject flag or missing keyword or wrong film stock fails rule
        rec.flag = CullFlag::Reject;
        assert!(!rule.matches(&rec));
        rec.flag = CullFlag::Pick;
        rule.film_stock = Some(FilmStock::Portra400);
        assert!(!rule.matches(&rec));
    }

    #[test]
    fn library_culling_collections_and_v2_columns_roundtrip() {
        let mut lib = PhotoLibrary::new();
        lib.upsert_photo(
            "photos/a.jpg",
            PhotoMeta {
                width: 800,
                height: 600,
                date: PhotoDate::new(2026, 5, 1, 10, 0, 0),
                camera: Some("Leica Q3".into()),
                ..Default::default()
            },
        );
        lib.upsert_photo(
            "photos/b.jpg",
            PhotoMeta {
                width: 800,
                height: 600,
                date: PhotoDate::new(2026, 5, 2, 10, 0, 0),
                camera: Some("Sony A7CR".into()),
                ..Default::default()
            },
        );
        lib.set_stars("photos/a.jpg", 4);
        lib.set_flag("photos/a.jpg", CullFlag::Pick);
        lib.set_color_label("photos/a.jpg", ColorLabel::Green);
        lib.add_keyword("photos/a.jpg", "sunset");
        let mut dev = DevelopRecipe::default();
        dev.film_stock = Some(FilmStock::Portra400);
        lib.set_develop("photos/a.jpg", dev);

        lib.set_stars("photos/b.jpg", 1);
        lib.set_flag("photos/b.jpg", CullFlag::Reject);

        assert_eq!(lib.keepers_count(), 1);
        assert_eq!(lib.picks_count(), 1);
        assert_eq!(lib.rejected_count(), 1);
        assert_eq!(lib.edited_count(), 1);
        assert_eq!(lib.visible_indices(&SidebarCollection::Keepers, &SmartQuery::default()).len(), 1);
        assert_eq!(lib.visible_indices(&SidebarCollection::Rejected, &SmartQuery::default()).len(), 1);

        // SmartQuery culling & film tokens
        assert_eq!(lib.visible_indices(&SidebarCollection::AllPhotos, &SmartQuery::parse("stars:4 pick label:green kw:sunset film:portra")).len(), 1);
        assert_eq!(lib.visible_indices(&SidebarCollection::AllPhotos, &SmartQuery::parse("keepers")).len(), 1);
        assert_eq!(lib.visible_indices(&SidebarCollection::AllPhotos, &SmartQuery::parse("reject")).len(), 1);

        // Full library serialize -> deserialize roundtrip
        let text = lib.serialize();
        let reloaded = PhotoLibrary::deserialize(&text);
        let a = reloaded.photo_by_path("photos/a.jpg").expect("a reloaded");
        assert_eq!(a.stars, 4);
        assert_eq!(a.flag, CullFlag::Pick);
        assert_eq!(a.color_label, ColorLabel::Green);
        assert_eq!(a.keywords, vec!["sunset"]);
        assert_eq!(a.develop.film_stock, Some(FilmStock::Portra400));
    }

    #[test]
    fn develop_identity_leaves_pixels_bit_exact() {
        let src = sample_image(24, 20);
        let dev = DevelopRecipe::default();
        assert!(dev.is_identity());
        let out = develop_image(&src, &dev).expect("identity develop");
        assert_eq!(out, src);

        let edit = EditRecipe {
            exposure_ev_x100: 25,
            preset: SharedFilterPreset::SCurveContrast,
            ..EditRecipe::default()
        };
        assert_eq!(
            apply_full_develop(&src, &edit, &dev).unwrap(),
            apply_edits(&src, &edit).unwrap()
        );
    }

    #[test]
    fn develop_color_wheels_lift_gamma_gain_offset_ev_and_wb() {
        let mid = Image {
            width: 4,
            height: 4,
            pixels: vec![0xFF70_7070; 16],
        };
        let orig = mid.clone();

        let mut ev_dev = DevelopRecipe::default();
        ev_dev.grade.exposure_ev = 1.0;
        let brighter = develop_image(&mid, &ev_dev).expect("ev +1");
        assert_eq!(mid, orig);
        assert!(channel_rgb(brighter.pixels[0]).0 > channel_rgb(mid.pixels[0]).0);

        let mut warm_dev = DevelopRecipe::default();
        warm_dev.grade.temperature = 0.6;
        let warmer = develop_image(&mid, &warm_dev).expect("warm wb");
        let (wr, _, wb) = channel_rgb(warmer.pixels[0]);
        assert!(wr > wb);

        let mut wheel_dev = DevelopRecipe::default();
        wheel_dev.grade.lift = [0.05, 0.0, 0.0];
        wheel_dev.grade.gain = [1.0, 1.15, 0.85];
        wheel_dev.grade.gamma = [1.0, 1.1, 1.0];
        let graded = develop_image(&mid, &wheel_dev).expect("wheels");
        assert_ne!(graded.pixels[0], mid.pixels[0]);
    }

    #[test]
    fn develop_highlights_shadows_vibrance_and_curves() {
        let dark_and_bright = Image {
            width: 2,
            height: 1,
            pixels: vec![0xFF20_2020, 0xFFE8_E8E8],
        };
        let mut dev = DevelopRecipe::default();
        dev.shadows = 0.7;
        dev.highlights = -0.6;
        let out = develop_image(&dark_and_bright, &dev).expect("tone zones");
        assert!(luma_u8(out.pixels[0]) > luma_u8(dark_and_bright.pixels[0]));
        assert!(luma_u8(out.pixels[1]) < luma_u8(dark_and_bright.pixels[1]));

        // Vibrance boosts muted color more strongly than vivid primary
        let mut vib = DevelopRecipe::default();
        vib.grade.vibrance = 0.9;
        let muted = Image { width: 1, height: 1, pixels: vec![0xFF78_6E68] };
        let muted_out = develop_image(&muted, &vib).expect("vibrance");
        let (mr, _, mb) = channel_rgb(muted.pixels[0]);
        let (or, _, ob) = channel_rgb(muted_out.pixels[0]);
        assert!((or as i32 - ob as i32) > (mr as i32 - mb as i32));
    }

    #[test]
    fn develop_all_12_film_stocks_and_cycling_produce_distinct_looks() {
        let mut cur = None;
        let mut seen = Vec::new();
        for _ in 0..12 {
            cur = next_film_stock(cur);
            seen.push(cur.expect("stock"));
        }
        assert_eq!(seen.len(), 12);
        assert_eq!(next_film_stock(cur), None);
        assert_eq!(prev_film_stock(None), Some(FilmStock::ALL[11]));

        let src = sample_image(16, 16);
        let mut d_2383 = DevelopRecipe::default();
        d_2383.film_stock = Some(FilmStock::Kodak2383Print);
        let mut d_portra = DevelopRecipe::default();
        d_portra.film_stock = Some(FilmStock::Portra400);
        let mut d_fuji = DevelopRecipe::default();
        d_fuji.film_stock = Some(FilmStock::Fuji3510Print);

        let o1 = develop_image(&src, &d_2383).unwrap();
        let o2 = develop_image(&src, &d_portra).unwrap();
        let o3 = develop_image(&src, &d_fuji).unwrap();
        assert_ne!(o1.pixels, o2.pixels);
        assert_ne!(o2.pixels, o3.pixels);
        assert_ne!(o1.pixels, src.pixels);

        // Film strength = 0.0 yields original
        d_2383.film_strength = 0.0;
        assert_eq!(develop_image(&src, &d_2383).unwrap(), src);
    }

    #[test]
    fn develop_tetrahedral_cube_lut_parsing_and_application() {
        let cube = "\
TITLE \"Warm Split\"
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
0.0 0.0 0.0
1.0 0.1 0.1
0.0 0.9 0.0
1.0 0.9 0.0
0.0 0.0 0.8
1.0 0.1 0.8
0.0 0.9 0.8
1.0 1.0 1.0
";
        let mut dev = DevelopRecipe::default();
        dev.set_lut_cube(None, cube, 1.0).expect("parse cube");
        assert_eq!(dev.lut_name.as_deref(), Some("Warm Split"));
        assert_eq!(dev.lut_interp, InterpMode::Tetrahedral);

        // Neutral gray stays exact on tetrahedral diagonal while blue corner compresses to 0.8
        let test_img = Image {
            width: 2,
            height: 1,
            pixels: vec![0xFF80_8080, 0xFF00_00FF],
        };
        let out = develop_image(&test_img, &dev).unwrap();
        let (gr, gg, gb) = channel_rgb(out.pixels[0]);
        assert!((gr as i16 - gg as i16).abs() <= 1 && (gg as i16 - gb as i16).abs() <= 1);
        let (_, _, bb) = channel_rgb(out.pixels[1]);
        assert!(bb < 245);
    }

    #[test]
    fn local_mask_brush_dabs_accumulates_feathered_strokes() {
        let src = Image {
            width: 20,
            height: 20,
            pixels: vec![0xFF50_5050; 400],
        };
        let mut dev = DevelopRecipe::default();
        dev.masks.push(LocalMask::brush(
            BrushMask::new(vec![BrushDab::new(0.2, 0.2, 0.2, 0.4, 1.0)]),
            MaskAdjustments {
                exposure_ev: 1.5,
                ..MaskAdjustments::default()
            },
        ));
        let out = develop_image(&src, &dev).unwrap();
        let center_luma = luma_u8(out.pixel(4, 4).unwrap());
        let far_luma = luma_u8(out.pixel(18, 18).unwrap());
        assert!(center_luma > far_luma + 30.0);
        assert_eq!(out.pixel(18, 18), src.pixel(18, 18));
    }

    #[test]
    fn local_mask_linear_gradient_transitions_smoothly() {
        let src = Image {
            width: 10,
            height: 20,
            pixels: vec![0xFF80_8080; 200],
        };
        let mut dev = DevelopRecipe::default();
        dev.masks.push(LocalMask::linear_gradient(
            LinearGradientMask {
                x0: 0.5,
                y0: 0.0,
                x1: 0.5,
                y1: 0.8,
            },
            MaskAdjustments {
                exposure_ev: -1.5,
                ..MaskAdjustments::default()
            },
        ));
        let out = develop_image(&src, &dev).unwrap();
        let top_luma = luma_u8(out.pixel(5, 0).unwrap());
        let mid_luma = luma_u8(out.pixel(5, 8).unwrap());
        let bot_luma = luma_u8(out.pixel(5, 19).unwrap());
        assert!(top_luma < mid_luma && mid_luma < bot_luma);
        assert_eq!(out.pixel(5, 19), src.pixel(5, 19));
    }

    #[test]
    fn local_mask_radial_elliptical_and_rotated_feathering() {
        let src = Image {
            width: 24,
            height: 24,
            pixels: vec![0xFF90_9090; 576],
        };
        let mut mask = LocalMask::radial(
            RadialMask {
                cx: 0.5,
                cy: 0.5,
                rx: 0.35,
                ry: 0.25,
                feather: 0.5,
                rotation_deg: 30.0,
            },
            MaskAdjustments {
                exposure_ev: -1.5,
                ..MaskAdjustments::default()
            },
        );
        mask.inverted = true; // classic radial vignette: darken outside, protect subject center
        let mut dev = DevelopRecipe::default();
        dev.masks.push(mask);

        let out = develop_image(&src, &dev).unwrap();
        assert_eq!(out.pixel(12, 12), src.pixel(12, 12));
        assert!(luma_u8(out.pixel(0, 0).unwrap()) < luma_u8(out.pixel(12, 12).unwrap()) - 25.0);
    }

    #[test]
    fn local_mask_luminance_range_targets_highlights_or_shadows() {
        let src = Image {
            width: 2,
            height: 1,
            pixels: vec![0xFF28_2828, 0xFFD8_D8D8],
        };
        let mut dev = DevelopRecipe::default();
        dev.masks.push(LocalMask::luminance_range(
            LuminanceRangeMask {
                min_luma: 0.65,
                max_luma: 1.0,
                feather: 0.1,
            },
            MaskAdjustments {
                exposure_ev: -1.2,
                ..MaskAdjustments::default()
            },
        ));
        let out = develop_image(&src, &dev).unwrap();
        // Dark shadow untouched; bright highlight darkened
        assert_eq!(out.pixel(0, 0), src.pixel(0, 0));
        assert!(luma_u8(out.pixel(1, 0).unwrap()) < luma_u8(src.pixel(1, 0).unwrap()) - 20.0);
    }

    #[test]
    fn local_mask_color_range_oklab_and_hsv_selects_target_hue_only() {
        let src = Image {
            width: 3,
            height: 1,
            pixels: vec![0xFF20_60E0, 0xFFE0_3020, 0xFF80_8080], // blue, red, neutral gray
        };
        let mut dev = DevelopRecipe::default();
        dev.masks.push(LocalMask::color_range(
            ColorRangeMask::from_hsv(220.0, 0.85, 0.88, 0.2, 0.25, 0.6, 0.15),
            MaskAdjustments {
                exposure_ev: -1.5,
                ..MaskAdjustments::default()
            },
        ));
        let out = develop_image(&src, &dev).unwrap();
        assert!(luma_u8(out.pixel(0, 0).unwrap()) < luma_u8(src.pixel(0, 0).unwrap()) - 15.0);
        assert_eq!(out.pixel(1, 0), src.pixel(1, 0));
        assert_eq!(out.pixel(2, 0), src.pixel(2, 0));
    }

    #[test]
    fn local_mask_adjustments_sharpness_and_temp_local_only() {
        let mut pixels = vec![0xFF70_7070; 16 * 16];
        // Textured high-frequency stripe on left half and right half
        for y in 0..16 {
            for x in 0..16 {
                let v = if (x + y) % 2 == 0 { 0x60u32 } else { 0x80u32 };
                pixels[y * 16 + x] = 0xFF00_0000 | (v << 16) | (v << 8) | v;
            }
        }
        let src = Image { width: 16, height: 16, pixels };
        let mut dev = DevelopRecipe::default();
        dev.masks.push(LocalMask::linear_gradient(
            LinearGradientMask { x0: 0.0, y0: 0.5, x1: 0.45, y1: 0.5 },
            MaskAdjustments {
                temperature: 0.5,
                sharpness: 0.8,
                ..MaskAdjustments::default()
            },
        ));
        let out = develop_image(&src, &dev).unwrap();
        let (lr, _, lb) = channel_rgb(out.pixel(1, 8).unwrap());
        assert!(lr > lb);
        assert_eq!(out.pixel(14, 8), src.pixel(14, 8));
    }

    #[test]
    fn denoise_bilateral_reduces_flat_noise_and_preserves_sharp_edge() {
        // Left half: dark noisy (~0.12 +/- chroma/luma jitter), Right half: bright noisy (~0.88 +/- jitter)
        let w = 24usize;
        let h = 16usize;
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let jitter = if ((x * 7 + y * 13) % 2) == 0 { 16i32 } else { -16i32 };
                let base = if x < w / 2 { 34i32 } else { 220i32 };
                let r = (base + jitter).clamp(0, 255) as u8;
                let g = (base - jitter).clamp(0, 255) as u8;
                let b = (base + jitter / 2).clamp(0, 255) as u8;
                pixels.push(0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
            }
        }
        let noisy = Image { width: w, height: h, pixels };
        let clean = denoise_bilateral_image(&noisy, &DenoiseParams::high_iso()).expect("denoise");

        // Measure chroma/luma oscillation in flat interior left region (x=2..8)
        let orig_spread = ((channel_rgb(noisy.pixel(4, 4).unwrap()).0 as i32)
            - (channel_rgb(noisy.pixel(4, 4).unwrap()).1 as i32))
        .abs();
        let clean_spread = ((channel_rgb(clean.pixel(4, 4).unwrap()).0 as i32)
            - (channel_rgb(clean.pixel(4, 4).unwrap()).1 as i32))
        .abs();
        assert!(clean_spread < orig_spread, "chroma noise not reduced: {clean_spread} vs {orig_spread}");

        // Sharp step edge across x=11 -> x=12 remains high-contrast!
        let left_edge = luma_u8(clean.pixel(11, 8).unwrap());
        let right_edge = luma_u8(clean.pixel(12, 8).unwrap());
        assert!(right_edge - left_edge > 140.0, "edge blurred too much: {left_edge} vs {right_edge}");
    }

    #[test]
    fn denoise_bilateral_preserves_alpha_and_uniform_image() {
        let uniform = Image {
            width: 8,
            height: 8,
            pixels: vec![0x8040_70A0; 64],
        };
        let out = denoise_bilateral_image(&uniform, &DenoiseParams::high_iso()).unwrap();
        for &px in &out.pixels {
            assert_eq!(px >> 24, 0x80);
            let (r, g, b) = channel_rgb(px);
            assert!((r as i16 - 0x40).abs() <= 1);
            assert!((g as i16 - 0x70).abs() <= 1);
            assert!((b as i16 - 0xA0).abs() <= 1);
        }
    }

    #[test]
    fn heal_region_auto_patch_match_removes_sensor_hotspot() {
        let w = 24usize;
        let h = 24usize;
        let mut img = Image {
            width: w,
            height: h,
            pixels: vec![0xFF40_7055; w * h],
        };
        // Inject a bright magenta sensor defect around (12, 12)
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let x = (12 + dx) as usize;
                let y = (12 + dy) as usize;
                img.pixels[y * w + x] = 0xFFFF_10FF;
            }
        }
        let healed = heal_region_image(&img, 12, 12, 3, None, 0.2,).expect("auto heal");
        let (hr, hg, hb) = channel_rgb(healed.pixel(12, 12).unwrap());
        assert!((hr as i16 - 0x40).abs() <= 3);
        assert!((hg as i16 - 0x70).abs() <= 3);
        assert!((hb as i16 - 0x55).abs() <= 3);
    }

    #[test]
    fn heal_region_explicit_source_with_collar_gain_matching_and_recipe() {
        let w = 24usize;
        let h = 24usize;
        let mut img = Image {
            width: w,
            height: h,
            pixels: vec![0xFF60_6060; w * h],
        };
        // Put dark blemish at center (12, 12)
        img.pixels[12 * w + 12] = 0xFF00_0000;
        let mut dev = DevelopRecipe::default();
        dev.heal_spots.push(
            HealPatch::auto(0.5, 0.5, 0.09).with_source(0.2, 0.2),
        );
        let out = develop_image(&img, &dev).unwrap();
        let (r, g, b) = channel_rgb(out.pixel(12, 12).unwrap());
        assert!(r > 85 && g > 85 && b > 85);
    }

    #[test]
    fn live_scopes_waveform_parade_vectorscope_histogram_and_stats() {
        // Warm skin-tone / portrait test patch
        let skin = Image {
            width: 16,
            height: 16,
            pixels: vec![0xFFC2_9175; 256],
        };
        let stats = analyze_scope_stats(&skin, ScopeStandard::Rec709);
        assert!(stats.mean_ire > 45.0 && stats.mean_ire < 75.0);
        assert_eq!(stats.shadow_clip_pct, 0.0);
        assert_eq!(stats.highlight_clip_pct, 0.0);
        assert_eq!(stats.histogram.total_pixels, 256);

        for kind in [
            ScopeKind::WaveformLuma,
            ScopeKind::WaveformRgb,
            ScopeKind::RgbParade,
            ScopeKind::Vectorscope,
            ScopeKind::Histogram,
        ] {
            let scope_img = render_scope_image(&skin, kind, 120, 90, ScopeStandard::Rec709);
            assert_eq!(scope_img.width, 120);
            assert_eq!(scope_img.height, 90);
            assert!(scope_img.pixels.iter().any(|&p| p != 0xFF0E_1014), "{kind:?} rendered blank");
        }
    }

    #[test]
    fn contact_sheet_renders_keepers_captions_stars_and_exports_png_jpeg() {
        let thumb_a = sample_image(120, 80);
        let thumb_b = Image {
            width: 80,
            height: 120,
            pixels: vec![0xFF33_88CC; 80 * 120],
        };
        let entries = [
            ContactSheetEntry {
                image: &thumb_a,
                filename: "paris.jpg",
                stars: 5,
                flag: CullFlag::Pick,
                color_label: ColorLabel::Green,
            },
            ContactSheetEntry {
                image: &thumb_b,
                filename: "kyoto.dng",
                stars: 4,
                flag: CullFlag::Unflagged,
                color_label: ColorLabel::Blue,
            },
            ContactSheetEntry {
                image: &thumb_a,
                filename: "oslo.raf",
                stars: 3,
                flag: CullFlag::Pick,
                color_label: ColorLabel::None,
            },
        ];

        let mut spec = ContactSheetSpec::dark_slate();
        spec.page_width = 640;
        spec.page_height = 480;
        spec.columns = 2;
        spec.rows = 1;

        let pages = render_contact_sheet_pages(&entries, &spec).expect("pages");
        assert_eq!(pages.len(), 2);
        assert_eq!((pages[0].width, pages[0].height), (640, 480));
        assert!(pages[0].pixels.iter().any(|&p| p != spec.background_argb));

        let png = export_contact_sheet_png(&entries[..2], &spec).expect("png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let jpg = export_contact_sheet_jpeg(&entries[..2], &spec, 85).expect("jpeg");
        assert!(jpg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }
}
