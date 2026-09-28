//! Public core API: compress, inspect, prepare_form.

mod common;

use common::pdf_with_content;

fn long_text() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 700 Td (xxxxxxxxxxxxxxxxxxxxxxxx) Tj ET\n".repeat(60);
    pdf_with_content(&content)
}

#[test]
fn compress_never_grows_and_stays_inspectable() {
    let pdf = long_text();
    let (out, report) = pdfx_core::compress_file(&pdf).unwrap();
    assert!(out.len() as u64 <= report.input_bytes);
    assert_eq!(out.len() as u64, report.output_bytes);
    assert!(!report.kept_original);
    let inspect = pdfx_core::inspect_file(&out).unwrap();
    assert_eq!(inspect.pages, 1);
    assert_eq!(inspect.bytes, out.len() as u64);
    assert!(inspect.notes.iter().any(|n| n.contains("images:")));
}

#[test]
fn prepare_form_names_the_underline() {
    let pdf = pdf_with_content("BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n");
    let (out, report) = pdfx_core::prepare_form(&pdf).unwrap();
    assert!(!report.kept_original);
    assert_eq!(report.fields.len(), 1);
    assert_eq!(report.fields[0].name, "Name");
    assert_eq!(report.fields[0].kind, "text");
    assert_eq!(report.fields[0].page, 1);
    let again = pdfx_core::inspect_file(&out).unwrap();
    assert_eq!(again.pages, 1);
}

#[test]
fn prepare_form_keeps_prose() {
    let pdf = pdf_with_content("BT /F1 12 Tf 72 700 Td (Hello) Tj ET\n");
    let (out, report) = pdfx_core::prepare_form(&pdf).unwrap();
    assert!(report.kept_original);
    assert!(report.fields.is_empty());
    assert_eq!(out, pdf);
}

#[test]
fn garbage_is_rejected() {
    let bad = b"not a pdf";
    assert!(pdfx_core::compress_file(bad).is_err());
    assert!(pdfx_core::inspect_file(bad).is_err());
    assert!(pdfx_core::prepare_form(bad).is_err());
}
