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

pub mod edits;
pub mod gazetteer;
pub mod library;
pub mod metadata;
pub mod search;

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

/// Compile-time and runtime marker confirming Gaze performs zero facial biometrics.
pub const FACES_FREE_POLICY: bool = true;

#[cfg(test)]
mod tests {
    use super::*;
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
}
