// SPDX-License-Identifier: GPL-3.0-only
//! ISO Base Media File Format (ISOBMFF) and MIAF box parser.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::image::MatrixCoefficients;

/// Representation of an ISOBMFF Box header.
#[derive(Clone, Copy, Debug)]
pub struct BoxHeader {
    pub fourcc: [u8; 4],
    pub offset: usize,
    pub header_size: usize,
    pub total_size: usize,
}

impl BoxHeader {
    #[inline]
    pub fn payload_range(&self) -> (usize, usize) {
        (self.offset + self.header_size, self.offset + self.total_size)
    }

    #[inline]
    pub fn payload<'a>(&self, data: &'a [u8]) -> Result<&'a [u8], Error> {
        let (start, end) = self.payload_range();
        if end > data.len() || start > end {
            return Err(Error::CorruptData);
        }
        Ok(&data[start..end])
    }
}

/// Reads a single box header at `offset` within `data`.
pub fn read_box_header(data: &[u8], offset: usize) -> Result<BoxHeader, Error> {
    read_box_header_bounded(data, offset, data.len())
}

/// Reads a single box header at `offset` within `data`, bounded to `max_end`.
pub fn read_box_header_bounded(data: &[u8], offset: usize, max_end: usize) -> Result<BoxHeader, Error> {
    if offset + 8 > data.len() || offset + 8 > max_end {
        return Err(Error::CorruptData);
    }
    let s32 = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
    let fourcc = [data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]];

    let (header_size, total_size) = if s32 == 1 {
        // 64-bit size follows
        if offset + 16 > data.len() || offset + 16 > max_end {
            return Err(Error::CorruptData);
        }
        let s64 = u64::from_be_bytes([
            data[offset + 8], data[offset + 9], data[offset + 10], data[offset + 11],
            data[offset + 12], data[offset + 13], data[offset + 14], data[offset + 15],
        ]);
        if s64 > usize::MAX as u64 || (s64 as usize) < 16 {
            return Err(Error::CorruptData);
        }
        (16, s64 as usize)
    } else if s32 == 0 {
        // Box extends to container boundary max_end
        let bound = max_end.min(data.len());
        let remaining = bound.saturating_sub(offset);
        (8, remaining)
    } else if s32 < 8 {
        return Err(Error::CorruptData);
    } else {
        (8, s32)
    };

    let box_end = offset.checked_add(total_size).ok_or(Error::CorruptData)?;
    if box_end > data.len() || box_end > max_end {
        return Err(Error::CorruptData);
    }

    Ok(BoxHeader {
        fourcc,
        offset,
        header_size,
        total_size,
    })
}

/// Iterates child boxes inside a byte range [start..end].
pub struct BoxIter<'a> {
    data: &'a [u8],
    curr: usize,
    end: usize,
}

impl<'a> BoxIter<'a> {
    pub fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self {
            data,
            curr: start,
            end: end.min(data.len()),
        }
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = BoxHeader;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr + 8 > self.end {
            return None;
        }
        let header = read_box_header_bounded(self.data, self.curr, self.end).ok()?;
        if header.total_size == 0 {
            self.curr = self.end;
        } else {
            self.curr += header.total_size;
        }
        Some(header)
    }
}

/// Information extracted from an `iloc` item extent.
#[derive(Clone, Copy, Debug)]
pub struct ItemExtent {
    pub offset: usize,
    pub length: usize,
}

/// Item location description from `iloc`.
#[derive(Clone, Debug)]
pub struct ItemLocation {
    pub item_id: u32,
    pub construction_method: u8, // 0 = file offset, 1 = idat offset
    pub base_offset: usize,
    pub extents: Vec<ItemExtent>,
}

/// Item info from `infe`.
#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub item_id: u32,
    pub item_type: [u8; 4],
    pub hidden: bool,
}

/// Visual properties associated with an item.
#[derive(Clone, Debug, Default)]
pub struct ItemProperties {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bit_depth: Option<u8>,
    pub matrix_coefficients: Option<MatrixCoefficients>,
    pub full_range: bool,
    pub av1c_config: Option<Vec<u8>>,
    pub hvcc_config: Option<Vec<u8>>,
    pub is_alpha: bool,
    pub irot_angle: Option<u8>, // 0=0°, 1=90° CCW, 2=180°, 3=270° CCW
    pub imir_axis: Option<u8>,  // 0=vertical, 1=horizontal
    pub clap: Option<super::transforms::CleanAperture>,
}


/// References between items from `iref`.
#[derive(Clone, Debug)]
pub struct ItemReference {
    pub ref_type: [u8; 4], // b"dimg" or b"auxl"
    pub from_item_id: u32,
    pub to_item_ids: Vec<u32>,
}

/// Complete MIAF metadata parsed from the `meta` box.
#[derive(Clone, Debug)]
pub struct MiafMeta {
    pub primary_item_id: u32,
    pub items: Vec<ItemInfo>,
    pub locations: Vec<ItemLocation>,
    pub properties: Vec<ItemProperties>,
    pub item_to_props: Vec<(u32, Vec<usize>)>, // (item_id, prop_indices)
    pub references: Vec<ItemReference>,
    pub idat_offset: Option<usize>,
}

impl MiafMeta {
    /// Parses the entire `meta` box and its children.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut meta_header: Option<BoxHeader> = None;
        for b in BoxIter::new(data, 0, data.len()) {
            if &b.fourcc == b"meta" {
                meta_header = Some(b);
                break;
            }
        }
        let meta = meta_header.ok_or(Error::CorruptData)?;
        let payload = meta.payload(data)?;
        if payload.len() < 4 {
            return Err(Error::CorruptData);
        }
        // meta is a FullBox: skip 1 byte version + 3 bytes flags
        let (meta_start, meta_end) = meta.payload_range();
        let child_start = meta_start + 4;

        let mut primary_item_id = 0u32;
        let mut items = Vec::new();
        let mut locations = Vec::new();
        let mut properties = Vec::new();
        let mut item_to_props = Vec::new();
        let mut references = Vec::new();
        let mut idat_offset = None;

        for child in BoxIter::new(data, child_start, meta_end) {
            match &child.fourcc {
                b"pitm" => {
                    let p = child.payload(data)?;
                    if p.len() >= 6 {
                        let ver = p[0];
                        if ver == 0 && p.len() >= 6 {
                            primary_item_id = u16::from_be_bytes([p[4], p[5]]) as u32;
                        } else if ver == 1 && p.len() >= 8 {
                            primary_item_id = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
                        }
                    }
                }
                b"iinf" => {
                    let p = child.payload(data)?;
                    if p.len() >= 4 {
                        // FullBox: version + flags
                        let (iinf_start, iinf_end) = child.payload_range();
                        let infe_start = iinf_start + if p[0] == 0 { 6 } else { 8 };
                        for infe in BoxIter::new(data, infe_start, iinf_end) {
                            if &infe.fourcc == b"infe" {
                                if let Ok(info) = parse_infe(data, &infe) {
                                    items.push(info);
                                }
                            }
                        }
                    }
                }
                b"iloc" => {
                    if let Ok(locs) = parse_iloc(data, &child) {
                        locations = locs;
                    }
                }
                b"iprp" => {
                    let (ipco_props, associations) = parse_iprp(data, &child)?;
                    properties = ipco_props;
                    item_to_props = associations;
                }
                b"iref" => {
                    if let Ok(refs) = parse_iref(data, &child) {
                        references = refs;
                    }
                }
                b"idat" => {
                    idat_offset = Some(child.offset + child.header_size);
                }
                _ => {}
            }
        }

        // If pitm was missing, default to the first visual item
        if primary_item_id == 0 {
            if let Some(first) = items.iter().find(|it| &it.item_type == b"av01" || &it.item_type == b"hvc1" || &it.item_type == b"grid") {
                primary_item_id = first.item_id;
            }
        }

        Ok(MiafMeta {
            primary_item_id,
            items,
            locations,
            properties,
            item_to_props,
            references,
            idat_offset,
        })
    }

    /// Resolves the continuous or extents byte slice for an item.
    pub fn get_item_bytes<'a>(&self, data: &'a [u8], item_id: u32, out: &mut Vec<u8>) -> Result<(), Error> {
        let loc = self.locations.iter().find(|l| l.item_id == item_id).ok_or(Error::CorruptData)?;
        let base = if loc.construction_method == 1 {
            self.idat_offset
                .ok_or(Error::CorruptData)?
                .checked_add(loc.base_offset)
                .ok_or(Error::CorruptData)?
        } else {
            loc.base_offset
        };

        for ext in &loc.extents {
            let start = base.checked_add(ext.offset).ok_or(Error::CorruptData)?;
            let end = start.checked_add(ext.length).ok_or(Error::CorruptData)?;
            if end > data.len() {
                return Err(Error::CorruptData);
            }
            out.try_reserve_exact(ext.length).map_err(|_| Error::OutOfMemory)?;
            out.extend_from_slice(&data[start..end]);
        }

        Ok(())
    }

    /// Computes consolidated properties for a specific item ID.
    pub fn get_properties_for(&self, item_id: u32) -> ItemProperties {
        let mut merged = ItemProperties::default();
        if let Some((_, indices)) = self.item_to_props.iter().find(|(id, _)| *id == item_id) {
            for &idx in indices {
                if idx > 0 && idx <= self.properties.len() {
                    let p = &self.properties[idx - 1];
                    if p.width.is_some() { merged.width = p.width; }
                    if p.height.is_some() { merged.height = p.height; }
                    if p.bit_depth.is_some() { merged.bit_depth = p.bit_depth; }
                    if p.matrix_coefficients.is_some() { merged.matrix_coefficients = p.matrix_coefficients; }
                    if p.full_range { merged.full_range = true; }
                    if p.av1c_config.is_some() { merged.av1c_config = p.av1c_config.clone(); }
                    if p.hvcc_config.is_some() { merged.hvcc_config = p.hvcc_config.clone(); }
                    if p.is_alpha { merged.is_alpha = true; }
                    if p.irot_angle.is_some() { merged.irot_angle = p.irot_angle; }
                    if p.imir_axis.is_some() { merged.imir_axis = p.imir_axis; }
                    if p.clap.is_some() { merged.clap = p.clap; }
                }
            }
        }
        merged
    }

}

fn parse_infe(data: &[u8], infe: &BoxHeader) -> Result<ItemInfo, Error> {
    let p = infe.payload(data)?;
    if p.len() < 8 {
        return Err(Error::CorruptData);
    }
    let ver = p[0];
    let flags = u32::from_be_bytes([0, p[1], p[2], p[3]]);
    let hidden = (flags & 1) != 0;

    let (item_id, item_type) = if ver == 2 {
        if p.len() < 12 { return Err(Error::CorruptData); }
        let id = u16::from_be_bytes([p[4], p[5]]) as u32;
        let itype = [p[8], p[9], p[10], p[11]];
        (id, itype)
    } else if ver == 3 {
        if p.len() < 14 { return Err(Error::CorruptData); }
        let id = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        let itype = [p[10], p[11], p[12], p[13]];
        (id, itype)
    } else {
        return Err(Error::Unsupported);
    };

    Ok(ItemInfo {
        item_id,
        item_type,
        hidden,
    })
}

fn parse_iloc(data: &[u8], iloc: &BoxHeader) -> Result<Vec<ItemLocation>, Error> {
    let p = iloc.payload(data)?;
    if p.len() < 8 {
        return Err(Error::CorruptData);
    }
    let ver = p[0];
    let offset_size = (p[4] >> 4) as usize;
    let length_size = (p[4] & 0x0f) as usize;
    let base_offset_size = (p[5] >> 4) as usize;
    let index_size = if matches!(ver, 1 | 2) { (p[5] & 0x0f) as usize } else { 0 };

    let mut idx = 6;
    let item_count = if ver < 2 {
        if p.len() < idx + 2 { return Err(Error::CorruptData); }
        let c = u16::from_be_bytes([p[idx], p[idx + 1]]) as usize;
        idx += 2;
        c
    } else {
        if p.len() < idx + 4 { return Err(Error::CorruptData); }
        let c = u32::from_be_bytes([p[idx], p[idx + 1], p[idx + 2], p[idx + 3]]) as usize;
        idx += 4;
        c
    };

    let mut locations = Vec::new();
    locations.try_reserve_exact(item_count).map_err(|_| Error::OutOfMemory)?;

    for _ in 0..item_count {
        let item_id = if ver < 2 {
            if p.len() < idx + 2 { return Err(Error::CorruptData); }
            let id = u16::from_be_bytes([p[idx], p[idx + 1]]) as u32;
            idx += 2;
            id
        } else {
            if p.len() < idx + 4 { return Err(Error::CorruptData); }
            let id = u32::from_be_bytes([p[idx], p[idx + 1], p[idx + 2], p[idx + 3]]);
            idx += 4;
            id
        };

        let mut construction_method = 0u8;
        if matches!(ver, 1 | 2) {
            if p.len() < idx + 2 { return Err(Error::CorruptData); }
            construction_method = p[idx + 1] & 0x0f;
            idx += 2;
        }

        // data_reference_index
        idx += 2;

        let base_offset = read_uint(&p, &mut idx, base_offset_size)?;
        if p.len() < idx + 2 { return Err(Error::CorruptData); }
        let extent_count = u16::from_be_bytes([p[idx], p[idx + 1]]) as usize;
        idx += 2;

        let mut extents = Vec::new();
        extents.try_reserve_exact(extent_count).map_err(|_| Error::OutOfMemory)?;

        for _ in 0..extent_count {
            if matches!(ver, 1 | 2) && index_size > 0 {
                idx += index_size;
            }
            let extent_offset = read_uint(&p, &mut idx, offset_size)?;
            let extent_length = read_uint(&p, &mut idx, length_size)?;
            extents.push(ItemExtent {
                offset: extent_offset,
                length: extent_length,
            });
        }

        locations.push(ItemLocation {
            item_id,
            construction_method,
            base_offset,
            extents,
        });
    }

    Ok(locations)
}

fn read_uint(p: &[u8], idx: &mut usize, bytes: usize) -> Result<usize, Error> {
    if bytes == 0 {
        return Ok(0);
    }
    if *idx + bytes > p.len() {
        return Err(Error::CorruptData);
    }
    let slice = &p[*idx..*idx + bytes];
    *idx += bytes;
    let mut val = 0usize;
    for &b in slice {
        val = (val << 8) | (b as usize);
    }
    Ok(val)
}

fn parse_iprp(data: &[u8], iprp: &BoxHeader) -> Result<(Vec<ItemProperties>, Vec<(u32, Vec<usize>)>), Error> {
    let (iprp_start, iprp_end) = iprp.payload_range();
    let mut properties = Vec::new();
    let mut associations = Vec::new();

    for child in BoxIter::new(data, iprp_start, iprp_end) {
        match &child.fourcc {
            b"ipco" => {
                let (ipco_start, ipco_end) = child.payload_range();
                for prop in BoxIter::new(data, ipco_start, ipco_end) {
                    let mut p_item = ItemProperties::default();
                    let payload = prop.payload(data)?;
                    match &prop.fourcc {
                        b"ispe" => {
                            if payload.len() >= 12 {
                                p_item.width = Some(u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]));
                                p_item.height = Some(u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]));
                            }
                        }
                        b"pixi" => {
                            if payload.len() >= 5 {
                                let channels = payload[4] as usize;
                                if payload.len() >= 5 + channels && channels > 0 {
                                    p_item.bit_depth = Some(payload[5]);
                                }
                            }
                        }
                        b"colr" => {
                            if payload.len() >= 4 && &payload[0..4] == b"nclx" && payload.len() >= 11 {
                                let matrix = u16::from_be_bytes([payload[8], payload[9]]);
                                let full = (payload[10] & 0x80) != 0;
                                p_item.matrix_coefficients = Some(MatrixCoefficients::from(matrix));
                                p_item.full_range = full;
                            }
                        }
                        b"av1C" => {
                            let mut buf = Vec::new();
                            buf.extend_from_slice(payload);
                            p_item.av1c_config = Some(buf);
                        }
                        b"hvcC" => {
                            let mut buf = Vec::new();
                            buf.extend_from_slice(payload);
                            p_item.hvcc_config = Some(buf);
                        }
                        b"auxC" => {
                            // Check if auxiliary type names alpha
                            if payload.windows(5).any(|w| w == b"alpha") {
                                p_item.is_alpha = true;
                            }
                        }
                        b"irot" => {
                            if !payload.is_empty() {
                                p_item.irot_angle = Some(payload[0] & 3);
                            }
                        }
                        b"imir" => {
                            if !payload.is_empty() {
                                p_item.imir_axis = Some(payload[0] & 1);
                            }
                        }
                        b"clap" => {
                            if payload.len() >= 32 {
                                let width_n = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                                let width_d = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                                let height_n = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
                                let height_d = u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]);
                                let horiz_off_n = i32::from_be_bytes([payload[16], payload[17], payload[18], payload[19]]);
                                let horiz_off_d = u32::from_be_bytes([payload[20], payload[21], payload[22], payload[23]]);
                                let vert_off_n = i32::from_be_bytes([payload[24], payload[25], payload[26], payload[27]]);
                                let vert_off_d = u32::from_be_bytes([payload[28], payload[29], payload[30], payload[31]]);
                                p_item.clap = Some(super::transforms::CleanAperture {
                                    width_n,
                                    width_d,
                                    height_n,
                                    height_d,
                                    horiz_off_n,
                                    horiz_off_d,
                                    vert_off_n,
                                    vert_off_d,
                                });
                            }
                        }
                        _ => {}
                    }
                    properties.push(p_item);
                }
            }
            b"ipma" => {
                let p = child.payload(data)?;
                if p.len() >= 8 {
                    let ver = p[0];
                    let flags = u32::from_be_bytes([0, p[1], p[2], p[3]]);
                    let entry_count = u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize;
                    let mut idx = 8;
                    for _ in 0..entry_count {
                        if idx >= p.len() { break; }
                        let item_id = if ver < 1 {
                            if idx + 2 > p.len() { break; }
                            let id = u16::from_be_bytes([p[idx], p[idx + 1]]) as u32;
                            idx += 2;
                            id
                        } else {
                            if idx + 4 > p.len() { break; }
                            let id = u32::from_be_bytes([p[idx], p[idx + 1], p[idx + 2], p[idx + 3]]);
                            idx += 4;
                            id
                        };
                        if idx >= p.len() { break; }
                        let assoc_count = p[idx] as usize;
                        idx += 1;
                        let mut prop_indices = Vec::new();
                        for _ in 0..assoc_count {
                            if (flags & 1) != 0 {
                                if idx + 2 > p.len() { break; }
                                let val = u16::from_be_bytes([p[idx], p[idx + 1]]);
                                idx += 2;
                                let prop_idx = (val & 0x7fff) as usize;
                                prop_indices.push(prop_idx);
                            } else {
                                if idx >= p.len() { break; }
                                let val = p[idx];
                                idx += 1;
                                let prop_idx = (val & 0x7f) as usize;
                                prop_indices.push(prop_idx);
                            }
                        }
                        associations.push((item_id, prop_indices));
                    }
                }
            }
            _ => {}
        }
    }

    Ok((properties, associations))
}

fn parse_iref(data: &[u8], iref: &BoxHeader) -> Result<Vec<ItemReference>, Error> {
    let p = iref.payload(data)?;
    if p.len() < 4 { return Err(Error::CorruptData); }
    let ver = p[0];
    let (iref_start, iref_end) = iref.payload_range();
    let child_start = iref_start + 4;
    let mut refs = Vec::new();

    for child in BoxIter::new(data, child_start, iref_end) {
        let payload = child.payload(data)?;
        let mut idx;
        let (from_id, to_count) = if ver == 0 {
            if payload.len() < 4 { continue; }
            let from = u16::from_be_bytes([payload[0], payload[1]]) as u32;
            let count = u16::from_be_bytes([payload[2], payload[3]]) as usize;
            idx = 4;
            (from, count)
        } else {
            if payload.len() < 6 { continue; }
            let from = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let count = u16::from_be_bytes([payload[4], payload[5]]) as usize;
            idx = 6;
            (from, count)
        };

        let mut to_ids = Vec::new();
        for _ in 0..to_count {
            if ver == 0 {
                if idx + 2 > payload.len() { break; }
                to_ids.push(u16::from_be_bytes([payload[idx], payload[idx + 1]]) as u32);
                idx += 2;
            } else {
                if idx + 4 > payload.len() { break; }
                to_ids.push(u32::from_be_bytes([payload[idx], payload[idx + 1], payload[idx + 2], payload[idx + 3]]));
                idx += 4;
            }
        }

        refs.push(ItemReference {
            ref_type: child.fourcc,
            from_item_id: from_id,
            to_item_ids: to_ids,
        });
    }

    Ok(refs)
}
