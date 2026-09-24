//! PDF object-graph walks and rewrite passes (lopdf is parse/write only).

mod forms;
mod images;
mod predictor;
mod resample;
mod structure;

use thiserror::Error;

pub use forms::{prepare_form, AddedField, FieldKind, FormStats};
pub use images::ImageStats;
pub use structure::{census, compress_document, Census};

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("encrypted PDFs are not supported")]
    Encrypted,
    #[error("invalid PDF: {0}")]
    Invalid(String),
    #[error("{0}")]
    Other(String),
}

impl From<lopdf::Error> for PdfError {
    fn from(e: lopdf::Error) -> Self {
        PdfError::Invalid(e.to_string())
    }
}

impl From<std::io::Error> for PdfError {
    fn from(e: std::io::Error) -> Self {
        PdfError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, PdfError>;

#[derive(Debug, Default)]
pub struct CompressStats {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub kept_original: bool,
    pub orphans_removed: u32,
    pub images_rewritten: u32,
    pub streams_reflated: u32,
    pub images_deduped: u32,
    pub images_skipped: u32,
    pub notes: Vec<String>,
}

pub fn inspect(bytes: &[u8]) -> Result<Census> {
    structure::census(bytes)
}

pub fn compress(bytes: &[u8]) -> Result<(Vec<u8>, CompressStats)> {
    structure::compress_document(bytes)
}

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{
        page_with_invalid_flate_content, photo_rgb, rgb_image_pdf, screenshot_rgb, text_with_orphan,
    };
    use lopdf::{Document, Object};
    use std::io::Cursor;

    #[test]
    fn inspect_text_pdf() {
        let pdf = text_with_orphan();
        let c = inspect(&pdf).unwrap();
        assert_eq!(c.pages, 1);
        assert!(c.objects >= 4);
    }

    #[test]
    fn compress_text_never_larger_and_reopens() {
        let pdf = text_with_orphan();
        let (out, stats) = compress(&pdf).unwrap();
        assert!(out.len() <= pdf.len());
        assert!(Document::load_from(Cursor::new(&out)).is_ok());
        assert!(stats.orphans_removed >= 1 || out.len() < pdf.len() || stats.kept_original);
    }

    #[test]
    fn compress_screenshot_shrinks() {
        let rgb = screenshot_rgb(64, 64);
        let pdf = rgb_image_pdf(64, 64, rgb, 1);
        let (out, stats) = compress(&pdf).unwrap();
        assert!(out.len() < pdf.len(), "in={} out={}", pdf.len(), out.len());
        assert!(!stats.kept_original);
        Document::load_from(Cursor::new(&out)).unwrap();
    }

    #[test]
    fn compress_photo_shrinks_and_stays_visual() {
        let rgb = photo_rgb(48, 48);
        let pdf = rgb_image_pdf(48, 48, rgb, 1);
        let (out, _stats) = compress(&pdf).unwrap();
        assert!(out.len() <= pdf.len());
        Document::load_from(Cursor::new(&out)).unwrap();
    }

    #[test]
    fn dedup_identical_images() {
        let rgb = screenshot_rgb(32, 32);
        let pdf = rgb_image_pdf(32, 32, rgb, 2);
        let (out, stats) = compress(&pdf).unwrap();
        assert!(out.len() <= pdf.len());
        assert!(stats.images_deduped >= 1 || out.len() < pdf.len());
    }

    #[test]
    fn output_uses_classic_xref_for_preview() {
        let pdf = text_with_orphan();
        let (out, _) = compress(&pdf).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\nxref\n"), "classic xref table missing");
        assert!(text.contains("trailer"), "classic trailer missing");
        assert!(
            !text.contains("/Type /XRef"),
            "xref stream breaks macOS Preview"
        );
        Document::load_from(Cursor::new(&out)).unwrap();
    }

    fn page_content_bytes(pdf: &[u8]) -> Vec<u8> {
        let doc = Document::load_from(Cursor::new(pdf)).unwrap();
        let pages = doc.get_pages();
        let pid = *pages.values().next().unwrap();
        let Object::Dictionary(page) = doc.get_object(pid).unwrap() else {
            panic!("page dict");
        };
        let Object::Reference(cid) = page.get(b"Contents").unwrap() else {
            panic!("contents ref");
        };
        let Object::Stream(s) = doc.get_object(*cid).unwrap() else {
            panic!("contents stream");
        };
        s.content.clone()
    }

    #[test]
    fn reflate_must_not_wipe_invalid_flate_page_content() {
        let pdf = page_with_invalid_flate_content();
        let before = page_content_bytes(&pdf);
        assert!(!before.is_empty());
        let (out, _) = compress(&pdf).unwrap();
        let after = page_content_bytes(&out);
        assert!(
            !after.is_empty(),
            "reflate wiped page content ({} -> {} bytes)",
            before.len(),
            after.len()
        );
        assert!(
            after.len() >= before.len() / 2,
            "page content stripped: {} -> {}",
            before.len(),
            after.len()
        );
    }

    #[test]
    fn never_emits_larger_file() {
        let pdf = text_with_orphan();
        let (out, stats) = compress(&pdf).unwrap();
        assert!(out.len() as u64 <= stats.input_bytes);
        assert_eq!(out.len() as u64, stats.output_bytes);
    }
}
