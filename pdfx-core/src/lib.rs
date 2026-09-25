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

#[derive(Debug)]
pub struct FormField {
    pub name: String,
    pub kind: String,
    pub page: u32,
    pub rect: [f64; 4],
}

#[derive(Debug)]
pub struct FormReport {
    pub kept_original: bool,
    pub notes: Vec<String>,
    pub fields: Vec<FormField>,
}

/// Add AcroForm widgets in empty underlines, boxes, and checkboxes.
pub fn prepare_form(input: &[u8]) -> Result<(Vec<u8>, FormReport), PdfError> {
    let (out, stats) = pdfx_pdf::prepare_form(input)?;
    Ok((
        out,
        FormReport {
            kept_original: stats.kept_original,
            notes: stats.notes,
            fields: stats
                .fields
                .into_iter()
                .map(|field| FormField {
                    name: field.name,
                    kind: field.kind.as_str().to_string(),
                    page: field.page,
                    rect: field.rect,
                })
                .collect(),
        },
    ))
}
