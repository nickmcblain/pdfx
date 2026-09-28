//! Image rewrite through the public compress/inspect API.

mod common;

use common::image_pdf;
use lopdf::Document;
use pdfx_pdf::{compress, inspect};
use std::io::Cursor;

fn reopens(pdf: &[u8]) {
    Document::load_from(Cursor::new(pdf)).expect("reopen");
}

#[test]
fn gray_blocks_shrink() {
    let mut gray = vec![20u8; 64 * 64];
    for (i, px) in gray.iter_mut().enumerate() {
        if i % 64 < 32 {
            *px = 230;
        }
    }
    let pdf = image_pdf(64, 64, "DeviceGray", None, false, 8, gray);
    let before = inspect(&pdf).unwrap();
    assert_eq!(before.pages, 1);
    assert_eq!(before.image_streams, 1);

    let (out, stats) = compress(&pdf).unwrap();
    assert!(out.len() < pdf.len(), "in={} out={}", pdf.len(), out.len());
    assert!(stats.images_rewritten >= 1, "{stats:?}");
    assert!(!stats.kept_original);
    reopens(&out);
}

#[test]
fn unsupported_images_are_skipped() {
    let cases = [
        image_pdf(8, 8, "DeviceGray", None, true, 1, vec![0x00; 8]),
        image_pdf(4, 4, "DeviceCMYK", None, false, 8, vec![0u8; 4 * 4 * 4]),
        image_pdf(8, 8, "DeviceRGB", Some("DCTDecode"), false, 8, b"not-a-jpeg".to_vec()),
        image_pdf(8, 8, "DeviceGray", Some("CCITTFaxDecode"), false, 1, vec![0, 1, 2]),
        image_pdf(0, 0, "DeviceRGB", None, false, 8, Vec::new()),
    ];
    for pdf in cases {
        let (out, stats) = compress(&pdf).unwrap();
        assert!(stats.images_skipped >= 1, "{stats:?}");
        assert!(out.len() <= pdf.len());
        reopens(&out);
    }
}

#[test]
fn garbage_is_rejected() {
    assert!(inspect(b"not a pdf").is_err());
    assert!(compress(b"not a pdf").is_err());
}
