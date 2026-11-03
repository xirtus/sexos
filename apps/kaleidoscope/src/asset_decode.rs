// SPDX-License-Identifier: GPL-3.0-only
//! Private, cooperative decoding after the parent obtained positive RELEASE.
use alloc::{boxed::Box,vec::Vec};
use sex_graphics::qoi::{Decoder,State,MAX_PIXELS};
use sex_graphics::png;

pub struct Image {
    body:Option<sex_tls::Asset>, decoder:Option<Decoder>, pixels:Option<Box<[u32]>>,
    pub width:usize,pub height:usize,pub ticket:u32,pub wire_bytes:usize,pub wire_hash:u64,
    ready:bool,
    /// A one-step (PNG) decode finished in `begin`; `advance` reports it.
    decoded:bool,
}
impl Image {
    pub const fn new()->Self {Self{body:None,decoder:None,pixels:None,width:0,height:0,ticket:0,wire_bytes:0,wire_hash:0,ready:false,decoded:false}}
    pub fn begin(&mut self,body:sex_tls::Asset,ticket:u32)->Result<(),()> {
        self.clear();
        // Reserve directly in the ELF byte arena; no 9 KiB stack temporary.
        let mut pixels=Vec::new();pixels.try_reserve_exact(MAX_PIXELS).map_err(|_|())?;
        pixels.resize(MAX_PIXELS,0u32);
        if png::is_png(body.as_bytes()) {
            // PNG decodes in one bounded step through a fallible heap scratch.
            let mut scratch=Vec::new();scratch.try_reserve_exact(png::SCRATCH_BYTES).map_err(|_|())?;
            scratch.resize(png::SCRATCH_BYTES,0u8);
            let scratch:&mut[u8;png::SCRATCH_BYTES]=scratch.as_mut_slice().try_into().map_err(|_|())?;
            let out:&mut[u32;MAX_PIXELS]=pixels.as_mut_slice().try_into().map_err(|_|())?;
            let (w,h)=png::decode(body.as_bytes(),scratch,out).map_err(|_|())?;
            for b in scratch.iter_mut(){unsafe{core::ptr::write_volatile(b,0);}}
            self.width=w;self.height=h;self.ticket=ticket;
            self.wire_bytes=body.as_bytes().len();self.wire_hash=body.as_bytes().iter().fold(0xcbf29ce484222325,|h,&b|(h^b as u64).wrapping_mul(0x100000001b3));
            self.pixels=Some(pixels.into_boxed_slice());self.decoded=true;return Ok(());
        }
        let decoder=Decoder::new(body.as_bytes()).map_err(|_|())?;
        self.width=decoder.width();self.height=decoder.height();self.ticket=ticket;
        self.wire_bytes=body.as_bytes().len();self.wire_hash=body.as_bytes().iter().fold(0xcbf29ce484222325,|h,&b|(h^b as u64).wrapping_mul(0x100000001b3));
        self.body=Some(body);self.decoder=Some(decoder);self.pixels=Some(pixels.into_boxed_slice());Ok(())
    }
    pub fn advance(&mut self)->Result<bool,()> {
        if self.ready{return Ok(false);}
        if self.decoded{self.decoded=false;self.ready=true;return Ok(true);}
        let (Some(body),Some(decoder),Some(pixels))=(&self.body,&mut self.decoder,&mut self.pixels)else{return Ok(false);};
        let out:&mut[u32;MAX_PIXELS]=pixels.as_mut().try_into().map_err(|_|())?;
        match decoder.advance(body.as_bytes(),out) {
            Ok(State::Pending)=>Ok(false),
            Ok(State::Ready)=>{self.ready=true;self.body=None;self.decoder=None;Ok(true)},
            Err(_)=>{self.clear();Err(())},
        }
    }
    pub fn pending(&self)->bool {self.decoder.is_some()||self.decoded}
    pub fn pixels(&self)->&[u32] {if self.ready{self.pixels.as_ref().map_or(&[],|p|&p[..self.width*self.height])}else{&[]}}
    pub fn allocated_payload_bytes(&self)->usize {(if self.body.is_some(){sex_tls::MAX_DOCUMENT}else{0})+(if self.pixels.is_some(){MAX_PIXELS*4}else{0})}
    pub fn clear(&mut self) {
        if let Some(pixels)=self.pixels.as_mut(){
            if let Some(decoder)=self.decoder.as_mut(){if let Ok(out)=pixels.as_mut().try_into(){decoder.cancel(out);}}
            for pixel in pixels.iter_mut(){unsafe{core::ptr::write_volatile(pixel,0);}}
        }
        self.body=None;self.decoder=None;self.pixels=None;self.ready=false;self.decoded=false;self.width=0;self.height=0;self.ticket=0;self.wire_bytes=0;self.wire_hash=0;
    }
}
impl Drop for Image {fn drop(&mut self){self.clear();}}
