//! Ordered inspect / compress / form runs.

mod common;

use common::pdf_with_content;
use pdfx_core::{run_pipeline, PdfError, Step};

fn long_text() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 700 Td (xxxxxxxxxxxxxxxxxxxxxxxx) Tj ET\n".repeat(60);
    pdf_with_content(&content)
}

fn underline() -> Vec<u8> {
    pdf_with_content("BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n")
}

fn encrypted() -> Vec<u8> {
    let raw = pdf_with_content("BT /F1 12 Tf 72 700 Td (Hello) Tj ET\n");
    let mut doc = lopdf::Document::load_mem(&raw).expect("load");
    doc.trailer.set(
        "ID",
        lopdf::Object::Array(vec![
            lopdf::Object::string_literal(b"pdfx-test-id-1"),
            lopdf::Object::string_literal(b"pdfx-test-id-2"),
        ]),
    );
    let version = lopdf::EncryptionVersion::V1 {
        document: &doc,
        owner_password: "owner",
        user_password: "user",
        permissions: lopdf::Permissions::all(),
    };
    let state = lopdf::EncryptionState::try_from(version).expect("state");
    doc.encrypt(&state).expect("encrypt");
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

#[test]
fn compress_then_form_matches_separate_calls() {
    let pdf = long_text();
    let (piped, reports) = run_pipeline(&pdf, &[Step::Compress, Step::PrepareForm]).unwrap();
    let (compressed, _) = pdfx_core::compress_file(&pdf).unwrap();
    let (formed, _) = pdfx_core::prepare_form(&compressed).unwrap();
    assert_eq!(piped, formed);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].step, Step::Compress);
    assert_eq!(reports[1].step, Step::PrepareForm);
}

#[test]
fn form_then_compress_matches_separate_calls() {
    let pdf = underline();
    let (piped, reports) = run_pipeline(&pdf, &[Step::PrepareForm, Step::Compress]).unwrap();
    let (formed, form) = pdfx_core::prepare_form(&pdf).unwrap();
    let (compressed, _) = pdfx_core::compress_file(&formed).unwrap();
    assert_eq!(piped, compressed);
    assert!(!form.kept_original);
    assert_eq!(reports[0].step, Step::PrepareForm);
    assert!(reports[0].lines.iter().any(|line| line.contains("Name")));
    assert_eq!(reports[1].step, Step::Compress);
}

#[test]
fn inspect_in_the_middle_does_not_change_bytes() {
    let pdf = underline();
    let steps = [Step::PrepareForm, Step::Inspect, Step::Compress];
    let (with_inspect, reports) = run_pipeline(&pdf, &steps).unwrap();
    let (without, _) = run_pipeline(&pdf, &[Step::PrepareForm, Step::Compress]).unwrap();
    assert_eq!(with_inspect, without);
    assert_eq!(reports[1].step, Step::Inspect);
    assert!(reports[1]
        .lines
        .iter()
        .any(|line| line.starts_with("pages:")));

    let (only, _) = run_pipeline(&pdf, &[Step::Inspect]).unwrap();
    assert_eq!(only, pdf);
}

#[test]
fn encrypted_file_stops_the_pipe() {
    let pdf = encrypted();
    let err = run_pipeline(&pdf, &[Step::Inspect, Step::Compress]).unwrap_err();
    assert_eq!(err.step, Step::Inspect);
    assert!(err.reports.is_empty());
    assert!(matches!(err.error, PdfError::Encrypted));
    assert!(err.to_string().contains("encrypted"));
}
