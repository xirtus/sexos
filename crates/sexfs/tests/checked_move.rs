use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
struct Disk(Rc<RefCell<Vec<[u8; BLOCK]>>>);

impl Disk {
    fn new() -> Self { Self(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024]))) }
}
impl Device for Disk {
    fn blocks(&self) -> u64 { self.0.borrow().len() as u64 }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        *out = self.0.borrow()[block as usize];
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        self.0.borrow_mut()[block as usize] = *data;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> { Ok(()) }
}

#[derive(Clone)]
struct FlushFailDisk(Disk, Rc<Cell<bool>>);
impl Device for FlushFailDisk {
    fn blocks(&self) -> u64 { self.0.blocks() }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> { self.0.read(block, out) }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> { self.0.write(block, data) }
    fn flush(&mut self) -> Result<(), ()> { if self.1.get() { Err(()) } else { Ok(()) } }
}

#[test]
fn backing_flush_failure_is_returned_by_home_style_commit() {
    let fail = Rc::new(Cell::new(false));
    let disk = FlushFailDisk(Disk::new(), fail.clone());
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk, memory).unwrap();
    fs.create(b"/sealed-home-image", 0).unwrap();
    fail.set(true);
    assert_eq!(fs.commit(), Err(Error::Io));
}

#[test]
fn checked_move_preserves_identity_bytes_and_durable_location() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), memory).unwrap();
    fs.mkdir(b"/Inbox", 0).unwrap();
    fs.mkdir(b"/Documents", 0).unwrap();
    let id = fs.create(b"/Inbox/note.txt", 0).unwrap();
    fs.write(id, 0, b"keep this", 0).unwrap();
    fs.commit().unwrap();

    assert_eq!(fs.move_file_if_id(b"/Inbox/note.txt", b"/Documents/note.txt", id), Ok(id));
    fs.commit().unwrap();
    assert_eq!(fs.open(b"/Inbox/note.txt"), Err(Error::NotFound));
    assert_eq!(fs.open(b"/Documents/note.txt").unwrap().id, id);
    let mut contents = [0; 9];
    assert_eq!(fs.read(id, 0, &mut contents), Ok(9));
    assert_eq!(&contents, b"keep this");
    assert_eq!(fs.move_file_if_id(b"/Documents/note.txt", b"/Documents/note.txt", id), Ok(id));
    drop(fs);

    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk, memory).unwrap();
    assert_eq!(fs.open(b"/Documents/note.txt").unwrap().id, id);
    assert_eq!(fs.open(b"/Inbox/note.txt"), Err(Error::NotFound));
}

#[test]
fn stale_plan_and_collision_never_displace_a_file() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk, memory).unwrap();
    fs.mkdir(b"/Inbox", 0).unwrap();
    fs.mkdir(b"/Documents", 0).unwrap();
    let source = fs.create(b"/Inbox/file", 0).unwrap();
    let destination = fs.create(b"/Documents/file", 0).unwrap();
    fs.commit().unwrap();

    assert_eq!(fs.move_file_if_id(b"/Inbox/file", b"/Documents/file", source), Err(Error::Exists));
    assert_eq!(fs.move_file_if_id(b"/Inbox/file", b"/Documents/other", destination), Err(Error::Stale));
    assert_eq!(fs.move_file_if_id(b"/Inbox/file", b"/Documents/../escape", source), Err(Error::Name));
    assert_eq!(fs.open(b"/Inbox/file").unwrap().id, source);
    assert_eq!(fs.open(b"/Documents/file").unwrap().id, destination);
    assert_eq!(fs.open(b"/Documents/other"), Err(Error::NotFound));

    fs.remove_path(b"/Inbox/file").unwrap();
    let replacement = fs.create(b"/Inbox/file", 0).unwrap();
    fs.commit().unwrap();
    assert_ne!(replacement, source);
    assert_eq!(fs.move_file_if_id(b"/Inbox/file", b"/Documents/other", source), Err(Error::Stale));
    assert_eq!(fs.open(b"/Inbox/file").unwrap().id, replacement);
}

#[test]
fn archive_part_promotion_refuses_a_name_taken_during_packing() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), memory).unwrap();
    fs.mkdir(b"/Documents", 0).unwrap();
    let part = fs.create(b"/Documents/.notes.zip.part", 0).unwrap();
    fs.write(part, 0, b"new archive", 0).unwrap();
    fs.commit().unwrap();
    // A second actor claims the name after the archive chose it but before
    // promotion. Checked promotion must preserve both complete files.
    let other = fs.create(b"/Documents/notes.zip", 0).unwrap();
    fs.write(other, 0, b"other actor", 0).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.move_file_if_id(b"/Documents/.notes.zip.part", b"/Documents/notes.zip", part), Err(Error::Exists));
    fs.commit().unwrap();
    drop(fs);
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk, memory).unwrap();
    let mut got = [0; 11];
    let part_id = fs.open(b"/Documents/.notes.zip.part").unwrap().id;
    fs.read(part_id, 0, &mut got).unwrap();
    assert_eq!(&got, b"new archive");
    let other_id = fs.open(b"/Documents/notes.zip").unwrap().id;
    fs.read(other_id, 0, &mut got).unwrap();
    assert_eq!(&got, b"other actor");
}
