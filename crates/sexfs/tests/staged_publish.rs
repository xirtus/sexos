use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Cut(Rc<RefCell<Disk>>);
struct Disk {
    blocks: Vec<[u8; BLOCK]>,
    left: usize,
}
impl Device for Cut {
    fn blocks(&self) -> u64 {
        self.0.borrow().blocks.len() as u64
    }
    fn read(&mut self, b: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        *out = self.0.borrow().blocks[b as usize];
        Ok(())
    }
    fn write(&mut self, b: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        let mut disk = self.0.borrow_mut();
        if disk.left == 0 {
            return Err(());
        }
        disk.left -= 1;
        disk.blocks[b as usize] = *data;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }
}

fn with_fs<T>(disk: Cut, f: impl FnOnce(&mut Fs<'_, Cut>) -> T) -> T {
    let mut cache = [[0; BLOCK]; 24];
    let mut slots = [Slot::EMPTY; 24];
    let mut used = [0; 32];
    let mut pending = [0; 32];
    let mut scratch = [0; BLOCK];
    let mem = Memory {
        cache: &mut cache,
        slots: &mut slots,
        used: &mut used,
        pending: &mut pending,
        scratch: &mut scratch,
    };
    let mut fs = Fs::mount(disk, mem).unwrap();
    f(&mut fs)
}

#[test]
fn staged_bytes_remain_unpublished_across_every_publish_cut() {
    let disk = Cut(Rc::new(RefCell::new(Disk {
        blocks: vec![[0; BLOCK]; 1024],
        left: usize::MAX,
    })));
    {
        let mut cache = [[0; BLOCK]; 24];
        let mut slots = [Slot::EMPTY; 24];
        let mut used = [0; 32];
        let mut pending = [0; 32];
        let mut scratch = [0; BLOCK];
        let mem = Memory {
            cache: &mut cache,
            slots: &mut slots,
            used: &mut used,
            pending: &mut pending,
            scratch: &mut scratch,
        };
        let mut fs = Fs::format(disk.clone(), mem).unwrap();
        let id = fs.create(b"/.stage", 0).unwrap();
        fs.commit().unwrap();
        fs.write_path_if_id(b"/.stage", id, 0, b"complete attachment", 0)
            .unwrap();
        fs.commit().unwrap();
    }
    let snapshot = disk.0.borrow().blocks.clone();
    for budget in 0..80 {
        let cut = Cut(Rc::new(RefCell::new(Disk {
            blocks: snapshot.clone(),
            left: budget,
        })));
        with_fs(cut.clone(), |fs| {
            let id = fs.open(b"/.stage").unwrap().id;
            fs.move_file_if_id(b"/.stage", b"/chosen", id).unwrap();
            let _ = fs.commit();
        });
        cut.0.borrow_mut().left = usize::MAX;
        with_fs(cut, |fs| {
            fs.verify().unwrap();
            match fs.open(b"/chosen") {
                Ok(st) => {
                    assert_eq!(st.size, 19);
                    let mut bytes = [0; 19];
                    assert_eq!(fs.read(st.id, 0, &mut bytes), Ok(19));
                    assert_eq!(&bytes, b"complete attachment");
                    assert_eq!(fs.open(b"/.stage"), Err(Error::NotFound));
                }
                Err(Error::NotFound) => assert!(fs.open(b"/.stage").is_ok()),
                other => panic!("unexpected destination: {other:?}"),
            }
        });
    }
}

#[test]
fn publish_refuses_an_existing_chosen_name() {
    let disk = Cut(Rc::new(RefCell::new(Disk {
        blocks: vec![[0; BLOCK]; 1024],
        left: usize::MAX,
    })));
    {
        let mut cache = [[0; BLOCK]; 24];
        let mut slots = [Slot::EMPTY; 24];
        let mut used = [0; 32];
        let mut pending = [0; 32];
        let mut scratch = [0; BLOCK];
        let mem = Memory {
            cache: &mut cache,
            slots: &mut slots,
            used: &mut used,
            pending: &mut pending,
            scratch: &mut scratch,
        };
        let mut fs = Fs::format(disk.clone(), mem).unwrap();
        let stage = fs.create(b"/.stage", 0).unwrap();
        let chosen = fs.create(b"/chosen", 0).unwrap();
        fs.write(stage, 0, b"new", 0).unwrap();
        fs.write(chosen, 0, b"old", 0).unwrap();
        fs.commit().unwrap();
    }
    with_fs(disk, |fs| {
        let stage = fs.open(b"/.stage").unwrap().id;
        assert_eq!(
            fs.move_file_if_id(b"/.stage", b"/chosen", stage),
            Err(Error::Exists)
        );
        let chosen = fs.open(b"/chosen").unwrap();
        let mut bytes = [0; 3];
        assert_eq!(fs.read(chosen.id, 0, &mut bytes), Ok(3));
        assert_eq!(&bytes, b"old");
    });
}
