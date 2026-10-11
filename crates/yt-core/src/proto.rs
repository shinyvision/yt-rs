//! Builds the small protobuf messages YouTube uses as opaque `params` and
//! action tokens.
//!
//! Most of these come in responses, but the TV client never receives the
//! ones for writing comments (the TV app sends viewers to their phone for
//! that), so the app builds them itself, in the format YouTube's own tokens
//! use: URL-safe base64 with the padding percent-encoded.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;

#[derive(Clone, Debug, Default)]
pub(crate) struct Message(Vec<u8>);

#[yt_export::export(skip = "a protobuf builder; tokens are opaque strings to plugins")]
impl Message {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn varint(mut self, field: u32, value: u64) -> Self {
        self.key(field, 0);
        self.raw_varint(value);
        self
    }

    pub fn string(self, field: u32, value: &str) -> Self {
        self.bytes(field, value.as_bytes())
    }

    pub fn message(self, field: u32, value: Message) -> Self {
        self.bytes(field, &value.0)
    }

    fn bytes(mut self, field: u32, value: &[u8]) -> Self {
        self.key(field, 2);
        self.raw_varint(value.len() as u64);
        self.0.extend_from_slice(value);
        self
    }

    fn key(&mut self, field: u32, wire_type: u8) {
        self.raw_varint((u64::from(field) << 3) | u64::from(wire_type));
    }

    fn raw_varint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.0.push(byte);
                return;
            }
            self.0.push(byte | 0x80);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The message as YouTube writes tokens.
    pub fn to_token(&self) -> String {
        URL_SAFE.encode(&self.0).replace('=', "%3D")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_youtube_tokens() {
        // The create-comment params YouTube's TV JS builds for a video.
        let params = Message::new()
            .string(2, "Gp3BRbNSLEA")
            .message(5, Message::new())
            .varint(10, 7);
        assert_eq!(params.to_token(), "EgtHcDNCUmJOU0xFQSoAUAc%3D");
    }

    #[test]
    fn large_field_numbers_and_values() {
        let m = Message::new().varint(23, 300);
        assert_eq!(m.0, vec![0xb8, 0x01, 0xac, 0x02]);
    }
}
