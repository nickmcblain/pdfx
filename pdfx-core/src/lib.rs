//! Pipeline orchestration and never-larger guard.

pub use pdfx_pdf::PdfError;
use pdfx_pdf::{compress as pdf_compress, inspect as pdf_inspect};

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

/// One pass in a desktop or scripted pipeline. The same step may appear more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Inspect,
    Compress,
    PrepareForm,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Inspect => "Inspect",
            Step::Compress => "Compress",
            Step::PrepareForm => "Prepare form",
        }
    }
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// What one step recorded. `lines` is the text the CLI would have printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub step: Step,
    pub lines: Vec<String>,
}

/// A step rejected the PDF. `reports` are the steps that finished before it.
#[derive(Debug)]
pub struct PipelineFailure {
    pub step: Step,
    pub error: PdfError,
    pub reports: Vec<StepReport>,
}

impl std::fmt::Display for PipelineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.step, self.error)
    }
}

impl std::error::Error for PipelineFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Run `steps` in order. Each step reads the bytes the previous step wrote.
/// Inspect leaves the bytes unchanged. The first error stops the pipe.
pub fn run_pipeline(
    input: &[u8],
    steps: &[Step],
) -> Result<(Vec<u8>, Vec<StepReport>), PipelineFailure> {
    drive(input, steps, apply_step)
}

fn drive<F>(
    input: &[u8],
    steps: &[Step],
    mut apply: F,
) -> Result<(Vec<u8>, Vec<StepReport>), PipelineFailure>
where
    F: FnMut(Step, &[u8]) -> Result<(Vec<u8>, StepReport), PdfError>,
{
    let mut bytes = input.to_vec();
    let mut reports = Vec::new();
    for &step in steps {
        match apply(step, &bytes) {
            Ok((next, report)) => {
                bytes = next;
                reports.push(report);
            }
            Err(error) => {
                return Err(PipelineFailure {
                    step,
                    error,
                    reports,
                });
            }
        }
    }
    Ok((bytes, reports))
}

fn apply_step(step: Step, bytes: &[u8]) -> Result<(Vec<u8>, StepReport), PdfError> {
    let (next, lines) = match step {
        Step::Inspect => {
            let report = inspect_file(bytes)?;
            (bytes.to_vec(), inspect_lines(&report))
        }
        Step::Compress => {
            let (out, report) = compress_file(bytes)?;
            (out, compress_lines(&report))
        }
        Step::PrepareForm => {
            let (out, report) = prepare_form(bytes)?;
            (out, form_lines(&report))
        }
    };
    Ok((next, StepReport { step, lines }))
}

fn inspect_lines(report: &InspectReport) -> Vec<String> {
    let mut lines = vec![
        format!("pages: {}", report.pages),
        format!("objects: {}", report.objects),
        format!("bytes: {}", report.bytes),
    ];
    lines.extend(report.notes.iter().cloned());
    lines
}

fn compress_lines(report: &CompressReport) -> Vec<String> {
    let mut lines = vec![format!(
        "{} -> {} bytes ({})",
        report.input_bytes,
        report.output_bytes,
        if report.kept_original {
            "kept original"
        } else {
            "compressed"
        }
    )];
    lines.extend(report.notes.iter().cloned());
    lines
}

fn form_lines(report: &FormReport) -> Vec<String> {
    let mut lines = Vec::new();
    if report.kept_original {
        lines.push("no empty slots; kept original".to_string());
    } else {
        lines.push(format!("fields: {}", report.fields.len()));
        for field in &report.fields {
            lines.push(format!(
                "{} {} page {} [{:.1} {:.1} {:.1} {:.1}]",
                field.name,
                field.kind,
                field.page,
                field.rect[0],
                field.rect[1],
                field.rect[2],
                field.rect[3]
            ));
        }
    }
    for note in &report.notes {
        if !lines.iter().any(|line| line == note) {
            lines.push(note.clone());
        }
    }
    lines
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    #[test]
    fn failure_keeps_earlier_reports() {
        let err = drive(b"pdf", &[Step::Inspect, Step::Compress], |step, bytes| {
            if step == Step::Compress {
                Err(PdfError::Encrypted)
            } else {
                Ok((
                    bytes.to_vec(),
                    StepReport {
                        step,
                        lines: vec!["pages: 1".into()],
                    },
                ))
            }
        })
        .unwrap_err();
        assert_eq!(err.step, Step::Compress);
        assert!(matches!(err.error, PdfError::Encrypted));
        assert_eq!(err.reports.len(), 1);
        assert_eq!(err.reports[0].step, Step::Inspect);
        assert_eq!(err.reports[0].lines, vec!["pages: 1".to_string()]);
    }
}
