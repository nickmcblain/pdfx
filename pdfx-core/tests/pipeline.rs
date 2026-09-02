use std::io::Write;
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
    // Uncompressed content stream — plenty of Flate work.
    let body = "BT /F1 12 Tf 50 700 Td (hello hello hello hello hello) Tj ET\n".repeat(80);
    let stream = format!("<< /Length {} >>\nstream\n{}endstream\n", body.len(), body);
    // Build with lopdf via pdfx-pdf tests is nicer; keep a tiny valid file here.
    let _ = stream;
    pdfx_pdf_text()
}

fn pdfx_pdf_text() -> Vec<u8> {
    // Duplicate the text fixture path through compress/inspect only.
    // Hand-rolled PDF with a long uncompressed stream.
    let content = "BT /F1 12 Tf 72 700 Td (xxxxxxxxxxxxxxxxxxxxxxxx) Tj ET\n".repeat(60);
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
    for o in &objects {
        offsets.push(body.len());
        body.push_str(o);
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

// keep Write in scope for possible future fixture writers
#[allow(dead_code)]
fn _touch(w: &mut impl Write) {
    let _ = w;
}
