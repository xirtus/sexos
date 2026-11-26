use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};
use std::cell::RefCell;
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

#[test]
fn origin_note_create_is_complete_durable_and_no_clobber() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), memory).unwrap();
    let id = fs.create_with_data(b"/origin", b"/Documents/letter.odt", 0).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.create_with_data(b"/origin", b"foreign", 0), Err(Error::Exists));
    fs.rollback().unwrap();
    drop(fs);

    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk.clone(), memory).unwrap();
    assert_eq!(fs.open(b"/origin").unwrap().id, id);
    let mut got = [0; 21];
    assert_eq!(fs.read(id, 0, &mut got), Ok(got.len()));
    assert_eq!(&got, b"/Documents/letter.odt");

    fs.remove_path(b"/origin").unwrap();
    let replacement = fs.create_with_data(b"/origin", b"another item", 0).unwrap();
    fs.commit().unwrap();
    assert_ne!(replacement, id);
    assert_eq!(fs.remove_path_if_id(b"/origin", id), Err(Error::Stale));
    fs.rollback().unwrap();
    assert_eq!(fs.open(b"/origin").unwrap().id, replacement);
    fs.remove_path_if_id(b"/origin", replacement).unwrap();
    fs.commit().unwrap();
    drop(fs);

    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk, memory).unwrap();
    assert_eq!(fs.open(b"/origin"), Err(Error::NotFound));
}
