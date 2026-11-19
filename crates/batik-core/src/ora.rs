// SPDX-License-Identifier: GPL-3.0-only
//! OpenRaster (.ora) image reader and writer.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_archive::{no_progress, Archive, ArchiveBuilder, OutputFormat, VecReader};
use sex_image::Limits;

use crate::admit_limits;
use crate::blend::BlendMode;
use crate::document::{BackgroundMode, Document};
use crate::error::BatikError;
use crate::layer::{Layer, LayerKind, RasterLayer};
use crate::png::encode_rgba_png;

struct XmlAttrParser<'a> {
    s: &'a str,
}

impl<'a> XmlAttrParser<'a> {
    fn new(s: &'a str) -> Self {
        Self { s: s.trim() }
    }

    fn next_attr(&mut self) -> Option<(&'a str, &'a str)> {
        while !self.s.is_empty() {
            let eq_pos = self.s.find('=')?;
            let key = self.s.get(..eq_pos)?.trim();
            let mut rest = self.s.get(eq_pos + 1..)?.trim_start();
            if rest.is_empty() {
                return None;
            }

            let quote = rest.as_bytes()[0];
            if quote != b'"' && quote != b'\'' {
                self.s = rest;
                continue;
            }
            rest = rest.get(1..)?;

            let end_quote = rest.find(quote as char)?;
            let val = rest.get(..end_quote)?;
            self.s = rest.get(end_quote + 1..).unwrap_or("");

            return Some((key, val));
        }
        None
    }
}

struct ParsedLayer {
    name: String,
    src: String,
    x: i32,
    y: i32,
    opacity: u8,
    visible: bool,
    blend_mode: BlendMode,
}

/// Reads an OpenRaster (.ora) archive and constructs a Batik `Document`.
pub fn read_ora(data: &[u8], limits: &Limits) -> Result<Document, BatikError> {
    let archive = Archive::open(data).map_err(BatikError::Archive)?;

    // Find stack.xml
    let stack_idx = archive.find("stack.xml").ok_or(BatikError::CorruptOra("missing stack.xml"))?;
    let stack_entry = &archive.entries()[stack_idx];
    if !stack_entry.safe || stack_entry.size > 2 * 1024 * 1024 {
        return Err(BatikError::TooLarge);
    }

    let mut stack_bytes = Vec::new();
    archive.extract(stack_idx, &mut stack_bytes, &mut no_progress).map_err(BatikError::Archive)?;

    let xml_str = core::str::from_utf8(&stack_bytes).map_err(|_| BatikError::CorruptOra("invalid UTF-8 in stack.xml"))?;

    // Parse root <image> element
    let image_tag_start = xml_str.find("<image").ok_or(BatikError::CorruptOra("missing <image> tag"))?;
    let image_rem = &xml_str[image_tag_start..];
    if image_rem.len() > 6 {
        let after = image_rem.as_bytes()[6];
        if after != b' ' && after != b'\t' && after != b'\n' && after != b'\r' && after != b'>' && after != b'/' {
            return Err(BatikError::CorruptOra("malformed <image> tag"));
        }
    }
    let image_tag_end = image_rem.find('>').ok_or(BatikError::CorruptOra("malformed <image> tag"))?;
    let image_tag_content = image_rem.get(6..image_tag_end).ok_or(BatikError::CorruptOra("malformed <image> tag"))?;

    let mut doc_w: Option<usize> = None;
    let mut doc_h: Option<usize> = None;

    let mut parser = XmlAttrParser::new(image_tag_content);
    while let Some((k, v)) = parser.next_attr() {
        if k == "w" {
            doc_w = v.parse::<usize>().ok();
        } else if k == "h" {
            doc_h = v.parse::<usize>().ok();
        }
    }

    let width = doc_w.ok_or(BatikError::CorruptOra("missing canvas width"))?;
    let height = doc_h.ok_or(BatikError::CorruptOra("missing canvas height"))?;
    admit_limits(limits, width, height)?;

    // Parse <layer> tags
    let mut parsed_layers = Vec::new();
    let mut search_pos = 0;

    while search_pos < xml_str.len() {
        let rem = match xml_str.get(search_pos..) {
            Some(r) => r,
            None => break,
        };
        let layer_rel_start = match rem.find("<layer") {
            Some(s) => s,
            None => break,
        };
        let after_start = &rem[layer_rel_start..];
        if after_start.len() > 6 {
            let next_b = after_start.as_bytes()[6];
            if next_b != b' ' && next_b != b'\t' && next_b != b'\n' && next_b != b'\r' && next_b != b'>' && next_b != b'/' {
                search_pos += layer_rel_start + 6;
                while search_pos < xml_str.len() && !xml_str.is_char_boundary(search_pos) {
                    search_pos += 1;
                }
                continue;
            }
        }
        let tag_rel_end = match after_start.find('>') {
            Some(e) => e,
            None => break,
        };
        let tag_body = match after_start.get(6..tag_rel_end) {
            Some(b) => b,
            None => break,
        };

        search_pos = search_pos + layer_rel_start + tag_rel_end + 1;
        while search_pos < xml_str.len() && !xml_str.is_char_boundary(search_pos) {
            search_pos += 1;
        }

        let mut name = String::from("Layer");
        let mut src = String::new();
        let mut x = 0i32;
        let mut y = 0i32;
        let mut opacity = 255u8;
        let mut visible = true;
        let mut blend_mode = BlendMode::Normal;

        let mut layer_parser = XmlAttrParser::new(tag_body);
        while let Some((k, v)) = layer_parser.next_attr() {
            match k {
                "name" => name = String::from(v),
                "src" => src = String::from(v),
                "x" => x = v.parse::<i32>().unwrap_or(0),
                "y" => y = v.parse::<i32>().unwrap_or(0),
                "opacity" => {
                    if let Ok(op_f) = v.parse::<f32>() {
                        opacity = libm::roundf(op_f.clamp(0.0, 1.0) * 255.0) as u8;
                    }
                }
                "visibility" => {
                    if v == "hidden" {
                        visible = false;
                    }
                }
                "composite-op" => {
                    if let Some(bm) = BlendMode::from_ora_op(v) {
                        blend_mode = bm;
                    }
                }
                _ => {}
            }
        }

        if !src.is_empty() {
            if parsed_layers.len() >= 512 {
                return Err(BatikError::TooLarge);
            }
            parsed_layers.push(ParsedLayer {
                name,
                src,
                x,
                y,
                opacity,
                visible,
                blend_mode,
            });
        }
    }

    let mut doc = Document::new(width, height, BackgroundMode::Transparent, *limits)?;
    let mut accumulated_pixels: usize = 0;

    // Extract each layer PNG from the archive
    for (idx, pl) in parsed_layers.into_iter().enumerate() {
        if pl.src.contains("..") || pl.src.starts_with('/') || pl.src.starts_with('\\') {
            return Err(BatikError::CorruptOra("path traversal in layer src"));
        }

        let entry_idx = match archive.find(&pl.src) {
            Some(i) => i,
            None => continue,
        };

        let entry = &archive.entries()[entry_idx];
        if !entry.safe || entry.size > (limits.max_input_bytes as u64) {
            return Err(BatikError::TooLarge);
        }

        let mut png_bytes = Vec::new();
        archive.extract(entry_idx, &mut png_bytes, &mut no_progress).map_err(BatikError::Archive)?;

        let decoded = sex_image::decode(&png_bytes, limits)?;
        let layer_pixels = decoded.width.checked_mul(decoded.height).ok_or(BatikError::TooLarge)?;
        accumulated_pixels = accumulated_pixels.checked_add(layer_pixels).ok_or(BatikError::TooLarge)?;
        if accumulated_pixels > limits.max_pixels {
            return Err(BatikError::TooLarge);
        }

        let raster = RasterLayer {
            width: decoded.width,
            height: decoded.height,
            offset_x: pl.x,
            offset_y: pl.y,
            pixels: decoded.pixels,
            mask: None,
            alpha_lock: false,
            pixel_lock: false,
        };

        let layer = Layer {
            id: (idx + 1) as u64,
            name: pl.name,
            visible: pl.visible,
            opacity: pl.opacity,
            blend_mode: pl.blend_mode,
            kind: LayerKind::Raster(raster),
            filter_stack: Vec::new(),
        };

        doc.add_layer(layer);
    }

    // Fallback if no layers were decoded: check for mergedimage.png
    if doc.layers.is_empty() {
        if let Some(merged_idx) = archive.find("mergedimage.png") {
            let mut png_bytes = Vec::new();
            if archive.extract(merged_idx, &mut png_bytes, &mut no_progress).is_ok() {
                if let Ok(decoded) = sex_image::decode(&png_bytes, limits) {
                    let raster = RasterLayer {
                        width: decoded.width,
                        height: decoded.height,
                        offset_x: 0,
                        offset_y: 0,
                        pixels: decoded.pixels,
                        mask: None,
                        alpha_lock: false,
                        pixel_lock: false,
                    };
                    doc.add_layer(Layer {
                        id: 1,
                        name: String::from("Merged Image"),
                        visible: true,
                        opacity: 255,
                        blend_mode: BlendMode::Normal,
                        kind: LayerKind::Raster(raster),
                        filter_stack: Vec::new(),
                    });
                }
            }
        }
    }

    Ok(doc)
}

/// Serializes a Batik `Document` to OpenRaster (.ora) archive bytes.
pub fn write_ora(doc: &Document) -> Result<Vec<u8>, BatikError> {
    let mut builder = ArchiveBuilder::new(OutputFormat::Zip);
    builder.deflate_level(6);

    // 1. mimetype file: image/openraster
    let mimetype = b"image/openraster".to_vec();
    builder.add_file("mimetype", mimetype.len() as u64, 0, VecReader::new(mimetype))?;

    // 2. Encode raster layers and collect stack.xml tags
    let mut layer_xmls = Vec::new();
    let mut layer_pngs: Vec<(String, Vec<u8>)> = Vec::new();

    let mut layer_count = 0;
    let exported_layers = doc.exportable_raster_layers();
    for (layer, raster) in &exported_layers {
        let png_data = encode_rgba_png(&raster.pixels, raster.width, raster.height)?;
        let src = format!("data/layer{layer_count}.png");

        let op_f = (layer.opacity as f32) / 255.0;
        let vis_str = if layer.visible { "visible" } else { "hidden" };
        let comp_op = layer.blend_mode.to_ora_op();

        let tag = format!(
            "    <layer name=\"{}\" src=\"{}\" x=\"{}\" y=\"{}\" opacity=\"{:.3}\" visibility=\"{}\" composite-op=\"{}\"/>\n",
            layer.name, src, raster.offset_x, raster.offset_y, op_f, vis_str, comp_op
        );
        layer_xmls.push(tag);
        layer_pngs.push((src, png_data));
        layer_count += 1;
    }

    // 3. Construct stack.xml
    let mut stack_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.1\" w=\"{}\" h=\"{}\">\n  <stack>\n",
        doc.width, doc.height
    );
    for tag in layer_xmls {
        stack_xml.push_str(&tag);
    }
    stack_xml.push_str("  </stack>\n</image>\n");

    let stack_bytes = stack_xml.into_bytes();
    builder.add_file("stack.xml", stack_bytes.len() as u64, 0, VecReader::new(stack_bytes))?;

    // 4. Add layer PNG files
    for (src, data) in layer_pngs {
        let len = data.len() as u64;
        builder.add_file(&src, len, 0, VecReader::new(data))?;
    }

    // 5. Add flattened composite as mergedimage.png
    let flattened = doc.flatten()?;
    let merged_png = encode_rgba_png(&flattened.pixels, flattened.width, flattened.height)?;
    let merged_len = merged_png.len() as u64;
    builder.add_file("mergedimage.png", merged_len, 0, VecReader::new(merged_png))?;

    // 6. Write ZIP archive into bytes
    let mut out_zip = Vec::new();
    builder.write(&mut out_zip, &mut no_progress)?;

    Ok(out_zip)
}
