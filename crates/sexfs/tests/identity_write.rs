use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Disk(Rc<RefCell<Vec<[u8; BLOCK]>>>);
impl Disk { fn new() -> Self { Self(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024]))) } }
impl Device for Disk {
    fn blocks(&self) -> u64 { self.0.borrow().len() as u64 }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> { *out = self.0.borrow()[block as usize]; Ok(()) }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> { self.0.borrow_mut()[block as usize] = *data; Ok(()) }
    fn flush(&mut self) -> Result<(), ()> { Ok(()) }
}

#[test]
fn replacement_never_receives_bytes_or_cleanup() {
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(Disk::new(), memory).unwrap();
    let original = fs.create(b"/attachment", 0).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.write_path_if_id(b"/attachment", original, 0, b"first", 0), Ok(5));
    fs.commit().unwrap();

    fs.remove_path(b"/attachment").unwrap();
    let replacement = fs.create(b"/attachment", 0).unwrap();
    fs.write(replacement, 0, b"private", 0).unwrap();
    fs.commit().unwrap();
    assert_ne!(original, replacement);
    assert_eq!(fs.write_path_if_id(b"/attachment", original, 5, b"leak", 0), Err(Error::Stale));
    assert_eq!(fs.remove_path_if_id(b"/attachment", original), Err(Error::Stale));
    assert_eq!(fs.write_path_if_id(b"/attachment", 0, 0, b"leak", 0), Err(Error::Name));
    assert_eq!(fs.open(b"/attachment").unwrap().id, replacement);
    let mut bytes = [0; 7];
    assert_eq!(fs.read(replacement, 0, &mut bytes), Ok(7));
    assert_eq!(&bytes, b"private");
}

#[test]
fn failed_export_removes_only_its_partial_file() {
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(Disk::new(), memory).unwrap();
    let id = fs.create(b"/attachment", 0).unwrap();
    fs.commit().unwrap();
    fs.write_path_if_id(b"/attachment", id, 0, b"partial", 0).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.remove_path_if_id(b"/attachment", id), Ok(()));
    fs.commit().unwrap();
    assert_eq!(fs.open(b"/attachment"), Err(Error::NotFound));
}
