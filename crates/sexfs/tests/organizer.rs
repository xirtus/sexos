use sexfs::{organizer, Device, Error, Fs, Memory, Slot, BLOCK};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Disk(Rc<RefCell<Vec<[u8; BLOCK]>>>);
impl Disk { fn new() -> Self { Self(Rc::new(RefCell::new(vec![[0; BLOCK]; 2048]))) } }
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
fn opt_in_move_and_undo_survive_remount() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 32];
    let mut pending = [0; 32];
    let mut scratch = [0; BLOCK];
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), mem).unwrap();
    fs.mkdir(b"/Inbox", 0).unwrap(); fs.mkdir(b"/Documents", 0).unwrap();
    fs.mkdir(b"/Pictures", 0).unwrap();
    let a = fs.create(b"/Inbox/a.pdf", 0).unwrap();
    fs.write(a, 0, b"invoice", 0).unwrap();
    organizer::set_enabled(&mut fs, true).unwrap(); fs.commit().unwrap();
    assert!(organizer::enabled(&mut fs).unwrap());
    assert_eq!(organizer::move_and_record(&mut fs, b"/Inbox/a.pdf", b"/Documents/a.pdf", a), Ok(1));
    fs.commit().unwrap();
    drop(fs);

    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk.clone(), mem).unwrap();
    assert!(organizer::enabled(&mut fs).unwrap());
    assert_eq!(fs.open(b"/Documents/a.pdf").unwrap().id, a);
    assert_eq!(organizer::undo_last(&mut fs), Ok(0));
    fs.commit().unwrap();
    assert_eq!(fs.open(b"/Inbox/a.pdf").unwrap().id, a);
    assert_eq!(fs.open(b"/Documents/a.pdf"), Err(Error::NotFound));
    assert_eq!(organizer::undo_last(&mut fs), Err(Error::NotFound));
    assert!(!organizer::enabled(&mut fs).unwrap());
    drop(fs);
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk.clone(), mem).unwrap();
    assert!(!organizer::enabled(&mut fs).unwrap());
    assert_eq!(fs.open(b"/Inbox/a.pdf").unwrap().id, a);
}

#[test]
fn journal_rolls_after_thirty_two_moves_and_preserves_recent_undo() {
    let disk = Disk(Rc::new(RefCell::new(vec![[0; BLOCK]; 8192])));
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 128];
    let mut pending = [0; 128];
    let mut scratch = [0; BLOCK];
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk.clone(), mem).unwrap();
    fs.mkdir(b"/Inbox", 0).unwrap(); fs.mkdir(b"/Documents", 0).unwrap();
    organizer::set_enabled(&mut fs, true).unwrap(); fs.commit().unwrap();
    for n in 0..33 {
        let from = format!("/Inbox/f{n:02}.pdf");
        let to = format!("/Documents/f{n:02}.pdf");
        let id = fs.create(from.as_bytes(), 0).unwrap();
        fs.commit().unwrap();
        assert_eq!(organizer::move_and_record(&mut fs, from.as_bytes(), to.as_bytes(), id).unwrap(), ((n + 1) as u64).min(32));
        fs.commit().unwrap();
    }
    drop(fs);
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::mount(disk, mem).unwrap();
    assert!(organizer::enabled(&mut fs).unwrap());
    for remaining in (0..32).rev() {
        assert_eq!(organizer::undo_last(&mut fs), Ok(remaining));
        fs.commit().unwrap();
    }
    assert_eq!(organizer::undo_last(&mut fs), Err(Error::NotFound));
    assert!(fs.open(b"/Documents/f00.pdf").is_ok());
    assert_eq!(fs.open(b"/Inbox/f00.pdf"), Err(Error::NotFound));
    assert!(fs.open(b"/Inbox/f32.pdf").is_ok());
    assert!(!organizer::enabled(&mut fs).unwrap());
}

#[test]
fn moved_object_replacement_and_collision_block_undo() {
    let disk = Disk::new();
    let mut cache = [[0; BLOCK]; 16];
    let mut slots = [Slot::EMPTY; 16];
    let mut used = [0; 32];
    let mut pending = [0; 32];
    let mut scratch = [0; BLOCK];
    let mem = Memory { cache: &mut cache, slots: &mut slots, used: &mut used, pending: &mut pending, scratch: &mut scratch };
    let mut fs = Fs::format(disk, mem).unwrap();
    fs.mkdir(b"/Inbox", 0).unwrap(); fs.mkdir(b"/Documents", 0).unwrap();
    let a = fs.create(b"/Inbox/a.pdf", 0).unwrap(); fs.commit().unwrap();
    assert_eq!(organizer::move_and_record(&mut fs, b"/Inbox/a.pdf", b"/Documents/a.pdf", a), Ok(1));
    fs.commit().unwrap();
    fs.create(b"/Inbox/a.pdf", 0).unwrap(); fs.commit().unwrap();
    assert_eq!(organizer::undo_last(&mut fs), Err(Error::Exists));
    fs.remove_path(b"/Inbox/a.pdf").unwrap();
    fs.remove_path(b"/Documents/a.pdf").unwrap();
    fs.create(b"/Documents/a.pdf", 0).unwrap(); fs.commit().unwrap();
    assert_eq!(organizer::undo_last(&mut fs), Err(Error::Stale));
    assert_eq!(organizer::move_and_record(&mut fs, b"/Documents/a.pdf", b"/Inbox/a.pdf", a), Err(Error::Name));
}
