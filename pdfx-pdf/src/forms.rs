//! Turn empty slots into AcroForm fields.
//!
//! Acrobat Prepare Form looks for underlines, boxes, and checkboxes, then
//! names them from the text sitting beside the slot. Widgets are real fields
//! (`/FT /Tx` or `/FT /Btn`) so a later tool can set `/V` by name. Checkbox
//! on-state is `/Yes`. Text appearances are left to the viewer
//! (`/NeedAppearances`).

use crate::structure::{load, save_doc};
use crate::Result;
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Checkbox,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Checkbox => "checkbox",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddedField {
    pub name: String,
    pub kind: FieldKind,
    pub page: u32,
    pub rect: [f64; 4],
}

#[derive(Debug)]
pub struct FormStats {
    pub fields_added: u32,
    pub kept_original: bool,
    pub notes: Vec<String>,
    pub fields: Vec<AddedField>,
}

pub fn prepare_form(bytes: &[u8]) -> Result<(Vec<u8>, FormStats)> {
    let mut doc = load(bytes)?;
    let pages: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    let existing_names = existing_field_names(&doc);
    let mut slots = Vec::new();

    for (page_no, page_id) in &pages {
        let page = page_box(&doc, *page_id);
        let occupied = widget_rects(&doc, *page_id);
        let marks = page_marks(&doc, *page_id);
        let mut found = propose(&marks, &page, *page_no, *page_id, &occupied);
        found.sort_by(|a, b| cmp_f(b.rect[3], a.rect[3]).then(cmp_f(a.rect[0], b.rect[0])));
        slots.extend(found);
    }

    if slots.is_empty() {
        return Ok((
            bytes.to_vec(),
            FormStats {
                fields_added: 0,
                kept_original: true,
                notes: vec!["no empty slots; kept original".into()],
                fields: Vec::new(),
            },
        ));
    }

    name_slots(&mut slots, &existing_names);

    let mut by_page: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    let mut fields = Vec::new();
    for slot in &slots {
        let id = match slot.kind {
            FieldKind::Text => add_text_widget(&mut doc, slot),
            FieldKind::Checkbox => add_check_widget(&mut doc, slot),
        };
        by_page.entry(slot.page_id).or_default().push(id);
        fields.push(AddedField {
            name: slot.name.clone(),
            kind: slot.kind,
            page: slot.page,
            rect: slot.rect,
        });
    }

    let mut all_ids = Vec::new();
    for (page_id, ids) in by_page {
        append_annots(&mut doc, page_id, &ids);
        all_ids.extend(ids);
    }
    upsert_acroform(&mut doc, &all_ids)?;

    let out = save_doc(&mut doc)?;
    let text_n = fields.iter().filter(|f| f.kind == FieldKind::Text).count();
    let check_n = fields.len() - text_n;
    Ok((
        out,
        FormStats {
            fields_added: fields.len() as u32,
            kept_original: false,
            notes: vec![format!(
                "fields={} text={} checkbox={}",
                fields.len(),
                text_n,
                check_n
            )],
            fields,
        },
    ))
}

struct Slot {
    page: u32,
    page_id: ObjectId,
    kind: FieldKind,
    rect: [f64; 4],
    name: String,
}

#[derive(Clone, Copy)]
struct PageBox {
    llx: f64,
    lly: f64,
    urx: f64,
    ury: f64,
}

impl PageBox {
    fn width(self) -> f64 {
        self.urx - self.llx
    }

    fn area(self) -> f64 {
        self.width() * (self.ury - self.lly)
    }

    fn holds(self, rect: [f64; 4]) -> bool {
        let cx = (rect[0] + rect[2]) * 0.5;
        let cy = (rect[1] + rect[3]) * 0.5;
        cx >= self.llx - 4.0 && cx <= self.urx + 4.0 && cy >= self.lly - 4.0 && cy <= self.ury + 4.0
    }
}

#[derive(Default)]
struct Marks {
    rects: Vec<[f64; 4]>,
    segs: Vec<Seg>,
    texts: Vec<TextRun>,
}

#[derive(Clone)]
struct Seg {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    horiz: bool,
    vert: bool,
}

#[derive(Clone)]
struct TextRun {
    text: String,
    x: f64,
    y: f64,
    w: f64,
    size: f64,
    up: f64,
}

#[derive(Clone, Copy)]
struct HLine {
    x0: f64,
    x1: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Helvetica,
    Times,
    Courier,
    Generic,
}

type Mat = [f64; 6];

const ID: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Copy)]
struct GState {
    ctm: Mat,
    face: Face,
    font_size: f64,
    leading: f64,
    hscale: f64,
    char_space: f64,
    word_space: f64,
    line_width: f64,
    stroke_invisible: bool,
    fill_light: bool,
}

impl GState {
    fn new(ctm: Mat) -> Self {
        Self {
            ctm,
            face: Face::Generic,
            font_size: 12.0,
            leading: 0.0,
            hscale: 100.0,
            char_space: 0.0,
            word_space: 0.0,
            line_width: 1.0,
            stroke_invisible: false,
            fill_light: false,
        }
    }
}

struct Path {
    cur: Option<(f64, f64)>,
    sub_start: Option<(f64, f64)>,
    segs: Vec<Seg>,
    rects: Vec<[f64; 4]>,
}

impl Path {
    fn new() -> Self {
        Self {
            cur: None,
            sub_start: None,
            segs: Vec::new(),
            rects: Vec::new(),
        }
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

fn propose(
    marks: &Marks,
    page: &PageBox,
    page_no: u32,
    page_id: ObjectId,
    occupied: &[[f64; 4]],
) -> Vec<Slot> {
    let mut rects: Vec<[f64; 4]> = marks.rects.iter().copied().map(normalize).collect();
    rects.extend(rects_from_segs(&marks.segs));
    let mut lines = horiz_lines(&marks.segs);
    let mut flat = Vec::new();
    rects.retain(|r| {
        let w = r[2] - r[0];
        let h = r[3] - r[1];
        if h <= 7.0 && w >= 18.0 && h >= 0.3 {
            flat.push(HLine {
                x0: r[0],
                x1: r[2],
                y: (r[1] + r[3]) * 0.5,
            });
            false
        } else {
            true
        }
    });
    lines.extend(flat);
    dedup_lines(&mut lines);
    join_lines(&mut lines);
    dedup_rects(&mut rects);

    let mut blocked: Vec<[f64; 4]> = occupied.iter().copied().map(normalize).collect();
    let mut out = Vec::new();
    let blanks: Vec<HLine> = lines
        .iter()
        .copied()
        .filter(|line| !grid_border(line, &marks.segs))
        .collect();

    for run in underscore_spans(&marks.texts) {
        if run.w < 18.0 || on_vector_blank(&run, &blanks) {
            continue;
        }
        let Some(mut rect) = clamp_rect(underscore_rect(&run), *page, 8.0) else {
            continue;
        };
        let Some(clipped) = clip_off_text(rect, &marks.texts, run.y) else {
            continue;
        };
        rect = clipped;
        let Some(rect) = clamp_rect(rect, *page, 8.0) else {
            continue;
        };
        if !page.holds(rect) || blocked.iter().any(|b| overlaps(rect, *b)) {
            continue;
        }
        let name = label_left(&marks.texts, rect[0], run.y).unwrap_or_default();
        blocked.push(rect);
        out.push(Slot {
            page: page_no,
            page_id,
            kind: FieldKind::Text,
            rect,
            name,
        });
    }

    for rect in rects {
        let Some(kind) = classify_rect(rect, page) else {
            continue;
        };
        if contains_text(&marks.texts, rect) {
            continue;
        }
        let min_w = if kind == FieldKind::Checkbox {
            6.0
        } else {
            8.0
        };
        let Some(rect) = clamp_rect(rect, *page, min_w) else {
            continue;
        };
        if blocked.iter().any(|b| overlaps(rect, *b)) {
            continue;
        }
        let name = match kind {
            FieldKind::Checkbox => label_beside(&marks.texts, rect, true)
                .or_else(|| label_beside(&marks.texts, rect, false))
                .or_else(|| label_above(&marks.texts, rect))
                .unwrap_or_default(),
            FieldKind::Text => label_beside(&marks.texts, rect, false)
                .or_else(|| label_above(&marks.texts, rect))
                .or_else(|| label_beside(&marks.texts, rect, true))
                .unwrap_or_default(),
        };
        blocked.push(rect);
        out.push(Slot {
            page: page_no,
            page_id,
            kind,
            rect,
            name,
        });
    }

    for line in lines {
        if grid_border(&line, &marks.segs) || is_emphasis(&line, &marks.texts) {
            continue;
        }
        for piece in gaps_on_line(&line, &marks.texts) {
            let len = piece.x1 - piece.x0;
            if on_edge(&piece, &blocked) {
                continue;
            }
            let name = label_left(&marks.texts, piece.x0, piece.y)
                .or_else(|| label_above_line(&marks.texts, &piece))
                .unwrap_or_default();
            if name.is_empty()
                && (len < 18.0
                    || len > page.width() * 0.85
                    || piece.y < page.lly + 40.0
                    || piece.y > page.ury - 40.0)
            {
                continue;
            }
            let height = nearby_size(&marks.texts, &piece);
            let rect = [piece.x0, piece.y - 1.5, piece.x1, piece.y - 1.5 + height];
            let Some(rect) = clamp_rect(rect, *page, 8.0) else {
                continue;
            };
            if !page.holds(rect) || blocked.iter().any(|b| overlaps(rect, *b)) {
                continue;
            }
            blocked.push(rect);
            out.push(Slot {
                page: page_no,
                page_id,
                kind: FieldKind::Text,
                rect,
                name,
            });
        }
    }

    out
}

fn classify_rect(rect: [f64; 4], page: &PageBox) -> Option<FieldKind> {
    let w = rect[2] - rect[0];
    let h = rect[3] - rect[1];
    if w < 6.0 || h < 6.0 || w * h > page.area() * 0.45 || is_page_frame(rect, page) {
        return None;
    }
    let ratio = w / h;
    if w <= 24.0 && h <= 24.0 && (0.7..=1.45).contains(&ratio) {
        return Some(FieldKind::Checkbox);
    }
    if w >= 36.0 && (10.0..=220.0).contains(&h) && w <= page.width() * 0.95 {
        return Some(FieldKind::Text);
    }
    None
}

fn is_page_frame(rect: [f64; 4], page: &PageBox) -> bool {
    (rect[0] - page.llx).abs() < 8.0
        && (rect[1] - page.lly).abs() < 8.0
        && (page.urx - rect[2]).abs() < 8.0
        && (page.ury - rect[3]).abs() < 8.0
}

fn underscore_rect(run: &TextRun) -> [f64; 4] {
    let h = run.size.clamp(8.0, 18.0);
    if run.up >= 0.0 {
        [run.x, run.y - h * 0.25, run.x + run.w, run.y + h * 0.95]
    } else {
        [run.x, run.y - h * 0.95, run.x + run.w, run.y + h * 0.25]
    }
}

fn contains_text(texts: &[TextRun], rect: [f64; 4]) -> bool {
    texts.iter().any(|run| {
        if bad_label_text(&run.text) || underscore_slot(&run.text) {
            return false;
        }
        let cx = run.x + run.w * 0.5;
        let cy = run.y + run.up.signum() * run.size * 0.35;
        cx > rect[0] + 1.5 && cx < rect[2] - 1.5 && cy > rect[1] + 1.5 && cy < rect[3] - 1.5
    })
}

fn is_emphasis(line: &HLine, texts: &[TextRun]) -> bool {
    let len = line.x1 - line.x0;
    texts.iter().any(|run| {
        if run.w < 8.0 {
            return false;
        }
        let dy = run.y - line.y;
        if dy < -2.0 || dy > run.size * 1.15 {
            return false;
        }
        let ov = overlap_len(run.x, run.x + run.w, line.x0, line.x1);
        ov >= run.w * 0.55 && ov >= len * 0.55 && len <= run.w * 1.4 + 6.0
    })
}

fn on_edge(line: &HLine, blocked: &[[f64; 4]]) -> bool {
    let len = (line.x1 - line.x0).max(1.0);
    blocked.iter().any(|rect| {
        let ov = overlap_len(line.x0, line.x1, rect[0], rect[2]);
        ov / len >= 0.65 && ((line.y - rect[1]).abs() < 3.0 || (line.y - rect[3]).abs() < 3.0)
    })
}

fn nearby_size(texts: &[TextRun], line: &HLine) -> f64 {
    texts
        .iter()
        .filter(|run| (run.y - line.y).abs() < 28.0 && run.x <= line.x0 + 8.0)
        .map(|run| run.size)
        .next()
        .unwrap_or(12.0)
        .clamp(10.0, 16.0)
}

fn label_left(texts: &[TextRun], llx: f64, anchor_y: f64) -> Option<String> {
    let mut best: Option<(usize, f64)> = None;
    for (i, run) in texts.iter().enumerate() {
        if bad_label_text(&run.text) || underscore_slot(&run.text) {
            continue;
        }
        let gap = llx - (run.x + run.w);
        if !(-4.0..=80.0).contains(&gap) {
            continue;
        }
        let dy = run.y - anchor_y;
        if dy < -4.0 || dy > run.size * 0.9 + 2.0 {
            continue;
        }
        if best.map(|(_, gap_best)| gap < gap_best).unwrap_or(true) {
            best = Some((i, gap));
        }
    }
    best.and_then(|(i, _)| sanitize(&cluster(texts, i)))
}

fn label_beside(texts: &[TextRun], rect: [f64; 4], right: bool) -> Option<String> {
    let mid = (rect[1] + rect[3]) * 0.5;
    let mut best: Option<(usize, f64)> = None;
    for (i, run) in texts.iter().enumerate() {
        if bad_label_text(&run.text) || underscore_slot(&run.text) {
            continue;
        }
        let run_mid = run.y + run.up.signum() * run.size * 0.3;
        let tol = (run.size * 0.85).max((rect[3] - rect[1]) * 0.7);
        if (run_mid - mid).abs() > tol {
            continue;
        }
        let gap = if right {
            run.x - rect[2]
        } else {
            rect[0] - (run.x + run.w)
        };
        if !(-6.0..=90.0).contains(&gap) {
            continue;
        }
        if best.map(|(_, gap_best)| gap < gap_best).unwrap_or(true) {
            best = Some((i, gap));
        }
    }
    best.and_then(|(i, _)| sanitize(&cluster(texts, i)))
}

fn label_above(texts: &[TextRun], rect: [f64; 4]) -> Option<String> {
    let mut best: Option<(usize, f64)> = None;
    for (i, run) in texts.iter().enumerate() {
        if bad_label_text(&run.text) || underscore_slot(&run.text) {
            continue;
        }
        let dy = run.y - rect[3];
        if !(-2.0..=36.0).contains(&dy) {
            continue;
        }
        let ov = overlap_len(run.x, run.x + run.w, rect[0], rect[2]);
        let near_left = (run.x - rect[0]).abs() < 48.0;
        if ov < 8.0 && !near_left {
            continue;
        }
        if best.map(|(_, dy_best)| dy < dy_best).unwrap_or(true) {
            best = Some((i, dy));
        }
    }
    best.and_then(|(i, _)| sanitize(&cluster(texts, i)))
}

fn label_above_line(texts: &[TextRun], line: &HLine) -> Option<String> {
    let mut best: Option<(usize, f64)> = None;
    for (i, run) in texts.iter().enumerate() {
        if bad_label_text(&run.text) || underscore_slot(&run.text) {
            continue;
        }
        let dy = run.y - line.y;
        if !(1.0..=32.0).contains(&dy) {
            continue;
        }
        let ov = overlap_len(run.x, run.x + run.w, line.x0, line.x1);
        if ov < 10.0 && (run.x - line.x0).abs() > 40.0 {
            continue;
        }
        if best.map(|(_, dy_best)| dy < dy_best).unwrap_or(true) {
            best = Some((i, dy));
        }
    }
    best.and_then(|(i, _)| sanitize(&cluster(texts, i)))
}

fn cluster(texts: &[TextRun], seed: usize) -> String {
    let seed_y = texts[seed].y;
    let seed_size = texts[seed].size;
    let mut idx = vec![seed];
    let mut grew = true;
    while grew {
        grew = false;
        for (i, run) in texts.iter().enumerate() {
            if idx.contains(&i) || underscore_slot(&run.text) || run.text.trim().is_empty() {
                continue;
            }
            if (run.y - seed_y).abs() > (seed_size * 0.45).max(3.5) {
                continue;
            }
            let close = idx.iter().any(|&j| {
                let other = &texts[j];
                let gap = if run.x >= other.x {
                    run.x - (other.x + other.w)
                } else {
                    other.x - (run.x + run.w)
                };
                (-1.5..16.0).contains(&gap)
            });
            if close {
                idx.push(i);
                grew = true;
            }
        }
    }
    idx.sort_by(|&a, &b| cmp_f(texts[a].x, texts[b].x));
    let mut out = String::new();
    let mut prev_right: Option<f64> = None;
    for i in idx {
        let run = &texts[i];
        if let Some(right) = prev_right {
            if run.x - right > 1.4 && !out.ends_with(' ') && !run.text.starts_with(' ') {
                out.push(' ');
            }
        }
        out.push_str(&run.text);
        prev_right = Some(run.x + run.w);
    }
    out
}

fn bad_label_text(text: &str) -> bool {
    text.trim().is_empty() || underscore_slot(text.trim())
}

fn underscore_slot(text: &str) -> bool {
    let t = text.trim();
    t.len() >= 3 && t.chars().all(|c| c == '_')
}

/// Join `___/___` into one blank. The slash is a few points, not a new field.
fn underscore_spans(texts: &[TextRun]) -> Vec<TextRun> {
    let mut spans: Vec<TextRun> = texts
        .iter()
        .filter(|run| {
            let t = run.text.trim();
            !t.is_empty() && t.chars().all(|c| c == '_')
        })
        .cloned()
        .collect();
    spans.sort_by(|a, b| cmp_f(a.y, b.y).then(cmp_f(a.x, b.x)));
    let mut out: Vec<TextRun> = Vec::new();
    for run in spans {
        if let Some(prev) = out.last_mut() {
            let gap = run.x - (prev.x + prev.w);
            if (prev.y - run.y).abs() < 2.5 && (-1.0..14.0).contains(&gap) {
                let right = run.x + run.w;
                prev.w = right - prev.x;
                prev.text.push_str(&run.text);
                continue;
            }
        }
        out.push(run);
    }
    out
}

fn on_vector_blank(run: &TextRun, lines: &[HLine]) -> bool {
    let x1 = run.x + run.w;
    lines.iter().any(|line| {
        let dy = run.y - line.y;
        if !(-1.5..=6.0).contains(&dy) {
            return false;
        }
        let ov = overlap_len(run.x, x1, line.x0, line.x1);
        ov >= 12.0 && ov >= (x1 - run.x).min(line.x1 - line.x0) * 0.45
    })
}

fn clip_off_text(rect: [f64; 4], texts: &[TextRun], baseline: f64) -> Option<[f64; 4]> {
    let mut x0 = rect[0];
    let mut x1 = rect[2];
    for run in texts {
        if bad_label_text(&run.text) || !run.text.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let dy = (run.y - baseline).abs();
        if dy > run.size * 0.6 + 2.0 {
            continue;
        }
        let rx0 = run.x;
        let rx1 = run.x + run.w;
        if rx1 <= x0 + 0.8 || rx0 >= x1 - 0.8 {
            continue;
        }
        if rx0 <= x0 + 1.0 && rx1 < x1 {
            x0 = rx1;
        } else if rx1 >= x1 - 1.0 && rx0 > x0 {
            x1 = rx0;
        } else if rx0 > x0 && rx1 < x1 {
            if x1 - rx1 >= rx0 - x0 {
                x0 = rx1;
            } else {
                x1 = rx0;
            }
        }
    }
    if x1 - x0 < 18.0 {
        None
    } else {
        Some([x0, rect[1], x1, rect[3]])
    }
}

/// A long rule crossed by two or more verticals is a table or box edge.
fn grid_border(line: &HLine, segs: &[Seg]) -> bool {
    let len = line.x1 - line.x0;
    if len < 80.0 {
        return false;
    }
    let mut xs = Vec::new();
    for seg in segs {
        if !seg.vert {
            continue;
        }
        let vx = (seg.x0 + seg.x1) * 0.5;
        if vx < line.x0 - 1.5 || vx > line.x1 + 1.5 {
            continue;
        }
        let y0 = seg.y0.min(seg.y1);
        let y1 = seg.y0.max(seg.y1);
        if line.y < y0 - 2.0 || line.y > y1 + 2.0 {
            continue;
        }
        xs.push(vx);
    }
    xs.sort_by(|a, b| cmp_f(*a, *b));
    let mut count = 0;
    let mut last = f64::NEG_INFINITY;
    for x in xs {
        if x - last > 2.5 {
            count += 1;
            last = x;
        }
    }
    count >= 2
}

/// Cut the stretches of a rule that sit under words. One line can hold several blanks.
fn gaps_on_line(line: &HLine, texts: &[TextRun]) -> Vec<HLine> {
    let mut cuts: Vec<(f64, f64)> = Vec::new();
    for run in texts {
        if run.w < 1.0 || bad_label_text(&run.text) {
            continue;
        }
        let dy = run.y - line.y;
        if dy < -2.0 || dy > run.size + 2.0 {
            continue;
        }
        if overlap_len(run.x, run.x + run.w, line.x0, line.x1) < 2.0 {
            continue;
        }
        let a = (run.x - 0.5).max(line.x0);
        let b = (run.x + run.w + 0.5).min(line.x1);
        if b > a {
            cuts.push((a, b));
        }
    }
    cuts.sort_by(|a, b| cmp_f(a.0, b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in cuts {
        if let Some(prev) = merged.last_mut() {
            if a <= prev.1 + 1.0 {
                prev.1 = prev.1.max(b);
                continue;
            }
        }
        merged.push((a, b));
    }
    let mut gaps = Vec::new();
    let mut cursor = line.x0;
    for (a, b) in merged {
        if a - cursor >= 20.0 {
            gaps.push(HLine {
                x0: cursor,
                x1: a,
                y: line.y,
            });
        }
        cursor = cursor.max(b);
    }
    if line.x1 - cursor >= 20.0 {
        gaps.push(HLine {
            x0: cursor,
            x1: line.x1,
            y: line.y,
        });
    }
    gaps
}

fn sanitize(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '•' | '·') || c.is_whitespace());
    let trimmed = trimmed.trim_end_matches(|c: char| matches!(c, ':' | '.' | '*' | ' ' | '\t'));
    let trimmed = trimmed.trim();
    if trimmed.chars().filter(|c| c.is_alphanumeric()).count() < 1 {
        return None;
    }
    let words = trimmed.split_whitespace().count();
    if words > 8 || trimmed.chars().count() > 48 {
        return None;
    }
    let mut s = String::new();
    for c in trimmed.chars() {
        if c.is_control() {
            continue;
        }
        if c == '.' {
            s.push('_');
        } else {
            s.push(c);
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn name_slots(slots: &mut [Slot], existing: &HashSet<String>) {
    let mut used = existing.clone();
    let mut text_n = 1u32;
    let mut check_n = 1u32;
    for slot in slots.iter_mut() {
        if slot.name.is_empty() {
            loop {
                let candidate = match slot.kind {
                    FieldKind::Text => {
                        let name = format!("Text{text_n}");
                        text_n += 1;
                        name
                    }
                    FieldKind::Checkbox => {
                        let name = format!("Check{check_n}");
                        check_n += 1;
                        name
                    }
                };
                if !used.contains(&candidate) {
                    slot.name = candidate;
                    break;
                }
            }
        } else if used.contains(&slot.name) {
            let base = slot.name.clone();
            let mut n = 2u32;
            loop {
                let candidate = format!("{base}_{n}");
                if !used.contains(&candidate) {
                    slot.name = candidate;
                    break;
                }
                n += 1;
            }
        }
        used.insert(slot.name.clone());
    }
}

fn horiz_lines(segs: &[Seg]) -> Vec<HLine> {
    let mut lines = Vec::new();
    for seg in segs {
        if !seg.horiz {
            continue;
        }
        let x0 = seg.x0.min(seg.x1);
        let x1 = seg.x0.max(seg.x1);
        if x1 - x0 < 8.0 {
            continue;
        }
        lines.push(HLine {
            x0,
            x1,
            y: (seg.y0 + seg.y1) * 0.5,
        });
    }
    lines
}

/// A slash between `___` and `___` is a few points. A letter between blanks is wider.
fn join_lines(lines: &mut Vec<HLine>) {
    lines.sort_by(|a, b| cmp_f(a.y, b.y).then(cmp_f(a.x0, b.x0)));
    let mut out: Vec<HLine> = Vec::new();
    for line in lines.drain(..) {
        if let Some(prev) = out.last_mut() {
            let gap = line.x0 - prev.x1;
            if (prev.y - line.y).abs() < 1.6 && (-1.0..=6.0).contains(&gap) {
                prev.x1 = prev.x1.max(line.x1);
                continue;
            }
        }
        out.push(line);
    }
    *lines = out;
}

fn dedup_lines(lines: &mut Vec<HLine>) {
    lines.sort_by(|a, b| cmp_f(a.y, b.y).then(cmp_f(a.x0, b.x0)));
    let mut out: Vec<HLine> = Vec::new();
    for line in lines.drain(..) {
        if let Some(prev) = out.last_mut() {
            let shorter = (prev.x1 - prev.x0).min(line.x1 - line.x0);
            let ov = overlap_len(prev.x0, prev.x1, line.x0, line.x1);
            if (prev.y - line.y).abs() < 2.0 && shorter > 0.0 && ov / shorter > 0.8 {
                prev.x0 = prev.x0.min(line.x0);
                prev.x1 = prev.x1.max(line.x1);
                continue;
            }
        }
        out.push(line);
    }
    *lines = out;
}

fn rects_from_segs(segs: &[Seg]) -> Vec<[f64; 4]> {
    let horiz: Vec<&Seg> = segs.iter().filter(|s| s.horiz).collect();
    let vert: Vec<&Seg> = segs.iter().filter(|s| s.vert).collect();
    let mut out = Vec::new();
    for (i, a) in horiz.iter().enumerate() {
        for b in horiz.iter().skip(i + 1) {
            let y0 = a.y0.min(b.y0);
            let y1 = a.y0.max(b.y0);
            let height = y1 - y0;
            if !(6.0..=220.0).contains(&height) {
                continue;
            }
            let left = a.x0.max(b.x0) - 3.0;
            let right = a.x1.min(b.x1) + 3.0;
            if right - left < 8.0 {
                continue;
            }
            let mut xs = Vec::new();
            for v in &vert {
                let vx = (v.x0 + v.x1) * 0.5;
                if vx < left || vx > right {
                    continue;
                }
                let cover = (v.y1.min(y1) - v.y0.max(y0)).max(0.0);
                if cover >= height * 0.7 {
                    xs.push(vx);
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|p, q| cmp_f(*p, *q));
            let mut clustered = Vec::new();
            for x in xs {
                if clustered
                    .last()
                    .map(|prev: &f64| (x - prev).abs() < 2.5)
                    .unwrap_or(false)
                {
                    continue;
                }
                clustered.push(x);
            }
            for pair in clustered.windows(2) {
                let w = pair[1] - pair[0];
                if (6.0..=500.0).contains(&w) {
                    out.push([pair[0], y0, pair[1], y1]);
                }
            }
        }
    }
    out
}

fn dedup_rects(rects: &mut Vec<[f64; 4]>) {
    let mut out: Vec<[f64; 4]> = Vec::new();
    for rect in rects.drain(..) {
        if out.iter().any(|prev| overlaps(rect, *prev)) {
            continue;
        }
        out.push(rect);
    }
    *rects = out;
}

fn page_marks(doc: &Document, page_id: ObjectId) -> Marks {
    let mut marks = Marks::default();
    let Ok(bytes) = doc.get_page_content(page_id) else {
        return marks;
    };
    let xobjects = page_xobjects(doc, page_id);
    let fonts = page_fonts(doc, page_id);
    let mut seen = HashSet::new();
    interpret(doc, &bytes, ID, &xobjects, &fonts, &mut marks, 0, &mut seen);
    marks
}

fn interpret(
    doc: &Document,
    data: &[u8],
    ctm: Mat,
    xobjects: &HashMap<Vec<u8>, ObjectId>,
    fonts: &HashMap<Vec<u8>, Face>,
    marks: &mut Marks,
    depth: u32,
    seen: &mut HashSet<ObjectId>,
) {
    if depth > 8 {
        return;
    }
    let Ok(content) = Content::decode(data) else {
        return;
    };
    let mut gs = GState::new(ctm);
    let mut stack: Vec<GState> = Vec::new();
    let mut path = Path::new();
    let mut in_text = false;
    let mut tm = ID;
    let mut tlm = ID;

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => stack.push(gs),
            "Q" => {
                if let Some(prev) = stack.pop() {
                    gs = prev;
                }
            }
            "cm" => {
                if let Some(m) = mat_operands(&op.operands) {
                    gs.ctm = mul(m, gs.ctm);
                }
            }
            "w" => {
                if let Some(w) = nth_num(&op.operands, 0) {
                    gs.line_width = w;
                }
            }
            "g" => {
                if let Some(gray) = nth_num(&op.operands, 0) {
                    gs.fill_light = gray >= 0.94;
                }
            }
            "G" => {
                if let Some(gray) = nth_num(&op.operands, 0) {
                    gs.stroke_invisible = gray >= 0.94;
                }
            }
            "rg" => gs.fill_light = rgb_light(&op.operands),
            "RG" => gs.stroke_invisible = rgb_light(&op.operands),
            "k" => gs.fill_light = cmyk_light(&op.operands),
            "K" => gs.stroke_invisible = cmyk_light(&op.operands),
            "m" => {
                if let (Some(x), Some(y)) = (nth_num(&op.operands, 0), nth_num(&op.operands, 1)) {
                    path.cur = Some((x, y));
                    path.sub_start = Some((x, y));
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (nth_num(&op.operands, 0), nth_num(&op.operands, 1)) {
                    line_to(&mut path, gs.ctm, x, y);
                }
            }
            "h" => close_path(&mut path, gs.ctm),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    nth_num(&op.operands, 0),
                    nth_num(&op.operands, 1),
                    nth_num(&op.operands, 2),
                    nth_num(&op.operands, 3),
                ) {
                    rect_to(&mut path, gs.ctm, x, y, w, h);
                }
            }
            "c" => {
                if let (Some(x), Some(y)) = (nth_num(&op.operands, 4), nth_num(&op.operands, 5)) {
                    path.cur = Some((x, y));
                }
            }
            "v" | "y" => {
                if let (Some(x), Some(y)) = (nth_num(&op.operands, 2), nth_num(&op.operands, 3)) {
                    path.cur = Some((x, y));
                }
            }
            "S" => paint(&mut path, marks, &gs, true, false),
            "s" => {
                close_path(&mut path, gs.ctm);
                paint(&mut path, marks, &gs, true, false);
            }
            "f" | "F" | "f*" => paint(&mut path, marks, &gs, false, true),
            "B" | "B*" => paint(&mut path, marks, &gs, true, true),
            "b" | "b*" => {
                close_path(&mut path, gs.ctm);
                paint(&mut path, marks, &gs, true, true);
            }
            "n" => path.clear(),
            "BT" => {
                in_text = true;
                tm = ID;
                tlm = ID;
            }
            "ET" => in_text = false,
            "Tm" => {
                if let Some(m) = mat_operands(&op.operands) {
                    tm = m;
                    tlm = m;
                }
            }
            "Td" => {
                if let (Some(tx), Some(ty)) = (nth_num(&op.operands, 0), nth_num(&op.operands, 1)) {
                    text_move(&mut tlm, &mut tm, tx, ty);
                }
            }
            "TD" => {
                if let (Some(tx), Some(ty)) = (nth_num(&op.operands, 0), nth_num(&op.operands, 1)) {
                    gs.leading = -ty;
                    text_move(&mut tlm, &mut tm, tx, ty);
                }
            }
            "T*" => text_move(&mut tlm, &mut tm, 0.0, -gs.leading),
            "TL" => {
                if let Some(lead) = nth_num(&op.operands, 0) {
                    gs.leading = lead;
                }
            }
            "Tc" => {
                if let Some(sp) = nth_num(&op.operands, 0) {
                    gs.char_space = sp;
                }
            }
            "Tw" => {
                if let Some(sp) = nth_num(&op.operands, 0) {
                    gs.word_space = sp;
                }
            }
            "Tz" => {
                if let Some(scale) = nth_num(&op.operands, 0) {
                    gs.hscale = scale;
                }
            }
            "Tf" => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    gs.face = fonts.get(name).copied().unwrap_or(Face::Generic);
                }
                if let Some(size) = nth_num(&op.operands, 1) {
                    gs.font_size = size.abs().max(0.1);
                }
            }
            "Tj" => {
                if in_text {
                    if let Some(bytes) = op.operands.first().and_then(|o| o.as_str().ok()) {
                        emit_string(marks, &gs, &mut tm, bytes);
                    }
                }
            }
            "TJ" => {
                if in_text {
                    if let Some(Object::Array(items)) = op.operands.first() {
                        for item in items {
                            match item {
                                Object::String(bytes, _) => emit_string(marks, &gs, &mut tm, bytes),
                                Object::Integer(n) => kern(&mut tm, &gs, *n as f64),
                                Object::Real(n) => kern(&mut tm, &gs, *n as f64),
                                _ => {}
                            }
                        }
                    }
                }
            }
            "'" => {
                if in_text {
                    text_move(&mut tlm, &mut tm, 0.0, -gs.leading);
                    if let Some(bytes) = op.operands.first().and_then(|o| o.as_str().ok()) {
                        emit_string(marks, &gs, &mut tm, bytes);
                    }
                }
            }
            "\"" => {
                if in_text {
                    if let Some(aw) = nth_num(&op.operands, 0) {
                        gs.word_space = aw;
                    }
                    if let Some(ac) = nth_num(&op.operands, 1) {
                        gs.char_space = ac;
                    }
                    text_move(&mut tlm, &mut tm, 0.0, -gs.leading);
                    if let Some(bytes) = op.operands.get(2).and_then(|o| o.as_str().ok()) {
                        emit_string(marks, &gs, &mut tm, bytes);
                    }
                }
            }
            "Do" => {
                let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else {
                    continue;
                };
                let Some(&id) = xobjects.get(name) else {
                    continue;
                };
                if depth >= 8 || !seen.insert(id) {
                    continue;
                }
                if let Some((bytes, matrix, child, child_fonts)) =
                    form_source(doc, id, xobjects, fonts)
                {
                    interpret(
                        doc,
                        &bytes,
                        mul(matrix, gs.ctm),
                        &child,
                        &child_fonts,
                        marks,
                        depth + 1,
                        seen,
                    );
                }
                seen.remove(&id);
            }
            _ => {}
        }
    }
}

fn paint(path: &mut Path, marks: &mut Marks, gs: &GState, stroke: bool, fill: bool) {
    let stroke_ok = stroke && !gs.stroke_invisible && gs.line_width <= 5.0;
    let fill_ok = fill && gs.fill_light;
    if stroke_ok || fill_ok {
        marks.rects.extend(path.rects.iter().copied());
        if stroke_ok {
            marks.segs.extend(path.segs.iter().cloned());
        }
    } else if fill {
        // A black rule is often a filled sliver, not a stroke. A solid box is not.
        keep_ink_rules(path, marks);
    }
    path.clear();
}

fn keep_ink_rules(path: &Path, marks: &mut Marks) {
    let mut y0 = f64::INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    for seg in &path.segs {
        y0 = y0.min(seg.y0.min(seg.y1));
        y1 = y1.max(seg.y0.max(seg.y1));
    }
    for rect in &path.rects {
        y0 = y0.min(rect[1]);
        y1 = y1.max(rect[3]);
    }
    if !y0.is_finite() || y1 - y0 > 3.5 {
        return;
    }
    for seg in &path.segs {
        if seg.horiz && seg.x1 - seg.x0 >= 8.0 {
            marks.segs.push(seg.clone());
        }
    }
    for rect in &path.rects {
        let w = rect[2] - rect[0];
        if w >= 8.0 && rect[3] - rect[1] <= 3.5 {
            let y = (rect[1] + rect[3]) * 0.5;
            marks.segs.push(Seg {
                x0: rect[0],
                y0: y,
                x1: rect[2],
                y1: y,
                horiz: true,
                vert: false,
            });
        }
    }
}

fn line_to(path: &mut Path, ctm: Mat, x: f64, y: f64) {
    let Some((x0, y0)) = path.cur else {
        path.cur = Some((x, y));
        path.sub_start = Some((x, y));
        return;
    };
    let p0 = apply(ctm, x0, y0);
    let p1 = apply(ctm, x, y);
    let dx = (p1.0 - p0.0).abs();
    let dy = (p1.1 - p0.1).abs();
    let horiz = dy <= 1.6 && dx >= 2.0;
    let vert = dx <= 1.6 && dy >= 6.0;
    if horiz || vert {
        path.segs.push(Seg {
            x0: p0.0.min(p1.0),
            y0: if horiz {
                (p0.1 + p1.1) * 0.5
            } else {
                p0.1.min(p1.1)
            },
            x1: p0.0.max(p1.0),
            y1: if horiz {
                (p0.1 + p1.1) * 0.5
            } else {
                p0.1.max(p1.1)
            },
            horiz,
            vert,
        });
    }
    path.cur = Some((x, y));
}

fn close_path(path: &mut Path, ctm: Mat) {
    if let (Some(_), Some((x, y))) = (path.cur, path.sub_start) {
        line_to(path, ctm, x, y);
        path.cur = Some((x, y));
    }
}

fn rect_to(path: &mut Path, ctm: Mat, x: f64, y: f64, w: f64, h: f64) {
    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    let mapped: Vec<(f64, f64)> = corners
        .iter()
        .map(|(px, py)| apply(ctm, *px, *py))
        .collect();
    let min_x = mapped.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = mapped.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_x = mapped.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let max_y = mapped.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if max_x.is_finite() && max_y.is_finite() {
        path.rects.push([min_x, min_y, max_x, max_y]);
    }
    path.cur = Some((x, y));
    path.sub_start = Some((x, y));
}

fn emit_string(marks: &mut Marks, gs: &GState, tm: &mut Mat, raw: &[u8]) {
    let text = pdf_to_string(raw);
    if text.is_empty() {
        return;
    }
    let placed = mul(*tm, gs.ctm);
    let (x0, y0) = apply(placed, 0.0, 0.0);
    let (xu, yu) = apply(placed, 0.0, gs.font_size);
    let horizontal = (xu - x0).abs() + 0.5 <= (yu - y0).abs();
    let size = hypot(xu - x0, yu - y0).max(1.0);
    let up = yu - y0;

    let mut buf = String::new();
    let mut in_under: Option<bool> = None;
    for ch in text.chars() {
        let under = ch == '_';
        if in_under.map(|flag| flag != under).unwrap_or(false) {
            flush_run(marks, gs, tm, &buf, horizontal, size, up);
            buf.clear();
        }
        in_under = Some(under);
        buf.push(ch);
    }
    if !buf.is_empty() {
        flush_run(marks, gs, tm, &buf, horizontal, size, up);
    }
}

fn flush_run(
    marks: &mut Marks,
    gs: &GState,
    tm: &mut Mat,
    text: &str,
    horizontal: bool,
    size: f64,
    up: f64,
) {
    let tx = displacement(text, gs);
    if horizontal && !text.is_empty() {
        let placed = mul(*tm, gs.ctm);
        let (x0, y0) = apply(placed, 0.0, 0.0);
        let (x1, _) = apply(placed, tx, 0.0);
        marks.texts.push(TextRun {
            text: text.to_string(),
            x: x0.min(x1),
            y: y0,
            w: (x1 - x0).abs(),
            size,
            up,
        });
    }
    *tm = mul([1.0, 0.0, 0.0, 1.0, tx, 0.0], *tm);
}

fn displacement(text: &str, gs: &GState) -> f64 {
    let th = gs.hscale / 100.0;
    let mut tx = 0.0;
    for ch in text.chars() {
        let mut adv = em_width(gs.face, ch) * gs.font_size + gs.char_space;
        if ch == ' ' {
            adv += gs.word_space;
        }
        tx += adv * th;
    }
    tx
}

fn kern(tm: &mut Mat, gs: &GState, n: f64) {
    let tx = -n / 1000.0 * gs.font_size * (gs.hscale / 100.0);
    *tm = mul([1.0, 0.0, 0.0, 1.0, tx, 0.0], *tm);
}

fn text_move(tlm: &mut Mat, tm: &mut Mat, tx: f64, ty: f64) {
    *tlm = mul([1.0, 0.0, 0.0, 1.0, tx, ty], *tlm);
    *tm = *tlm;
}

fn em_width(face: Face, ch: char) -> f64 {
    match face {
        Face::Courier => 0.6,
        Face::Helvetica => table_width(&HELVETICA, ch).unwrap_or_else(|| char_em(ch)),
        Face::Times => table_width(&TIMES_ROMAN, ch).unwrap_or_else(|| char_em(ch)),
        Face::Generic => char_em(ch),
    }
}

fn table_width(table: &[u16; 95], ch: char) -> Option<f64> {
    let b = u32::from(ch);
    if (32..=126).contains(&b) {
        Some(f64::from(table[(b - 32) as usize]) / 1000.0)
    } else {
        None
    }
}

fn char_em(c: char) -> f64 {
    match c {
        ' ' | '\t' => 0.28,
        'i' | 'l' | 'I' | 'j' | 't' | 'f' | 'r' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' => 0.28,
        '_' | '-' => 0.50,
        'm' | 'w' | 'M' | 'W' => 0.86,
        c if c.is_uppercase() => 0.70,
        c if c.is_ascii_digit() => 0.56,
        _ => 0.50,
    }
}

// AFM widths, ASCII 32..=126, in 1000-unit ems. Bold and italic map to the roman face.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

const TIMES_ROMAN: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611,
    556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722,
    722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500,
    278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];

fn face_from_base(name: &[u8]) -> Face {
    let name = strip_subset(name);
    let raw = String::from_utf8_lossy(name);
    let folded: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    if folded.contains("courier") || folded.contains("nimbusmon") {
        return Face::Courier;
    }
    if folded.contains("times")
        || folded.contains("liberationserif")
        || folded.contains("nimbusrom")
    {
        return Face::Times;
    }
    if folded.contains("helvetica")
        || folded.contains("arial")
        || folded.contains("nimbussans")
        || folded.contains("liberationsans")
    {
        return Face::Helvetica;
    }
    Face::Generic
}

fn strip_subset(name: &[u8]) -> &[u8] {
    if name.len() > 7 && name[6] == b'+' && name[..6].iter().all(|b| b.is_ascii_uppercase()) {
        &name[7..]
    } else {
        name
    }
}

fn pdf_to_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|c| {
                let pair: [u8; 2] = c.try_into().ok()?;
                Some(u16::from_be_bytes(pair))
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|&b| b as char).collect()
}

fn pdf_text(s: &str) -> Object {
    if s.chars().all(|c| (c as u32) <= 0xFF) {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        return Object::string_literal(bytes);
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in s.encode_utf16() {
        bytes.extend(unit.to_be_bytes());
    }
    Object::string_literal(bytes)
}

fn form_source(
    doc: &Document,
    id: ObjectId,
    parent: &HashMap<Vec<u8>, ObjectId>,
    parent_fonts: &HashMap<Vec<u8>, Face>,
) -> Option<(
    Vec<u8>,
    Mat,
    HashMap<Vec<u8>, ObjectId>,
    HashMap<Vec<u8>, Face>,
)> {
    let (dict, bytes) = {
        let stream = doc.get_object(id).ok()?.as_stream().ok()?;
        let subtype = stream.dict.get(b"Subtype").ok()?.as_name().ok()?;
        if subtype != b"Form" {
            return None;
        }
        let raw = stream.content.clone();
        let bytes = match stream.decompressed_content() {
            Ok(decoded) if !decoded.is_empty() || raw.is_empty() => decoded,
            _ => raw,
        };
        (stream.dict.clone(), bytes)
    };
    let matrix = matrix_from(dict.get(b"Matrix").ok());
    let (child, fonts) = match dict.get(b"Resources") {
        Ok(Object::Dictionary(resources)) => resource_maps(doc, resources, parent_fonts),
        Ok(Object::Reference(rid)) => match doc.get_dictionary(*rid) {
            Ok(resources) => resource_maps(doc, resources, parent_fonts),
            Err(_) => (parent.clone(), parent_fonts.clone()),
        },
        _ => (parent.clone(), parent_fonts.clone()),
    };
    Some((bytes, matrix, child, fonts))
}

fn resource_maps(
    doc: &Document,
    resources: &Dictionary,
    parent_fonts: &HashMap<Vec<u8>, Face>,
) -> (HashMap<Vec<u8>, ObjectId>, HashMap<Vec<u8>, Face>) {
    let mut xobjects = HashMap::new();
    insert_xobjects(doc, resources, &mut xobjects);
    let mut fonts = HashMap::new();
    insert_fonts(doc, resources, &mut fonts);
    if fonts.is_empty() {
        fonts = parent_fonts.clone();
    }
    (xobjects, fonts)
}

fn page_xobjects(doc: &Document, page_id: ObjectId) -> HashMap<Vec<u8>, ObjectId> {
    let mut map = HashMap::new();
    for dict in page_resource_dicts(doc, page_id) {
        insert_xobjects(doc, &dict, &mut map);
    }
    map
}

fn page_fonts(doc: &Document, page_id: ObjectId) -> HashMap<Vec<u8>, Face> {
    let mut map = HashMap::new();
    for dict in page_resource_dicts(doc, page_id) {
        insert_fonts(doc, &dict, &mut map);
    }
    map
}

fn page_resource_dicts(doc: &Document, page_id: ObjectId) -> Vec<Dictionary> {
    let mut chain = Vec::new();
    let mut current = Some(page_id);
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > 20 {
            break;
        }
        let step = {
            let Ok(dict) = doc.get_dictionary(id) else {
                break;
            };
            let parent = match dict.get(b"Parent") {
                Ok(Object::Reference(pid)) => Some(*pid),
                _ => None,
            };
            let resources = dict.get(b"Resources").ok().cloned();
            (resources, parent)
        };
        if let Some(resources) = step.0 {
            match resources {
                Object::Dictionary(dict) => chain.push(dict),
                Object::Reference(rid) => {
                    if let Ok(dict) = doc.get_dictionary(rid) {
                        chain.push(dict.clone());
                    }
                }
                _ => {}
            }
        }
        current = step.1;
    }
    chain.reverse();
    chain
}

fn insert_fonts(doc: &Document, dict: &Dictionary, map: &mut HashMap<Vec<u8>, Face>) {
    let fonts = match dict.get(b"Font") {
        Ok(Object::Dictionary(inner)) => inner.clone(),
        Ok(Object::Reference(id)) => match doc.get_dictionary(*id) {
            Ok(inner) => inner.clone(),
            Err(_) => return,
        },
        _ => return,
    };
    for (name, value) in fonts.iter() {
        let Ok(id) = value.as_reference() else {
            continue;
        };
        let Ok(font) = doc.get_dictionary(id) else {
            continue;
        };
        let Some(base) = font.get(b"BaseFont").ok().and_then(|o| o.as_name().ok()) else {
            continue;
        };
        map.insert(name.clone(), face_from_base(base));
    }
}

fn insert_xobjects(doc: &Document, dict: &Dictionary, map: &mut HashMap<Vec<u8>, ObjectId>) {
    let resources = match dict.get(b"XObject") {
        Ok(Object::Dictionary(inner)) => inner.clone(),
        Ok(Object::Reference(id)) => match doc.get_dictionary(*id) {
            Ok(inner) => inner.clone(),
            Err(_) => return,
        },
        _ => return,
    };
    for (name, value) in resources.iter() {
        if let Ok(id) = value.as_reference() {
            map.insert(name.clone(), id);
        }
    }
}

fn page_box(doc: &Document, page_id: ObjectId) -> PageBox {
    let mut media = None;
    let mut current = Some(page_id);
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > 20 {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        if let Some(rect) = dict.get(b"CropBox").ok().and_then(|o| rect_object(doc, o)) {
            return PageBox::from_rect(rect);
        }
        if media.is_none() {
            media = dict.get(b"MediaBox").ok().and_then(|o| rect_object(doc, o));
        }
        current = match dict.get(b"Parent") {
            Ok(Object::Reference(pid)) => Some(*pid),
            _ => None,
        };
    }
    PageBox::from_rect(media.unwrap_or([0.0, 0.0, 612.0, 792.0]))
}

impl PageBox {
    fn from_rect(rect: [f64; 4]) -> Self {
        let rect = normalize(rect);
        Self {
            llx: rect[0],
            lly: rect[1],
            urx: rect[2],
            ury: rect[3],
        }
    }
}

fn widget_rects(doc: &Document, page_id: ObjectId) -> Vec<[f64; 4]> {
    let annots = {
        let Ok(page) = doc.get_dictionary(page_id) else {
            return Vec::new();
        };
        page.get(b"Annots").ok().cloned()
    };
    let Some(annots) = annots else {
        return Vec::new();
    };
    let mut rects = Vec::new();
    for obj in deref_array(doc, &annots) {
        let Ok(id) = obj.as_reference() else {
            continue;
        };
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        if let Some(rect) = dict.get(b"Rect").ok().and_then(|o| rect_object(doc, o)) {
            rects.push(normalize(rect));
        }
    }
    rects
}

fn existing_field_names(doc: &Document) -> HashSet<String> {
    let mut names = HashSet::new();
    let Some(acro) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok().cloned())
    else {
        return names;
    };
    collect_names(doc, &acro, &mut names, 0);
    names
}

fn collect_names(doc: &Document, obj: &Object, names: &mut HashSet<String>, depth: u32) {
    if depth > 16 {
        return;
    }
    match obj {
        Object::Array(items) => {
            for item in items {
                collect_names(doc, item, names, depth + 1);
            }
        }
        Object::Reference(id) => {
            let Ok(dict) = doc.get_dictionary(*id) else {
                return;
            };
            let dict = dict.clone();
            if let Ok(bytes) = dict.get(b"T").and_then(|o| o.as_str()) {
                let name = pdf_to_string(bytes);
                if !name.is_empty() {
                    names.insert(name);
                }
            }
            if let Ok(kids) = dict.get(b"Kids") {
                let kids = kids.clone();
                collect_names(doc, &kids, names, depth + 1);
            }
            if let Ok(fields) = dict.get(b"Fields") {
                let fields = fields.clone();
                collect_names(doc, &fields, names, depth + 1);
            }
        }
        Object::Dictionary(dict) => {
            if let Ok(bytes) = dict.get(b"T").and_then(|o| o.as_str()) {
                let name = pdf_to_string(bytes);
                if !name.is_empty() {
                    names.insert(name);
                }
            }
            let kids = dict.get(b"Kids").ok().cloned();
            let fields = dict.get(b"Fields").ok().cloned();
            if let Some(kids) = kids {
                collect_names(doc, &kids, names, depth + 1);
            }
            if let Some(fields) = fields {
                collect_names(doc, &fields, names, depth + 1);
            }
        }
        _ => {}
    }
}

fn add_text_widget(doc: &mut Document, slot: &Slot) -> ObjectId {
    let height = slot.rect[3] - slot.rect[1];
    let size = (height * 0.62).clamp(8.0, 12.0);
    let mut dict = Dictionary::new();
    dict.set("Type", "Annot");
    dict.set("Subtype", "Widget");
    dict.set("FT", "Tx");
    dict.set("T", pdf_text(&slot.name));
    dict.set("Rect", rect_array(slot.rect));
    dict.set("F", 4_i64);
    dict.set("P", slot.page_id);
    dict.set("DA", pdf_text(&format!("/Helv {size:.1} Tf 0 g")));
    dict.set("Q", 0_i64);
    let mut border = Dictionary::new();
    border.set("W", 0_i64);
    border.set("S", "S");
    dict.set("BS", border);
    if height >= 36.0 {
        dict.set("Ff", 4096_i64);
    }
    doc.add_object(dict)
}

fn add_check_widget(doc: &mut Document, slot: &Slot) -> ObjectId {
    let w = slot.rect[2] - slot.rect[0];
    let h = slot.rect[3] - slot.rect[1];
    let yes_id = doc.add_object(appearance(w, h, true));
    let off_id = doc.add_object(appearance(w, h, false));
    let mut normal = Dictionary::new();
    normal.set("Yes", yes_id);
    normal.set("Off", off_id);
    let mut ap = Dictionary::new();
    ap.set("N", normal);

    let mut dict = Dictionary::new();
    dict.set("Type", "Annot");
    dict.set("Subtype", "Widget");
    dict.set("FT", "Btn");
    dict.set("T", pdf_text(&slot.name));
    dict.set("Rect", rect_array(slot.rect));
    dict.set("F", 4_i64);
    dict.set("P", slot.page_id);
    dict.set("V", "Off");
    dict.set("AS", "Off");
    dict.set("AP", ap);
    dict.set("Ff", 0_i64);
    doc.add_object(dict)
}

fn appearance(w: f64, h: f64, checked: bool) -> Stream {
    let body = if checked {
        let lw = (w.min(h) * 0.12).clamp(0.8, 2.2);
        format!(
            "{lw:.2} w 1 J 1 j {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l {x3:.2} {y3:.2} l S",
            x1 = 0.20 * w,
            y1 = 0.52 * h,
            x2 = 0.42 * w,
            y2 = 0.18 * h,
            x3 = 0.82 * w,
            y3 = 0.78 * h,
        )
    } else {
        String::new()
    };
    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Form");
    dict.set("BBox", vec![0.0.into(), 0.0.into(), w.into(), h.into()]);
    dict.set("FormType", 1_i64);
    dict.set("Resources", Dictionary::new());
    let mut stream = Stream::new(dict, body.into_bytes());
    stream.allows_compression = false;
    stream
}

fn append_annots(doc: &mut Document, page_id: ObjectId, widgets: &[ObjectId]) {
    let mut annots = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Annots").ok().cloned())
        .map(|obj| deref_array(doc, &obj))
        .unwrap_or_default();
    for id in widgets {
        annots.push(Object::Reference(*id));
    }
    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        page.set("Annots", annots);
    }
}

fn upsert_acroform(doc: &mut Document, field_ids: &[ObjectId]) -> Result<()> {
    let helv_id = {
        let mut font = Dictionary::new();
        font.set("Type", "Font");
        font.set("Subtype", "Type1");
        font.set("BaseFont", "Helvetica");
        font.set("Encoding", "WinAnsiEncoding");
        doc.add_object(font)
    };

    let existing = doc
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"AcroForm").ok().cloned());
    let acro_id = match existing {
        Some(Object::Reference(id)) => id,
        Some(Object::Dictionary(dict)) => doc.add_object(dict),
        _ => doc.add_object(Dictionary::new()),
    };
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.set("AcroForm", Object::Reference(acro_id));
    }

    let (fields_obj, dr_obj) = {
        let acro = doc.get_dictionary(acro_id)?;
        (
            acro.get(b"Fields").ok().cloned(),
            acro.get(b"DR").ok().cloned(),
        )
    };
    let mut fields = fields_obj
        .as_ref()
        .map(|obj| deref_array(doc, obj))
        .unwrap_or_default();
    let mut dr = match dr_obj.as_ref() {
        Some(Object::Dictionary(dict)) => dict.clone(),
        Some(Object::Reference(id)) => doc.get_dictionary(*id).ok().cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    };
    for id in field_ids {
        fields.push(Object::Reference(*id));
    }
    let mut fonts = match dr.get(b"Font") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).ok().cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    };
    fonts.set("Helv", Object::Reference(helv_id));
    dr.set("Font", fonts);

    let acro = doc.get_dictionary_mut(acro_id)?;
    acro.set("Fields", fields);
    acro.set("NeedAppearances", true);
    acro.set("DR", dr);
    if !acro.has(b"DA") {
        acro.set("DA", pdf_text("/Helv 10 Tf 0 g"));
    }
    Ok(())
}

fn deref_array(doc: &Document, obj: &Object) -> Vec<Object> {
    match obj {
        Object::Array(items) => items.clone(),
        Object::Reference(id) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn rect_object(doc: &Document, obj: &Object) -> Option<[f64; 4]> {
    match obj {
        Object::Array(items) => array_rect(items),
        Object::Reference(id) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_array().ok())
            .and_then(|items| array_rect(items)),
        _ => None,
    }
}

fn array_rect(items: &[Object]) -> Option<[f64; 4]> {
    if items.len() < 4 {
        return None;
    }
    Some([
        nth_num(items, 0)?,
        nth_num(items, 1)?,
        nth_num(items, 2)?,
        nth_num(items, 3)?,
    ])
}

fn rect_array(rect: [f64; 4]) -> Vec<Object> {
    rect.into_iter().map(|n| Object::Real(n as f32)).collect()
}

fn matrix_from(obj: Option<&Object>) -> Mat {
    let Some(Object::Array(items)) = obj else {
        return ID;
    };
    mat_operands(items).unwrap_or(ID)
}

fn mat_operands(items: &[Object]) -> Option<Mat> {
    if items.len() < 6 {
        return None;
    }
    Some([
        nth_num(items, 0)?,
        nth_num(items, 1)?,
        nth_num(items, 2)?,
        nth_num(items, 3)?,
        nth_num(items, 4)?,
        nth_num(items, 5)?,
    ])
}

fn nth_num(items: &[Object], index: usize) -> Option<f64> {
    items.get(index)?.as_float().ok().map(|n| n as f64)
}

fn rgb_light(items: &[Object]) -> bool {
    match (nth_num(items, 0), nth_num(items, 1), nth_num(items, 2)) {
        (Some(r), Some(g), Some(b)) => r >= 0.94 && g >= 0.94 && b >= 0.94,
        _ => false,
    }
}

fn cmyk_light(items: &[Object]) -> bool {
    match (
        nth_num(items, 0),
        nth_num(items, 1),
        nth_num(items, 2),
        nth_num(items, 3),
    ) {
        (Some(c), Some(m), Some(y), Some(k)) => c <= 0.06 && m <= 0.06 && y <= 0.06 && k <= 0.06,
        _ => false,
    }
}

fn mul(op: Mat, ctm: Mat) -> Mat {
    let [a, b, c, d, e, f] = op;
    let [a1, b1, c1, d1, e1, f1] = ctm;
    [
        a * a1 + b * c1,
        a * b1 + b * d1,
        c * a1 + d * c1,
        c * b1 + d * d1,
        e * a1 + f * c1 + e1,
        e * b1 + f * d1 + f1,
    ]
}

fn apply(m: Mat, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

fn hypot(x: f64, y: f64) -> f64 {
    x.hypot(y)
}

fn normalize(rect: [f64; 4]) -> [f64; 4] {
    [
        rect[0].min(rect[2]),
        rect[1].min(rect[3]),
        rect[0].max(rect[2]),
        rect[1].max(rect[3]),
    ]
}

fn clamp_rect(rect: [f64; 4], page: PageBox, min_w: f64) -> Option<[f64; 4]> {
    let rect = normalize(rect);
    let clamped = [
        rect[0].max(page.llx),
        rect[1].max(page.lly),
        rect[2].min(page.urx),
        rect[3].min(page.ury),
    ];
    if clamped[2] - clamped[0] < min_w || clamped[3] - clamped[1] < 6.0 {
        None
    } else {
        Some(clamped)
    }
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    let inter = inter_area(a, b);
    if inter <= 0.0 {
        return false;
    }
    let smaller = area(a).min(area(b)).max(1.0);
    inter / smaller > 0.4
}

fn area(rect: [f64; 4]) -> f64 {
    ((rect[2] - rect[0]) * (rect[3] - rect[1])).abs()
}

fn inter_area(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = overlap_len(a[0], a[2], b[0], b[2]);
    let h = overlap_len(a[1], a[3], b[1], b[3]);
    w * h
}

fn overlap_len(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    let left = a0.max(b0);
    let right = a1.min(b1);
    (right - left).max(0.0)
}

fn cmp_f(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream};
    use std::io::Cursor;

    fn save(doc: &mut Document) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save");
        out
    }

    fn page_pdf(ops: &str) -> Vec<u8> {
        page_font("Helvetica", ops)
    }

    fn page_font(base: &str, ops: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut stream = Stream::new(dictionary! {}, ops.as_bytes().to_vec());
        stream.allows_compression = false;
        let content_id = doc.add_object(stream);
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => Object::Name(base.as_bytes().to_vec()),
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            },
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", Object::Reference(catalog));
        save(&mut doc)
    }

    fn field_map(pdf: &[u8]) -> Vec<(String, String)> {
        let doc = Document::load_from(Cursor::new(pdf)).unwrap();
        let acro_id = doc
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        let acro = doc.get_dictionary(acro_id).unwrap();
        assert!(acro.get(b"NeedAppearances").unwrap().as_bool().unwrap());
        let fields = acro.get(b"Fields").unwrap().as_array().unwrap();
        let mut out = Vec::new();
        for field in fields {
            let id = field.as_reference().unwrap();
            let dict = doc.get_dictionary(id).unwrap();
            let name = pdf_to_string(dict.get(b"T").unwrap().as_str().unwrap());
            let kind =
                String::from_utf8_lossy(dict.get(b"FT").unwrap().as_name().unwrap()).into_owned();
            out.push((name, kind));
        }
        out
    }

    #[test]
    fn underscores_become_named_text_fields() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n\
             BT /F1 12 Tf 72 660 Td (Email: ___________) Tj ET\n",
        );
        let (out, stats) = prepare_form(&pdf).unwrap();
        assert!(!stats.kept_original);
        assert_eq!(stats.fields_added, 2);
        let mut names: Vec<_> = field_map(&out).into_iter().map(|(n, _)| n).collect();
        names.sort();
        assert_eq!(names, vec!["Email".to_string(), "Name".to_string()]);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\nxref\n"));
        assert!(!text.contains("/Type /XRef"));
        assert!(text.contains("Name"));

        let doc = Document::load_from(Cursor::new(&out)).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        let annots = doc
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(annots.len(), 2);
    }

    #[test]
    fn compress_keeps_the_new_fields() {
        let pdf = page_pdf("BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n");
        let (formed, _) = prepare_form(&pdf).unwrap();
        let (compressed, _) = crate::compress(&formed).unwrap();
        let fields = field_map(&compressed);
        assert_eq!(fields, vec![("Name".to_string(), "Tx".to_string())]);
    }

    #[test]
    fn line_beside_label_and_checkbox() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 700 Td (Name) Tj ET\n\
             140 698 m 360 698 l S\n\
             72 640 14 14 re S\n\
             BT /F1 12 Tf 96 642 Td (Agree) Tj ET\n",
        );
        let (out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields_added, 2, "{:?}", stats.fields);
        let fields = field_map(&out);
        assert!(fields.iter().any(|(n, k)| n == "Name" && k == "Tx"));
        assert!(fields.iter().any(|(n, k)| n == "Agree" && k == "Btn"));
        let agree = stats.fields.iter().find(|f| f.name == "Agree").unwrap();
        assert_eq!(agree.kind, FieldKind::Checkbox);
        let doc = Document::load_from(Cursor::new(&out)).unwrap();
        let widget = doc
            .objects
            .values()
            .find_map(|obj| {
                let dict = obj.as_dict().ok()?;
                let name = dict.get(b"T").ok()?.as_str().ok()?;
                if pdf_to_string(name) == "Agree" {
                    Some(dict.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(widget.get(b"V").unwrap().as_name().unwrap(), b"Off");
        let ap = widget.get(b"AP").unwrap().as_dict().unwrap();
        let normal = ap.get(b"N").unwrap().as_dict().unwrap();
        assert!(normal.has(b"Yes"));
        assert!(normal.has(b"Off"));
    }

    #[test]
    fn comment_box_uses_the_caption_above_it() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 720 Td (Notes) Tj ET\n\
             72 640 220 60 re S\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 1);
        assert_eq!(stats.fields[0].name, "Notes");
        assert_eq!(stats.fields[0].kind, FieldKind::Text);
        assert!(stats.fields[0].rect[3] - stats.fields[0].rect[1] > 40.0);
    }

    #[test]
    fn skips_page_frame_emphasis_and_filled_boxes() {
        let frame = page_pdf("0 0 612 792 re S\n");
        let (out, stats) = prepare_form(&frame).unwrap();
        assert!(stats.kept_original);
        assert_eq!(out, frame);

        let prose = page_pdf("BT /F1 12 Tf 72 700 Td (Hello) Tj ET\n");
        assert!(prepare_form(&prose).unwrap().1.kept_original);

        let underline = page_pdf(
            "BT /F1 12 Tf 72 700 Td (Hello there) Tj ET\n\
             72 697 m 150 697 l S\n",
        );
        assert!(
            prepare_form(&underline).unwrap().1.kept_original,
            "emphasis underline should not become a field"
        );

        let filled = page_pdf(
            "BT /F1 12 Tf 80 660 Td (already filled) Tj ET\n\
             72 640 220 40 re S\n",
        );
        assert!(prepare_form(&filled).unwrap().1.fields.is_empty());
    }

    #[test]
    fn second_pass_does_not_duplicate_fields() {
        let pdf = page_pdf("BT /F1 12 Tf 72 700 Td (Name: ____________) Tj ET\n");
        let (once, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields_added, 1);
        let (twice, again) = prepare_form(&once).unwrap();
        assert!(again.kept_original, "{:?}", again.fields);
        assert_eq!(twice, once);
    }

    #[test]
    fn translated_line_and_form_xobject() {
        let moved = page_pdf(
            "BT /F1 12 Tf 72 700 Td (City) Tj ET\n\
             q 1 0 0 1 100 0 cm 40 698 m 220 698 l S Q\n",
        );
        let (_out, stats) = prepare_form(&moved).unwrap();
        assert_eq!(stats.fields.len(), 1, "{:?}", stats.fields);
        assert_eq!(stats.fields[0].name, "City");
        assert!(stats.fields[0].rect[0] > 120.0);

        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut form = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 300.into(), 20.into()],
                "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 72.into(), 690.into()],
            },
            b"0 8 m 200 8 l S".to_vec(),
        );
        form.allows_compression = false;
        let form_id = doc.add_object(form);
        let mut content = Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf 72 720 Td (City) Tj ET\n/Fm0 Do\n".to_vec(),
        );
        content.allows_compression = false;
        let content_id = doc.add_object(content);
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
                "XObject" => dictionary! { "Fm0" => form_id },
            },
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", Object::Reference(catalog));
        let pdf = save(&mut doc);
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 1, "{:?}", stats.fields);
        assert_eq!(stats.fields[0].name, "City");
    }

    #[test]
    fn line_rectangle_becomes_one_text_field() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 700 Td (Office) Tj ET\n\
             150 660 m 150 700 l 360 700 l 360 660 l 150 660 l S\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 1, "{:?}", stats.fields);
        assert_eq!(stats.fields[0].name, "Office");
        assert_eq!(stats.fields[0].kind, FieldKind::Text);
    }

    #[test]
    fn rejects_garbage() {
        assert!(prepare_form(b"not a pdf").is_err());
    }

    fn advance(face: Face, text: &str, size: f64) -> f64 {
        text.chars().map(|ch| em_width(face, ch) * size).sum()
    }

    #[test]
    fn standard_widths_and_font_names() {
        assert!((em_width(Face::Helvetica, '_') - 0.556).abs() < 0.0001);
        assert!((em_width(Face::Times, ' ') - 0.250).abs() < 0.0001);
        assert!((em_width(Face::Times, '_') - 0.500).abs() < 0.0001);
        assert!((em_width(Face::Courier, 'W') - 0.6).abs() < 0.0001);
        assert_eq!(face_from_base(b"ABCDEF+TimesNewRomanPSMT"), Face::Times);
        assert_eq!(face_from_base(b"Arial-BoldMT"), Face::Helvetica);
        assert_eq!(face_from_base(b"CourierNew"), Face::Courier);
        assert_eq!(face_from_base(b"Calibri"), Face::Generic);
    }

    #[test]
    fn line_under_address_starts_after_the_label() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 700 Td (Address:) Tj ET\n\
             72 697 m 400 697 l S\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 1, "{:?}", stats.fields);
        let field = &stats.fields[0];
        assert_eq!(field.name, "Address");
        let label_end = 72.0 + advance(Face::Helvetica, "Address:", 12.0);
        assert!(
            field.rect[0] > label_end - 2.0 && field.rect[0] < label_end + 8.0,
            "field covers the label: {:?} label_end={label_end}",
            field.rect
        );
        assert!(field.rect[2] > 380.0, "{:?}", field.rect);
    }

    #[test]
    fn one_rule_with_three_captions_splits() {
        let pdf = page_pdf(
            "BT /F1 12 Tf 72 700 Td (City:) Tj ET\n\
             BT /F1 12 Tf 220 700 Td (State:) Tj ET\n\
             BT /F1 12 Tf 380 700 Td (Zip Code:) Tj ET\n\
             72 697 m 540 697 l S\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 3, "{:?}", stats.fields);
        let mut fields = stats.fields.clone();
        fields.sort_by(|a, b| cmp_f(a.rect[0], b.rect[0]));
        assert_eq!(fields[0].name, "City");
        assert_eq!(fields[1].name, "State");
        assert_eq!(fields[2].name, "Zip Code");
        let city_end = 72.0 + advance(Face::Helvetica, "City:", 12.0);
        assert!(fields[0].rect[0] > city_end - 2.0, "{:?}", fields[0].rect);
        assert!(fields[0].rect[2] < 220.0, "{:?}", fields[0].rect);
        assert!(fields[1].rect[0] > 240.0, "{:?}", fields[1].rect);
        assert!(fields[1].rect[2] < 380.0, "{:?}", fields[1].rect);
        assert!(fields[2].rect[0] > 420.0, "{:?}", fields[2].rect);
    }

    #[test]
    fn table_border_is_not_a_field_but_the_blank_is() {
        let pdf = page_pdf(
            "72 600 m 540 600 l S\n\
             72 640 m 540 640 l S\n\
             72 600 m 72 640 l S\n\
             200 600 m 200 640 l S\n\
             360 600 m 360 640 l S\n\
             540 600 m 540 640 l S\n\
             BT /F1 12 Tf 80 612 Td (Existing) Tj ET\n\
             BT /F1 12 Tf 210 612 Td (___/___) Tj ET\n\
             BT /F1 12 Tf 370 612 Td (NRIS) Tj ET\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert!(
            stats.fields.iter().all(|f| f.rect[2] - f.rect[0] < 220.0),
            "row rule became a field: {:?}",
            stats.fields
        );
        assert_eq!(stats.fields.len(), 1, "{:?}", stats.fields);
        let field = &stats.fields[0];
        assert!(
            (210.0..220.0).contains(&field.rect[0]) && field.rect[2] > 248.0,
            "{:?}",
            field.rect
        );
    }

    #[test]
    fn times_prefix_lands_on_the_underscore_run() {
        let pdf = page_font(
            "Times-Roman",
            "BT /F1 12 Tf 72 700 Td (located in ____________ Count) Tj ET\n\
             BT /F1 12 Tf 72 660 Td (Operation Date ________,) Tj ET\n",
        );
        let (_out, stats) = prepare_form(&pdf).unwrap();
        assert_eq!(stats.fields.len(), 2, "{:?}", stats.fields);
        let mut fields = stats.fields.clone();
        fields.sort_by(|a, b| cmp_f(b.rect[3], a.rect[3]));

        let prefix = advance(Face::Times, "located in ", 12.0);
        let blank = advance(Face::Times, "____________", 12.0);
        assert!(
            (fields[0].rect[0] - (72.0 + prefix)).abs() < 2.0,
            "start {:?} expected {}",
            fields[0].rect,
            72.0 + prefix
        );
        assert!(
            (fields[0].rect[2] - (72.0 + prefix + blank)).abs() < 2.0,
            "end {:?} expected {}",
            fields[0].rect,
            72.0 + prefix + blank
        );

        let date = advance(Face::Times, "Operation Date ", 12.0);
        let slots = advance(Face::Times, "________", 12.0);
        assert!(
            (fields[1].rect[0] - (72.0 + date)).abs() < 2.0,
            "date start {:?} expected {}",
            fields[1].rect,
            72.0 + date
        );
        assert!(
            fields[1].rect[2] < 72.0 + date + slots + 2.0,
            "date field runs through the comma: {:?}",
            fields[1].rect
        );
    }

    #[test]
    fn filled_rules_and_short_date_blanks_are_fields() {
        let ruled = page_pdf(
            "BT /F1 12 Tf 72 700 Td (City:) Tj ET\n\
             BT /F1 12 Tf 250 700 Td (State:) Tj ET\n\
             72 697 400 1.2 re f\n\
             BT /F1 12 Tf 72 640 Td (GPS Coordinates:) Tj ET\n\
             BT /F1 12 Tf 280 640 Td (N) Tj ET\n\
             BT /F1 12 Tf 420 640 Td (W) Tj ET\n\
             230 637 40 1 re f\n\
             300 637 110 1 re f\n\
             210 560 36 1 re f\n\
             249 560 36 1 re f\n",
        );
        let (_out, stats) = prepare_form(&ruled).unwrap();
        let mut fields = stats.fields.clone();
        fields.sort_by(|a, b| cmp_f(b.rect[3], a.rect[3]).then(cmp_f(a.rect[0], b.rect[0])));
        let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"City") && names.contains(&"State"),
            "city line missed: {names:?} {:?}",
            fields.iter().map(|f| f.rect).collect::<Vec<_>>()
        );
        assert!(
            names
                .iter()
                .filter(|n| **n == "GPS Coordinates" || **n == "N")
                .count()
                >= 2
                || fields
                    .iter()
                    .filter(|f| f.rect[1] > 600.0 && f.rect[1] < 660.0)
                    .count()
                    >= 2,
            "gps blanks missed: {names:?} {:?}",
            fields
                .iter()
                .map(|f| (f.name.as_str(), f.rect))
                .collect::<Vec<_>>()
        );
        let dates: Vec<_> = fields.iter().filter(|f| f.rect[3] < 590.0).collect();
        assert_eq!(dates.len(), 1, "date halves: {:?}", fields);
        assert!(
            dates[0].rect[2] - dates[0].rect[0] > 60.0,
            "{:?}",
            dates[0].rect
        );

        let solid = page_pdf("20 600 14 14 re f\n");
        assert!(prepare_form(&solid).unwrap().1.kept_original);
    }
}
