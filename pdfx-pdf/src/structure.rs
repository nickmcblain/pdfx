use crate::images::{is_image, process_images};
use crate::{CompressStats, PdfError, Result};
use lopdf::xref::XrefType;
use lopdf::{Document, Object, ObjectId};
use std::collections::HashSet;
use std::io::Cursor;

#[derive(Debug, Default)]
pub struct Census {
    pub pages: u32,
    pub objects: u32,
    pub bytes: u64,
    pub stream_bytes: u64,
    pub image_streams: u32,
    pub notes: Vec<String>,
}

pub fn census(bytes: &[u8]) -> Result<Census> {
    let doc = load(bytes)?;
    let mut c = Census {
        pages: doc.get_pages().len() as u32,
        objects: doc.objects.len() as u32,
        bytes: bytes.len() as u64,
        ..Census::default()
    };
    for obj in doc.objects.values() {
        if let Object::Stream(s) = obj {
            c.stream_bytes += s.content.len() as u64;
            if is_image(&s.dict) {
                c.image_streams += 1;
            }
        }
    }
    c.notes.push(format!(
        "images: {}  stream_bytes: {}",
        c.image_streams, c.stream_bytes
    ));
    Ok(c)
}

pub fn compress_document(bytes: &[u8]) -> Result<(Vec<u8>, CompressStats)> {
    let mut stats = CompressStats {
        input_bytes: bytes.len() as u64,
        ..CompressStats::default()
    };
    let mut doc = load(bytes)?;

    let before = doc.objects.len();
    let pruned = doc.prune_objects();
    if pruned.is_empty() {
        let kept = reachable(&doc);
        doc.objects.retain(|id, _| kept.contains(id));
    }
    stats.orphans_removed = (before - doc.objects.len()) as u32;

    let img = process_images(&mut doc)?;
    stats.images_rewritten = img.rewritten;
    stats.images_deduped = img.deduped;
    stats.images_skipped = img.skipped;

    let after_img = doc.objects.len();
    let kept = reachable(&doc);
    doc.objects.retain(|id, _| kept.contains(id));
    stats.orphans_removed += (after_img - doc.objects.len()) as u32;

    stats.streams_reflated = reflate_streams(&mut doc);

    let out = save_doc(&mut doc)?;
    if out.len() >= bytes.len() {
        stats.kept_original = true;
        stats.output_bytes = bytes.len() as u64;
        stats.notes.push("output not smaller; kept original".into());
        return Ok((bytes.to_vec(), stats));
    }
    stats.output_bytes = out.len() as u64;
    stats.notes.push(format!(
        "orphans={} reflate={} images={} dedup={} skipped={}",
        stats.orphans_removed,
        stats.streams_reflated,
        stats.images_rewritten,
        stats.images_deduped,
        stats.images_skipped
    ));
    Ok((out, stats))
}

fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_from(Cursor::new(bytes))?;
    if doc.is_encrypted() {
        return Err(PdfError::Encrypted);
    }
    Ok(doc)
}

fn save_doc(doc: &mut Document) -> Result<Vec<u8>> {
    // Gaps from prune/dedup + xref/object streams break macOS Preview.
    doc.renumber_objects();
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

fn reachable(doc: &Document) -> HashSet<ObjectId> {
    let mut stack = Vec::new();
    push_ref(doc.trailer.get(b"Root").ok(), &mut stack);
    push_ref(doc.trailer.get(b"Info").ok(), &mut stack);
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(obj) = doc.objects.get(&id) {
            collect_refs(obj, &mut stack);
        }
    }
    seen
}

fn push_ref(obj: Option<&Object>, stack: &mut Vec<ObjectId>) {
    if let Some(Object::Reference(id)) = obj {
        stack.push(*id);
    }
}

fn collect_refs(obj: &Object, stack: &mut Vec<ObjectId>) {
    match obj {
        Object::Reference(id) => stack.push(*id),
        Object::Array(a) => {
            for o in a {
                collect_refs(o, stack);
            }
        }
        Object::Dictionary(d) => {
            for (_, o) in d.iter() {
                collect_refs(o, stack);
            }
        }
        Object::Stream(s) => {
            for (_, o) in s.dict.iter() {
                collect_refs(o, stack);
            }
        }
        _ => {}
    }
}

fn reflate_streams(doc: &mut Document) -> u32 {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut n = 0u32;
    for id in ids {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            continue;
        };
        if is_image(&stream.dict) {
            continue;
        }
        if !flate_eligible(&stream.dict) {
            continue;
        }
        let Ok(raw) = stream.decompressed_content() else {
            continue;
        };
        // lopdf treats failed zlib as Ok(empty). Replacing would blank pages.
        if raw.is_empty() {
            continue;
        }
        if stream.content.len() > 64 && raw.len() < 16 {
            continue;
        }
        let compressed = pdfx_deflate::compress(&raw);
        if compressed.len() >= stream.content.len() {
            continue;
        }
        if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
            stream.content = compressed.clone();
            stream
                .dict
                .set("Filter", Object::Name(b"FlateDecode".to_vec()));
            stream
                .dict
                .set("Length", Object::Integer(compressed.len() as i64));
            // Old /DecodeParms (predictor, LZW early-change) no longer apply.
            stream.dict.remove(b"DecodeParms");
            stream.allows_compression = false;
            n += 1;
        }
    }
    n
}

fn flate_eligible(dict: &lopdf::Dictionary) -> bool {
    match dict.get(b"Filter").ok() {
        None => true,
        Some(Object::Name(n)) => matches!(
            n.as_slice(),
            b"LZWDecode" | b"ASCII85Decode" | b"ASCIIHexDecode"
        ),
        Some(Object::Array(a)) => a.iter().all(|o| match o {
            Object::Name(n) => !matches!(
                n.as_slice(),
                b"DCTDecode" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode"
            ),
            _ => true,
        }),
        _ => false,
    }
}
