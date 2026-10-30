// SPDX-License-Identifier: GPL-3.0-only
//! Pure bounded Browser text composition. Bytes are presentation data, never
//! capability authority or permission to contact an endpoint.
use crate::{dns_probe,http_probe,network_probe};

pub struct Text { pub bytes:[u8;512], pub len:usize }

fn readable(bytes:&[u8])->bool { bytes.iter().all(|b|*b==b'\n'||(b' '..=b'~').contains(b)) }
fn append(out:&mut[u8],at:&mut usize,bytes:&[u8])->Option<()> {
 let end=at.checked_add(bytes.len())?;out.get_mut(*at..end)?.copy_from_slice(bytes);*at=end;Some(())
}
fn decimal(out:&mut[u8],at:&mut usize,value:u8)->Option<()> {
 if value>=100 { append(out,at,&[b'0'+value/100])?; }
 if value>=10 { append(out,at,&[b'0'+(value/10)%10])?; }
 append(out,at,&[b'0'+value%10])
}

/// Every status occupies its complete64-byte replacement extent. The body has
/// one fixed64-byte extent as well, and becomes spaces whenever HTTP is not
/// Ready. The caller publishes the resulting complete snapshot atomically.
pub fn compose(preview:&[u8],echo:network_probe::View,dns:dns_probe::View,http:http_probe::View,body:&[u8;64])->Option<Text> {
 let len=preview.len().checked_add(64*3+2+64)?;
 if len>512||!readable(preview) { return None; }
 if matches!(http,http_probe::View::Ready)&&!readable(body) { return None; }
 let mut text=Text {bytes:[b' ';512],len};text.bytes[..preview.len()].copy_from_slice(preview);
 let echo_label:&[u8]=match echo {
  network_probe::View::Pending=>b"\n\nPeer probe: checking\nR retry  C cancel",
  network_probe::View::Verified=>b"\n\nPeer probe: bytes verified\nR retry  C cancel",
  network_probe::View::Unavailable=>b"\n\nPeer probe: unavailable\nR retry  C cancel",
  network_probe::View::Cancelled=>b"\n\nPeer probe: cancelled\nR retry  C cancel",
  network_probe::View::Uncertain=>b"\n\nPeer probe: waiting receipt\nR retry  C cancel",
 };
 let mut at=preview.len();append(&mut text.bytes[..at+64],&mut at,echo_label)?;
 let dns_start=preview.len()+64;let mut at=dns_start;
 append(&mut text.bytes[..dns_start+64],&mut at,b"\n\nControlled DNS: ")?;
 match dns {
  dns_probe::View::Idle=>append(&mut text.bytes[..dns_start+64],&mut at,b"idle")?,
  dns_probe::View::Pending=>append(&mut text.bytes[..dns_start+64],&mut at,b"checking")?,
  dns_probe::View::Answer(address)=>for(index,byte)in address.iter().enumerate() {
   if index!=0 { append(&mut text.bytes[..dns_start+64],&mut at,b".")?; }
   decimal(&mut text.bytes[..dns_start+64],&mut at,*byte)?;
  },
  dns_probe::View::Unavailable=>append(&mut text.bytes[..dns_start+64],&mut at,b"unavailable")?,
  dns_probe::View::Cancelled=>append(&mut text.bytes[..dns_start+64],&mut at,b"cancelled")?,
  dns_probe::View::Uncertain=>append(&mut text.bytes[..dns_start+64],&mut at,b"waiting receipt")?,
 }
 append(&mut text.bytes[..dns_start+64],&mut at,b"\nD query  X cancel")?;
 let http_start=dns_start+64;let mut at=http_start;
 append(&mut text.bytes[..http_start+64],&mut at,b"\n\nPlain HTTP / no TLS: ")?;
 let http_label:&[u8]=match http {
  http_probe::View::Idle=>b"idle",http_probe::View::Pending=>b"checking",
  http_probe::View::Ready=>b"ready",http_probe::View::Unavailable=>b"unavailable",
  http_probe::View::Cancelled=>b"cancelled",http_probe::View::Uncertain=>b"waiting receipt",
 };
 append(&mut text.bytes[..http_start+64],&mut at,http_label)?;
 append(&mut text.bytes[..http_start+64],&mut at,b"\nH fetch  J cancel  V read")?;
 let body_start=http_start+64;text.bytes[body_start..body_start+2].copy_from_slice(b"\n\n");
 if matches!(http,http_probe::View::Ready) { text.bytes[body_start+2..len].copy_from_slice(body); }
 Some(text)
}
