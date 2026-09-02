//! Pipeline orchestration and never-larger guard.

use pdfx_pdf::{compress as pdf_compress, inspect as pdf_inspect, PdfError};

#[derive(Debug)]
pub struct CompressReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub kept_original: bool,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub struct InspectReport {
    pub pages: u32,
    pub objects: u32,
    pub bytes: u64,
    pub notes: Vec<String>,
}

pub fn compress_file(input: &[u8]) -> Result<(Vec<u8>, CompressReport), PdfError> {
    let (out, stats) = pdf_compress(input)?;
    debug_assert!(out.len() as u64 <= stats.input_bytes);
    Ok((
        out,
        CompressReport {
            input_bytes: stats.input_bytes,
            output_bytes: stats.output_bytes,
            kept_original: stats.kept_original,
            notes: stats.notes,
        },
    ))
}

pub fn inspect_file(input: &[u8]) -> Result<InspectReport, PdfError> {
    let c = pdf_inspect(input)?;
    Ok(InspectReport {
        pages: c.pages,
        objects: c.objects,
        bytes: c.bytes,
        notes: c.notes,
    })
}
