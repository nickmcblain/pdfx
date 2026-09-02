use crate::predictor::{decode_png, encode_png_best};
use crate::resample::{fit, resize_gray, resize_rgb};
use crate::Result;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdfx_jpeg::Chroma;
use std::collections::{HashMap, HashSet};

/// Slide-deck bitmaps above this get boxed down before JPEG.
const MAX_EDGE: u32 = 1920;
/// Single JPEG pass. Fat Annex-K encoder needs this to beat source JPEGs after resize.
const JPEG_QUALITY: u8 = 62;

#[derive(Debug, Default)]
pub struct ImageStats {
    pub rewritten: u32,
    pub deduped: u32,
    pub skipped: u32,
}

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub skip_reason: Option<&'static str>,
    pub original_len: usize,
    pub fingerprint: [u8; 32],
}

pub fn process_images(doc: &mut lopdf::Document) -> Result<ImageStats> {
    let smask_ids = collect_smask_ids(doc);
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut decoded: HashMap<ObjectId, DecodedImage> = HashMap::new();

    for id in &ids {
        if let Ok(Object::Stream(stream)) = doc.get_object(*id) {
            if !is_image(&stream.dict) {
                continue;
            }
            decoded.insert(*id, decode_image(stream, smask_ids.contains(id))?);
        }
    }

    let mut stats = ImageStats::default();
    let mut by_hash: HashMap<[u8; 32], ObjectId> = HashMap::new();
    let mut remap: HashMap<ObjectId, ObjectId> = HashMap::new();

    for (id, img) in &decoded {
        if img.skip_reason.is_some() {
            stats.skipped += 1;
            continue;
        }
        if let Some(&first) = by_hash.get(&img.fingerprint) {
            remap.insert(*id, first);
        } else {
            by_hash.insert(img.fingerprint, *id);
        }
    }

    if !remap.is_empty() {
        rewrite_refs(doc, &remap);
        for (old, _) in &remap {
            doc.objects.remove(old);
            stats.deduped += 1;
        }
    }

    let remaining: Vec<ObjectId> = decoded
        .keys()
        .filter(|id| !remap.contains_key(id))
        .copied()
        .collect();

    for id in remaining {
        let img = match decoded.get(&id) {
            Some(i) if i.skip_reason.is_none() => i,
            _ => continue,
        };
        let Ok(Object::Stream(old)) = doc.get_object(id) else {
            continue;
        };
        let old_dict = old.dict.clone();
        let old_len = old.content.len();
        let Some(new_stream) = recompress(img, &old_dict) else {
            continue;
        };
        if new_stream.content.len() >= old_len {
            continue;
        }
        let new_w = dict_int(&new_stream.dict, b"Width").unwrap_or(img.width as i64) as u32;
        let new_h = dict_int(&new_stream.dict, b"Height").unwrap_or(img.height as i64) as u32;
        if new_w != img.width || new_h != img.height {
            if !sync_smask(doc, &old_dict, new_w, new_h) {
                continue;
            }
        }
        doc.objects.insert(id, Object::Stream(new_stream));
        stats.rewritten += 1;
    }

    Ok(stats)
}

fn recompress(img: &DecodedImage, old_dict: &Dictionary) -> Option<Stream> {
    let (tw, th) = fit(img.width, img.height, MAX_EDGE);
    let rgb_owned: Vec<u8>;
    let (w, h, rgb) = if tw != img.width || th != img.height {
        rgb_owned = resize_rgb(&img.rgb, img.width, img.height, tw, th);
        (tw, th, rgb_owned.as_slice())
    } else {
        (img.width, img.height, img.rgb.as_slice())
    };
    let jpeg = pdfx_jpeg::encode_rgb_ex(w, h, rgb, JPEG_QUALITY, Chroma::Sample420);
    if jpeg.len() < img.original_len {
        return Some(image_stream(
            old_dict,
            w,
            h,
            jpeg,
            Object::Name(b"DCTDecode".to_vec()),
            None,
        ));
    }
    let pixels = w.saturating_mul(h);
    if pixels <= 80 * 80 {
        let tmp = DecodedImage {
            width: w,
            height: h,
            rgb: rgb.to_vec(),
            skip_reason: None,
            original_len: img.original_len,
            fingerprint: img.fingerprint,
        };
        if let Some(pred) = recompress_predictor(&tmp, old_dict) {
            if pred.content.len() < img.original_len {
                return Some(pred);
            }
        }
    }
    None
}

fn sync_smask(doc: &mut Document, parent_dict: &Dictionary, width: u32, height: u32) -> bool {
    let Some(Object::Reference(id)) = parent_dict.get(b"SMask").ok() else {
        return true;
    };
    let id = *id;
    let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
        return false;
    };
    let Some(gray) = decode_gray(stream) else {
        return false;
    };
    let sw = dict_int(&stream.dict, b"Width").unwrap_or(0) as u32;
    let sh = dict_int(&stream.dict, b"Height").unwrap_or(0) as u32;
    if sw == 0 || sh == 0 {
        return false;
    }
    let resized = if sw == width && sh == height {
        gray
    } else {
        resize_gray(&gray, sw, sh, width, height)
    };
    let compressed = pdfx_deflate::compress(&resized);
    let mut dict = stream.dict.clone();
    dict.set("Width", Object::Integer(i64::from(width)));
    dict.set("Height", Object::Integer(i64::from(height)));
    dict.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
    dict.set("BitsPerComponent", Object::Integer(8));
    dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
    dict.remove(b"DecodeParms");
    dict.set("Length", Object::Integer(compressed.len() as i64));
    let mut out = Stream::new(dict, compressed);
    out.allows_compression = false;
    if smask_refcount(doc, id) > 1 {
        return false;
    }
    doc.objects.insert(id, Object::Stream(out));
    true
}

fn smask_refcount(doc: &Document, target: ObjectId) -> usize {
    let mut n = 0usize;
    for obj in doc.objects.values() {
        let dict = match obj {
            Object::Stream(s) => &s.dict,
            Object::Dictionary(d) => d,
            _ => continue,
        };
        if matches!(dict.get(b"SMask").ok(), Some(Object::Reference(id)) if *id == target) {
            n += 1;
        }
    }
    n
}

fn decode_gray(stream: &Stream) -> Option<Vec<u8>> {
    let width = dict_int(&stream.dict, b"Width")? as u32;
    let height = dict_int(&stream.dict, b"Height")? as u32;
    let n = width as usize * height as usize;
    if n == 0 {
        return None;
    }
    let filter = leaf_filter(&stream.dict);
    if filter.as_deref() == Some(b"DCTDecode") {
        let mut dec = zune_jpeg::JpegDecoder::new(&stream.content);
        let pixels = dec.decode().ok()?;
        if pixels.len() == n {
            return Some(pixels);
        }
        if pixels.len() == n * 3 {
            let mut gray = vec![0u8; n];
            for i in 0..n {
                gray[i] = pixels[i * 3];
            }
            return Some(gray);
        }
        return None;
    }
    raw_to_gray(stream, width, height)
}

fn raw_to_gray(stream: &Stream, width: u32, height: u32) -> Option<Vec<u8>> {
    let data = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    let bpc = dict_int(&stream.dict, b"BitsPerComponent").unwrap_or(8);
    if bpc != 8 {
        return None;
    }
    let predictor = decode_parm_int(&stream.dict, b"Predictor").unwrap_or(1);
    let samples = if predictor >= 10 {
        decode_png(&data, width as usize, 1)?
    } else {
        data
    };
    let n = width as usize * height as usize;
    if samples.len() < n {
        return None;
    }
    Some(samples[..n].to_vec())
}

fn recompress_predictor(img: &DecodedImage, old_dict: &Dictionary) -> Option<Stream> {
    let filtered = encode_png_best(&img.rgb, img.width as usize, 3);
    let compressed = pdfx_deflate::compress(&filtered);
    let mut parms = Dictionary::new();
    parms.set("Predictor", Object::Integer(15));
    parms.set("Colors", Object::Integer(3));
    parms.set("BitsPerComponent", Object::Integer(8));
    parms.set("Columns", Object::Integer(i64::from(img.width)));
    Some(image_stream(
        old_dict,
        img.width,
        img.height,
        compressed,
        Object::Name(b"FlateDecode".to_vec()),
        Some(Object::Dictionary(parms)),
    ))
}

fn image_stream(
    old_dict: &Dictionary,
    width: u32,
    height: u32,
    content: Vec<u8>,
    filter: Object,
    decode_parms: Option<Object>,
) -> Stream {
    let mut dict = old_dict.clone();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    dict.set("Width", Object::Integer(i64::from(width)));
    dict.set("Height", Object::Integer(i64::from(height)));
    dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    dict.set("BitsPerComponent", Object::Integer(8));
    dict.set("Filter", filter);
    if let Some(p) = decode_parms {
        dict.set("DecodeParms", p);
    } else {
        dict.remove(b"DecodeParms");
    }
    dict.remove(b"Decode");
    dict.set("Length", Object::Integer(content.len() as i64));
    let mut stream = Stream::new(dict, content);
    stream.allows_compression = false;
    stream
}

fn skipped(
    width: u32,
    height: u32,
    original_len: usize,
    fingerprint: [u8; 32],
    reason: &'static str,
) -> DecodedImage {
    DecodedImage {
        width,
        height,
        rgb: Vec::new(),
        skip_reason: Some(reason),
        original_len,
        fingerprint,
    }
}

fn decode_image(stream: &Stream, is_smask: bool) -> Result<DecodedImage> {
    let original_len = stream.content.len();
    let raw_fp = hash_bytes(&stream.content);
    if should_skip(&stream.dict, is_smask) {
        return Ok(skipped(0, 0, original_len, raw_fp, "stencil or gray mask"));
    }
    let width = dict_int(&stream.dict, b"Width").unwrap_or(0) as u32;
    let height = dict_int(&stream.dict, b"Height").unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Ok(skipped(
            width,
            height,
            original_len,
            raw_fp,
            "bad dimensions",
        ));
    }
    let filter = leaf_filter(&stream.dict);
    let rgb = match filter.as_deref() {
        Some(b"DCTDecode") => {
            let mut dec = zune_jpeg::JpegDecoder::new(&stream.content);
            match dec.decode() {
                Ok(p) => p,
                Err(_) => {
                    return Ok(skipped(
                        width,
                        height,
                        original_len,
                        raw_fp,
                        "jpeg decode failed",
                    ));
                }
            }
        }
        Some(b"JPXDecode") | Some(b"JBIG2Decode") | Some(b"CCITTFaxDecode") => {
            return Ok(skipped(
                width,
                height,
                original_len,
                raw_fp,
                "unsupported filter",
            ));
        }
        _ => match raw_to_rgb(stream, width, height) {
            Some(p) => p,
            None => {
                return Ok(skipped(
                    width,
                    height,
                    original_len,
                    raw_fp,
                    "raw decode failed",
                ));
            }
        },
    };
    if rgb.len() != width as usize * height as usize * 3 {
        return Ok(skipped(
            width,
            height,
            original_len,
            raw_fp,
            "pixel size mismatch",
        ));
    }
    Ok(DecodedImage {
        width,
        height,
        fingerprint: hash_bytes(&rgb),
        rgb,
        skip_reason: None,
        original_len,
    })
}

fn raw_to_rgb(stream: &Stream, width: u32, height: u32) -> Option<Vec<u8>> {
    let data = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    let bpc = dict_int(&stream.dict, b"BitsPerComponent").unwrap_or(8);
    let cs = color_components(&stream.dict)?;
    let predictor = decode_parm_int(&stream.dict, b"Predictor").unwrap_or(1);
    let samples = if predictor >= 10 {
        decode_png(&data, width as usize, cs)?
    } else {
        data
    };
    if bpc != 8 {
        return None;
    }
    let n = width as usize * height as usize;
    match cs {
        1 => {
            if samples.len() < n {
                return None;
            }
            let mut rgb = vec![0u8; n * 3];
            for i in 0..n {
                rgb[i * 3] = samples[i];
                rgb[i * 3 + 1] = samples[i];
                rgb[i * 3 + 2] = samples[i];
            }
            Some(rgb)
        }
        3 => {
            if samples.len() < n * 3 {
                return None;
            }
            Some(samples[..n * 3].to_vec())
        }
        _ => None,
    }
}

fn color_components(dict: &Dictionary) -> Option<usize> {
    match dict.get(b"ColorSpace").ok() {
        Some(Object::Name(n)) if n == b"DeviceGray" || n == b"DeviceGrey" => Some(1),
        Some(Object::Name(n)) if n == b"DeviceRGB" => Some(3),
        Some(Object::Name(n)) if n == b"DeviceCMYK" => None,
        Some(Object::Array(a)) => {
            if let Some(Object::Name(n)) = a.first() {
                if n == b"DeviceGray" || n == b"CalGray" {
                    return Some(1);
                }
                if n == b"DeviceRGB" || n == b"CalRGB" || n == b"ICCBased" {
                    return Some(3);
                }
            }
            Some(3)
        }
        _ => Some(3),
    }
}

fn should_skip(dict: &Dictionary, is_smask: bool) -> bool {
    if matches!(dict.get(b"ImageMask").ok(), Some(Object::Boolean(true))) {
        return true;
    }
    // Color-key /Mask arrays are interpreted against the original samples.
    if matches!(dict.get(b"Mask").ok(), Some(Object::Array(_))) {
        return true;
    }
    // Soft masks stay DeviceGray; parent rewrite resizes them when needed.
    if is_smask {
        return true;
    }
    false
}

fn collect_smask_ids(doc: &Document) -> HashSet<ObjectId> {
    let mut out = HashSet::new();
    for obj in doc.objects.values() {
        let dict = match obj {
            Object::Stream(s) => &s.dict,
            Object::Dictionary(d) => d,
            _ => continue,
        };
        if let Ok(Object::Reference(id)) = dict.get(b"SMask") {
            out.insert(*id);
        }
    }
    out
}

pub fn is_image(dict: &Dictionary) -> bool {
    let subtype = dict.get(b"Subtype").ok().and_then(|o| match o {
        Object::Name(n) => Some(n.as_slice()),
        _ => None,
    });
    if subtype == Some(b"Image") {
        return true;
    }
    dict.get(b"Width").is_ok()
        && dict.get(b"Height").is_ok()
        && dict.get(b"BitsPerComponent").is_ok()
}

fn leaf_filter(dict: &Dictionary) -> Option<Vec<u8>> {
    match dict.get(b"Filter").ok() {
        Some(Object::Name(n)) => Some(n.clone()),
        Some(Object::Array(a)) => a.last().and_then(|o| match o {
            Object::Name(n) => Some(n.clone()),
            _ => None,
        }),
        _ => None,
    }
}

fn dict_int(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(key).ok()? {
        Object::Integer(i) => Some(*i),
        Object::Real(r) => Some(*r as i64),
        _ => None,
    }
}

fn decode_parm_int(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(b"DecodeParms").ok()? {
        Object::Dictionary(p) => dict_int(p, key),
        Object::Array(a) => a.iter().find_map(|o| match o {
            Object::Dictionary(p) => dict_int(p, key),
            _ => None,
        }),
        _ => None,
    }
}

fn hash_bytes(data: &[u8]) -> [u8; 32] {
    // FNV-1a 256-bit-ish fold — not crypto, just dedup.
    let mut h = [0u8; 32];
    let mut x: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, &b) in data.iter().enumerate() {
        x ^= u64::from(b);
        x = x.wrapping_mul(0x0100_0000_01b3);
        h[i % 32] ^= (x >> ((i % 8) * 8)) as u8;
        h[(i + 7) % 32] ^= b;
    }
    h[0] ^= (data.len() as u64).to_le_bytes()[0];
    h
}

fn rewrite_refs(doc: &mut lopdf::Document, remap: &HashMap<ObjectId, ObjectId>) {
    fn walk(obj: &mut Object, remap: &HashMap<ObjectId, ObjectId>) {
        match obj {
            Object::Reference(id) => {
                if let Some(new) = remap.get(id) {
                    *id = *new;
                }
            }
            Object::Array(a) => {
                for o in a {
                    walk(o, remap);
                }
            }
            Object::Dictionary(d) => {
                for (_, o) in d.iter_mut() {
                    walk(o, remap);
                }
            }
            Object::Stream(s) => {
                for (_, o) in s.dict.iter_mut() {
                    walk(o, remap);
                }
            }
            _ => {}
        }
    }
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        if let Some(obj) = doc.objects.get_mut(&id) {
            walk(obj, remap);
        }
    }
}
