//! Box-average resize. Used to cap huge slide bitmaps.

pub fn fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_edge || long == 0 {
        return (width, height);
    }
    let scale = max_edge as f64 / long as f64;
    let w = ((width as f64 * scale).round() as u32).max(1);
    let h = ((height as f64 * scale).round() as u32).max(1);
    (w, h)
}

pub fn resize_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    resize(src, sw, sh, dw, dh, 3)
}

pub fn resize_gray(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    resize(src, sw, sh, dw, dh, 1)
}

fn resize(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32, ch: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw as usize * dh as usize * ch];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return out;
    }
    for y in 0..dh {
        let y0 = (y as u64 * sh as u64 / dh as u64) as u32;
        let y1 = (((y as u64 + 1) * sh as u64 / dh as u64) as u32)
            .max(y0 + 1)
            .min(sh);
        for x in 0..dw {
            let x0 = (x as u64 * sw as u64 / dw as u64) as u32;
            let x1 = (((x as u64 + 1) * sw as u64 / dw as u64) as u32)
                .max(x0 + 1)
                .min(sw);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in y0..y1 {
                let row = (yy * sw) as usize;
                for xx in x0..x1 {
                    let i = (row + xx as usize) * ch;
                    for c in 0..ch {
                        acc[c] += u32::from(src[i + c]);
                    }
                    n += 1;
                }
            }
            let o = ((y * dw + x) as usize) * ch;
            for c in 0..ch {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_leaves_small() {
        assert_eq!(fit(800, 600, 1920), (800, 600));
    }

    #[test]
    fn fit_4k_to_1080p() {
        assert_eq!(fit(3840, 2160, 1920), (1920, 1080));
    }

    #[test]
    fn box_2x2_rgb() {
        let src = [
            0, 0, 0, 20, 0, 0, //
            40, 0, 0, 80, 0, 0,
        ];
        let out = resize_rgb(&src, 2, 2, 1, 1);
        assert_eq!(out, vec![35, 0, 0]);
    }
}
