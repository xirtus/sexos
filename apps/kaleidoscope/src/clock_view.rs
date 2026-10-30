// SPDX-License-Identifier: GPL-3.0-only
//! Public last-query status, not a continuously trusted clock or TLS label.
use crate::clock_probe::View;
pub struct Status{pub bytes:[u8;24],pub len:usize}
fn label(value:&[u8])->Status{let mut s=Status{bytes:[b' ';24],len:value.len()};s.bytes[..value.len()].copy_from_slice(value);s}
fn leap(year:u32)->bool{year%4==0&&(year%100!=0||year%400==0)}
fn days(year:u32,month:u32)->u32{match month{2=>28+leap(year)as u32,4|6|9|11=>30,_=>31}}
pub fn status(view:View)->Option<Status>{
    let seconds=match view{
        View::Idle=>return None,View::Pending=>return Some(label(b"checking")),
        View::Unavailable=>return Some(label(b"unavailable")),View::Cancelled=>return Some(label(b"cancelled")),
        View::Uncertain=>return Some(label(b"waiting receipt")),View::Rollback=>return Some(label(b"rollback; blocked")),View::Ready(value)=>value as u64,
    };
    if !(sex_pdx::clock_source::MIN_SECONDS..=sex_pdx::clock_source::MAX_SECONDS).contains(&seconds){return Some(label(b"unavailable"));}
    let mut n=seconds/86400;let mut year=1970;
    while n>=(365+leap(year)as u64){n-=365+leap(year)as u64;year+=1;}
    let mut month=1;while n>=days(year,month)as u64{n-=days(year,month)as u64;month+=1;}
    let day=n as u32+1;let within=(seconds%86400)as u32;
    let mut out=label(b"0000-00-00T00:00:00Z");
    for (at,value,width)in[(0,year,4),(5,month,2),(8,day,2),(11,within/3600,2),(14,within/60%60,2),(17,within%60,2)]{
        let mut value=value;for digit in (at..at+width).rev(){out.bytes[digit]=b'0'+(value%10)as u8;value/=10;}
    }
    Some(out)
}
