// SPDX-License-Identifier: GPL-3.0-only
//! Feedback-safe multigraph patchbay and scheduler for EuropaRack.

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::modules::{create_module, Module, ModuleKind};
use crate::serialize;
use crate::signal::{CableColor, RackTheme, SignalBuffer, BLOCK_FRAMES};

pub type NodeId = u32;
pub type PortId = u8;
pub type CableId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    InvalidNode,
    InvalidPort,
    InvalidCable,
    CorruptData,
    CompilationFailed,
    Range,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cable {
    pub id: CableId,
    pub from_node: NodeId,
    pub from_port: PortId,
    pub to_node: NodeId,
    pub to_port: PortId,
    pub color: CableColor,
    pub attenuation: f32,
}

pub struct ModuleNode {
    pub id: NodeId,
    pub kind: ModuleKind,
    pub module: Box<dyn Module>,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug)]
struct ForwardCable {
    to_node_idx: usize,
    to_port: PortId,
    from_port: PortId,
    attenuation: f32,
}

#[derive(Clone, Debug)]
struct FeedbackDelay {
    to_node_idx: usize,
    to_port: PortId,
    from_node_idx: usize,
    from_port: PortId,
    attenuation: f32,
    delayed_buffer: SignalBuffer,
}

pub struct PatchGraph {
    pub nodes: Vec<ModuleNode>,
    pub cables: Vec<Cable>,
    pub theme: RackTheme,
    pub next_node_id: u32,
    pub next_cable_id: u32,
    needs_recompile: bool,
    pub external_audio: [[f32; 2]; BLOCK_FRAMES],
    pub external_midi: crate::signal::MidiBuffer,

    // Pre-allocated execution state
    execution_order: Vec<usize>,
    forward_cables: Vec<Vec<ForwardCable>>,
    feedback_delays: Vec<FeedbackDelay>,
    node_inputs: Vec<Vec<SignalBuffer>>,
    node_outputs: Vec<Vec<SignalBuffer>>,
}

impl Default for PatchGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl PatchGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            cables: Vec::new(),
            theme: RackTheme::default(),
            next_node_id: 1,
            next_cable_id: 1,
            needs_recompile: true,
            external_audio: [[0.0; 2]; BLOCK_FRAMES],
            external_midi: crate::signal::MidiBuffer::new(),
            execution_order: Vec::new(),
            forward_cables: Vec::new(),
            feedback_delays: Vec::new(),
            node_inputs: Vec::new(),
            node_outputs: Vec::new(),
        }
    }

    pub fn push_midi_event(&mut self, event: crate::signal::ModularMidiEvent) {
        self.external_midi.push(event);
    }

    pub fn set_external_audio(&mut self, audio: &[[f32; 2]; BLOCK_FRAMES]) {
        self.external_audio = *audio;
    }

    pub fn add_module(&mut self, kind: ModuleKind) -> Result<NodeId, Error> {
        let id = self.next_node_id;
        self.next_node_id += 1;
        let module = create_module(kind);

        self.nodes.try_reserve(1).map_err(|_| Error::Memory)?;
        self.nodes.push(ModuleNode {
            id,
            kind,
            module,
            x: 0.0,
            y: 0.0,
        });

        self.needs_recompile = true;
        Ok(id)
    }

    pub fn remove_module(&mut self, id: NodeId) -> Result<(), Error> {
        let idx = self.nodes.iter().position(|n| n.id == id).ok_or(Error::InvalidNode)?;
        self.nodes.remove(idx);
        self.cables.retain(|c| c.from_node != id && c.to_node != id);
        self.needs_recompile = true;
        Ok(())
    }

    pub fn connect(
        &mut self,
        from_node: NodeId,
        from_port: PortId,
        to_node: NodeId,
        to_port: PortId,
        color: Option<CableColor>,
        attenuation: Option<f32>,
    ) -> Result<CableId, Error> {
        let from = self.find_node(from_node).ok_or(Error::InvalidNode)?;
        let to = self.find_node(to_node).ok_or(Error::InvalidNode)?;

        if (from_port as usize) >= from.module.outputs().len() {
            return Err(Error::InvalidPort);
        }
        if (to_port as usize) >= to.module.inputs().len() {
            return Err(Error::InvalidPort);
        }

        let default_color = from.module.outputs()[from_port as usize].kind.default_color();
        let cable_color = color.unwrap_or(default_color);
        let cable_atten = attenuation.unwrap_or(1.0);

        let id = self.next_cable_id;
        self.next_cable_id += 1;

        self.cables.try_reserve(1).map_err(|_| Error::Memory)?;
        self.cables.push(Cable {
            id,
            from_node,
            from_port,
            to_node,
            to_port,
            color: cable_color,
            attenuation: cable_atten,
        });

        self.needs_recompile = true;
        Ok(id)
    }

    pub fn disconnect(&mut self, cable_id: CableId) -> Result<(), Error> {
        let idx = self.cables.iter().position(|c| c.id == cable_id).ok_or(Error::InvalidCable)?;
        self.cables.remove(idx);
        self.needs_recompile = true;
        Ok(())
    }

    pub fn find_node(&self, id: NodeId) -> Option<&ModuleNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn find_node_mut(&mut self, id: NodeId) -> Option<&mut ModuleNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn reset(&mut self) {
        for node in &mut self.nodes {
            node.module.reset();
        }
        for delay in &mut self.feedback_delays {
            delay.delayed_buffer.clear();
        }
    }

    /// Compile graph: topological sort + feedback cycle classification + pre-allocations.
    pub fn compile(&mut self) -> Result<(), Error> {
        let n = self.nodes.len();
        let mut id_to_idx = Vec::new();
        id_to_idx.try_reserve(n).map_err(|_| Error::Memory)?;
        for (idx, node) in self.nodes.iter().enumerate() {
            id_to_idx.push((node.id, idx));
        }

        let lookup = |id: NodeId| -> Option<usize> {
            for &(nid, idx) in &id_to_idx {
                if nid == id {
                    return Some(idx);
                }
            }
            None
        };

        // Build adjacency for cycle detection
        let mut adj = Vec::new();
        adj.try_reserve(n).map_err(|_| Error::Memory)?;
        for _ in 0..n {
            adj.push(Vec::new());
        }

        for (cable_idx, cable) in self.cables.iter().enumerate() {
            if let (Some(u), Some(v)) = (lookup(cable.from_node), lookup(cable.to_node)) {
                adj[u].push((cable_idx, v));
            }
        }

        // Cycle detection via 3-color DFS
        let mut state = Vec::new();
        state.try_reserve(n).map_err(|_| Error::Memory)?;
        state.resize(n, 0u8); // 0 = unvisited, 1 = visiting, 2 = visited

        let mut is_feedback = Vec::new();
        is_feedback.try_reserve(self.cables.len()).map_err(|_| Error::Memory)?;
        is_feedback.resize(self.cables.len(), false);

        fn dfs(
            u: usize,
            adj: &[Vec<(usize, usize)>],
            state: &mut [u8],
            is_feedback: &mut [bool],
        ) {
            state[u] = 1;
            for &(c_idx, v) in &adj[u] {
                if v == u {
                    // Direct self-patch
                    is_feedback[c_idx] = true;
                } else if state[v] == 1 {
                    // Back-edge closing a cycle
                    is_feedback[c_idx] = true;
                } else if state[v] == 0 {
                    dfs(v, adj, state, is_feedback);
                }
            }
            state[u] = 2;
        }

        for i in 0..n {
            if state[i] == 0 {
                dfs(i, &adj, &mut state, &mut is_feedback);
            }
        }

        // Topological Sort of DAG edges using Kahn's algorithm
        let mut in_degrees = Vec::new();
        in_degrees.try_reserve(n).map_err(|_| Error::Memory)?;
        in_degrees.resize(n, 0usize);

        for (c_idx, cable) in self.cables.iter().enumerate() {
            if !is_feedback[c_idx] {
                if let Some(v) = lookup(cable.to_node) {
                    in_degrees[v] += 1;
                }
            }
        }

        let mut queue = Vec::new();
        queue.try_reserve(n).map_err(|_| Error::Memory)?;
        for i in 0..n {
            if in_degrees[i] == 0 {
                queue.push(i);
            }
        }

        let mut order = Vec::new();
        order.try_reserve(n).map_err(|_| Error::Memory)?;

        let mut head = 0;
        while head < queue.len() {
            let u = queue[head];
            head += 1;
            order.push(u);

            for &(c_idx, v) in &adj[u] {
                if !is_feedback[c_idx] {
                    in_degrees[v] -= 1;
                    if in_degrees[v] == 0 {
                        queue.push(v);
                    }
                }
            }
        }

        // If any nodes were not included, append them
        for i in 0..n {
            if !order.contains(&i) {
                order.push(i);
            }
        }

        // Pre-allocate buffers and cable structures
        let mut forward_cables = Vec::new();
        forward_cables.try_reserve(n).map_err(|_| Error::Memory)?;
        for _ in 0..n {
            forward_cables.push(Vec::new());
        }

        let mut feedback_delays = Vec::new();

        for (c_idx, cable) in self.cables.iter().enumerate() {
            let u = lookup(cable.from_node).ok_or(Error::InvalidNode)?;
            let v = lookup(cable.to_node).ok_or(Error::InvalidNode)?;

            if is_feedback[c_idx] {
                let from_port_kind = self.nodes[u].module.outputs()[cable.from_port as usize].kind;
                feedback_delays.push(FeedbackDelay {
                    to_node_idx: v,
                    to_port: cable.to_port,
                    from_node_idx: u,
                    from_port: cable.from_port,
                    attenuation: cable.attenuation,
                    delayed_buffer: SignalBuffer::new(from_port_kind),
                });
            } else {
                forward_cables[u].push(ForwardCable {
                    to_node_idx: v,
                    to_port: cable.to_port,
                    from_port: cable.from_port,
                    attenuation: cable.attenuation,
                });
            }
        }

        let mut node_inputs = Vec::new();
        let mut node_outputs = Vec::new();
        node_inputs.try_reserve(n).map_err(|_| Error::Memory)?;
        node_outputs.try_reserve(n).map_err(|_| Error::Memory)?;

        for node in &self.nodes {
            let in_count = node.module.inputs().len();
            let mut ins = Vec::new();
            ins.try_reserve(in_count).map_err(|_| Error::Memory)?;
            for p in node.module.inputs() {
                ins.push(SignalBuffer::new(p.kind));
            }
            node_inputs.push(ins);

            let out_count = node.module.outputs().len();
            let mut outs = Vec::new();
            outs.try_reserve(out_count).map_err(|_| Error::Memory)?;
            for p in node.module.outputs() {
                outs.push(SignalBuffer::new(p.kind));
            }
            node_outputs.push(outs);
        }

        self.execution_order = order;
        self.forward_cables = forward_cables;
        self.feedback_delays = feedback_delays;
        self.node_inputs = node_inputs;
        self.node_outputs = node_outputs;
        self.needs_recompile = false;

        Ok(())
    }

    /// Process a 64-sample block with ZERO HEAP ALLOCATIONS!
    pub fn process_block(&mut self) {
        if self.needs_recompile {
            let _ = self.compile();
        }

        // 1. Clear input buffers
        for node_in in &mut self.node_inputs {
            for buf in node_in {
                buf.clear();
            }
        }

        // 2. Inject external MIDI events into PolyMidiToCv modules
        if self.external_midi.len > 0 {
            for (u, node) in self.nodes.iter().enumerate() {
                if node.kind == ModuleKind::PolyMidiToCv && !self.node_inputs[u].is_empty() {
                    for ev in self.external_midi.events() {
                        self.node_inputs[u][0].midi.push(*ev);
                    }
                }
            }
            self.external_midi.clear();
        }

        // 3. Inject 1-block delayed feedback signals into destination inputs
        for delay in &self.feedback_delays {
            if delay.to_node_idx < self.node_inputs.len() {
                let target = &mut self.node_inputs[delay.to_node_idx];
                if (delay.to_port as usize) < target.len() {
                    target[delay.to_port as usize].mix_from(&delay.delayed_buffer, delay.attenuation);
                }
            }
        }

        // 4. Execute nodes in topological order
        for &u in &self.execution_order {
            if u >= self.nodes.len() {
                continue;
            }

            for out_buf in &mut self.node_outputs[u] {
                out_buf.clear();
            }

            // If TrackIn, inject external audio into its output buffer
            if self.nodes[u].kind == ModuleKind::TrackIn && !self.node_outputs[u].is_empty() {
                let gain = self.nodes[u].module.get_param(0);
                for f in 0..BLOCK_FRAMES {
                    self.node_outputs[u][0].set_audio(f, [
                        self.external_audio[f][0] * gain,
                        self.external_audio[f][1] * gain,
                    ]);
                }
            } else {
                self.nodes[u].module.process_block(&self.node_inputs[u], &mut self.node_outputs[u]);
            }

            // Forward DAG transfers
            for cable in &self.forward_cables[u] {
                if (cable.from_port as usize) < self.node_outputs[u].len()
                    && cable.to_node_idx < self.node_inputs.len()
                {
                    let src = &self.node_outputs[u][cable.from_port as usize];
                    let target_node = &mut self.node_inputs[cable.to_node_idx];
                    if (cable.to_port as usize) < target_node.len() {
                        target_node[cable.to_port as usize].mix_from(src, cable.attenuation);
                        target_node[cable.to_port as usize].soft_limit();
                    }
                }
            }

            // Update feedback delay buffers originating from this node
            for delay in &mut self.feedback_delays {
                if delay.from_node_idx == u && (delay.from_port as usize) < self.node_outputs[u].len() {
                    delay.delayed_buffer = self.node_outputs[u][delay.from_port as usize];
                    delay.delayed_buffer.soft_limit();
                }
            }
        }
    }

    pub fn export_selection(&self, nodes: &[NodeId]) -> Result<Vec<u8>, Error> {
        serialize::export_selection(self, nodes)
    }

    pub fn import_selection(&mut self, bytes: &[u8]) -> Result<Vec<NodeId>, Error> {
        serialize::import_selection(self, bytes)
    }

    pub fn encode_patch(&self) -> Result<Vec<u8>, Error> {
        serialize::encode_patch(self)
    }

    pub fn decode_patch(bytes: &[u8]) -> Result<Self, Error> {
        serialize::decode_patch(bytes)
    }

    /// Render EuropaRack directly to an immutable Moan Clip PCM buffer.
    pub fn bake_to_frames(
        &mut self,
        total_frames: usize,
        input_audio: Option<&[moan_core::Frame]>,
        midi_notes: &[moan_core::midi::Note],
    ) -> Result<Arc<Vec<moan_core::Frame>>, Error> {
        if self.needs_recompile {
            self.compile()?;
        }

        let mut pcm: Vec<moan_core::Frame> = Vec::new();
        pcm.try_reserve_exact(total_frames).map_err(|_| Error::Memory)?;

        let mut frame_pos = 0usize;

        while frame_pos < total_frames {
            let block_len = (total_frames - frame_pos).min(BLOCK_FRAMES);

            // 1. Inject audio into TrackIn nodes
            if let Some(audio) = input_audio {
                let mut block_audio = [[0.0f32; 2]; BLOCK_FRAMES];
                for i in 0..block_len {
                    let idx = frame_pos + i;
                    if idx < audio.len() {
                        block_audio[i][0] = audio[idx][0] as f32 / 32768.0;
                        block_audio[i][1] = audio[idx][1] as f32 / 32768.0;
                    }
                }
                self.set_external_audio(&block_audio);
            }

            // 2. Inject MIDI notes
            for note in midi_notes {
                let note_start = note.start as usize;
                let note_end = (note.start + note.frames) as usize;

                if note_start >= frame_pos && note_start < frame_pos + block_len {
                    let offset = (note_start - frame_pos) as u8;
                    self.push_midi_event(
                        crate::signal::ModularMidiEvent::note_on(offset, 0, note.key, note.velocity)
                    );
                }

                if note_end >= frame_pos && note_end < frame_pos + block_len {
                    let offset = (note_end - frame_pos) as u8;
                    self.push_midi_event(
                        crate::signal::ModularMidiEvent::note_off(offset, 0, note.key)
                    );
                }
            }

            // 3. Process the block
            self.process_block();

            // 4. Capture master stereo output
            let mut captured = false;
            for (u, node) in self.nodes.iter().enumerate() {
                if node.kind == ModuleKind::MasterOut {
                    if !self.node_inputs[u].is_empty() {
                        let audio_in = &self.node_inputs[u][0];
                        let vol = node.module.get_param(0);
                        let drive = node.module.get_param(1);
                        for i in 0..block_len {
                            let sample = audio_in.get_audio(i);
                            let l = (crate::math::soft_clip(sample[0] * vol * drive) * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            let r = (crate::math::soft_clip(sample[1] * vol * drive) * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            pcm.push([l, r]);
                        }
                        captured = true;
                        break;
                    }
                }
            }

            if !captured {
                for _ in 0..block_len {
                    pcm.push([0, 0]);
                }
            }

            frame_pos += block_len;
        }

        Ok(Arc::new(pcm))
    }
}
