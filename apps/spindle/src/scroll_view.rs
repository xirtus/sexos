//! Scrollback selection over the retained ring, independent of lifetime count.
pub const CAPACITY:usize=1024;
pub fn retained(total:u32)->usize{(total as usize).min(CAPACITY)}
pub fn index(write:usize,total:u32,logical:usize)->Option<usize>{let count=retained(total);if logical>=count{return None}Some((write+CAPACITY-count+logical)%CAPACITY)}
pub fn clamp(offset:u32,total:u32,visible:usize)->u32{if visible==0{0}else{offset.min(retained(total).saturating_sub(visible)as u32)}}
pub fn page(offset:u32,up:bool,total:u32,visible:usize)->u32{if visible==0{return 0}let offset=clamp(offset,total,visible);let step=visible.min(CAPACITY)as u32;clamp(if up{offset.saturating_add(step)}else{offset.saturating_sub(step)},total,visible)}
pub fn after_push(offset:u32,total:u32)->u32{if offset==0{0}else{offset.saturating_add(1).min(retained(total).saturating_sub(1)as u32)}}
