use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Disk(Rc<RefCell<Vec<[u8; BLOCK]>>>);
impl Device for Disk {
    fn blocks(&self) -> u64 { self.0.borrow().len() as u64 }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        *out = self.0.borrow()[block as usize]; Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        self.0.borrow_mut()[block as usize] = *data; Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> { Ok(()) }
}

#[test]
fn same_size_writer_cannot_be_clobbered_by_conditional_replace() {
    let disk = Disk(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024])));
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 16];
    let mut pending = [0; 16];
    let mut scratch = [0; BLOCK];
    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), memory).unwrap();
    let old = fs.create(b"/source.pdf", 0).unwrap();
    fs.write(old, 0, b"opened PDF", 0).unwrap();
    let staged = fs.create(b"/staged.pdf", 0).unwrap();
    fs.write(staged, 0, b"signed PDF", 0).unwrap();
    fs.commit().unwrap();
    let expected: [u8; 32] = Sha256::digest(b"opened PDF").into();

    // Another Files client changes the saved document, without changing its length.
    fs.write(old, 0, b"newer  PDF", 0).unwrap();
    fs.commit().unwrap();
    assert_eq!(fs.rename_if_sha256(b"/staged.pdf", b"/source.pdf", &expected), Err(Error::Stale));
    assert_eq!(fs.open(b"/staged.pdf").unwrap().id, staged);
    assert_eq!(fs.open(b"/source.pdf").unwrap().id, old);
    let mut actual = [0u8; 10];
    fs.read(old, 0, &mut actual).unwrap();
    assert_eq!(&actual, b"newer  PDF");
    fs.rollback().unwrap();

    let fresh: [u8; 32] = Sha256::digest(b"newer  PDF").into();
    fs.rename_if_sha256(b"/staged.pdf", b"/source.pdf", &fresh).unwrap();
    fs.commit().unwrap();
    let now = fs.open(b"/source.pdf").unwrap();
    assert_eq!(now.id, staged);
    let mut actual = [0u8; 10];
    fs.read(now.id, 0, &mut actual).unwrap();
    assert_eq!(&actual, b"signed PDF");
    drop(fs);

    let memory = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut reopened = Fs::mount(disk, memory).unwrap();
    assert_eq!(reopened.open(b"/source.pdf").unwrap().id, staged);
    assert_eq!(reopened.open(b"/staged.pdf"), Err(Error::NotFound));
}
