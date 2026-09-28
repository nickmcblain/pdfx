pub fn pdf_with_content(content: &str) -> Vec<u8> {
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
