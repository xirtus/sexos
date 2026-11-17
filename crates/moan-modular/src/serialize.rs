// SPDX-License-Identifier: GPL-3.0-only
//! Binary serialization for EuropaRack patches and sub-patch selections.

extern crate alloc;
use alloc::vec::Vec;

use crate::graph::{Cable, Error, ModuleNode, PatchGraph};
use crate::modules::{create_module, ModuleKind};
use crate::signal::{CableColor, RackTheme};

const MAGIC_PATCH: &[u8; 8] = b"EURORACK";
const MAGIC_SELECTION: &[u8; 8] = b"EUROSELC";
const FORMAT_VERSION: u16 = 1;

pub fn encode_patch(graph: &PatchGraph) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    out.try_reserve(1024).map_err(|_| Error::Memory)?;

    out.extend_from_slice(MAGIC_PATCH);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.push(graph.theme.dark_room_brightness);

    // Nodes
    let node_count = graph.nodes.len() as u16;
    out.extend_from_slice(&node_count.to_le_bytes());

    for node in &graph.nodes {
        encode_node(node, &mut out)?;
    }

    // Cables
    let cable_count = graph.cables.len() as u16;
    out.extend_from_slice(&cable_count.to_le_bytes());

    for cable in &graph.cables {
        encode_cable(cable, &mut out)?;
    }

    Ok(out)
}

pub fn decode_patch(bytes: &[u8]) -> Result<PatchGraph, Error> {
    if bytes.len() < 11 {
        return Err(Error::CorruptData);
    }
    if &bytes[0..8] != MAGIC_PATCH {
        return Err(Error::CorruptData);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FORMAT_VERSION {
        return Err(Error::CorruptData);
    }
    let brightness = bytes[10];

    let mut cursor = 11;
    if bytes.len() < cursor + 2 {
        return Err(Error::CorruptData);
    }
    let node_count = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
    cursor += 2;

    let mut graph = PatchGraph::new();
    graph.theme = RackTheme {
        dark_room_brightness: brightness,
    };

    let mut max_node_id = 0u32;

    for _ in 0..node_count {
        let (node, new_cursor) = decode_node(bytes, cursor)?;
        if node.id > max_node_id {
            max_node_id = node.id;
        }
        cursor = new_cursor;
        graph.nodes.push(node);
    }
    graph.next_node_id = max_node_id + 1;

    if bytes.len() < cursor + 2 {
        return Err(Error::CorruptData);
    }
    let cable_count = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
    cursor += 2;

    let mut max_cable_id = 0u32;
    for _ in 0..cable_count {
        let (cable, new_cursor) = decode_cable(bytes, cursor)?;
        if cable.id > max_cable_id {
            max_cable_id = cable.id;
        }
        cursor = new_cursor;
        graph.cables.push(cable);
    }
    graph.next_cable_id = max_cable_id + 1;

    graph.compile()?;
    Ok(graph)
}

pub fn export_selection(graph: &PatchGraph, selection: &[u32]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    out.try_reserve(512).map_err(|_| Error::Memory)?;

    out.extend_from_slice(MAGIC_SELECTION);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());

    let mut sel_nodes = Vec::new();
    for &id in selection {
        if let Some(node) = graph.find_node(id) {
            sel_nodes.push(node);
        }
    }

    let node_count = sel_nodes.len() as u16;
    out.extend_from_slice(&node_count.to_le_bytes());

    for node in sel_nodes {
        encode_node(node, &mut out)?;
    }

    // Only cables that connect between selected nodes
    let mut sel_cables = Vec::new();
    for cable in &graph.cables {
        let from_in = selection.contains(&cable.from_node);
        let to_in = selection.contains(&cable.to_node);
        if from_in && to_in {
            sel_cables.push(cable);
        }
    }

    let cable_count = sel_cables.len() as u16;
    out.extend_from_slice(&cable_count.to_le_bytes());

    for cable in sel_cables {
        encode_cable(cable, &mut out)?;
    }

    Ok(out)
}

pub fn import_selection(graph: &mut PatchGraph, bytes: &[u8]) -> Result<Vec<u32>, Error> {
    if bytes.len() < 10 {
        return Err(Error::CorruptData);
    }
    if &bytes[0..8] != MAGIC_SELECTION {
        return Err(Error::CorruptData);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FORMAT_VERSION {
        return Err(Error::CorruptData);
    }

    let mut cursor = 10;
    if bytes.len() < cursor + 2 {
        return Err(Error::CorruptData);
    }
    let node_count = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
    cursor += 2;

    // Mapping from old NodeId in byte stream -> new NodeId in target graph
    let mut id_map = Vec::new();
    let mut new_node_ids = Vec::new();

    for _ in 0..node_count {
        let (mut node, new_cursor) = decode_node(bytes, cursor)?;
        cursor = new_cursor;

        let old_id = node.id;
        let new_id = graph.next_node_id;
        graph.next_node_id += 1;

        node.id = new_id;
        id_map.push((old_id, new_id));
        new_node_ids.push(new_id);
        graph.nodes.push(node);
    }

    if bytes.len() < cursor + 2 {
        return Err(Error::CorruptData);
    }
    let cable_count = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
    cursor += 2;

    for _ in 0..cable_count {
        let (mut cable, new_cursor) = decode_cable(bytes, cursor)?;
        cursor = new_cursor;

        let new_cable_id = graph.next_cable_id;
        graph.next_cable_id += 1;
        cable.id = new_cable_id;

        // Remap from_node and to_node
        let mut mapped_from = None;
        let mut mapped_to = None;
        for &(old_id, new_id) in &id_map {
            if cable.from_node == old_id {
                mapped_from = Some(new_id);
            }
            if cable.to_node == old_id {
                mapped_to = Some(new_id);
            }
        }

        if let (Some(new_from), Some(new_to)) = (mapped_from, mapped_to) {
            cable.from_node = new_from;
            cable.to_node = new_to;
            graph.cables.push(cable);
        }
    }

    graph.compile()?;
    Ok(new_node_ids)
}

fn encode_node(node: &ModuleNode, out: &mut Vec<u8>) -> Result<(), Error> {
    out.extend_from_slice(&node.id.to_le_bytes());
    out.push(node.kind as u8);
    out.extend_from_slice(&node.x.to_le_bytes());
    out.extend_from_slice(&node.y.to_le_bytes());

    let param_count = node.module.param_count() as u16;
    out.extend_from_slice(&param_count.to_le_bytes());
    for p in 0..node.module.param_count() {
        let val = node.module.get_param(p);
        out.extend_from_slice(&val.to_le_bytes());
    }

    // Module-specific extras
    if node.kind == ModuleKind::BidulePluginHost {
        let host = match node.module.kind() {
            ModuleKind::BidulePluginHost => true,
            _ => false,
        };
        if host {
            // Encode dummy byte for compatibility
            out.push(0);
        }
    }

    Ok(())
}

fn decode_node(bytes: &[u8], mut cursor: usize) -> Result<(ModuleNode, usize), Error> {
    if bytes.len() < cursor + 15 {
        return Err(Error::CorruptData);
    }
    let id = u32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    let kind_byte = bytes[cursor];
    cursor += 1;
    let kind = match kind_byte {
        0 => ModuleKind::TrackIn,
        1 => ModuleKind::MasterOut,
        2 => ModuleKind::PolyMidiToCv,
        3 => ModuleKind::AnalogVco,
        4 => ModuleKind::Fm4OpMacro,
        5 => ModuleKind::ArielFormantCv,
        6 => ModuleKind::LadderVcf,
        7 => ModuleKind::StateVariableVcf,
        8 => ModuleKind::PolyVca,
        9 => ModuleKind::Wavefolder,
        10 => ModuleKind::PolyAdsr,
        11 => ModuleKind::TempoLfo,
        12 => ModuleKind::ClockDivider,
        13 => ModuleKind::EuclideanSeq,
        14 => ModuleKind::TuringQuantizer,
        15 => ModuleKind::SampleAndHoldSlew,
        16 => ModuleKind::MatrixMixer,
        17 => ModuleKind::StereoDelay,
        18 => ModuleKind::BidulePluginHost,
        _ => return Err(Error::CorruptData),
    };

    let x = f32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;
    let y = f32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    if bytes.len() < cursor + 2 {
        return Err(Error::CorruptData);
    }
    let param_count = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
    cursor += 2;

    let mut module = create_module(kind);

    for p in 0..param_count {
        if bytes.len() < cursor + 4 {
            return Err(Error::CorruptData);
        }
        let val = f32::from_le_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]);
        cursor += 4;
        module.set_param(p, val);
    }

    if kind == ModuleKind::BidulePluginHost {
        if bytes.len() < cursor + 1 {
            return Err(Error::CorruptData);
        }
        cursor += 1;
    }

    Ok((
        ModuleNode {
            id,
            kind,
            module,
            x,
            y,
        },
        cursor,
    ))
}

fn encode_cable(cable: &Cable, out: &mut Vec<u8>) -> Result<(), Error> {
    out.extend_from_slice(&cable.id.to_le_bytes());
    out.extend_from_slice(&cable.from_node.to_le_bytes());
    out.push(cable.from_port);
    out.extend_from_slice(&cable.to_node.to_le_bytes());
    out.push(cable.to_port);
    out.push(cable.color as u8);
    out.extend_from_slice(&cable.attenuation.to_le_bytes());
    Ok(())
}

fn decode_cable(bytes: &[u8], mut cursor: usize) -> Result<(Cable, usize), Error> {
    if bytes.len() < cursor + 16 {
        return Err(Error::CorruptData);
    }
    let id = u32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    let from_node = u32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    let from_port = bytes[cursor];
    cursor += 1;

    let to_node = u32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    let to_port = bytes[cursor];
    cursor += 1;

    let color_byte = bytes[cursor];
    cursor += 1;
    let color = match color_byte {
        0 => CableColor::CrimsonAudio,
        1 => CableColor::GoldPitchCv,
        2 => CableColor::CyanModCv,
        3 => CableColor::EmeraldGate,
        4 => CableColor::VioletMidi,
        _ => CableColor::CrimsonAudio,
    };

    let attenuation = f32::from_le_bytes([
        bytes[cursor],
        bytes[cursor + 1],
        bytes[cursor + 2],
        bytes[cursor + 3],
    ]);
    cursor += 4;

    Ok((
        Cable {
            id,
            from_node,
            from_port,
            to_node,
            to_port,
            color,
            attenuation,
        },
        cursor,
    ))
}
