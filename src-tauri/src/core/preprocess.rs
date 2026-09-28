//! Image preprocessing stage (ARCHITECTURE.md §3: "RenderPage -> ImagePreprocess -> OCR").
//!
//! Phase 2 scope: grayscale + adaptive binarization, which is the single
//! biggest accuracy lever for OCR on scanned pages (uneven scan lighting is
//! the dominant real-world failure mode). Deskew/denoise are noted as
//! accuracy-tuning follow-ups once the Phase 2 benchmark gate
//! (MODEL_STRATEGY.md §1) has real fixtures to measure against, rather than
//! guessed at now without a way to verify they help.

use crate::core::traits::{CoreError, CoreResult};
use image::{GrayImage, ImageReader, Luma};
use std::path::Path;

/// Sauvola-style local adaptive threshold: robust to the uneven lighting
/// gradients scanned book pages typically have, unlike a single global
/// threshold (e.g. Otsu), which fails when brightness varies across a page.
fn adaptive_binarize(img: &GrayImage, window: u32, k: f32) -> GrayImage {
    let (w, h) = img.dimensions();
    let half = (window / 2) as i64;
    let mut out = GrayImage::new(w, h);

    // Integral images for O(1) windowed mean/variance per pixel.
    let mut sum = vec![0f64; ((w + 1) * (h + 1)) as usize];
    let mut sum_sq = vec![0f64; ((w + 1) * (h + 1)) as usize];
    let stride = (w + 1) as usize;
    for y in 0..h {
        let mut row_sum = 0f64;
        let mut row_sum_sq = 0f64;
        for x in 0..w {
            let v = img.get_pixel(x, y).0[0] as f64;
            row_sum += v;
            row_sum_sq += v * v;
            let idx = (y as usize + 1) * stride + (x as usize + 1);
            sum[idx] = sum[idx - stride] + row_sum;
            sum_sq[idx] = sum_sq[idx - stride] + row_sum_sq;
        }
    }
    let region = |x0: i64, y0: i64, x1: i64, y1: i64, table: &[f64]| -> f64 {
        let x0 = x0.clamp(0, w as i64) as usize;
        let y0 = y0.clamp(0, h as i64) as usize;
        let x1 = x1.clamp(0, w as i64) as usize;
        let y1 = y1.clamp(0, h as i64) as usize;
        table[y1 * stride + x1] - table[y0 * stride + x1] - table[y1 * stride + x0]
            + table[y0 * stride + x0]
    };

    for y in 0..h {
        for x in 0..w {
            let x0 = x as i64 - half;
            let y0 = y as i64 - half;
            let x1 = x as i64 + half + 1;
            let y1 = y as i64 + half + 1;
            let count = ((x1.clamp(0, w as i64) - x0.clamp(0, w as i64))
                * (y1.clamp(0, h as i64) - y0.clamp(0, h as i64))) as f64;
            let s = region(x0, y0, x1, y1, &sum);
            let sq = region(x0, y0, x1, y1, &sum_sq);
            let mean = s / count;
            let variance = (sq / count - mean * mean).max(0.0);
            let std_dev = variance.sqrt();
            // Sauvola: threshold = mean * (1 + k * (std_dev / R - 1)), R=128 for 8-bit images.
            let threshold = mean * (1.0 + k as f64 * (std_dev / 128.0 - 1.0));
            let v = img.get_pixel(x, y).0[0] as f64;
            out.put_pixel(x, y, Luma([if v > threshold { 255 } else { 0 }]));
        }
    }
    out
}

/// Loads `input`, converts to grayscale, adaptively binarizes, and writes the
/// result to `output` (PNG). Returns `output` for chaining.
pub fn preprocess(input: &Path, output: &Path) -> CoreResult<()> {
    let img = ImageReader::open(input)
        .map_err(CoreError::Io)?
        .with_guessed_format()
        .map_err(CoreError::Io)?
        .decode()
        .map_err(|e| CoreError::Engine(format!("failed to decode image: {e}")))?;
    let gray = img.to_luma8();
    let binarized = adaptive_binarize(&gray, 25, 0.34);
    binarized
        .save(output)
        .map_err(|e| CoreError::Engine(format!("failed to write preprocessed image: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    #[test]
    fn preprocess_produces_binary_output() {
        let dir = std::env::temp_dir().join(format!("documind-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.png");
        let output = dir.join("out.png");

        // Half-bright, half-dark synthetic page so binarization has real work to do.
        let img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(100, 100, |x, _y| {
            Luma([if x < 50 { 220 } else { 40 }])
        });
        img.save(&input).unwrap();

        preprocess(&input, &output).unwrap();
        let result = image::open(&output).unwrap().to_luma8();
        for pixel in result.pixels() {
            assert!(pixel.0[0] == 0 || pixel.0[0] == 255);
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
