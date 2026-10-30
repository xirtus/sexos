// SPDX-License-Identifier: GPL-3.0-only
//! Narrow allocation-free HTTP message codec for a configured controlled document peer.
//! The fixed request grants no network authority. Parse only after complete
//! transport receipt: a successful parse publishes a body range, never a prefix.
//! Navigation accepts bounded same-endpoint origin-form redirect data only.
//! No arbitrary URLs, cookies, HTML interpretation, TLS, caching or online-web claim.
//! For200, Content-Type must match the selected fixed resource. Legacy text
//! permits one unquoted charset=us-ascii parameter. Redirect MIME is optional
//! syntax-checked data; it never selects rendering. Body bytes remain raw data.

pub const MAX_REQUEST: usize = 128;
/// Colon plus the immutable BOOT TLS endpoint's maximum five decimal digits.
pub const HTTPS_MAX_REQUEST: usize = MAX_REQUEST + 6;
pub const MAX_RESPONSE: usize = MAX_HEADERS + MAX_BODY;
pub const MAX_HEADERS: usize = 1024;
pub const MAX_BODY: usize = 12288;
pub const MAX_PATH: usize = 48;
pub const MAX_REDIRECT_BODY: usize = 1024;
// Exact supported-profile fixture for host/native wire oracles. Runtime Host
// bytes come from the shared BOOT profile below; the actual request test binds it.
const REQUEST: &[u8] = b"GET /welcome HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n";

/// Fixed application data on the same immutable BOOT endpoint. Selecting a
/// resource never selects a trust profile, host, port, route or capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Resource { WelcomeText, ArtworkQoi }
impl Resource {
    pub const fn path(self) -> Path {
        let source: &[u8] = match self { Self::WelcomeText=>b"/welcome",Self::ArtworkQoi=>b"/art.qoi" };
        let mut bytes=[0;MAX_PATH];let mut at=0;
        while at<source.len(){bytes[at]=source[at];at+=1;}
        Path{bytes,len:source.len()}
    }
    pub const fn mime(self) -> &'static [u8] {
        match self {Self::WelcomeText=>b"text/plain",Self::ArtworkQoi=>b"image/qoi"}
    }
}

/// Canonical bounded request-target data, never an endpoint or capability.
/// Restrict syntax instead of decoding/normalizing aliases or URL authorities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Path { bytes: [u8; MAX_PATH], len: usize }
impl Path {
    pub const fn initial() -> Self {
        let mut bytes=[0;MAX_PATH];let source=b"/welcome";let mut at=0;
        while at<source.len() {bytes[at]=source[at];at+=1;}
        Self {bytes,len:source.len()}
    }
    pub fn from_location(input:&[u8]) -> Result<Self,Error> {
        if input.is_empty() || input.len()>MAX_PATH || input[0]!=b'/'
            || input.iter().any(|byte|!byte.is_ascii_alphanumeric() && !matches!(*byte,b'/'|b'-'|b'.'|b'_'|b'~')) {
            return Err(Error::InvalidLocation);
        }
        if input.windows(2).any(|pair|pair==b"//")
            || input.split(|byte|*byte==b'/').any(|part|part==b"."||part==b"..") {
            return Err(Error::InvalidLocation);
        }
        let mut bytes=[0;MAX_PATH];bytes[..input.len()].copy_from_slice(input);
        Ok(Self {bytes,len:input.len()})
    }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub body: core::ops::Range<usize>,
    pub content_length: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Headers { pub header_len: usize, pub content_length: usize }
/// Framing and optional next-path data only. A redirect requires complete
/// transport validation and positive job RELEASE before any follow-up work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationHeaders {
    pub header_len: usize, pub content_length: usize,
    pub status: u16, pub redirect: Option<Path>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    Malformed,
    ResponseLimit,
    HeaderLimit,
    BodyLimit,
    UnsupportedVersion,
    UnsupportedStatus,
    MissingContentLength,
    InvalidContentLength,
    DuplicateContentLength,
    MissingContentType,
    UnsupportedContentType,
    UnsupportedEncoding,
    TransferEncoding,
    TrailingData,
    MissingLocation,
    DuplicateLocation,
    InvalidLocation,
    /// Receiver lifecycle misuse; never a transport or TLS diagnosis.
    State,
    OutputLimit,
}

/// Emit only the fixed controlled request; the returned length is the extent
/// to send. Bytes beyond it are not part of the request and are left untouched.
pub fn request(out: &mut [u8; MAX_REQUEST]) -> usize {
    request_path(out,&Path::initial())
}

/// Emit GET data under the same immutable BOOT hostname. Path construction
/// cannot select a host, scheme, port, route, owner or DNS generation.
pub fn request_path(out: &mut [u8; MAX_REQUEST], path: &Path) -> usize {
    emit_request(out,path,false)
}

/// HTTPS Host data includes only the immutable BOOT profile's TLS port. This
/// codec never accepts a caller-selected hostname, endpoint, scheme or port.
pub fn request_https(out:&mut[u8;HTTPS_MAX_REQUEST],path:&Path)->usize{
    emit_request(out,path,true)
}
/// The path and Accept value are selected together from fixed resource data.
pub fn request_https_resource(out:&mut[u8;HTTPS_MAX_REQUEST],resource:Resource)->usize{
    emit_request_resource(out,&resource.path(),true,resource)
}
fn emit_request(out:&mut[u8],path:&Path,https:bool)->usize{
    emit_request_resource(out,path,https,Resource::WelcomeText)
}
fn emit_request_resource(out:&mut[u8],path:&Path,https:bool,resource:Resource)->usize{
    let prefix=b"GET ";let path=path.as_bytes();let host=sex_net_config::HOST_NAME;
    let version=b" HTTP/1.1\r\nHost: ";let suffix=b"\r\nConnection: close\r\nAccept: ";let end=b"\r\n\r\n";
    let mut digits=[0u8;5];let mut start=digits.len();let mut port=sex_net_config::TLS_PORT;
    assert!(port!=0,"BOOT_HTTP_TLS_PORT");
    while port!=0{start-=1;digits[start]=b'0'+(port%10)as u8;port/=10;}
    let tls_suffix=if https{1+digits.len()-start}else{0};
    let length=prefix.len()+path.len()+version.len()+host.len()+tls_suffix+suffix.len()+resource.mime().len()+end.len();
    assert!(length<=out.len(),"BOOT_HTTP_PROFILE_BOUND");let mut at=0;
    for part in [prefix.as_slice(),path,version.as_slice(),host]{out[at..at+part.len()].copy_from_slice(part);at+=part.len();}
    if https{out[at]=b':';at+=1;out[at..at+digits.len()-start].copy_from_slice(&digits[start..]);at+=digits.len()-start;}
    for part in [suffix.as_slice(),resource.mime(),end.as_slice()]{out[at..at+part.len()].copy_from_slice(part);at+=part.len();}length
}

fn line<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], Error> {
    let start = *cursor;
    let mut at = start;
    while let Some(&byte) = input.get(at) {
        if at >= MAX_HEADERS { return Err(Error::HeaderLimit); }
        if byte == b'\r' {
            let next = at.checked_add(1).ok_or(Error::HeaderLimit)?;
            match input.get(next) {
                None => return Err(Error::Truncated),
                Some(b'\n') => {},
                _ => return Err(Error::Malformed),
            }
            let end = next.checked_add(1).ok_or(Error::HeaderLimit)?;
            if end > MAX_HEADERS { return Err(Error::HeaderLimit); }
            *cursor = end;
            return Ok(&input[start..at]);
        }
        // HTAB is allowed only as field-value whitespace. Name/status checks
        // below restrict its position; obs-fold is rejected independently.
        if (byte < 0x20 && byte != b'\t') || byte >= 0x7F { return Err(Error::Malformed); }
        at += 1;
    }
    Err(if at >= MAX_HEADERS { Error::HeaderLimit } else { Error::Truncated })
}
fn trim(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.first(), Some(b' ' | b'\t')) { bytes = &bytes[1..]; }
    while matches!(bytes.last(), Some(b' ' | b'\t')) { bytes = &bytes[..bytes.len()-1]; }
    bytes
}
fn token(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&b| b.is_ascii_alphanumeric()
        || matches!(b, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+'
            | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'))
}
fn length(bytes: &[u8]) -> Result<usize, Error> {
    if bytes.is_empty() { return Err(Error::InvalidContentLength); }
    let mut value = 0usize;
    for &byte in bytes {
        if !byte.is_ascii_digit() { return Err(Error::InvalidContentLength); }
        value = value.checked_mul(10).and_then(|v| v.checked_add((byte-b'0') as usize))
            .ok_or(Error::InvalidContentLength)?;
    }
    if value > MAX_BODY { return Err(Error::BodyLimit); }
    Ok(value)
}
fn content_type(bytes: &[u8]) -> Result<(), Error> {
    let mut parts = bytes.split(|&b| b == b';');
    if !trim(parts.next().unwrap_or(&[])).eq_ignore_ascii_case(b"text/plain") {
        return Err(Error::UnsupportedContentType);
    }
    if let Some(parameter) = parts.next() {
        let equals = parameter.iter().position(|&b| b == b'=').ok_or(Error::UnsupportedContentType)?;
        if !trim(&parameter[..equals]).eq_ignore_ascii_case(b"charset")
            || !trim(&parameter[equals+1..]).eq_ignore_ascii_case(b"us-ascii") {
            return Err(Error::UnsupportedContentType);
        }
    }
    if parts.next().is_some() { return Err(Error::UnsupportedContentType); }
    Ok(())
}
fn resource_content_type(bytes:&[u8],resource:Resource)->Result<(),Error>{
    match resource {
        Resource::WelcomeText=>content_type(bytes),
        Resource::ArtworkQoi=>if bytes.eq_ignore_ascii_case(resource.mime()){Ok(())}else{Err(Error::UnsupportedContentType)},
    }
}

// Redirect bodies are discarded presentation data. Their optional MIME does
// not select a renderer; validate bounded media-type syntax without interpreting
// its type or charset. Quoted parameter bytes remain ordinary header data.
fn ignored_media_type(bytes:&[u8]) -> Result<(),Error> {
    let mut cursor=0;
    while cursor<bytes.len() && bytes[cursor]!=b';' {cursor+=1;}
    let main=trim(&bytes[..cursor]);
    let slash=main.iter().position(|byte|*byte==b'/').ok_or(Error::UnsupportedContentType)?;
    if !token(&main[..slash]) || !token(&main[slash+1..]) {return Err(Error::UnsupportedContentType);}
    while cursor<bytes.len() {
        cursor+=1;
        while matches!(bytes.get(cursor),Some(b' '|b'\t')) {cursor+=1;}
        let start=cursor;
        while cursor<bytes.len() && !matches!(bytes[cursor],b'='|b' '|b'\t') {cursor+=1;}
        if !token(&bytes[start..cursor]) {return Err(Error::UnsupportedContentType);}
        while matches!(bytes.get(cursor),Some(b' '|b'\t')) {cursor+=1;}
        if bytes.get(cursor)!=Some(&b'=') {return Err(Error::UnsupportedContentType);}
        cursor+=1;
        while matches!(bytes.get(cursor),Some(b' '|b'\t')) {cursor+=1;}
        if bytes.get(cursor)==Some(&b'"') {
            cursor+=1;let mut closed=false;
            while let Some(&byte)=bytes.get(cursor) {
                cursor+=1;
                if byte==b'"' {closed=true;break;}
                if byte==b'\\' {
                    if bytes.get(cursor).is_none() {return Err(Error::UnsupportedContentType);}
                    cursor+=1;
                }
            }
            if !closed {return Err(Error::UnsupportedContentType);}
        } else {
            let start=cursor;
            while cursor<bytes.len() && !matches!(bytes[cursor],b';'|b' '|b'\t') {cursor+=1;}
            if !token(&bytes[start..cursor]) {return Err(Error::UnsupportedContentType);}
        }
        while matches!(bytes.get(cursor),Some(b' '|b'\t')) {cursor+=1;}
        if cursor<bytes.len() && bytes[cursor]!=b';' {return Err(Error::UnsupportedContentType);}
    }
    Ok(())
}

/// Validate status and all fields through the first CRLFCRLF. This helper
/// returns framing metadata only, without claiming complete body receipt.
fn header_prefix(input: &[u8]) -> Result<Headers, Error> {
    resource_header_prefix(input,Resource::WelcomeText)
}
fn resource_header_prefix(input:&[u8],resource:Resource)->Result<Headers,Error>{
    let parsed=navigation_prefix(input,false,resource)?;
    Ok(Headers {header_len:parsed.header_len,content_length:parsed.content_length})
}
fn navigation_prefix(input: &[u8], navigation: bool, resource:Resource) -> Result<NavigationHeaders, Error> {
    let mut cursor = 0;
    let status = line(input, &mut cursor)?;
    if status.len() < 13 || &status[..9] != b"HTTP/1.1 " {
        return Err(if status.starts_with(b"HTTP/") { Error::UnsupportedVersion } else { Error::Malformed });
    }
    if !status[9..12].iter().all(u8::is_ascii_digit) || status[12] != b' '
        || status[13..].iter().any(|&b| b < 0x20) { return Err(Error::Malformed); }
    let code=((status[9]-b'0')as u16)*100+((status[10]-b'0')as u16)*10+(status[11]-b'0')as u16;
    let redirect_status=navigation&&matches!(code,301|302|303|307|308);
    if code!=200 && !redirect_status { return Err(Error::UnsupportedStatus); }
    let mut body_length = None;
    let mut has_type = false;
    let mut redirect = None;
    loop {
        let header = line(input, &mut cursor)?;
        if header.is_empty() { break; }
        if matches!(header.first(), Some(b' ' | b'\t')) { return Err(Error::Malformed); }
        let colon = header.iter().position(|&b| b == b':').ok_or(Error::Malformed)?;
        let name = &header[..colon];
        if !token(name) { return Err(Error::Malformed); }
        let value = trim(&header[colon+1..]);
        if name.eq_ignore_ascii_case(b"transfer-encoding") { return Err(Error::TransferEncoding); }
        if name.eq_ignore_ascii_case(b"content-encoding") { return Err(Error::UnsupportedEncoding); }
        if name.eq_ignore_ascii_case(b"content-length") {
            if body_length.is_some() { return Err(Error::DuplicateContentLength); }
            body_length = Some(length(value)?);
        } else if name.eq_ignore_ascii_case(b"content-type") {
            if has_type { return Err(Error::UnsupportedContentType); }
            if !redirect_status {resource_content_type(value,resource)?;}else{ignored_media_type(value)?;}
            has_type = true;
        } else if redirect_status && name.eq_ignore_ascii_case(b"location") {
            if redirect.is_some() {return Err(Error::DuplicateLocation);}
            redirect=Some(Path::from_location(value)?);
        }
    }
    let content_length = body_length.ok_or(Error::MissingContentLength)?;
    if redirect_status {
        if redirect.is_none() {return Err(Error::MissingLocation);}
        if content_length>MAX_REDIRECT_BODY {return Err(Error::BodyLimit);}
    } else if !has_type { return Err(Error::MissingContentType); }
    Ok(NavigationHeaders { header_len: cursor, content_length, status:code, redirect })
}

/// Parse an exact complete header extent without requiring a body copy.
/// Call only when CRLFCRLF has arrived; a header does not publish body data.
pub fn parse_headers(input: &[u8]) -> Result<Headers, Error> {
    parse_headers_resource(input,Resource::WelcomeText)
}
/// Exact resource-specific framing. This neither decodes an image nor grants
/// publication authority; all size, duplicate and encoding rules stay shared.
pub fn parse_headers_resource(input:&[u8],resource:Resource)->Result<Headers,Error>{
    if input.len() > MAX_HEADERS { return Err(Error::HeaderLimit); }
    let parsed=resource_header_prefix(input,resource)?;
    if parsed.header_len!=input.len() { return Err(Error::TrailingData); }
    Ok(parsed)
}

/// Exact navigation header parsing. This does not validate body receipt,
/// authorize follow-up I/O, count hops or publish a document. Legacy parsers
/// intentionally remain200-only. Redirect Content-Length is required and<=1024.
pub fn parse_navigation_headers(input:&[u8]) -> Result<NavigationHeaders,Error> {
    if input.len()>MAX_HEADERS {return Err(Error::HeaderLimit);}
    let parsed=navigation_prefix(input,true,Resource::WelcomeText)?;
    if parsed.header_len!=input.len() {return Err(Error::TrailingData);}
    Ok(parsed)
}

/// Complete-message convenience validation retains the same header restrictions.
pub fn parse(input: &[u8]) -> Result<Response, Error> {
    if input.len() > MAX_RESPONSE { return Err(Error::ResponseLimit); }
    let parsed=header_prefix(input)?;
    let end=parsed.header_len.checked_add(parsed.content_length).ok_or(Error::InvalidContentLength)?;
    if input.len() < end { return Err(Error::Truncated); }
    if input.len() > end { return Err(Error::TrailingData); }
    Ok(Response { body: parsed.header_len..end, content_length: parsed.content_length })
}

fn erase(bytes:&mut[u8]){for byte in bytes{unsafe{core::ptr::write_volatile(byte,0)}}}

/// Finite raw HTTP message custody. complete() means only that the declared
/// Content-Length has arrived. It proves no TLS, peer EOF, job release or UI
/// publication. The caller must finish once before borrowing/copying the body.
/// Bytes after that extent within any feed are rejected, never silently dropped.
pub struct Receiver{
    header:[u8;MAX_HEADERS],body:[u8;MAX_BODY],header_len:usize,body_len:usize,
    expected:Option<usize>,total:usize,error:Option<Error>,finished:bool,taken:bool,resource:Resource,
}
impl Receiver{
    pub const fn new()->Self{Self::for_resource(Resource::WelcomeText)}
    pub const fn for_resource(resource:Resource)->Self{Self{header:[0;MAX_HEADERS],body:[0;MAX_BODY],header_len:0,body_len:0,
        expected:None,total:0,error:None,finished:false,taken:false,resource}}
    pub fn failure(&self)->Option<Error>{self.error}
    pub fn expected_length(&self)->Option<usize>{self.expected}
    pub fn length(&self)->usize{self.body_len}
    pub fn total_received(&self)->usize{self.total}
    pub fn complete(&self)->bool{self.error.is_none()&&self.expected==Some(self.body_len)}
    fn wipe(&mut self){erase(&mut self.header);erase(&mut self.body);self.header_len=0;self.body_len=0;
        self.expected=None;self.total=0;self.finished=false;self.taken=false;}
    fn reject<T>(&mut self,error:Error)->Result<T,Error>{
        let first=*self.error.get_or_insert(error);self.wipe();Err(first)
    }
    /// Terminal local cancellation. Reuse requires a fresh Receiver; this does
    /// not reset an endpoint, replay fence or any external authority.
    pub fn scrub(&mut self){let _=self.reject::<()>(Error::State);}
    /// A failing call leaves no retained prefix and never exposes a body.
    /// Arbitrary nonempty/empty feed boundaries are accepted before finish.
    pub fn feed(&mut self,input:&[u8])->Result<(),Error>{
        if let Some(error)=self.error{return Err(error);}
        if self.finished||self.taken{return self.reject(if input.is_empty(){Error::State}else{Error::TrailingData});}
        let Some(total)=self.total.checked_add(input.len()).filter(|n|*n<=MAX_RESPONSE)else{return self.reject(Error::ResponseLimit);};
        for &byte in input{
            if let Some(expected)=self.expected{
                if self.body_len==expected{return self.reject(Error::TrailingData);}
                self.body[self.body_len]=byte;self.body_len+=1;
            }else{
                if self.header_len==MAX_HEADERS{return self.reject(Error::HeaderLimit);}
                self.header[self.header_len]=byte;self.header_len+=1;
                if self.header[..self.header_len].ends_with(b"\r\n\r\n"){
                    match parse_headers_resource(&self.header[..self.header_len],self.resource){
                        Ok(headers)=>self.expected=Some(headers.content_length),
                        Err(error)=>return self.reject(error),
                    }
                }else if self.header_len==MAX_HEADERS{return self.reject(Error::HeaderLimit);}
            }
        }
        self.total=total;Ok(())
    }
    /// Once-only framing completion; no EOF or transport assumption is made.
    pub fn finish(&mut self)->Result<usize,Error>{
        if let Some(error)=self.error{return Err(error);}
        if self.finished||self.taken{return self.reject(Error::State);}
        if !self.complete(){return self.reject(Error::Truncated);}
        self.finished=true;erase(&mut self.header);Ok(self.body_len)
    }
    /// Raw bytes remain caller-private data. Sanitization and publication after
    /// authenticated TLS and positive lease RELEASE belong to the parent.
    pub fn body(&self)->Option<&[u8]>{
        if self.finished&&!self.taken&&self.error.is_none(){Some(&self.body[..self.body_len])}else{None}
    }
    /// Copy exactly once. A short output fails before changing caller bytes and
    /// permanently wipes this receiver; success erases both retained arrays.
    pub fn copy_body(&mut self,out:&mut[u8])->Result<usize,Error>{
        if let Some(error)=self.error{return Err(error);}
        if !self.finished||self.taken{return self.reject(Error::State);}
        if out.len()<self.body_len{return self.reject(Error::OutputLimit);}
        let length=self.body_len;out[..length].copy_from_slice(&self.body[..length]);
        erase(&mut self.body);erase(&mut self.header);self.taken=true;Ok(length)
    }
}
impl Drop for Receiver{fn drop(&mut self){self.wipe();}}
const _:()=assert!(core::mem::size_of::<Receiver>()<=MAX_RESPONSE+128);

#[cfg(test)]
mod resource_tests {
    use super::*;
    use std::{format,vec,vec::Vec};
    fn response(fields:&str,body:&[u8])->Vec<u8>{
        let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{fields}\r\n",body.len()).into_bytes();bytes.extend_from_slice(body);bytes
    }
    fn wiped(r:&Receiver){assert!(r.header.iter().chain(r.body.iter()).all(|b|*b==0));assert!(r.body().is_none());}
    #[test] fn fixed_resource_request_selects_path_accept_and_same_boot_host(){
        for(resource,path,mime)in[(Resource::WelcomeText,b"/welcome".as_slice(),b"text/plain".as_slice()),(Resource::ArtworkQoi,b"/art.qoi",b"image/qoi")]{
            assert_eq!(resource.path().as_bytes(),path);assert_eq!(resource.mime(),mime);assert_eq!(Path::from_location(path).unwrap(),resource.path());
            let mut out=[0xa5;HTTPS_MAX_REQUEST];let len=request_https_resource(&mut out,resource);
            let expected=format!("GET {} HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: {}\r\n\r\n",core::str::from_utf8(path).unwrap(),core::str::from_utf8(mime).unwrap());
            assert_eq!(&out[..len],expected.as_bytes());assert!(out[len..].iter().all(|b|*b==0xa5));
            if resource==Resource::WelcomeText{let mut legacy=[0;HTTPS_MAX_REQUEST];let n=request_https(&mut legacy,&Path::initial());assert_eq!(&legacy[..n],&out[..len]);}
        }
    }
    #[test] fn qoi_profile_preserves_every_raw_byte_up_to_the_same_body_limit(){
        let body:Vec<u8>=(0..MAX_BODY).map(|i|(i%256)as u8).collect();let bytes=response("Content-Type: image/qoi\r\n",&body);
        for size in [1,4,511,512,MAX_RESPONSE]{
            let mut receiver=Receiver::for_resource(Resource::ArtworkQoi);
            for chunk in bytes.chunks(size){receiver.feed(chunk).unwrap();assert!(receiver.body().is_none());}
            assert!(receiver.complete());assert_eq!(receiver.finish(),Ok(MAX_BODY));assert_eq!(receiver.body(),Some(body.as_slice()));
            let mut out=vec![0xa5;MAX_BODY];assert_eq!(receiver.copy_body(&mut out),Ok(MAX_BODY));assert_eq!(out,body);wiped(&receiver);
        }
        let excessive=response("Content-Type: image/qoi\r\n",&vec![0;MAX_BODY+1]);let mut r=Receiver::for_resource(Resource::ArtworkQoi);assert_eq!(r.feed(&excessive),Err(Error::BodyLimit));wiped(&r);
    }
    #[test] fn resource_mime_is_strict_and_legacy_text_never_accepts_qoi(){
        for mime in ["text/plain","application/octet-stream","image/png","image/qoi; charset=us-ascii","image/qoi; x=y",""]{
            let mut r=Receiver::for_resource(Resource::ArtworkQoi);assert_eq!(r.feed(&response(&format!("Content-Type: {mime}\r\n"),b"binary")),Err(Error::UnsupportedContentType));wiped(&r);
        }
        let bytes=response("Content-Type: image/qoi\r\n",b"\x00\xff");let mut legacy=Receiver::new();assert_eq!(legacy.feed(&bytes),Err(Error::UnsupportedContentType));wiped(&legacy);
        assert_eq!(parse(&bytes),Err(Error::UnsupportedContentType));
        let mut r=Receiver::for_resource(Resource::ArtworkQoi);r.feed(&response("cOnTeNt-TyPe:\tIMAGE/QOI \t\r\n",b"\0\xff")).unwrap();assert_eq!(r.finish(),Ok(2));assert_eq!(r.body(),Some(b"\0\xff".as_slice()));
    }
    #[test] fn binary_profile_preserves_framing_rejections_and_erases_on_truncation_or_trailing(){
        for(fields,error)in[("Content-Type: image/qoi\r\nContent-Type: image/qoi\r\n",Error::UnsupportedContentType),("Content-Type: image/qoi\r\nContent-Length: 4\r\n",Error::DuplicateContentLength),("Content-Type: image/qoi\r\nTransfer-Encoding: chunked\r\n",Error::TransferEncoding),("Content-Type: image/qoi\r\nContent-Encoding: identity\r\n",Error::UnsupportedEncoding),("",Error::MissingContentType)]{
            let mut r=Receiver::for_resource(Resource::ArtworkQoi);assert_eq!(r.feed(&response(fields,b"BODY")),Err(error));wiped(&r);
        }
        let bytes=response("Content-Type: image/qoi\r\n",b"qoif\0\0\0\x30\0\0\0\x30\x04\0\xff\0\xff");
        for cut in 0..bytes.len(){let mut r=Receiver::for_resource(Resource::ArtworkQoi);r.feed(&bytes[..cut]).unwrap();assert_eq!(r.finish(),Err(Error::Truncated));wiped(&r);}
        for split in [0,bytes.len()-1,bytes.len()]{let mut all=bytes.clone();all.push(0xff);let mut r=Receiver::for_resource(Resource::ArtworkQoi);r.feed(&all[..split]).unwrap();assert_eq!(r.feed(&all[split..]),Err(Error::TrailingData));wiped(&r);}
        let mut r=Receiver::for_resource(Resource::ArtworkQoi);r.feed(&bytes).unwrap();r.finish().unwrap();assert_eq!(r.feed(&[0]),Err(Error::TrailingData));wiped(&r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, vec, vec::Vec};
    fn response(body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);bytes
    }
    fn message(headers: &str, body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 200 OK\r\n{headers}\r\n").into_bytes();bytes.extend_from_slice(body);bytes
    }
    #[test] fn actual_fixed_request_bytes() {
        let mut out = [0xA5; MAX_REQUEST];let len = request(&mut out);
        assert_eq!(&out[..len], b"GET /welcome HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n");
        assert!(out[len..].iter().all(|b| *b == 0xA5));
    }
    #[test] fn complete_200_exposes_exact_body_only_after_full_receipt() {
        let bytes = response(b"controlled peer body\n");let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.content_length, 21);assert_eq!(&bytes[parsed.body], b"controlled peer body\n");
    }
    #[test] fn every_truncation_cut_fails_without_body_publication() {
        for body in [b"real response".as_slice(), b"".as_slice()] {
            let bytes = response(body);
            for cut in 0..bytes.len() { assert!(parse(&bytes[..cut]).is_err(), "cut={cut}"); }
        }
    }
    #[test] fn empty_body_has_explicit_zero_length_range() {
        let bytes = response(b"");let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.content_length, 0);assert_eq!(parsed.body, bytes.len()..bytes.len());
    }
    #[test] fn header_names_mime_and_ascii_charset_are_case_insensitive() {
        let bytes = message("cOnTeNt-LeNgTh:\t 0004 \t\r\ncOnTeNt-TyPe: TeXt/PlAiN ; Charset = US-ASCII\r\nX-Test: valid\tvalue\r\n", b"body");
        let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], b"body");
    }
    #[test] fn duplicates_conflicting_lengths_and_any_transfer_encoding_rejected() {
        for duplicate in ["4", "5"] {
            assert_eq!(parse(&message(&format!("Content-Length: 4\r\ncontent-length: {duplicate}\r\nContent-Type: text/plain\r\n"), b"body")), Err(Error::DuplicateContentLength));
        }
        for encoding in ["", "identity", "chunked", "gzip, chunked"] {
            let bytes = message(&format!("Content-Length: 4\r\nContent-Type: text/plain\r\nTransfer-Encoding: {encoding}\r\n"), b"body");
            assert_eq!(parse(&bytes), Err(Error::TransferEncoding));
        }
    }
    #[test] fn invalid_decimal_or_overflow_never_becomes_a_body_range() {
        for value in ["", "-1", "+4", "4,4", "4 4", "0x4", "4a", "999999999999999999999999999999999999999999999999"] {
            assert_eq!(parse(&message(&format!("Content-Length: {value}\r\nContent-Type: text/plain\r\n"), b"body")), Err(Error::InvalidContentLength), "{value}");
        }
        assert_eq!(parse(&message(&format!("Content-Length: {}\r\nContent-Type: text/plain\r\n",MAX_BODY+1), b"")), Err(Error::BodyLimit));
        assert_eq!(parse(&message("Content-Type: text/plain\r\n", b"")), Err(Error::MissingContentLength));
    }
    #[test] fn malformed_crlf_controls_names_and_obs_fold_rejected() {
        for bytes in [
            b"HTTP/1.1 200 OK\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\rX".as_slice(),
            b"HTTP/1.1 200 OK\r\n Bad: fold\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\n\tBad: fold\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length : 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad@Name: value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\0y\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\x7Fy\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\x80y\r\n\r\n".as_slice(),
        ] { assert_eq!(parse(bytes), Err(Error::Malformed)); }
    }
    #[test] fn unsupported_status_version_and_content_types_rejected() {
        for status in ["301 Moved", "302 Found", "204 Empty", "404 Missing", "500 Error"] {
            let bytes = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nContent-Type: text/plain\r\n\r\n");
            assert_eq!(parse(bytes.as_bytes()), Err(Error::UnsupportedStatus));
        }
        assert_eq!(parse(b"HTTP/1.0 200 OK\r\n\r\n"), Err(Error::UnsupportedVersion));
        for mime in ["text/html", "application/json", "text/plain; charset=utf-8", "text/plain; charset=\"us-ascii\"", "text/plain; charset=us-ascii; x=y", "text/plain;"] {
            assert_eq!(parse(&message(&format!("Content-Length: 0\r\nContent-Type: {mime}\r\n"), b"")), Err(Error::UnsupportedContentType));
        }
        assert_eq!(parse(&message("Content-Length: 0\r\n", b"")), Err(Error::MissingContentType));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\nContent-Type: text/plain\r\n", b"")), Err(Error::UnsupportedContentType));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\n", b"")), Err(Error::UnsupportedEncoding));
    }
    #[test] fn maximum_body_and_exact_header_limit_are_accepted_but_excess_is_rejected() {
        let body = vec![0xA5; MAX_BODY];let bytes = response(&body);let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], body);
        let base = "Content-Length: 0\r\nContent-Type: text/plain\r\nX-Pad: ";
        let overhead = message(base, b"").len()+2;
        let headers = format!("{base}{}\r\n", "a".repeat(MAX_HEADERS-overhead));
        let bytes = message(&headers, b"");assert_eq!(bytes.len(), MAX_HEADERS);assert!(parse(&bytes).is_ok());
        let bytes = message(&format!("{base}{}\r\n", "a".repeat(MAX_HEADERS-overhead+1)), b"");assert_eq!(parse(&bytes), Err(Error::HeaderLimit));
        assert_eq!(parse(&vec![b'A'; MAX_RESPONSE+1]), Err(Error::ResponseLimit));
    }
    #[test] fn trailing_data_and_body_length_mismatch_rejected() {
        let mut bytes = response(b"body");bytes.push(b'!');assert_eq!(parse(&bytes), Err(Error::TrailingData));
        let mut bytes = response(b"body");bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\n\r\n");assert_eq!(parse(&bytes), Err(Error::TrailingData));
        assert_eq!(parse(&message("Content-Length: 5\r\nContent-Type: text/plain\r\n", b"body")), Err(Error::Truncated));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\n", b"x")), Err(Error::TrailingData));
    }
    #[test] fn malformed_later_header_cannot_publish_an_earlier_valid_body_extent() {
        assert_eq!(parse(&message("Content-Length: 4\r\nContent-Type: text/plain\r\nBroken later header\r\n", b"body")), Err(Error::Malformed));
        let bytes = response(&[0, 0xFF, b'\r', b'\n']);let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], &[0, 0xFF, b'\r', b'\n']);
    }
    #[test] fn headers_can_arrive_at_every_four_byte_boundary_without_a_body_buffer() {
        let full=response(&vec![b'D';MAX_BODY]);let end=full.windows(4).position(|w|w==b"\r\n\r\n").unwrap()+4;
        let mut retained=[0;MAX_HEADERS];let mut have=0;
        for chunk in full[..end].chunks(4) {
            retained[have..have+chunk.len()].copy_from_slice(chunk);have+=chunk.len();
            if have<end { assert_eq!(parse_headers(&retained[..have]),Err(Error::Truncated)); }
        }
        assert_eq!(parse_headers(&retained[..have]),Ok(Headers {header_len:end,content_length:MAX_BODY}));
        assert_eq!(parse_headers(&full[..end+1]),Err(Error::TrailingData));
    }
    #[test] fn every_header_cut_and_all_existing_restrictions_apply_before_streamed_body() {
        let full=response(b"abc");let end=full.len()-3;
        for cut in 0..end {assert!(parse_headers(&full[..cut]).is_err(),"cut={cut}");}
        for fields in ["Content-Length: 1\r\nContent-Length: 1\r\nContent-Type: text/plain\r\n", "Content-Length: 1\r\nContent-Type: text/html\r\n", "Content-Length: 1\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n", "Content-Length: 1\r\nContent-Type: text/plain\r\nContent-Encoding: identity\r\n"] {
            assert!(parse_headers(&message(fields,b"")).is_err());
        }
        assert_eq!(parse_headers(&vec![b'A';MAX_HEADERS+1]),Err(Error::HeaderLimit));
        assert_eq!(MAX_RESPONSE,MAX_HEADERS+MAX_BODY);assert_eq!(MAX_BODY,12288);
    }

    fn redirect_headers(status:u16,fields:&str)->Vec<u8> {
        format!("HTTP/1.1 {status} Redirect\r\n{fields}\r\n").into_bytes()
    }
    #[test] fn canonical_path_is_private_bounded_data_and_initial_request_is_unchanged() {
        assert_eq!(Path::initial().as_bytes(),b"/welcome");
        for target in [b"/".as_slice(),b"/guide/",b"/guide/text-1.2_~",b"/.../file..txt"] {
            let path=Path::from_location(target).unwrap();assert_eq!(path.as_bytes(),target);
            let mut out=[0xA5;MAX_REQUEST];let count=request_path(&mut out,&path);
            let expected=format!("GET {} HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n",core::str::from_utf8(target).unwrap());
            assert_eq!(&out[..count],expected.as_bytes());assert!(out[count..].iter().all(|byte|*byte==0xA5));
        }
        let mut initial=[0;MAX_REQUEST];let count=request_path(&mut initial,&Path::initial());assert_eq!(&initial[..count],REQUEST);
        let mut longest=[b'a';MAX_PATH];longest[0]=b'/';let path=Path::from_location(&longest).unwrap();
        assert_eq!(request_path(&mut initial,&path),MAX_REQUEST);
        let mut excessive=[b'a';MAX_PATH+1];excessive[0]=b'/';assert_eq!(Path::from_location(&excessive),Err(Error::InvalidLocation));
    }
    #[test] fn location_denies_authority_changes_aliases_and_injection() {
        for target in [b"".as_slice(),b"guide",b"http://probe.sexos.test/guide",b"https://probe.sexos.test/guide",b"http://probe.sexos.test:46078/guide",b"http://other.test/guide",b"//probe.sexos.test/guide",b"/guide//text",b"/./guide",b"/guide/../text",b"/guide/.",b"/..",b"/%2e%2e/guide",b"/a?b",b"/a#b",b"/a\\b",b"/a:b",b"/a@b",b"/a b",b"/a\tb",b"/a\r\nHost: other",b"/\0",b"/\x7f",b"/\xff"] {
            assert_eq!(Path::from_location(target),Err(Error::InvalidLocation),"{target:?}");
        }
    }
    #[test] fn navigation_accepts_only_five_get_redirect_statuses_without_changing_legacy() {
        for status in [301,302,303,307,308] {
            let bytes=redirect_headers(status,"Location: /guide/text\r\nContent-Length: 0\r\n");
            assert_eq!(parse_navigation_headers(&bytes),Ok(NavigationHeaders {header_len:bytes.len(),content_length:0,status,redirect:Some(Path::from_location(b"/guide/text").unwrap())}));
            assert_eq!(parse_headers(&bytes),Err(Error::UnsupportedStatus));assert_eq!(parse(&bytes),Err(Error::UnsupportedStatus));
        }
        for status in [100,201,204,300,304,305,306,309,404,500] {
            assert_eq!(parse_navigation_headers(&redirect_headers(status,"Location: /guide\r\nContent-Length: 0\r\n")),Err(Error::UnsupportedStatus));
        }
    }
    #[test] fn navigation_200_preserves_mime_body_and_unknown_location_semantics() {
        let bytes=message("Content-Length: 3\r\nContent-Type: text/plain; charset=us-ascii\r\nLocation: https://untrusted.invalid/\r\n",b"");
        assert_eq!(parse_navigation_headers(&bytes),Ok(NavigationHeaders {header_len:bytes.len(),content_length:3,status:200,redirect:None}));
        for fields in ["Content-Length: 0\r\n", "Content-Length: 0\r\nContent-Type: text/html\r\n", "Content-Length: 0\r\nContent-Type: text/plain; charset=utf-8\r\n"] {
            let bytes=message(fields,b"");assert_eq!(parse_navigation_headers(&bytes).unwrap_err(),parse_headers(&bytes).unwrap_err());
        }
        let bytes=message(&format!("Content-Length: {MAX_BODY}\r\nContent-Type: text/plain\r\n"),b"");assert_eq!(parse_navigation_headers(&bytes).unwrap().content_length,MAX_BODY);
    }
    #[test] fn redirect_requires_one_valid_location_and_one_bounded_content_length() {
        for(fields,error)in [
            ("Content-Length: 0\r\n",Error::MissingLocation),
            ("Location: /guide\r\n",Error::MissingContentLength),
            ("Location: /guide\r\nContent-Length: 0\r\nlocation: /guide\r\n",Error::DuplicateLocation),
            ("Location: /guide\r\nContent-Length: 0\r\nLocation: /other\r\n",Error::DuplicateLocation),
            ("Location: \r\nContent-Length: 0\r\n",Error::InvalidLocation),
            ("Location: //other/\r\nContent-Length: 0\r\n",Error::InvalidLocation),
            ("Location: /guide\r\nContent-Length: 0\r\nContent-Length: 0\r\n",Error::DuplicateContentLength),
            ("Location: /guide\r\nContent-Length: -1\r\n",Error::InvalidContentLength),
            ("Location: /guide\r\nContent-Length: 1025\r\n",Error::BodyLimit),
        ] {assert_eq!(parse_navigation_headers(&redirect_headers(302,fields)),Err(error),"{fields}");}
        let bytes=redirect_headers(302,"lOcAtIoN:\t /guide \t\r\nContent-Length: 001024\r\n");assert_eq!(parse_navigation_headers(&bytes).unwrap().content_length,MAX_REDIRECT_BODY);
    }
    #[test] fn redirect_mime_is_optional_syntax_checked_data_without_renderer_selection() {
        for mime in ["text/html","application/octet-stream","text/plain; charset=utf-8","text/html ; charset = \"utf-8\"; x=\"a;b\"","text/plain; x=\"a\\\"b\""] {
            let bytes=redirect_headers(307,&format!("Location: /guide\r\nContent-Length: 12\r\nContent-Type: {mime}\r\n"));assert!(parse_navigation_headers(&bytes).is_ok(),"{mime}");
        }
        for mime in ["","text","/plain","text/","text/html/extra","text html/plain","text/html;","text/html; charset","text/html; charset=","text/html; charset=\"broken","text/html; charset=\"utf-8\"junk"] {
            let bytes=redirect_headers(307,&format!("Location: /guide\r\nContent-Length: 0\r\nContent-Type: {mime}\r\n"));assert_eq!(parse_navigation_headers(&bytes),Err(Error::UnsupportedContentType),"{mime}");
        }
        let bytes=redirect_headers(307,"Location: /guide\r\nContent-Length: 0\r\nContent-Type: text/html\r\nContent-Type: text/html\r\n");assert_eq!(parse_navigation_headers(&bytes),Err(Error::UnsupportedContentType));
    }
    #[test] fn navigation_never_accepts_transfer_or_content_encoding_and_late_bad_headers() {
        for(field,error)in [("Transfer-Encoding: chunked\r\n",Error::TransferEncoding),("Content-Encoding: identity\r\n",Error::UnsupportedEncoding),(" Broken: fold\r\n",Error::Malformed),("Broken\r\n",Error::Malformed),("Location: /guide\r\n",Error::DuplicateLocation)] {
            let bytes=redirect_headers(308,&format!("Location: /guide\r\nContent-Length: 0\r\n{field}"));assert_eq!(parse_navigation_headers(&bytes),Err(error));
        }
    }
    #[test] fn navigation_every_header_cut_and_turn_boundary_cannot_publish_early_metadata() {
        let bytes=redirect_headers(303,"Content-Length: 1024\r\nLocation: /guide/text\r\nContent-Type: text/html; charset=us-ascii\r\n");
        for cut in 0..bytes.len(){assert!(parse_navigation_headers(&bytes[..cut]).is_err(),"{cut}");}
        for split in 0..4 {
            let mut retained=[0;MAX_HEADERS];let mut have=0;
            for chunk in bytes[..split].chunks(4).chain(bytes[split..].chunks(4)) {
                retained[have..have+chunk.len()].copy_from_slice(chunk);have+=chunk.len();
                if have<bytes.len(){assert!(parse_navigation_headers(&retained[..have]).is_err());}
            }
            assert_eq!(parse_navigation_headers(&retained[..have]).unwrap().redirect.unwrap().as_bytes(),b"/guide/text");
        }
        let mut body=bytes.clone();body.push(b'x');assert_eq!(parse_navigation_headers(&body),Err(Error::TrailingData));
    }
    #[test] fn navigation_exact_header_bound_and_version_controls_stay_strict() {
        let base="Location: /guide\r\nContent-Length: 0\r\nX-Pad: ";let overhead=redirect_headers(302,base).len()+2;
        let exact=redirect_headers(302,&format!("{base}{}\r\n","a".repeat(MAX_HEADERS-overhead)));assert_eq!(exact.len(),MAX_HEADERS);assert!(parse_navigation_headers(&exact).is_ok());
        let excess=redirect_headers(302,&format!("{base}{}\r\n","a".repeat(MAX_HEADERS-overhead+1)));assert_eq!(parse_navigation_headers(&excess),Err(Error::HeaderLimit));
        assert_eq!(parse_navigation_headers(b"HTTP/1.0 302 Found\r\nContent-Length: 0\r\nLocation: /guide\r\n\r\n"),Err(Error::UnsupportedVersion));
        for value in [b"/a\0".as_slice(),b"/a\x7f",b"/a\xff",b"/a\rX"] {
            let mut bytes=b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nLocation: ".to_vec();bytes.extend_from_slice(value);bytes.extend_from_slice(b"\r\n\r\n");assert_eq!(parse_navigation_headers(&bytes),Err(Error::Malformed));
        }
    }

}

#[cfg(test)]
mod receiver_tests{
    use super::*;
    use std::{format,vec,vec::Vec};
    fn response(body:&[u8])->Vec<u8>{let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",body.len()).into_bytes();bytes.extend_from_slice(body);bytes}
    fn wiped(r:&Receiver){assert!(r.header.iter().chain(r.body.iter()).all(|b|*b==0));assert!(r.body().is_none());}
    #[test]fn https_request_uses_only_fixed_boot_host_and_tls_port(){
        let mut out=[0xa5;HTTPS_MAX_REQUEST];let n=request_https(&mut out,&Path::initial());
        assert_eq!(&out[..n],b"GET /welcome HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/plain\r\n\r\n");assert_eq!(sex_net_config::TLS_PORT,46080);assert_eq!(sex_net_config::HOST_NAME,b"probe.sexos.test");assert!(out[n..].iter().all(|b|*b==0xa5));
        let mut longest=[b'a';MAX_PATH];longest[0]=b'/';assert_eq!(request_https(&mut out,&Path::from_location(&longest).unwrap()),HTTPS_MAX_REQUEST);
        let mut plain=[0;MAX_REQUEST];let len=request(&mut plain);assert_eq!(&plain[..len],REQUEST);assert!(!plain[..len].windows(6).any(|b|b==b":46080"));
    }
    #[test]fn all_feed_splits_keep_raw_body_private_until_once_only_finish_and_copy(){
        let body=b" raw\n\x00\xff ";let wire=response(body);
        for split in 0..=wire.len(){let mut r=Receiver::new();assert_eq!(r.body(),None);r.feed(&wire[..split]).unwrap();assert!(r.body().is_none());r.feed(&wire[split..]).unwrap();
            assert!(r.complete());assert_eq!(r.expected_length(),Some(body.len()));assert_eq!(r.length(),body.len());assert_eq!(r.total_received(),wire.len());assert!(r.body().is_none());assert_eq!(r.finish(),Ok(body.len()));assert_eq!(r.body(),Some(body.as_slice()));
            let mut out=[0xa5;32];assert_eq!(r.copy_body(&mut out),Ok(body.len()));assert_eq!(&out[..body.len()],body);assert!(out[body.len()..].iter().all(|b|*b==0xa5));wiped(&r);assert_eq!(r.copy_body(&mut out),Err(Error::State));wiped(&r);
        }
    }
    #[test]fn arbitrary_byte_and_larger_chunk_boundaries_preserve_full_maximum_body(){
        let body:Vec<u8>=(0..MAX_BODY).map(|n|(n%256)as u8).collect();let wire=response(&body);
        for chunk in [1,2,3,4,7,511,512,1024,MAX_RESPONSE]{let mut r=Receiver::new();for part in wire.chunks(chunk){r.feed(part).unwrap();assert!(r.body().is_none());}assert!(r.complete());assert_eq!(r.finish(),Ok(MAX_BODY));let mut out=[0;MAX_BODY];assert_eq!(r.copy_body(&mut out),Ok(MAX_BODY));assert_eq!(out.as_slice(),body);wiped(&r);}
    }
    #[test]fn every_truncation_cut_fails_closed_and_cannot_resume_or_publish(){
        for body in [b"body".as_slice(),b"".as_slice()]{let wire=response(body);for cut in 0..wire.len(){let mut r=Receiver::new();r.feed(&wire[..cut]).unwrap();assert!(!r.complete());assert_eq!(r.finish(),Err(Error::Truncated));wiped(&r);assert_eq!(r.feed(&wire[cut..]),Err(Error::Truncated));assert_eq!(r.finish(),Err(Error::Truncated));}}
    }
    #[test]fn zero_body_still_requires_valid_headers_and_explicit_finish(){
        let mut r=Receiver::new();r.feed(&response(b"")).unwrap();assert!(r.complete());assert!(r.body().is_none());assert_eq!(r.finish(),Ok(0));assert_eq!(r.body(),Some(b"".as_slice()));assert_eq!(r.copy_body(&mut[]),Ok(0));wiped(&r);
    }
    #[test]fn malformed_duplicate_encoded_and_unsupported_headers_match_shared_parser(){
        for wire in[
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\nContent-Encoding: identity\r\n\r\n",
            b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\n\r\n",
            b"HTTP/1.1 302 Moved\r\nContent-Length: 0\r\nLocation: /guide\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/html\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\n Bad: fold\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\nBad: \0\r\n\r\n",
        ]{let want=parse_headers(wire).unwrap_err();for chunk in [1,4,MAX_HEADERS]{let mut r=Receiver::new();let mut failure=None;for part in wire.chunks(chunk){if let Err(e)=r.feed(part){failure=Some(e);break;}}assert_eq!(failure,Some(want));assert_eq!(r.failure(),Some(want));assert!(!r.complete());wiped(&r);assert_eq!(r.feed(b"HTTP/1.1 200 OK\r\n\r\n"),Err(want));}}
    }
    #[test]fn trailing_data_in_same_later_or_finished_feed_erases_the_entire_prefix(){
        let wire=response(b"body");for split in [0,wire.len()-1,wire.len()]{let mut all=wire.clone();all.extend_from_slice(b"extra");let mut r=Receiver::new();r.feed(&all[..split]).unwrap();assert_eq!(r.feed(&all[split..]),Err(Error::TrailingData));wiped(&r);assert!(!r.complete());assert_eq!(r.finish(),Err(Error::TrailingData));}
        let mut r=Receiver::new();r.feed(&wire).unwrap();r.finish().unwrap();assert_eq!(r.body(),Some(b"body".as_slice()));assert_eq!(r.feed(b"extra"),Err(Error::TrailingData));wiped(&r);
    }
    #[test]fn repeated_finish_and_output_too_small_are_terminal_without_partial_output(){
        let wire=response(b"body");let mut r=Receiver::new();r.feed(&wire).unwrap();r.finish().unwrap();assert_eq!(r.finish(),Err(Error::State));wiped(&r);
        let mut r=Receiver::new();r.feed(&wire).unwrap();r.finish().unwrap();let mut out=[0xa5;3];assert_eq!(r.copy_body(&mut out),Err(Error::OutputLimit));assert_eq!(out,[0xa5;3]);wiped(&r);assert_eq!(r.copy_body(&mut[0;4]),Err(Error::OutputLimit));
    }
    #[test]fn premature_copy_never_exposes_a_complete_or_partial_unfinished_body(){
        let wire=response(b"body");for end in [wire.len()-1,wire.len()]{let mut r=Receiver::new();r.feed(&wire[..end]).unwrap();let mut out=[0xa5;8];assert_eq!(r.copy_body(&mut out),Err(Error::State));assert_eq!(out,[0xa5;8]);wiped(&r);}
    }
    #[test]fn header_body_and_total_overflow_permanently_clear_retained_bytes(){
        let mut r=Receiver::new();assert_eq!(r.feed(&vec![b'A';MAX_HEADERS]),Err(Error::HeaderLimit));wiped(&r);
        let mut r=Receiver::new();assert_eq!(r.feed(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",MAX_BODY+1).as_bytes()),Err(Error::BodyLimit));wiped(&r);
        let mut r=Receiver::new();assert_eq!(r.feed(&vec![b'A';MAX_RESPONSE+1]),Err(Error::ResponseLimit));wiped(&r);
    }
    #[test]fn exact_header_bound_and_max_body_can_share_a_feed(){
        let mut header=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nX-Pad: ",MAX_BODY).into_bytes();header.resize(MAX_HEADERS-4,b'a');header.extend_from_slice(b"\r\n\r\n");assert_eq!(header.len(),MAX_HEADERS);assert!(parse_headers(&header).is_ok());
        let mut wire=header;wire.extend_from_slice(&vec![0x97;MAX_BODY]);assert_eq!(wire.len(),MAX_RESPONSE);let mut r=Receiver::new();r.feed(&wire).unwrap();assert!(r.complete());r.finish().unwrap();assert_eq!(r.body(),Some(vec![0x97;MAX_BODY].as_slice()));r.scrub();wiped(&r);assert!(!r.complete());assert_eq!(r.failure(),Some(Error::State));
    }
}
