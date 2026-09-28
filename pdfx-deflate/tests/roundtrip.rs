//! Public zlib compress: bytes in, inflatable bytes out.

use flate2::read::ZlibDecoder;
use std::io::Read;

fn inflate(bytes: &[u8]) -> Vec<u8> {
    let mut dec = ZlibDecoder::new(bytes);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).expect("zlib decode");
    out
}

#[test]
fn header_is_zlib_max_compression() {
    let out = pdfx_deflate::compress(b"abc");
    assert_eq!(&out[..2], &[0x78, 0xDA]);
    assert!(out.len() > 6);
    assert_eq!(inflate(&out), b"abc");
}

#[test]
fn one_byte_and_all_byte_values() {
    assert_eq!(inflate(&pdfx_deflate::compress(&[0x00])), &[0x00]);
    assert_eq!(inflate(&pdfx_deflate::compress(&[0xFF])), &[0xFF]);
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(inflate(&pdfx_deflate::compress(&all)), all);
}

#[test]
fn incompressible_still_roundtrips() {
    let src: Vec<u8> = (0..512).map(|i| (i * 17 + 3) as u8).collect();
    let out = pdfx_deflate::compress(&src);
    assert!(!out.is_empty());
    assert_eq!(inflate(&out), src);
}
