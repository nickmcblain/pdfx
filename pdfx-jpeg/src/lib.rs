//! Baseline SOF0 JPEG encoder for PDF `DCTDecode`.

mod huffman;
mod quant;
mod tables;

use huffman::JpegWriter;
use quant::scale_table;
use tables::{CHROMA_AC, CHROMA_DC, CHROMA_QUANT, LUMA_AC, LUMA_DC, LUMA_QUANT, ZIGZAG};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    /// Full chroma — better for UI/text.
    Sample444,
    /// 4:2:0 — smaller; fine for photos/slides.
    Sample420,
}

/// Encode 8-bit RGB as baseline JPEG. `quality` is 1..=100 (libjpeg-style).
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8], quality: u8) -> Vec<u8> {
    encode_rgb_ex(width, height, rgb, quality, Chroma::Sample444)
}

pub fn encode_rgb_ex(
    width: u32,
    height: u32,
    rgb: &[u8],
    quality: u8,
    chroma: Chroma,
) -> Vec<u8> {
    assert_eq!(rgb.len(), width as usize * height as usize * 3);
    let quality = quality.clamp(1, 100);
    let qy = scale_table(&LUMA_QUANT, quality);
    let qc = scale_table(&CHROMA_QUANT, quality);

    let mut w = JpegWriter::new();
    w.marker(0xD8); // SOI
    write_dqt(&mut w, 0, &qy);
    write_dqt(&mut w, 1, &qc);
    write_sof0(&mut w, width, height, chroma);
    write_dht(&mut w, 0x00, &LUMA_DC);
    write_dht(&mut w, 0x10, &LUMA_AC);
    write_dht(&mut w, 0x01, &CHROMA_DC);
    write_dht(&mut w, 0x11, &CHROMA_AC);
    write_sos(&mut w);

    let (ys, cbs, crs) = rgb_to_ycbcr(width, height, rgb, chroma);
    let mut prev_dc_y = 0i32;
    let mut prev_dc_cb = 0i32;
    let mut prev_dc_cr = 0i32;

    match chroma {
        Chroma::Sample444 => {
            let bw = width.div_ceil(8);
            let bh = height.div_ceil(8);
            for by in 0..bh {
                for bx in 0..bw {
                    let block = sample_block(&ys, width, height, bx, by);
                    prev_dc_y = encode_block(&mut w, &block, &qy, prev_dc_y, &LUMA_DC, &LUMA_AC);
                    let block = sample_block(&cbs, width, height, bx, by);
                    prev_dc_cb =
                        encode_block(&mut w, &block, &qc, prev_dc_cb, &CHROMA_DC, &CHROMA_AC);
                    let block = sample_block(&crs, width, height, bx, by);
                    prev_dc_cr =
                        encode_block(&mut w, &block, &qc, prev_dc_cr, &CHROMA_DC, &CHROMA_AC);
                }
            }
        }
        Chroma::Sample420 => {
            let cw = width.div_ceil(2);
            let ch = height.div_ceil(2);
            let mw = width.div_ceil(16);
            let mh = height.div_ceil(16);
            for my in 0..mh {
                for mx in 0..mw {
                    for iy in 0..2 {
                        for ix in 0..2 {
                            let block = sample_block(&ys, width, height, mx * 2 + ix, my * 2 + iy);
                            prev_dc_y =
                                encode_block(&mut w, &block, &qy, prev_dc_y, &LUMA_DC, &LUMA_AC);
                        }
                    }
                    let block = sample_block(&cbs, cw, ch, mx, my);
                    prev_dc_cb =
                        encode_block(&mut w, &block, &qc, prev_dc_cb, &CHROMA_DC, &CHROMA_AC);
                    let block = sample_block(&crs, cw, ch, mx, my);
                    prev_dc_cr =
                        encode_block(&mut w, &block, &qc, prev_dc_cr, &CHROMA_DC, &CHROMA_AC);
                }
            }
        }
    }
    w.flush_bits();
    w.marker(0xD9); // EOI
    w.into_inner()
}

fn write_dqt(w: &mut JpegWriter, id: u8, table: &[u8; 64]) {
    w.marker(0xDB);
    w.u16(67);
    w.u8(id);
    for &z in &ZIGZAG {
        w.u8(table[z as usize]);
    }
}

fn write_sof0(w: &mut JpegWriter, width: u32, height: u32, chroma: Chroma) {
    w.marker(0xC0);
    w.u16(17);
    w.u8(8);
    w.u16(height as u16);
    w.u16(width as u16);
    w.u8(3);
    let y_samp = match chroma {
        Chroma::Sample444 => 0x11,
        Chroma::Sample420 => 0x22,
    };
    w.u8(1);
    w.u8(y_samp);
    w.u8(0);
    w.u8(2);
    w.u8(0x11);
    w.u8(1);
    w.u8(3);
    w.u8(0x11);
    w.u8(1);
}

fn write_dht(w: &mut JpegWriter, class_id: u8, spec: &tables::HuffSpec) {
    let mut payload = Vec::new();
    payload.push(class_id);
    payload.extend_from_slice(&spec.bits);
    payload.extend_from_slice(&spec.vals);
    w.marker(0xC4);
    w.u16((payload.len() + 2) as u16);
    w.bytes(&payload);
}

fn write_sos(w: &mut JpegWriter) {
    w.marker(0xDA);
    w.u16(12);
    w.u8(3);
    w.u8(1);
    w.u8(0x00);
    w.u8(2);
    w.u8(0x11);
    w.u8(3);
    w.u8(0x11);
    w.u8(0);
    w.u8(63);
    w.u8(0);
}

fn rgb_to_ycbcr(
    width: u32,
    height: u32,
    rgb: &[u8],
    chroma: Chroma,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let n = (width * height) as usize;
    let mut y = vec![0u8; n];
    let mut cb_full = vec![0u8; n];
    let mut cr_full = vec![0u8; n];
    for i in 0..n {
        let r = rgb[i * 3] as f32;
        let g = rgb[i * 3 + 1] as f32;
        let b = rgb[i * 3 + 2] as f32;
        y[i] = (0.299 * r + 0.587 * g + 0.114 * b)
            .round()
            .clamp(0.0, 255.0) as u8;
        cb_full[i] = (128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b)
            .round()
            .clamp(0.0, 255.0) as u8;
        cr_full[i] = (128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    match chroma {
        Chroma::Sample444 => (y, cb_full, cr_full),
        Chroma::Sample420 => {
            let cw = width.div_ceil(2);
            let ch = height.div_ceil(2);
            let mut cb = vec![0u8; (cw * ch) as usize];
            let mut cr = vec![0u8; (cw * ch) as usize];
            for cy in 0..ch {
                for cx in 0..cw {
                    let x0 = (cx * 2).min(width - 1);
                    let y0 = (cy * 2).min(height - 1);
                    let x1 = (cx * 2 + 1).min(width - 1);
                    let y1 = (cy * 2 + 1).min(height - 1);
                    let i00 = (y0 * width + x0) as usize;
                    let i10 = (y0 * width + x1) as usize;
                    let i01 = (y1 * width + x0) as usize;
                    let i11 = (y1 * width + x1) as usize;
                    let o = (cy * cw + cx) as usize;
                    cb[o] = ((u16::from(cb_full[i00])
                        + u16::from(cb_full[i10])
                        + u16::from(cb_full[i01])
                        + u16::from(cb_full[i11]))
                        / 4) as u8;
                    cr[o] = ((u16::from(cr_full[i00])
                        + u16::from(cr_full[i10])
                        + u16::from(cr_full[i01])
                        + u16::from(cr_full[i11]))
                        / 4) as u8;
                }
            }
            (y, cb, cr)
        }
    }
}

fn sample_block(plane: &[u8], width: u32, height: u32, bx: u32, by: u32) -> [i32; 64] {
    let mut block = [0i32; 64];
    for row in 0..8u32 {
        let py = (by * 8 + row).min(height - 1);
        for col in 0..8u32 {
            let px = (bx * 8 + col).min(width - 1);
            let v = plane[(py * width + px) as usize] as i32;
            block[(row * 8 + col) as usize] = v - 128;
        }
    }
    block
}

fn encode_block(
    w: &mut JpegWriter,
    spatial: &[i32; 64],
    quant: &[u8; 64],
    prev_dc: i32,
    dc: &tables::HuffSpec,
    ac: &tables::HuffSpec,
) -> i32 {
    let mut coeff = fdct(spatial);
    for i in 0..64 {
        let q = quant[i].max(1) as i32;
        coeff[i] = div_round(coeff[i], q);
    }
    let dc_val = coeff[0];
    let diff = dc_val - prev_dc;
    encode_dc(w, diff, dc);

    let mut run = 0u8;
    for &z in ZIGZAG.iter().skip(1) {
        let v = coeff[z as usize];
        if v == 0 {
            run += 1;
            if run == 16 {
                // ZRL
                w.huffman(ac, 0xF0);
                run = 0;
            }
        } else {
            while run >= 16 {
                w.huffman(ac, 0xF0);
                run -= 16;
            }
            encode_ac(w, run, v, ac);
            run = 0;
        }
    }
    if run > 0 {
        w.huffman(ac, 0x00); // EOB
    }
    dc_val
}

fn encode_dc(w: &mut JpegWriter, diff: i32, spec: &tables::HuffSpec) {
    let (bits, amp) = category(diff);
    w.huffman(spec, bits);
    if bits > 0 {
        w.write_amp(amp, bits);
    }
}

fn encode_ac(w: &mut JpegWriter, run: u8, val: i32, spec: &tables::HuffSpec) {
    let (bits, amp) = category(val);
    let sym = (run << 4) | bits;
    w.huffman(spec, sym);
    w.write_amp(amp, bits);
}

fn category(v: i32) -> (u8, u16) {
    if v == 0 {
        return (0, 0);
    }
    let abs = v.unsigned_abs();
    let bits = (32 - abs.leading_zeros()) as u8;
    let amp = if v > 0 {
        abs as u16
    } else {
        (abs as u16) ^ ((1u16 << bits) - 1)
    };
    (bits, amp)
}

fn div_round(v: i32, q: i32) -> i32 {
    if v >= 0 {
        (v + q / 2) / q
    } else {
        -((-v + q / 2) / q)
    }
}

fn cos_table() -> &'static [[f32; 8]; 8] {
    use std::sync::OnceLock;
    static T: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0.0f32; 8]; 8];
        for k in 0..8 {
            for n in 0..8 {
                let a = if k == 0 { 1.0 / 2.0_f64.sqrt() } else { 1.0 };
                t[k][n] = (a
                    * ((2.0 * n as f64 + 1.0) * k as f64 * std::f64::consts::PI / 16.0).cos())
                    as f32;
            }
        }
        t
    })
}

/// Separable FDCT with a precomputed cosine table.
fn fdct(spatial: &[i32; 64]) -> [i32; 64] {
    let cos = cos_table();

    let mut tmp = [0.0f32; 64];
    for y in 0..8 {
        for k in 0..8 {
            let mut s = 0.0f32;
            for n in 0..8 {
                s += spatial[y * 8 + n] as f32 * cos[k][n];
            }
            tmp[y * 8 + k] = s;
        }
    }
    let mut out = [0i32; 64];
    for x in 0..8 {
        for k in 0..8 {
            let mut s = 0.0f32;
            for n in 0..8 {
                s += tmp[n * 8 + x] * cos[k][n];
            }
            out[k * 8 + x] = (s * 0.25).round() as i32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_solid_color_decodes() {
        let w = 16u32;
        let h = 16u32;
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for px in rgb.chunks_mut(3) {
            px[0] = 200;
            px[1] = 40;
            px[2] = 80;
        }
        let jpeg = encode_rgb(w, h, &rgb, 90);
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
        assert!(jpeg.ends_with(&[0xFF, 0xD9]));

        let mut dec = zune_jpeg::JpegDecoder::new(&jpeg);
        let out = dec.decode().expect("decode our jpeg");
        assert_eq!(out.len(), rgb.len());
        // Visually close — not bit-exact
        let mut err = 0u64;
        for (a, b) in rgb.iter().zip(out.iter()) {
            err += a.abs_diff(*b) as u64;
        }
        let mean = err as f64 / rgb.len() as f64;
        assert!(mean < 8.0, "mean abs err {mean}");
    }

    #[test]
    fn high_quality_beats_ssim_gate() {
        let w = 24u32;
        let h = 24u32;
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                rgb[i] = (x * 8) as u8;
                rgb[i + 1] = (y * 8) as u8;
                rgb[i + 2] = 120;
            }
        }
        let jpeg = encode_rgb(w, h, &rgb, 95);
        let mut dec = zune_jpeg::JpegDecoder::new(&jpeg);
        let out = dec.decode().unwrap();
        let a = pdfx_percept::RgbImage {
            width: w,
            height: h,
            rgb: &rgb,
        };
        let b = pdfx_percept::RgbImage {
            width: w,
            height: h,
            rgb: &out,
        };
        assert!(pdfx_percept::passes_gate(a, b));
    }

    #[test]
    fn encode_420_decodes() {
        let w = 16u32;
        let h = 16u32;
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for px in rgb.chunks_mut(3) {
            px[0] = 180;
            px[1] = 90;
            px[2] = 40;
        }
        let jpeg = encode_rgb_ex(w, h, &rgb, 85, Chroma::Sample420);
        let mut dec = zune_jpeg::JpegDecoder::new(&jpeg);
        let out = dec.decode().expect("decode 4:2:0");
        assert_eq!(out.len(), rgb.len());
    }
}
