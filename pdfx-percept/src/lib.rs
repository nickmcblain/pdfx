//! Image classification and SSIM visual-lossless gate.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Photo,
    Screenshot,
    LineArt,
    Bitonal,
}

#[derive(Clone, Copy, Debug)]
pub struct RgbImage<'a> {
    pub width: u32,
    pub height: u32,
    pub rgb: &'a [u8],
}

impl<'a> RgbImage<'a> {
    pub fn luma(&self, i: usize) -> u8 {
        let r = self.rgb[i * 3] as u32;
        let g = self.rgb[i * 3 + 1] as u32;
        let b = self.rgb[i * 3 + 2] as u32;
        ((77 * r + 150 * g + 29 * b) >> 8) as u8
    }

    pub fn pixels(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

pub fn classify(img: RgbImage<'_>) -> ImageKind {
    let n = img.pixels();
    if n == 0 {
        return ImageKind::Photo;
    }

    let mut hist = [0u32; 256];
    let mut unique_rgb = std::collections::HashSet::new();
    let mut edge = 0u32;
    let mut run_score = 0u32;
    let w = img.width as usize;

    for i in 0..n {
        hist[img.luma(i) as usize] += 1;
        if unique_rgb.len() < 4096 {
            unique_rgb.insert([img.rgb[i * 3], img.rgb[i * 3 + 1], img.rgb[i * 3 + 2]]);
        }
        if i + 1 < n && i % w != w - 1 {
            let d = img.luma(i).abs_diff(img.luma(i + 1));
            if d > 40 {
                edge += 1;
            }
            if d == 0 {
                run_score += 1;
            }
        }
    }

    let occupied: u32 = hist.iter().filter(|&&c| c > 0).count() as u32;
    let unique = unique_rgb.len();
    let edge_ratio = edge as f64 / n as f64;
    let run_ratio = run_score as f64 / n.max(1) as f64;

    if occupied <= 4 && unique <= 4 {
        return ImageKind::Bitonal;
    }
    if unique <= 24 && edge_ratio > 0.08 {
        return ImageKind::LineArt;
    }
    if unique < 512 && (run_ratio > 0.35 || edge_ratio > 0.12) {
        return ImageKind::Screenshot;
    }
    ImageKind::Photo
}

/// Mean SSIM on luma over 8×8 windows. 1.0 is identical.
pub fn ssim_rgb(a: RgbImage<'_>, b: RgbImage<'_>) -> f64 {
    assert_eq!(a.width, b.width);
    assert_eq!(a.height, b.height);
    let w = a.width as usize;
    let h = a.height as usize;
    if w == 0 || h == 0 {
        return 1.0;
    }

    const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);
    const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);

    let mut scores = Vec::new();
    let step = 8usize;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let x1 = (x + step).min(w);
            let y1 = (y + step).min(h);
            let (s, n) = window_stats(a, b, x, y, x1, y1, w);
            if n > 0.0 {
                let (mu_x, mu_y, var_x, var_y, cov) = s;
                let num = (2.0 * mu_x * mu_y + C1) * (2.0 * cov + C2);
                let den = (mu_x * mu_x + mu_y * mu_y + C1) * (var_x + var_y + C2);
                scores.push(num / den);
            }
            x += step;
        }
        y += step;
    }
    if scores.is_empty() {
        1.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    }
}

fn window_stats(
    a: RgbImage<'_>,
    b: RgbImage<'_>,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    w: usize,
) -> ((f64, f64, f64, f64, f64), f64) {
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    let mut sxy = 0.0;
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = y * w + x;
            let xv = a.luma(i) as f64;
            let yv = b.luma(i) as f64;
            sx += xv;
            sy += yv;
            sxx += xv * xv;
            syy += yv * yv;
            sxy += xv * yv;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return ((0.0, 0.0, 0.0, 0.0, 0.0), 0.0);
    }
    let mu_x = sx / n;
    let mu_y = sy / n;
    let var_x = (sxx / n) - mu_x * mu_x;
    let var_y = (syy / n) - mu_y * mu_y;
    let cov = (sxy / n) - mu_x * mu_y;
    ((mu_x, mu_y, var_x.max(0.0), var_y.max(0.0), cov), n)
}

/// Default visually-lossless threshold.
pub const SSIM_THRESHOLD: f64 = 0.98;

pub fn passes_gate(original: RgbImage<'_>, candidate: RgbImage<'_>) -> bool {
    if original.width != candidate.width || original.height != candidate.height {
        return false;
    }
    if original.rgb == candidate.rgb {
        return true;
    }
    ssim_rgb(original, candidate) >= SSIM_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_one() {
        let rgb = [10u8, 20, 30, 40, 50, 60, 70, 80, 90, 11, 22, 33];
        let img = RgbImage {
            width: 2,
            height: 2,
            rgb: &rgb,
        };
        assert!((ssim_rgb(img, img) - 1.0).abs() < 1e-6);
        assert!(passes_gate(img, img));
    }

    #[test]
    fn bitonal_classifier() {
        let mut rgb = vec![0u8; 32 * 32 * 3];
        for (i, px) in rgb.chunks_mut(3).enumerate() {
            if i % 2 == 0 {
                px[0] = 255;
                px[1] = 255;
                px[2] = 255;
            }
        }
        let img = RgbImage {
            width: 32,
            height: 32,
            rgb: &rgb,
        };
        assert_eq!(classify(img), ImageKind::Bitonal);
    }

    #[test]
    fn screenshot_blocks() {
        let mut rgb = vec![0u8; 48 * 48 * 3];
        for y in 0..48 {
            for x in 0..48 {
                let i = (y * 48 + x) * 3;
                let (r, g, b) = match (y / 8, x / 8) {
                    (0, _) => (40u8, 80, 180),
                    (_, 0) => (28, 28, 32),
                    (1, 2) => (220, 80, 70),
                    (2, 3) => (70, 180, 90),
                    (3, _) => (250, 200, 60),
                    (_, 5) => (160, 160, 170),
                    _ => (244, 244, 248),
                };
                rgb[i] = r;
                rgb[i + 1] = g;
                rgb[i + 2] = b;
            }
        }
        let img = RgbImage {
            width: 48,
            height: 48,
            rgb: &rgb,
        };
        let kind = classify(img);
        assert!(
            kind == ImageKind::Screenshot || kind == ImageKind::LineArt,
            "{kind:?}"
        );
    }

    #[test]
    fn photo_like_gradient() {
        let mut rgb = vec![0u8; 64 * 64 * 3];
        for y in 0..64 {
            for x in 0..64 {
                let i = (y * 64 + x) * 3;
                rgb[i] = (x * 3 + y / 2) as u8;
                rgb[i + 1] = (y * 3 + 40) as u8;
                rgb[i + 2] = 180u8.saturating_sub(x as u8);
            }
        }
        let img = RgbImage {
            width: 64,
            height: 64,
            rgb: &rgb,
        };
        assert_eq!(classify(img), ImageKind::Photo);
    }
}
