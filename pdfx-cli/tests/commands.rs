//! `pdfx` binary: inspect, compress, form.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn pdfx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pdfx"))
}

fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pdfx-cli-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pdf_with_content(content: &str) -> Vec<u8> {
    let content_len = content.len();
    let objects = [
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
        format!("4 0 obj\n<< /Length {content_len} >>\nstream\n{content}endstream\nendobj\n"),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
    ];
    let mut body = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for object in &objects {
        offsets.push(body.len());
        body.push_str(object);
    }
    let xref_at = body.len();
    body.push_str("xref\n0 6\n");
    body.push_str("0000000000 65535 f \n");
    for off in offsets {
        body.push_str(&format!("{off:010} 00000 n \n"));
    }
    body.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n"
    ));
    body.into_bytes()
}

#[test]
fn help_lists_commands() {
    let out = pdfx().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("inspect"));
    assert!(text.contains("compress"));
    assert!(text.contains("form"));
}

#[test]
fn inspect_prints_pages() {
    let dir = scratch();
    let input = dir.join("in.pdf");
    std::fs::write(&input, pdf_with_content("BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n")).unwrap();
    let out = pdfx().arg("inspect").arg(&input).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("pages:"));
    assert!(text.contains("objects:"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn compress_writes_smaller_file() {
    let dir = scratch();
    let input = dir.join("in.pdf");
    let dest = dir.join("out.pdf");
    let content = "BT /F1 12 Tf 72 700 Td (xxxxxxxxxxxxxxxxxxxxxxxx) Tj ET\n".repeat(60);
    let pdf = pdf_with_content(&content);
    std::fs::write(&input, &pdf).unwrap();
    let out = pdfx()
        .args(["compress", "--report"])
        .arg(&input)
        .args(["-o", dest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let written = std::fs::read(&dest).unwrap();
    assert!(written.len() < pdf.len());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("compressed"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn compress_default_extension() {
    let dir = scratch();
    let input = dir.join("in.pdf");
    std::fs::write(&input, pdf_with_content("BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n")).unwrap();
    let out = pdfx().arg("compress").arg(&input).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("in.pdfx.pdf").is_file());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn form_adds_a_field_and_prose_keeps_original() {
    let dir = scratch();
    let input = dir.join("form.pdf");
    let dest = dir.join("out.pdf");
    std::fs::write(
        &input,
        pdf_with_content("BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n"),
    )
    .unwrap();
    let out = pdfx()
        .arg("form")
        .arg(&input)
        .args(["-o", dest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Name"));
    assert!(text.contains("text"));
    assert!(text.contains("wrote"));

    let prose = dir.join("prose.pdf");
    std::fs::write(&prose, pdf_with_content("BT /F1 12 Tf 72 700 Td (Hello) Tj ET\n")).unwrap();
    let out = pdfx().arg("form").arg(&prose).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("kept original"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_file_and_garbage_fail() {
    let missing = pdfx().args(["inspect", "/no/such/pdfx.pdf"]).output().unwrap();
    assert!(!missing.status.success());

    let dir = scratch();
    let bad = dir.join("bad.pdf");
    std::fs::write(&bad, b"not a pdf").unwrap();
    let garbage = pdfx().arg("compress").arg(&bad).output().unwrap();
    assert!(!garbage.status.success());
    let _ = std::fs::remove_dir_all(dir);
}
