// SPDX-License-Identifier: GPL-3.0-only
//! The files of a torrent: runs the engine's `DiskOp`s in order. Bytes of deselected
//! files that share a piece with wanted ones go to a part file in `.swingers/`, never into
//! the deselected file itself.
use sex_torrent::storage::{DiskOp, TorrentLayout};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Lists the files under `path` (sorted) with their paths relative to it.
pub fn walk(path: &Path, rel: &mut Vec<String>, out: &mut Vec<(Vec<String>, u64)>) {
    if path.is_file() {
        out.push((rel.clone(), path.metadata().map(|m| m.len()).unwrap_or(0)));
        return;
    }
    let Ok(rd) = std::fs::read_dir(path) else { return };
    let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for n in names {
        rel.push(n.clone());
        walk(&path.join(&n), rel, out);
        rel.pop();
    }
}

/// A read finished: the request id, where it goes in the requested range, the bytes.
pub struct ReadDone { pub id: u64, pub at: u32, pub data: Result<Vec<u8>, ()> }

pub struct Disk {
    pub root: PathBuf,
    layout: TorrentLayout,
    part: PathBuf,
    open: HashMap<usize, File>,
    part_file: Option<File>,
}

impl Disk {
    pub fn new(root: &Path, layout: TorrentLayout, hash_hex: &str) -> Disk {
        let part = root.join(".swingers").join(format!("{hash_hex}.parts"));
        Disk { root: root.to_path_buf(), layout, part, open: HashMap::new(), part_file: None }
    }

    pub fn path_of(&self, file: usize) -> Option<PathBuf> {
        let f = self.layout.files.get(file)?;
        Some(f.path.iter().fold(self.root.clone(), |p, c| p.join(c)))
    }

    pub fn layout(&self) -> &TorrentLayout { &self.layout }

    fn file(&mut self, file: usize, create: bool) -> std::io::Result<&mut File> {
        if !self.open.contains_key(&file) {
            let path = self.path_of(file).ok_or(std::io::ErrorKind::NotFound)?;
            if create { if let Some(d) = path.parent() { std::fs::create_dir_all(d)?; } }
            let f = OpenOptions::new().read(true).write(create).create(create).truncate(false).open(&path)?;
            if self.open.len() > 64 { self.open.clear(); }
            self.open.insert(file, f);
        }
        Ok(self.open.get_mut(&file).expect("just opened"))
    }

    fn part(&mut self) -> std::io::Result<&mut File> {
        if self.part_file.is_none() {
            if let Some(d) = self.part.parent() { std::fs::create_dir_all(d)?; }
            self.part_file = Some(OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&self.part)?);
        }
        Ok(self.part_file.as_mut().expect("just opened"))
    }

    fn part_pos(&self, piece: u32, offset: u32) -> u64 { piece as u64 * self.layout.piece_length as u64 + offset as u64 }

    fn read_at(f: &mut File, at: u64, len: u32) -> std::io::Result<Vec<u8>> {
        f.seek(SeekFrom::Start(at))?;
        let mut buf = vec![0u8; len as usize];
        let mut got = 0;
        while got < buf.len() {
            let n = f.read(&mut buf[got..])?;
            if n == 0 { return Err(std::io::ErrorKind::UnexpectedEof.into()); }
            got += n;
        }
        Ok(buf)
    }

    /// Runs one operation. A read's answer comes back for the engine; a failed write is
    /// an `Err`, which stops the torrent with an error.
    pub fn run(&mut self, op: DiskOp) -> Result<Option<ReadDone>, String> {
        let w = |e: std::io::Error| e.to_string();
        match op {
            DiskOp::Write { file, offset, data } => {
                let f = self.file(file, true).map_err(w)?;
                f.seek(SeekFrom::Start(offset)).map_err(w)?;
                f.write_all(&data).map_err(w)?;
            }
            DiskOp::WritePart { piece, offset, data } => {
                let at = self.part_pos(piece, offset);
                let f = self.part().map_err(w)?;
                f.seek(SeekFrom::Start(at)).map_err(w)?;
                f.write_all(&data).map_err(w)?;
            }
            DiskOp::Read { id, file, offset, len, at } => {
                let data = self.file(file, false).and_then(|f| Self::read_at(f, offset, len)).map_err(|_| ());
                return Ok(Some(ReadDone { id, at, data }));
            }
            DiskOp::ReadPart { id, piece, offset, len, at } => {
                let pos = self.part_pos(piece, offset);
                let data = self.part().and_then(|f| Self::read_at(f, pos, len)).map_err(|_| ());
                return Ok(Some(ReadDone { id, at, data }));
            }
            DiskOp::Preallocate { file, len } => {
                let f = self.file(file, true).map_err(w)?;
                if len > 0 && f.metadata().map(|m| m.len() < len).unwrap_or(true) { f.set_len(len).map_err(w)?; }
            }
            DiskOp::PromotePart { piece, part_offset, file, file_offset, len } => {
                let pos = self.part_pos(piece, part_offset);
                let data = self.part().and_then(|f| Self::read_at(f, pos, len)).map_err(w)?;
                let f = self.file(file, true).map_err(w)?;
                f.seek(SeekFrom::Start(file_offset)).map_err(w)?;
                f.write_all(&data).map_err(w)?;
            }
            DiskOp::DiscardPart { .. } => {}
        }
        Ok(None)
    }

    /// Flushes and closes every open file (before hashing them for the record).
    pub fn close(&mut self) {
        for f in self.open.values_mut() { let _ = f.flush(); }
        self.open.clear();
        self.part_file = None;
    }
}
