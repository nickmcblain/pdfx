/// Scale an Annex-K quant table like libjpeg (quality 1..=100).
pub fn scale_table(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = quality.clamp(1, 100) as i32;
    let s = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u8; 64];
    for i in 0..64 {
        let v = (base[i] as i32 * s + 50) / 100;
        out[i] = v.clamp(1, 255) as u8;
    }
    out
}
