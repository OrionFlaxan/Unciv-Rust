//! Gzip + Base64 save-file codec.
//!
//! Unciv stores saves as `base64(gzip(json))` (see `FileConversions.kt`). On a Huge map past
//! turn 300 the JSON easily reaches tens of megabytes, and doing this through Java streams
//! on a phone causes multi-second autosave stalls plus large GC pressure. This module does the
//! whole transform in one native pass with a pure-Rust zlib implementation.
//!
//! Output is a standard RFC 1952 gzip stream encoded with the RFC 4648 standard alphabet
//! (with padding), so it is readable by `GZIPInputStream` + `Base64.getDecoder()` and vice versa.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// Compress `input` with gzip and encode the result as base64 text bytes.
pub fn gzip_base64_encode(input: &[u8], level: u32) -> Vec<u8> {
    let level = level.min(9);
    // Compressed JSON is typically 5-15% of the original size.
    let mut encoder = GzEncoder::new(Vec::with_capacity(input.len() / 6 + 64), Compression::new(level));
    encoder.write_all(input).expect("writing to a Vec cannot fail");
    let compressed = encoder.finish().expect("writing to a Vec cannot fail");
    let mut out = Vec::with_capacity(base64::encoded_len(compressed.len(), true).unwrap_or(0));
    out.resize(base64::encoded_len(compressed.len(), true).unwrap_or(0), 0);
    let written = STANDARD
        .encode_slice(&compressed, &mut out)
        .expect("buffer sized with encoded_len");
    out.truncate(written);
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Base64,
    Gzip,
}

/// Decode base64 text bytes and gunzip them. Whitespace (e.g. line breaks inserted by
/// MIME-style encoders) is ignored, matching the leniency of `Base64.getMimeDecoder()`.
pub fn gzip_base64_decode(input: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let has_ws = input.iter().any(|b| b.is_ascii_whitespace());
    let compressed = if has_ws {
        let cleaned: Vec<u8> = input.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        STANDARD.decode(cleaned).map_err(|_| DecodeError::Base64)?
    } else {
        STANDARD.decode(input).map_err(|_| DecodeError::Base64)?
    };
    let mut decoder = MultiGzDecoder::new(&compressed[..]);
    let mut out = Vec::with_capacity(compressed.len() * 8);
    decoder.read_to_end(&mut out).map_err(|_| DecodeError::Gzip)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small_and_large() {
        for text in [
            String::new(),
            "{}".to_string(),
            "{\"version\":4,\"turns\":350}".repeat(1),
            "{\"tile\":{\"x\":1,\"y\":-3,\"exploredBy\":[\"Rome\",\"Babylon\"]}},".repeat(50_000),
        ] {
            for level in [1, 6, 9] {
                let enc = gzip_base64_encode(text.as_bytes(), level);
                assert!(enc.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/' || *b == b'='));
                let dec = gzip_base64_decode(&enc).unwrap();
                assert_eq!(dec, text.as_bytes());
            }
        }
    }

    #[test]
    fn decodes_with_line_breaks() {
        let enc = gzip_base64_encode(b"hello unciv", 6);
        let mut wrapped = Vec::new();
        for (i, b) in enc.iter().enumerate() {
            if i > 0 && i % 4 == 0 {
                wrapped.extend_from_slice(b"\r\n");
            }
            wrapped.push(*b);
        }
        assert_eq!(gzip_base64_decode(&wrapped).unwrap(), b"hello unciv");
    }

    #[test]
    fn decodes_java_gzip_output() {
        // `Base64.getEncoder().encodeToString(gzip("Unciv"))`
        let java = b"H4sIAJY1AmsC/wvNS84sAwAIRuCqBQAAAA==";
        assert_eq!(gzip_base64_decode(java).unwrap(), b"Unciv");
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(gzip_base64_decode(b"!!!not base64!!!"), Err(DecodeError::Base64));
        assert_eq!(gzip_base64_decode(b"aGVsbG8="), Err(DecodeError::Gzip));
    }
}
