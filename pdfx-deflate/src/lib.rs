//! Zlib-wrapped DEFLATE encoder (RFC 1950 / 1951).
//! Decode with any standard inflater — we only implement encode.

mod adler;
mod bits;
mod encode;
mod huffman;
mod lz77;

pub use encode::compress_zlib;

/// Compress `data` to a zlib stream. Never returns an empty buffer.
pub fn compress(data: &[u8]) -> Vec<u8> {
    compress_zlib(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    fn inflate(bytes: &[u8]) -> Vec<u8> {
        let mut dec = ZlibDecoder::new(bytes);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).expect("zlib decode");
        out
    }

    #[test]
    fn empty_roundtrip() {
        let out = compress(&[]);
        assert_eq!(inflate(&out), b"");
    }

    #[test]
    fn short_literal_roundtrip() {
        let src = b"hello pdfx";
        assert_eq!(inflate(&compress(src)), src);
    }

    #[test]
    fn repeated_bytes_roundtrip() {
        let src = vec![0xABu8; 4096];
        let out = compress(&src);
        assert!(out.len() < src.len());
        assert_eq!(inflate(&out), src);
    }

    #[test]
    fn lorem_roundtrip() {
        let src = b"the quick brown fox jumps over the lazy dog. ".repeat(200);
        let out = compress(&src);
        assert!(out.len() < src.len() / 2);
        assert_eq!(inflate(&out), src.as_slice());
    }

    #[test]
    fn binary_noise_roundtrip() {
        let src: Vec<u8> = (0u32..10_000)
            .map(|i| (i.wrapping_mul(1_103_515_245) >> 16) as u8)
            .collect();
        assert_eq!(inflate(&compress(&src)), src);
    }

    #[test]
    fn beats_identity_on_text() {
        let src = include_str!("lib.rs").as_bytes();
        let out = compress(src);
        assert!(out.len() < src.len());
        assert_eq!(inflate(&out), src);
    }
}
