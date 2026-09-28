use eframe::egui;
use pdfx_core::{run_pipeline, Step, StepReport};
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 640.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "pdfx",
        options,
        Box::new(|_cc| Ok(Box::new(PdfxApp::default()))),
    )
}

struct PdfxApp {
    source_name: Option<String>,
    source_bytes: Option<Vec<u8>>,
    steps: Vec<Step>,
    reports: Vec<StepReport>,
    output: Option<Vec<u8>>,
    error: Option<String>,
    notice: Option<String>,
    /// Set on click. The next frame paints "Working..." before the run.
    pending_run: bool,
    run_now: bool,
}

impl Default for PdfxApp {
    fn default() -> Self {
        Self {
            source_name: None,
            source_bytes: None,
            steps: Vec::new(),
            reports: Vec::new(),
            output: None,
            error: None,
            notice: None,
            pending_run: false,
            run_now: false,
        }
    }
}

enum RowAction {
    Up(usize),
    Down(usize),
    Remove(usize),
}

impl PdfxApp {
    fn update_run_state(&mut self, ctx: &egui::Context) {
        if self.pending_run && !self.run_now {
            self.run_now = true;
            ctx.request_repaint();
        } else if self.run_now {
            self.execute();
            self.pending_run = false;
            self.run_now = false;
        }
    }

    fn execute(&mut self) {
        let Some(bytes) = self.source_bytes.clone() else {
            return;
        };
        self.reports.clear();
        self.output = None;
        self.error = None;
        match run_pipeline(&bytes, &self.steps) {
            Ok((out, reports)) => {
                self.reports = reports;
                self.output = Some(out);
            }
            Err(failure) => {
                self.error = Some(failure.to_string());
                self.reports = failure.reports;
            }
        }
    }

    fn take_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        for file in dropped {
            if !is_pdf(file.path()) {
                continue;
            }
            match file.bytes() {
                Ok(bytes) => {
                    let name = file
                        .path()
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "document.pdf".to_string());
                    self.source_name = Some(name);
                    self.source_bytes = Some(bytes);
                    self.reset_for_new_file();
                }
                Err(err) => {
                    self.error = Some(format!("Could not read {}: {err}", file.path().display()));
                }
            }
            return;
        }
        self.notice = Some("Only PDF files can be opened.".to_string());
    }

    fn load_path(&mut self, path: &Path) {
        if !is_pdf(path) {
            self.notice = Some("Only PDF files can be opened.".to_string());
            return;
        }
        match fs::read(path) {
            Ok(bytes) => {
                self.source_name = Some(
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "document.pdf".to_string()),
                );
                self.source_bytes = Some(bytes);
                self.reset_for_new_file();
            }
            Err(err) => {
                self.error = Some(format!("Could not read {}: {err}", path.display()));
            }
        }
    }

    fn reset_for_new_file(&mut self) {
        self.reports.clear();
        self.output = None;
        self.error = None;
        self.notice = None;
        self.pending_run = false;
        self.run_now = false;
    }

    fn invalidate_run(&mut self) {
        self.reports.clear();
        self.output = None;
        self.error = None;
    }

    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_title("Open PDF")
            .pick_file()
        else {
            return;
        };
        self.load_path(&path);
    }

    fn save_dialog(&mut self) {
        let Some(bytes) = self.output.clone() else {
            return;
        };
        let name = self
            .source_name
            .as_deref()
            .map(suggested_save_name)
            .unwrap_or_else(|| "document.pdfx.pdf".to_string());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(&name)
            .set_title("Save PDF")
            .save_file()
        else {
            return;
        };
        let path = ensure_pdf(path);
        if let Err(err) = fs::write(&path, bytes) {
            self.error = Some(format!("Could not save {}: {err}", path.display()));
        } else {
            self.error = None;
            self.notice = Some(format!("Saved {}", path.display()));
        }
    }
}

impl eframe::App for PdfxApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.take_drops(ctx);
        self.update_run_state(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw(&ctx, ui);
                });
        });
    }
}

impl PdfxApp {
    fn draw(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.heading("pdfx");
        ui.label("Drop a PDF, add steps, and run them from top to bottom. Each step receives the previous step's file.");
        ui.add_space(8.0);

        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        self.drop_zone(ui, hovering);
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("Open PDF").clicked() {
                self.open_dialog();
            }
            if let (Some(name), Some(bytes)) = (&self.source_name, &self.source_bytes) {
                ui.label(format!("{name}  ·  {} bytes", bytes.len()));
            }
        });

        if let Some(notice) = &self.notice {
            ui.add_space(4.0);
            ui.label(notice);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label("Pipeline");
        ui.horizontal(|ui| {
            if ui.button("Add inspect").clicked() {
                self.steps.push(Step::Inspect);
                self.invalidate_run();
            }
            if ui.button("Add compress").clicked() {
                self.steps.push(Step::Compress);
                self.invalidate_run();
            }
            if ui.button("Add prepare form").clicked() {
                self.steps.push(Step::PrepareForm);
                self.invalidate_run();
            }
        });
        ui.add_space(6.0);

        if self.steps.is_empty() {
            ui.label("No steps yet. Add at least one.");
        }

        let count = self.steps.len();
        let mut action = None;
        for (index, step) in self.steps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}. {}", index + 1, step.label()));
                if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                    action = Some(RowAction::Up(index));
                }
                if ui
                    .add_enabled(index + 1 < count, egui::Button::new("Down"))
                    .clicked()
                {
                    action = Some(RowAction::Down(index));
                }
                if ui.button("Remove").clicked() {
                    action = Some(RowAction::Remove(index));
                }
            });
        }
        if let Some(action) = action {
            apply_row_action(&mut self.steps, action);
            self.invalidate_run();
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        let can_run = self.source_bytes.is_some() && !self.steps.is_empty() && !self.pending_run;
        let run_label = if self.pending_run {
            "Working..."
        } else {
            "Run"
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_run, egui::Button::new(run_label))
                .clicked()
            {
                self.pending_run = true;
                ctx.request_repaint();
            }
            if ui
                .add_enabled(self.output.is_some(), egui::Button::new("Save PDF"))
                .clicked()
            {
                self.save_dialog();
            }
        });

        if let Some(error) = &self.error {
            ui.add_space(8.0);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }

        if !self.reports.is_empty() {
            ui.add_space(12.0);
            ui.label("Report");
            for report in &self.reports {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.strong(report.step.label());
                    for line in &report.lines {
                        ui.label(line);
                    }
                });
            }
        }

        if let Some(bytes) = &self.output {
            ui.add_space(8.0);
            ui.label(format!("Final size: {} bytes", bytes.len()));
        }
    }

    fn drop_zone(&self, ui: &mut egui::Ui, hovering: bool) {
        let mut frame = egui::Frame::group(ui.style()).inner_margin(16.0);
        if hovering {
            frame = frame.stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
        }
        frame.show(ui, |ui| {
            ui.set_min_height(72.0);
            ui.vertical_centered(|ui| {
                ui.add_space(18.0);
                let title = if hovering {
                    "Drop to open"
                } else if self.source_name.is_some() {
                    "Drop another PDF to replace this one"
                } else {
                    "Drop a PDF here"
                };
                ui.label(egui::RichText::new(title).size(16.0));
                ui.add_space(18.0);
            });
        });
    }
}

fn apply_row_action(steps: &mut Vec<Step>, action: RowAction) {
    match action {
        RowAction::Up(index) if index > 0 => steps.swap(index, index - 1),
        RowAction::Down(index) if index + 1 < steps.len() => steps.swap(index, index + 1),
        RowAction::Remove(index) if index < steps.len() => {
            steps.remove(index);
        }
        _ => {}
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

fn suggested_save_name(source: &str) -> String {
    let stem = Path::new(source)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document");
    format!("{stem}.pdfx.pdf")
}

fn ensure_pdf(path: PathBuf) -> PathBuf {
    if is_pdf(&path) {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".pdf");
        PathBuf::from(name)
    }
}
