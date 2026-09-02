//! Build uncompressed fixture PDFs for tests.

use lopdf::{dictionary, Document, Object, Stream};

pub fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save fixture");
    out
}

pub fn text_with_orphan() -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let payload =
        b"BT /F1 12 Tf 20 200 Td (the quick brown fox jumps over the lazy dog) Tj ET".repeat(40);
    let mut stream = Stream::new(dictionary! {}, payload);
    stream.allows_compression = false;
    let content_id = doc.add_object(stream);
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 300.into()],
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
    // Unreferenced — GC should drop it
    let _ = doc.add_object(dictionary! {
        "Type" => "UnusedJunk",
        "Blob" => "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", Object::Reference(catalog));
    save(&mut doc)
}

pub fn rgb_image_pdf(width: u32, height: u32, rgb: Vec<u8>, copies: u32) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::new();
    for _ in 0..copies {
        let mut img = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width as i64,
                "Height" => height as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            rgb.clone(),
        );
        img.allows_compression = false;
        let img_id = doc.add_object(img);
        let mut content = Stream::new(
            dictionary! {},
            format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q").into_bytes(),
        );
        content.allows_compression = false;
        let content_id = doc.add_object(content);
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![
                0.into(),
                0.into(),
                i64::from(width).into(),
                i64::from(height).into(),
            ],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Im0" => img_id },
            },
        });
        kids.push(page_id.into());
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => copies as i64,
        }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", Object::Reference(catalog));
    save(&mut doc)
}

pub fn screenshot_rgb(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            if x < w / 2 {
                rgb[i] = 30;
                rgb[i + 1] = 30;
                rgb[i + 2] = 40;
            } else {
                rgb[i] = 240;
                rgb[i + 1] = 240;
                rgb[i + 2] = 245;
            }
            if y % 8 == 0 {
                rgb[i] = 200;
                rgb[i + 1] = 40;
                rgb[i + 2] = 40;
            }
        }
    }
    rgb
}

/// Page content marked FlateDecode but not valid zlib. lopdf then yields empty.
pub fn page_with_invalid_flate_content() -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let ops = b"BT /F1 12 Tf 20 200 Td (visible page operators) Tj ET".repeat(80);
    let mut stream = Stream::new(
        dictionary! {
            "Filter" => "FlateDecode",
        },
        ops.to_vec(),
    );
    stream.allows_compression = false;
    let content_id = doc.add_object(stream);
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 300.into()],
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

pub fn photo_rgb(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            let xf = x as f32 / w as f32;
            let yf = y as f32 / h as f32;
            rgb[i] = (40.0 + 180.0 * xf + 20.0 * (yf * 9.0).sin()) as u8;
            rgb[i + 1] = (30.0 + 160.0 * yf + 15.0 * (xf * 7.0).cos()) as u8;
            rgb[i + 2] = (80.0 + 100.0 * ((xf + yf) * 3.0).sin().abs()) as u8;
        }
    }
    rgb
}
