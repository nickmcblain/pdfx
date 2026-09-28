mod common;

use common::pdf_with_content;
use std::process::Command;

#[test]
fn cli_types_roundtrip_via_lib() {
    // Core never-larger on a minimal hand-rolled PDF.
    let pdf = minimal_hello();
    let (out, report) = pdfx_core::compress_file(&pdf).unwrap();
    assert!(out.len() as u64 <= report.input_bytes);
    let inspect = pdfx_core::inspect_file(&out).unwrap();
    assert!(inspect.pages >= 1);
}

#[test]
fn compare_qpdf_and_gs_when_installed() {
    let pdf = minimal_hello();
    let (ours, _) = pdfx_core::compress_file(&pdf).unwrap();
    assert!(ours.len() <= pdf.len());

    if let Some(q) = run_qpdf(&pdf) {
        println!("qpdf={} pdfx={} orig={}", q.len(), ours.len(), pdf.len());
        assert!(ours.len() <= pdf.len());
        // We should not explode vs qpdf on a tiny text file
        assert!(ours.len() as f64 <= q.len() as f64 * 1.5 + 64.0);
    }
    if let Some(g) = run_gs(&pdf) {
        println!("gs={} pdfx={} orig={}", g.len(), ours.len(), pdf.len());
        assert!(ours.len() <= pdf.len() || g.len() < pdf.len());
    }
}

fn minimal_hello() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 700 Td (xxxxxxxxxxxxxxxxxxxxxxxx) Tj ET\n".repeat(60);
    pdf_with_content(&content)
}

fn run_qpdf(input: &[u8]) -> Option<Vec<u8>> {
    let dir = tempfile_dir()?;
    let inp = dir.join("in.pdf");
    let out = dir.join("qpdf.pdf");
    std::fs::write(&inp, input).ok()?;
    let status = Command::new("qpdf")
        .args(["--stream-data=compress", "--object-streams=generate"])
        .arg(&inp)
        .arg(&out)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    std::fs::read(out).ok()
}

fn run_gs(input: &[u8]) -> Option<Vec<u8>> {
    let dir = tempfile_dir()?;
    let inp = dir.join("in.pdf");
    let out = dir.join("gs.pdf");
    std::fs::write(&inp, input).ok()?;
    let gs = ["gs", "gswin64c"]
        .into_iter()
        .find(|b| Command::new(b).arg("-h").output().is_ok())?;
    let status = Command::new(gs)
        .args([
            "-sDEVICE=pdfwrite",
            "-dPDFSETTINGS=/ebook",
            "-dNOPAUSE",
            "-dQUIET",
            "-dBATCH",
        ])
        .arg(format!("-sOutputFile={}", out.display()))
        .arg(&inp)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    std::fs::read(out).ok()
}

fn tempfile_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("pdfx-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}
