//! Public classify / SSIM gate.

use pdfx_percept::{classify, passes_gate, ssim_rgb, ImageKind, RgbImage};

fn solid(w: u32, h: u32, px: [u8; 3]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    for _ in 0..w * h {
        rgb.extend_from_slice(&px);
    }
    rgb
}

#[test]
fn different_size_fails_gate() {
    let a = [0u8, 0, 0];
    let b = [0u8, 0, 0, 0, 0, 0];
    assert!(!passes_gate(
        RgbImage {
            width: 1,
            height: 1,
            rgb: &a
        },
        RgbImage {
            width: 2,
            height: 1,
            rgb: &b
        },
    ));
}

#[test]
fn black_versus_white_fails_gate() {
    let black = solid(8, 8, [0, 0, 0]);
    let white = solid(8, 8, [255, 255, 255]);
    let a = RgbImage {
        width: 8,
        height: 8,
        rgb: &black,
    };
    let b = RgbImage {
        width: 8,
        height: 8,
        rgb: &white,
    };
    assert!(ssim_rgb(a, b) < 0.98);
    assert!(!passes_gate(a, b));
}

#[test]
fn empty_image_is_photo_and_passes() {
    let img = RgbImage {
        width: 0,
        height: 0,
        rgb: &[],
    };
    assert_eq!(classify(img), ImageKind::Photo);
    assert!(passes_gate(img, img));
}
