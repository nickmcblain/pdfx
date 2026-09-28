use lopdf::{dictionary, Document, Object, Stream};

pub fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

pub fn image_pdf(
    width: u32,
    height: u32,
    color: &str,
    filter: Option<&str>,
    image_mask: bool,
    bpc: i64,
    data: Vec<u8>,
) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width as i64,
        "Height" => height as i64,
        "ColorSpace" => color,
        "BitsPerComponent" => bpc,
    };
    if image_mask {
        dict.set("ImageMask", Object::Boolean(true));
    }
    if let Some(filter) = filter {
        dict.set("Filter", Object::Name(filter.as_bytes().to_vec()));
    }
    let mut img = Stream::new(dict, data);
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
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 300.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {
            "XObject" => dictionary! { "Im0" => img_id },
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
