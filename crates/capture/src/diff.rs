//! Pixel comparison between two captures of the same screen.

use std::path::Path;

/// Channel difference above which a pixel counts as changed (absorbs anti-aliasing noise).
const CHANNEL_TOLERANCE: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diff {
    /// Fraction (0.0–1.0) of pixels that differ; different sizes count as 1.0.
    pub ratio: f64,
    /// Bounding box `[x, y, width, height]` of the changed pixels, in image pixels.
    pub bbox: Option<[u32; 4]>,
}

pub fn diff(a: &Path, b: &Path) -> Result<Diff, image::ImageError> {
    let a = image::open(a)?.to_rgba8();
    let b = image::open(b)?.to_rgba8();
    if a.dimensions() != b.dimensions() {
        return Ok(Diff {
            ratio: 1.0,
            bbox: None,
        });
    }
    let (w, h) = a.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    let mut changed = 0u64;
    for (x, y, p) in a.enumerate_pixels() {
        let q = b.get_pixel(x, y);
        if p.0
            .iter()
            .zip(q.0.iter())
            .any(|(s, t)| s.abs_diff(*t) > CHANNEL_TOLERANCE)
        {
            changed += 1;
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    let total = (w as u64 * h as u64).max(1);
    Ok(Diff {
        ratio: changed as f64 / total as f64,
        bbox: (changed > 0).then(|| [x0, y0, x1 - x0 + 1, y1 - y0 + 1]),
    })
}

pub fn dimensions(p: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(p).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn ratio_and_box_with_tolerance() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = RgbaImage::from_pixel(10, 10, Rgba([100, 100, 100, 255]));
        a.save(dir.path().join("a.png")).unwrap();
        // 5 pixels clearly changed, 10 pixels within tolerance
        for x in 0..5 {
            a.put_pixel(x, 0, Rgba([200, 100, 100, 255]));
        }
        for x in 0..10 {
            a.put_pixel(x, 5, Rgba([105, 100, 100, 255]));
        }
        a.save(dir.path().join("b.png")).unwrap();
        let d = diff(&dir.path().join("a.png"), &dir.path().join("b.png")).unwrap();
        assert!((d.ratio - 0.05).abs() < 1e-9, "{d:?}");
        assert_eq!(d.bbox, Some([0, 0, 5, 1]));
        RgbaImage::new(5, 5).save(dir.path().join("c.png")).unwrap();
        assert_eq!(
            diff(&dir.path().join("a.png"), &dir.path().join("c.png"))
                .unwrap()
                .ratio,
            1.0
        );
        assert_eq!(dimensions(&dir.path().join("c.png")), Some((5, 5)));
    }
}
