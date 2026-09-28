//! Public JPEG encode: decodable baseline, quality changes size.

use pdfx_jpeg::{encode_rgb, encode_rgb_ex, Chroma};

fn decode(jpeg: &[u8]) -> Vec<u8> {
    let mut dec = zune_jpeg::JpegDecoder::new(jpeg);
    dec.decode().expect("decode")
}

fn gradient(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            rgb[i] = (x * 17) as u8;
            rgb[i + 1] = (y * 13) as u8;
            rgb[i + 2] = 90;
        }
    }
    rgb
}

#[test]
fn odd_sizes_and_both_chroma_decode() {
    for (w, h, chroma) in [
        (1, 1, Chroma::Sample444),
        (7, 5, Chroma::Sample444),
        (15, 17, Chroma::Sample420),
        (3, 3, Chroma::Sample420),
    ] {
        let rgb = gradient(w, h);
        let jpeg = encode_rgb_ex(w, h, &rgb, 80, chroma);
        assert!(jpeg.starts_with(&[0xFF, 0xD8]), "{w}x{h} {chroma:?}");
        assert!(jpeg.ends_with(&[0xFF, 0xD9]));
        assert_eq!(decode(&jpeg).len(), rgb.len(), "{w}x{h} {chroma:?}");
    }
}

#[test]
fn quality_clamps_and_lower_quality_is_smaller() {
    let rgb = gradient(32, 32);
    let low = encode_rgb(32, 32, &rgb, 0);
    let high = encode_rgb(32, 32, &rgb, 200);
    assert_eq!(decode(&low).len(), rgb.len());
    assert_eq!(decode(&high).len(), rgb.len());
    let q15 = encode_rgb(32, 32, &rgb, 15);
    let q90 = encode_rgb(32, 32, &rgb, 90);
    assert!(q15.len() < q90.len(), "q15={} q90={}", q15.len(), q90.len());
}
