//! PNG predictors (ISO 32000 / DecodeParms /Predictor 10–15).

/// Apply the best PNG filter per row. Output is (filter_byte || samples) * height.
pub fn encode_png_best(samples: &[u8], width: usize, components: usize) -> Vec<u8> {
    let stride = width * components;
    assert!(stride == 0 || samples.len().is_multiple_of(stride));
    let height = if stride == 0 {
        0
    } else {
        samples.len() / stride
    };
    let mut out = Vec::with_capacity((stride + 1) * height);
    let mut prev = vec![0u8; stride];
    for row in 0..height {
        let raw = &samples[row * stride..(row + 1) * stride];
        let mut best = Vec::new();
        let mut best_score = u64::MAX;
        for kind in 0..5u8 {
            let filtered = filter_row(kind, raw, &prev, components);
            let score: u64 = filtered.iter().map(|&b| b as u64).sum();
            if score < best_score {
                best_score = score;
                best = filtered;
                best.insert(0, kind);
            }
        }
        out.extend_from_slice(&best);
        prev.copy_from_slice(raw);
    }
    out
}

/// Reverse PNG filters (Predictor 15).
pub fn decode_png(data: &[u8], width: usize, components: usize) -> Option<Vec<u8>> {
    let stride = width * components;
    if stride == 0 {
        return Some(Vec::new());
    }
    let row_len = stride + 1;
    if !data.len().is_multiple_of(row_len) {
        return None;
    }
    let height = data.len() / row_len;
    let mut out = vec![0u8; stride * height];
    let mut prev = vec![0u8; stride];
    for row in 0..height {
        let kind = data[row * row_len];
        let filt = &data[row * row_len + 1..row * row_len + 1 + stride];
        let dest = &mut out[row * stride..(row + 1) * stride];
        unfilter_row(kind, filt, &prev, dest, components)?;
        prev.copy_from_slice(dest);
    }
    Some(out)
}

fn filter_row(kind: u8, raw: &[u8], prev: &[u8], bpp: usize) -> Vec<u8> {
    let mut out = vec![0u8; raw.len()];
    for i in 0..raw.len() {
        let left = if i >= bpp { raw[i - bpp] } else { 0 };
        let up = prev[i];
        let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
        out[i] = match kind {
            0 => raw[i],
            1 => raw[i].wrapping_sub(left),
            2 => raw[i].wrapping_sub(up),
            3 => raw[i].wrapping_sub(((u16::from(left) + u16::from(up)) / 2) as u8),
            _ => raw[i].wrapping_sub(paeth(left, up, up_left)),
        };
    }
    out
}

fn unfilter_row(kind: u8, filt: &[u8], prev: &[u8], dest: &mut [u8], bpp: usize) -> Option<()> {
    for i in 0..dest.len() {
        let left = if i >= bpp { dest[i - bpp] } else { 0 };
        let up = prev[i];
        let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
        dest[i] = match kind {
            0 => filt[i],
            1 => filt[i].wrapping_add(left),
            2 => filt[i].wrapping_add(up),
            3 => filt[i].wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8),
            4 => filt[i].wrapping_add(paeth(left, up, up_left)),
            _ => return None,
        };
    }
    Some(())
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let aa = i16::from(a);
    let bb = i16::from(b);
    let cc = i16::from(c);
    let p = aa + bb - cc;
    let pa = (p - aa).abs();
    let pb = (p - bb).abs();
    let pc = (p - cc).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// TIFF predictor 2 (horizontal differencing).
#[allow(dead_code)]
pub fn encode_tiff2(samples: &[u8], width: usize, components: usize) -> Vec<u8> {
    let stride = width * components;
    let height = if stride == 0 {
        0
    } else {
        samples.len() / stride
    };
    let mut out = samples.to_vec();
    for row in 0..height {
        let base = row * stride;
        for x in (1..width).rev() {
            for c in 0..components {
                let i = base + x * components + c;
                let left = out[base + (x - 1) * components + c];
                out[i] = out[i].wrapping_sub(left);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip() {
        let w = 8usize;
        let samples: Vec<u8> = (0..w * 4 * 3).map(|i| (i * 13 % 256) as u8).collect();
        let enc = encode_png_best(&samples, w, 3);
        let dec = decode_png(&enc, w, 3).unwrap();
        assert_eq!(dec, samples);
    }

    #[test]
    fn tiff2_changes_bytes() {
        let samples = vec![10u8, 20, 30, 40, 11, 22, 33, 44];
        let enc = encode_tiff2(&samples, 4, 2);
        assert_ne!(enc, samples);
        assert_eq!(enc.len(), samples.len());
    }
}
