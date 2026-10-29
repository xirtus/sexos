//! Fixed-size, newline-based preview. The service may return a cached response
//! or its built-in sample; the current ABI does not distinguish their origin.
pub const CAPACITY: usize = 192;
const CONTENT_LIMIT: usize = 96;
pub struct Preview {
    bytes: [u8; CAPACITY],
    len: usize,
}
impl Preview {
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
    fn append(&mut self, text: &[u8]) {
        let count = text.len().min(CAPACITY - self.len);
        self.bytes[self.len..self.len + count].copy_from_slice(&text[..count]);
        self.len += count;
    }
}
// Preserve printable ASCII; collapse whitespace/control bytes so response data
// cannot replace the title/status layout. Unsupported glyph bytes become '?'.
fn clean(input: &[u8], output: &mut [u8; CONTENT_LIMIT]) -> usize {
    let mut len = 0;
    let mut space = false;
    for &byte in input {
        let byte = match byte { 0..=32 | 127 => b' ', 33..=126 => byte, _ => b'?' };
        if byte == b' ' { space = len != 0; continue; }
        let needed = 1 + usize::from(space);
        if len + needed > CONTENT_LIMIT {
            output[CONTENT_LIMIT - 3..].copy_from_slice(b"...");
            return CONTENT_LIMIT;
        }
        if space { output[len] = b' '; len += 1; space = false; }
        output[len] = byte;
        len += 1;
    }
    len
}
pub fn build(body: &[u8], network_summary: &[u8]) -> Preview {
    let mut preview = Preview { bytes: [0; CAPACITY], len: 0 };
    preview.append(b"Kaleidoscope\nCached / sample network text\n\n");
    let mut content = [0; CONTENT_LIMIT];
    let mut len = clean(body, &mut content);
    if len == 0 { len = clean(network_summary, &mut content); }
    if len == 0 { preview.append(b"No text available."); }
    else { preview.append(&content[..len]); }
    preview
}
