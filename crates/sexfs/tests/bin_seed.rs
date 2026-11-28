use sexfs::{Device, Fs, Kind, Memory, Slot, BLOCK};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct MemBlocks(Rc<RefCell<Vec<[u8; BLOCK]>>>);

impl MemBlocks {
    fn new(blocks: usize) -> Self {
        Self(Rc::new(RefCell::new(vec![[0; BLOCK]; blocks])))
    }
}

impl Device for MemBlocks {
    fn blocks(&self) -> u64 {
        self.0.borrow().len() as u64
    }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        *out = self.0.borrow()[block as usize];
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        self.0.borrow_mut()[block as usize] = *data;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }
}

#[test]
fn bin_program_seed_persists_across_remount() {
    let disk = MemBlocks::new(1024);
    let mut cache = [[0; BLOCK]; 32];
    let mut slots = [Slot::EMPTY; 32];
    let mut used = [0u64; 32];
    let mut pending = [0u64; 32];
    let mut scratch = [0u8; BLOCK];

    let tatami_elf: Vec<u8> = (0..131_072u32)
        .map(|i| ((i.wrapping_mul(29) ^ 0xA5) & 0xFF) as u8)
        .collect();
    let sexshop_elf: Vec<u8> = (0..98_304u32)
        .map(|i| ((i.wrapping_mul(17) ^ 0x5A) & 0xFF) as u8)
        .collect();

    {
        let mem = Memory {
            cache: &mut cache,
            slots: &mut slots,
            used: &mut used,
            pending: &mut pending,
            scratch: &mut scratch,
        };
        let mut fs = Fs::format(disk.clone(), mem).unwrap();
        fs.mkdir(b"/bin", 0).unwrap();
        for (path, bytes) in [
            (&b"/bin/tatami"[..], &tatami_elf[..]),
            (&b"/bin/sexshop"[..], &sexshop_elf[..]),
        ] {
            let id = fs.create(path, 0).unwrap();
            let mut off = 0usize;
            while off < bytes.len() {
                let end = (off + 16_384).min(bytes.len());
                let n = fs.write(id, off as u64, &bytes[off..end], 0).unwrap();
                assert!(n > 0);
                off += n;
            }
            fs.commit().unwrap();
        }
    }

    let mem = Memory {
        cache: &mut cache,
        slots: &mut slots,
        used: &mut used,
        pending: &mut pending,
        scratch: &mut scratch,
    };
    let mut fs = Fs::mount(disk, mem).unwrap();
    for (path, expected) in [
        (&b"/bin/tatami"[..], &tatami_elf[..]),
        (&b"/bin/sexshop"[..], &sexshop_elf[..]),
    ] {
        let st = fs.open(path).unwrap();
        assert_eq!(st.kind, Kind::File);
        assert_eq!(st.size, expected.len() as u64);
        let mut got = vec![0u8; expected.len()];
        let mut off = 0usize;
        while off < got.len() {
            let n = fs.read(st.id, off as u64, &mut got[off..]).unwrap();
            assert!(n > 0);
            off += n;
        }
        assert_eq!(got, expected);
    }
}
