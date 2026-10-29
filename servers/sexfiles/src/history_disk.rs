//! Existing BOOT block-buffer view and authenticated SexDrive completion path.
use crate::{history_journal::{Journal,SectorIo,Store,STORAGE},backends::diskfs::DiskFs,vfs};
use sex_pdx::{history_recovery,spindle_history as w,SLOT_BUF_LEND};
pub struct DiskIo;
impl SectorIo for DiskIo{
 fn read(&mut self,lba:u64,out:&mut[u8;512])->Result<(),u32>{let va=vfs::diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(STORAGE)}if DiskFs::diskfs_block_read(lba*512,512,SLOT_BUF_LEND)!=0{return Err(STORAGE)}for(i,b)in out.iter_mut().enumerate(){*b=unsafe{core::ptr::read_volatile((va as *const u8).add(i))};}Ok(())}
 fn write(&mut self,lba:u64,data:&[u8;512])->Result<(),u32>{let va=vfs::diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(STORAGE)}for(i,&b)in data.iter().enumerate(){unsafe{core::ptr::write_volatile((va as *mut u8).add(i),b);}}if DiskFs::diskfs_block_write(lba*512,512,SLOT_BUF_LEND)!=0{return Err(STORAGE)}Ok(())}
 fn sync(&mut self)->Result<(),u32>{if DiskFs::diskfs_block_sync()!=0{return Err(STORAGE)}Ok(())}
}
pub struct Durable{journal:Journal<DiskIo>}
impl Durable{pub fn new()->Self{let grant=history_recovery::get(history_recovery::SLOT).ok();sex_pdx::serial_record!("[files.history.authority] slot=53 present={} logical=12 runtime_mint=0",grant.is_some()as u8);Self{journal:Journal::new(DiskIo,grant)}}}
impl Store for Durable{
 fn save(&mut self,caller:u32,bytes:&[u8])->Result<(),u32>{self.journal.save(caller,bytes)?;let r=self.journal.last.unwrap();sex_pdx::serial_record!("[files.history.durable] logical=12 target={} generation={} bank={} bytes={} hash={:#x} barriers=3 readback=1",caller,r.generation,r.bank,r.len,r.hash);Ok(())}
 fn recover(&mut self,caller:u32,out:&mut[u8;w::MAX])->Result<usize,u32>{let len=self.journal.recover(caller,out)?;let r=self.journal.last.unwrap();sex_pdx::serial_record!("[files.history.recovered] logical=12 source={} source_task={} target={} generation={} bytes={} source_hash={:#x} owned_hash={:#x} data_hash={:#x} boot_grant=53",r.writer,r.task,caller,r.generation,len,r.hash,w::hash(&out[..len]),w::hash(&out[w::HEADER..len]));Ok(len)}
}
