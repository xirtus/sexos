// SPDX-License-Identifier: GPL-3.0-only
//! Bounded Ethernet/ARP only. Static prototype address, no DHCP/IP/online claim.
use sex_pdx::network_config::Config;
pub const LOCAL:[u8;4]=Config::CONTROLLED.local;
pub const GATEWAY:[u8;4]=Config::CONTROLLED.gateway;
fn base(mac:[u8;6],config:&Config)->[u8;60]{let mut out=[0;60];out[6..12].copy_from_slice(&mac);out[12..14].copy_from_slice(&[8,6]);
    out[14..22].copy_from_slice(&[0,1,8,0,6,4,0,1]);out[22..28].copy_from_slice(&mac);out[28..32].copy_from_slice(&config.local);out}
pub fn request_configured(mac:[u8;6],config:&Config)->Option<[u8;60]>{if !config.valid(){return None;}let mut out=base(mac,config);out[..6].fill(255);out[38..42].copy_from_slice(&config.gateway);Some(out)}
pub fn request(mac:[u8;6])->[u8;60]{request_configured(mac,&Config::CONTROLLED).unwrap()}
pub fn reply_configured(frame:&[u8],mac:[u8;6],config:&Config)->Option<[u8;60]>{
    if !config.valid(){return None;}
    if frame.len()<42||frame[12..22]!=[8,6,0,1,8,0,6,4,0,1]||frame[38..42]!=config.local
        ||(frame[..6]!=mac&&frame[..6]!=[255;6])||frame[6..12]!=frame[22..28]
        ||frame[6]&1!=0||frame[6..12]==[0;6]||frame[6..12]==mac{return None;}
    let mut out=base(mac,config);out[..6].copy_from_slice(&frame[6..12]);out[21]=2;
    out[32..38].copy_from_slice(&frame[22..28]);out[38..42].copy_from_slice(&frame[28..32]);Some(out)
}

pub fn reply(frame:&[u8],mac:[u8;6])->Option<[u8;60]>{reply_configured(frame,mac,&Config::CONTROLLED)}
