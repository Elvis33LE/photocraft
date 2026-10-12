//! Shared, full-frame artboard extraction and export planning. Export never edits the source.

use std::collections::HashSet;
use std::io::Write;

use flate2::{Compression, write::ZlibEncoder};
use photocraft_doc::{ArtboardBackground, Document, LayerId};
use serde_json::{Value, json};

use crate::file_cmds::{join, sanitize, stem, str_param, write_file};
use crate::{EngineError, Result, Session};

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "file.export.artboardsToFiles".into(), msg: msg.into() }
}

/// Chosen boards in Layers-panel order, with explicit validation and optional reversal.
pub fn selected(doc: &Document, p: &Value) -> Result<Vec<LayerId>> {
    let mut all: Vec<_> = doc.artboards().into_iter().rev().map(|b| b.0).collect();
    if let Some(want) = p.get("artboards") {
        let ids = want.as_array().ok_or_else(|| bad("artboards must be an array of layer IDs"))?;
        let mut chosen = HashSet::new();
        for id in ids {
            let id = LayerId(id.as_u64().ok_or_else(|| bad("artboard IDs must be unsigned integers"))?);
            if !all.contains(&id) {
                return Err(bad(format!("layer {} is not an artboard", id.0)));
            }
            chosen.insert(id);
        }
        all.retain(|id| chosen.contains(id));
    }
    if all.is_empty() {
        return Err(bad("no matching artboards"));
    }
    if all.len() > 1000 {
        return Err(bad("select at most 1000 artboards per export"));
    }
    if p.get("reverseOrder").and_then(Value::as_bool).unwrap_or(false) {
        all.reverse();
    }
    Ok(all)
}

/// Normalize a board's frame without applying export limits or changing its source. Optional
/// overlapping content keeps workspace layer order, with vectors rendered against the final geometry.
pub fn frame_document(doc: &Document, id: LayerId, include_overlapping: bool) -> Result<Document> {
    let mut one = crate::artboard_cmds::artboard_document(doc, id)?.ok_or(EngineError::NoLayer(id))?;
    if include_overlapping {
        let r = doc.layer(id).and_then(|l| l.artboard()).ok_or(EngineError::NoLayer(id))?.rect;
        let (dx, dy) = (
            r.x0.checked_neg().ok_or_else(|| bad("artboard origin cannot be translated"))?,
            r.y0.checked_neg().ok_or_else(|| bad("artboard origin cannot be translated"))?,
        );
        one.layers = doc.layers.clone();
        for l in &mut one.layers {
            if l.id == id {
                l.visible = true;
            } else if let Some(a) = l.artboard_mut() {
                a.guides = Default::default();
            }
            crate::commands::translate_layer(doc, l, dx, dy);
        }
        let geometry = one.clone();
        for l in &mut one.layers {
            crate::vector_cmds::translate_vectors(&geometry, l, f64::from(dx), f64::from(dy));
        }
    }
    Ok(one)
}

/// A board's complete frame, including an empty transparent board. Dimensions/resampling,
/// canvas padding, profile conversion and metadata all use existing engine commands/policies.
pub fn document(doc: &Document, id: LayerId, p: &Value) -> Result<Document> {
    let overlapping = p.get("includeOverlapping").and_then(Value::as_bool).unwrap_or(false);
    let mut one = frame_document(doc, id, overlapping)?;
    if !p.get("includeBackground").and_then(Value::as_bool).unwrap_or(true) {
        for layer in &mut one.layers {
            if (overlapping || layer.id == id)
                && let Some(a) = layer.artboard_mut()
            {
                a.background = ArtboardBackground::Transparent;
            }
        }
    }
    let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(100.0);
    if !scale.is_finite() || !(0.1..=1000.0).contains(&scale) {
        return Err(bad("scale must be 0.1..1000 percent"));
    }
    let dimension = |key: &str, original: u32| -> Result<u32> {
        let value = p.get(key).and_then(Value::as_f64).unwrap_or(f64::from(original) * scale / 100.0).round();
        if !value.is_finite() || !(1.0..=300_000.0).contains(&value) {
            return Err(bad(format!("{key} must be 1..300000 pixels")));
        }
        Ok(value as u32)
    };
    let (w, h) = (dimension("width", one.size.width)?, dimension("height", one.size.height)?);
    if u64::from(w) * u64::from(h) > 100_000_000 {
        return Err(bad("artboard export is limited to 100 million pixels per output"));
    }
    let mut scratch = Session::new();
    scratch.add_document(one, None);
    if p.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(false) {
        if scratch.active().is_some_and(|d| d.doc.mode != photocraft_doc::ColorMode::Rgb) {
            scratch.execute("image.mode.rgb", json!({}))?;
        }
        if scratch.active().is_some_and(|d| d.doc.icc_profile.is_some()) {
            scratch.execute("edit.convertToProfile", json!({"profile":"srgb", "intent":"perceptual"}))?;
        }
    }
    if scratch.active().is_some_and(|d| (d.doc.size.width, d.doc.size.height) != (w, h)) {
        scratch.execute("image.imageSize", json!({"width": w, "height": h, "resample":p.get("resample").and_then(Value::as_str).unwrap_or("bicubic")}))?;
    }
    if p.get("canvasWidth").is_some() || p.get("canvasHeight").is_some() {
        let canvas_dimension = |key: &str, fallback: u32| -> Result<u32> { if p.get(key).is_some() { dimension(key, fallback) } else { Ok(fallback) } };
        let (cw, ch) = (canvas_dimension("canvasWidth", w)?, canvas_dimension("canvasHeight", h)?);
        if u64::from(cw) * u64::from(ch) > 100_000_000 {
            return Err(bad("export canvas is limited to 100 million pixels"));
        }
        scratch.execute("image.canvasSize", json!({"width":cw,"height":ch,"anchor":"center","extensionColor":"transparent"}))?;
    }
    if p.get("smaller8Bit").and_then(Value::as_bool).unwrap_or(false) {
        scratch.execute("image.mode.bits8", json!({}))?;
    }
    if !p.get("transparency").and_then(Value::as_bool).unwrap_or(true) {
        scratch.execute("layer.flattenImage", json!({}))?;
    }
    let mut out = (*scratch.active().ok_or(EngineError::NoDocument)?.doc).clone();
    if p.get("nameLabel").and_then(|v| v.get("enabled")).and_then(Value::as_bool).unwrap_or(false) {
        let label = p.get("nameLabel").unwrap_or(&Value::Null);
        let size = label.get("fontSizePt").and_then(Value::as_f64).unwrap_or(12.0);
        if !size.is_finite() || !(1.0..=128.0).contains(&size) || out.name.chars().count() > 1024 {
            return Err(bad("name label requires 1..128 point text and at most 1024 characters"));
        }
        let pixel_size = size * f64::from(out.resolution_dpi) / 72.0;
        if !pixel_size.is_finite() || pixel_size > 4096.0 {
            return Err(bad("name label is too large at this document resolution"));
        }
        let mut text = photocraft_doc::TextLayer {
            text: out.name.replace(['\n', '\r'], " "),
            font_family: label.get("fontFamily").and_then(Value::as_str).unwrap_or(photocraft_text::fonts::DEFAULT_FAMILY).into(),
            size_pt: size as f32,
            color: photocraft_doc::Color::BLACK,
            transform: photocraft_doc::Affine::translate(8.0, pixel_size + 8.0),
            ..Default::default()
        };
        if label.get("color").is_some() {
            let c = crate::commands::color_param(label, "color", [0.0, 0.0, 0.0, 1.0]);
            text.color = photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3]);
        }
        crate::type_cmds::refresh(&out, &mut text);
        let bounds = text.cache.as_ref().map(|s| s.content_bounds()).unwrap_or_default();
        let header = (pixel_size * 1.5 + 16.0).ceil() as u32;
        let width = out.size.width.max(bounds.x1.max(0) as u32 + 8);
        let height = out.size.height.checked_add(header).ok_or_else(|| bad("name label canvas size overflow"))?;
        if width > 300_000 || height > 300_000 || u64::from(width) * u64::from(height) > 100_000_000 {
            return Err(bad("name label exceeds export size limit"));
        }
        let mut tmp = Session::new();
        tmp.add_document(out, None);
        tmp.execute("image.canvasSize", json!({"width":width,"height":height,"anchor":"bottomLeft","extensionColor":"transparent"}))?;
        out = (*tmp.active().ok_or(EngineError::NoDocument)?.doc).clone();
        out.layers.push(photocraft_doc::Layer::new("Artboard name", photocraft_doc::LayerContent::Text(text)));
    }
    if !p.get("embedIcc").and_then(Value::as_bool).unwrap_or(true) {
        out.icc_profile = None;
    }
    let metadata = match p.get("metadata").and_then(Value::as_str).unwrap_or("all") {
        "none" => crate::web_cmds::WebMetadata::None,
        "copyright" => crate::web_cmds::WebMetadata::Copyright,
        "copyrightAndContact" => crate::web_cmds::WebMetadata::CopyrightAndContact,
        "all" => crate::web_cmds::WebMetadata::All,
        _ => return Err(bad("unknown metadata policy")),
    };
    out.metadata.xmp = crate::web_cmds::web_xmp(doc, metadata);
    if !matches!(metadata, crate::web_cmds::WebMetadata::All) {
        out.metadata.text.clear();
    }
    Ok(out)
}

/// Portable names cannot collide after sanitizing or case folding. Windows device names are
/// escaped even on other platforms, so exports can be transferred without renaming them.
pub fn unique_name(name: &str, ext: &str, used: &mut HashSet<String>) -> String {
    let mut name = sanitize(name);
    if name.chars().count() > 120 {
        name = name.chars().take(120).collect();
    }
    let device = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4 && (device.starts_with("COM") || device.starts_with("LPT")) && device.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']))
    {
        name.insert(0, '_');
    }
    for i in 1.. {
        let candidate = if i == 1 { format!("{name}.{ext}") } else { format!("{name}_{i}.{ext}") };
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    String::new()
}

/// Validated output variants, scale in percent and a sanitized suffix.
pub fn variants(p: &Value) -> Result<Vec<(f64, String)>> {
    let Some(v) = p.get("variants") else {
        return Ok(vec![(100.0, String::new())]);
    };
    let v = v.as_array().filter(|a| !a.is_empty() && a.len() <= 32).ok_or_else(|| bad("variants must contain 1..32 entries"))?;
    v.iter()
        .map(|v| {
            let scale = v.get("scale").and_then(Value::as_f64).unwrap_or(100.0);
            if !scale.is_finite() || !(0.1..=1000.0).contains(&scale) {
                return Err(bad("variant scale must be 0.1..1000 percent"));
            }
            let suffix = v.get("suffix").and_then(Value::as_str).unwrap_or("");
            Ok((scale, if suffix.is_empty() { String::new() } else { sanitize(suffix) }))
        })
        .collect()
}

/// Shared file-count limit for engine and platform-service exports.
pub fn validate_output_count(boards: usize, variants: usize) -> Result<()> {
    if boards.saturating_mul(variants) > 1000 {
        return Err(bad("at most 1000 output files per export"));
    }
    Ok(())
}

/// Apply a scale variant to both inferred and explicitly supplied output dimensions.
pub fn variant_options(p: &Value, scale: f64) -> Result<Value> {
    if !p.is_object() || !scale.is_finite() || !(0.1..=1000.0).contains(&scale) {
        return Err(bad("variant options require an object and 0.1..1000 percent scale"));
    }
    let mut opts = p.clone();
    let base = p.get("scale").and_then(Value::as_f64).unwrap_or(100.0);
    let combined = base * scale / 100.0;
    if !combined.is_finite() || !(0.1..=1000.0).contains(&combined) {
        return Err(bad("combined variant scale must be 0.1..1000 percent"));
    }
    opts["scale"] = json!(combined);
    for key in ["width", "height", "canvasWidth", "canvasHeight"] {
        if let Some(value) = p.get(key).and_then(Value::as_f64) {
            let value = value * scale / 100.0;
            if !value.is_finite() || !(1.0..=300_000.0).contains(&value.round()) {
                return Err(bad(format!("variant {key} must be 1..300000 pixels")));
            }
            opts[key] = json!(value);
        }
    }
    Ok(opts)
}

pub fn to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = str_param(p, "dir", "file.export.artboardsToFiles").or_else(|_| str_param(p, "output", "file.export.artboardsToFiles"))?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let ids = selected(doc, p)?;
    let variants = variants(p)?;
    validate_output_count(ids.len(), variants.len())?;
    let ext = p.get("format").and_then(Value::as_str).unwrap_or("png").to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "psd" | "psb" | "pcraft" | "tif" | "tiff" | "webp" | "bmp" | "tga" | "gif" | "exr" | "qoi") {
        return Err(bad("unsupported artboard export format"));
    }
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&doc.name));
    let mut used = existing_names(dir)?;
    let mut plans = Vec::new();
    for id in ids {
        // Validate geometry before the first write, but render each board only when needed.
        let board = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let r = board.artboard().ok_or(EngineError::NoLayer(id))?.rect;
        if r.x0.checked_neg().is_none() || r.y0.checked_neg().is_none() {
            return Err(bad(format!("artboard origin ({},{}) cannot be translated", r.x0, r.y0)));
        }
        for (scale, suffix) in &variants {
            let name = if prefix.is_empty() { format!("{}{suffix}", board.name) } else { format!("{}_{}{suffix}", prefix, board.name) };
            plans.push((id, *scale, join(dir, &unique_name(&name, &ext, &mut used))));
        }
    }
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for (id, scale, path) in plans {
        let opts = variant_options(p, scale)?;
        let result = document(doc, id, &opts).and_then(|one| crate::file_cmds::save_doc(&one, &path, crate::file_cmds::SaveOpts::from_params(p)));
        match result {
            Ok(w) => {
                files.push(path);
                warnings.extend(w);
            }
            Err(e) => return Err(EngineError::Other(format!("{e}; already exported: {}", files.join(", ")))),
        }
    }
    Ok(json!({"files":files,"warnings":warnings}))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn existing_names(dir: &str) -> Result<HashSet<String>> {
    let mut names = HashSet::new();
    let path = std::path::Path::new(dir);
    if path.exists() {
        for entry in std::fs::read_dir(path).map_err(|e| EngineError::Other(format!("{dir}: {e}")))? {
            let entry = entry.map_err(|e| EngineError::Other(e.to_string()))?;
            names.insert(entry.file_name().to_string_lossy().to_lowercase());
        }
    }
    Ok(names)
}
#[cfg(target_arch = "wasm32")]
pub fn existing_names(_: &str) -> Result<HashSet<String>> {
    Ok(HashSet::new())
}

fn zip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    encoder.finish().map_err(|e| EngineError::Other(e.to_string()))
}

struct Page {
    w: u32,
    h: u32,
    dpi: f32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
    jpeg: bool,
}

fn page(doc: &Document, p: &Value) -> Result<Page> {
    let st = crate::web_cmds::WebSettings { convert_to_srgb: p.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(true), ..Default::default() };
    let (doc, _, _) = crate::web_cmds::web_document(doc, &json!({}), &st)?;
    let buf = photocraft_compose::flatten(&doc);
    let jpeg = match p.get("compression").and_then(Value::as_str).unwrap_or("jpeg") {
        "jpeg" => true,
        "zip" => false,
        _ => return Err(bad("PDF compression must be jpeg or zip")),
    };
    let mut rgb = Vec::with_capacity(buf.px.len() * 3);
    let mut alpha = Vec::with_capacity(buf.px.len());
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for q in &buf.px {
        rgb.extend(q.iter().take(3).map(|v| byte(*v)));
        alpha.push(byte(q[3]));
    }
    let alpha = if alpha.iter().any(|v| *v != 255) { Some(zip(&alpha)?) } else { None };
    let rgb = if jpeg {
        let q = p.get("quality").and_then(Value::as_f64).unwrap_or(10.0);
        if !q.is_finite() || !(0.0..=12.0).contains(&q) {
            return Err(bad("PDF quality must be 0..12"));
        }
        photocraft_codecs::web::encode_jpeg_rgb8(doc.size.width, doc.size.height, &rgb, (q / 12.0 * 99.0 + 1.0).round() as u8, false, true, None, None, None)
            .map_err(|e| EngineError::Other(e.to_string()))?
    } else {
        zip(&rgb)?
    };
    Ok(Page { w: doc.size.width, h: doc.size.height, dpi: doc.resolution_dpi, rgb, alpha, icc: doc.icc_profile.as_deref().cloned(), jpeg })
}

fn pdf(pages: &[Page]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let mut kids = Vec::new();
    let stream = |header: String, data: &[u8]| {
        let mut out = format!("<< {header} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nendstream");
        out
    };
    for p in pages {
        let id = objects.len() + 1;
        kids.push(format!("{id} 0 R"));
        let pw = f64::from(p.w) * 72.0 / f64::from(p.dpi.max(1.0));
        let ph = f64::from(p.h) * 72.0 / f64::from(p.dpi.max(1.0));
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.3} {ph:.3}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                id + 2,
                id + 1
            )
            .into_bytes(),
        );
        objects.push(stream(String::new(), format!("q {pw:.3} 0 0 {ph:.3} 0 0 cm /Im0 Do Q").as_bytes()));
        let alpha_id = id + 3;
        let icc_id = id + 3 + usize::from(p.alpha.is_some());
        let cs = if p.icc.is_some() { format!("[/ICCBased {icc_id} 0 R]") } else { "/DeviceRGB".into() };
        let mask = if p.alpha.is_some() { format!(" /SMask {alpha_id} 0 R") } else { String::new() };
        objects.push(stream(
            format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /{}{mask}",
                p.w,
                p.h,
                if p.jpeg { "DCTDecode" } else { "FlateDecode" }
            ),
            &p.rgb,
        ));
        if let Some(alpha) = &p.alpha {
            objects.push(stream(
                format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode", p.w, p.h),
                alpha,
            ));
        }
        if let Some(icc) = &p.icc {
            objects.push(stream("/N 3 /Alternate /DeviceRGB".into(), icc));
        }
    }
    if let Some(tree) = objects.get_mut(1) {
        *tree = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    }
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

pub fn to_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.export.artboardsToPdf")?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let ids = selected(doc, p)?;
    let separate = p.get("separateFiles").and_then(Value::as_bool).unwrap_or(false);
    let mut pages = Vec::new();
    let mut files = Vec::new();
    let mut used = existing_names(&dir_for_pdf(path))?;
    let dir = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut pixel_work = 0_u64;
    for id in &ids {
        let one = document(doc, *id, p)?;
        pixel_work = pixel_work.saturating_add(u64::from(one.size.width) * u64::from(one.size.height));
        if !separate && pixel_work > 100_000_000 {
            return Err(bad("multi-page PDF is limited to 100 million total pixels; export separate files"));
        }
        let pg = page(&one, p)?;
        if separate {
            let target = join(&dir, &unique_name(&format!("{}_{}", stem(path), one.name), "pdf", &mut used));
            write_file(&target, &pdf(&[pg]))?;
            files.push(target);
        } else {
            pages.push(pg);
        }
    }
    if !separate {
        write_file(path, &pdf(&pages))?;
        files.push(path.to_string());
    }
    Ok(json!({"path":path,"files":files,"pages":ids.len()}))
}

fn dir_for_pdf(path: &str) -> String {
    std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size};
    use std::io::Read;

    fn fixture() -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.add_document(Document::new("boards", Size::new(100, 80), ColorMode::Rgb, SampleType::U8), None);
        let a = LayerId(
            s.execute("layer.new.artboard", json!({"name":"Card","rect":[334,-131,345,360],"background":"transparent"})).unwrap()["layer"].as_u64().unwrap(),
        );
        let b = LayerId(s.execute("layer.new.artboard", json!({"name":"card","rect":[0,0,10,8],"background":"black"})).unwrap()["layer"].as_u64().unwrap());
        (s, a, b)
    }

    #[test]
    fn selection_is_validated_ordered_and_reversible() {
        let (s, a, b) = fixture();
        let doc = &s.active().unwrap().doc;
        assert_eq!(selected(doc, &json!({})).unwrap(), vec![b, a]);
        assert_eq!(selected(doc, &json!({"artboards":[a.0,b.0,a.0],"reverseOrder":true})).unwrap(), vec![a, b]);
        for p in [json!({"artboards":[]}), json!({"artboards":[999]}), json!({"artboards":[-1]}), json!({"artboards":"all"})] {
            assert!(selected(doc, &p).is_err());
        }
    }

    #[test]
    fn negative_empty_board_keeps_full_frame_and_source() {
        let (s, a, _) = fixture();
        let doc = &s.active().unwrap().doc;
        let before = (**doc).clone();
        let one = document(doc, a, &json!({})).unwrap();
        assert_eq!(one.size, Size::new(345, 360));
        assert!(photocraft_compose::flatten(&one).px.iter().all(|q| q[3] == 0.0));
        let exported = photocraft_io::export(&one, "empty.png", &Default::default()).unwrap();
        let decoded = photocraft_io::import("empty.png", &exported.bytes).unwrap().document;
        assert_eq!(decoded.size, one.size);
        assert_eq!(**doc, before);
        let layer = crate::layer_menu_cmds::layer_document(doc, a).unwrap();
        assert_eq!(layer.size, one.size);
    }

    #[test]
    fn overlapping_oversized_shape_uses_normalized_workspace_geometry() {
        let mut s = Session::new();
        s.add_document(Document::new("small canvas", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        let target = LayerId(s.execute("layer.new.artboard", json!({"rect":[-334,-131,40,30],"background":"transparent"})).unwrap()["layer"].as_u64().unwrap());
        let overlap =
            LayerId(s.execute("layer.new.artboard", json!({"rect":[-314,-131,20,15],"background":"transparent"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("shape.create", json!({"rect":[-350,-150,400,400],"fill":"#0000ff"})).unwrap();
        let source = &s.active().unwrap().doc;
        let before = (**source).clone();
        let expected = photocraft_compose::render(source, source.layer(target).unwrap().artboard().unwrap().rect).px;
        let out = document(source, target, &json!({"includeOverlapping":true})).unwrap();
        assert_eq!(out.size, Size::new(40, 30));
        assert_eq!(out.layer(overlap).unwrap().artboard().unwrap().rect, photocraft_geom::Rect::new(20, 0, 40, 15));
        assert_eq!(photocraft_compose::flatten(&out).px, expected);
        assert!(expected[5 * 40 + 30][2] > 0.99 && expected[5 * 40 + 30][3] > 0.99);
        assert!(photocraft_compose::flatten(&document(source, target, &json!({})).unwrap()).px.iter().all(|p| p[3] == 0.0));
        assert_eq!(**source, before);
    }

    #[test]
    fn frame_extraction_has_no_export_size_limit() {
        let mut s = Session::new();
        s.add_document(Document::new("large board", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        let id = LayerId(s.execute("layer.new.artboard", json!({"rect":[-334,-131,20000,20000]})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(frame_document(&s.active().unwrap().doc, id, true).unwrap().size, Size::new(20000, 20000));
        assert!(document(&s.active().unwrap().doc, id, &json!({})).is_err());
    }

    #[test]
    fn resize_canvas_and_background_are_independent() {
        let (s, _, b) = fixture();
        let doc = &s.active().unwrap().doc;
        let out = document(doc, b, &json!({"scale":200,"canvasWidth":24,"includeBackground":false})).unwrap();
        assert_eq!(out.size, Size::new(24, 16));
        assert!(photocraft_compose::flatten(&out).px.iter().all(|q| q[3] == 0.0));
        let black = document(doc, b, &json!({"scale":200})).unwrap();
        assert_eq!(black.size, Size::new(20, 16));
        assert!(photocraft_compose::flatten(&black).px.iter().all(|q| q[3] > 0.99 && q[0] < 0.01));
        for p in [json!({"scale":0}), json!({"width":300001}), json!({"width":300000,"height":300000}), json!({"metadata":"unknown"})] {
            assert!(document(doc, b, &p).is_err());
        }
    }

    #[test]
    fn names_survive_case_folding_sanitization_and_device_names() {
        let mut used = HashSet::new();
        assert_eq!(unique_name("Card", "png", &mut used), "Card.png");
        assert_eq!(unique_name("card", "png", &mut used), "card_2.png");
        assert_eq!(unique_name("a/b", "png", &mut used), "a_b.png");
        assert_eq!(unique_name("a\\b", "png", &mut used), "a_b_2.png");
        assert_eq!(unique_name("CON", "png", &mut used), "_CON.png");
        assert_eq!(unique_name("NUL.txt", "png", &mut used), "_NUL.txt.png");
    }

    #[test]
    fn rendered_name_label_extends_export_without_changing_board() {
        let (s, _, b) = fixture();
        let source = &s.active().unwrap().doc;
        let before = (**source).clone();
        let out = document(source, b, &json!({"nameLabel":{"enabled":true,"fontSizePt":12},"includeBackground":false})).unwrap();
        assert!(out.size.width > 10);
        assert!(out.size.height > 8);
        assert!(out.layers.iter().any(|l| matches!(&l.content,photocraft_doc::LayerContent::Text(t) if t.text=="card")));
        assert!(photocraft_compose::flatten(&out).px.iter().any(|q| q[3] > 0.0));
        assert_eq!(**source, before);
    }

    #[test]
    fn pdf_streams_decode_and_keep_alpha_and_resolution() {
        let (s, a, b) = fixture();
        let doc = &s.active().unwrap().doc;
        let mut empty = document(doc, a, &json!({"width":3,"height":2})).unwrap();
        empty.resolution_dpi = 144.0;
        let pg = page(&empty, &json!({"compression":"zip"})).unwrap();
        let mut rgb = Vec::new();
        flate2::read::ZlibDecoder::new(pg.rgb.as_slice()).read_to_end(&mut rgb).unwrap();
        assert_eq!(rgb.len(), 3 * 2 * 3);
        let mut alpha = Vec::new();
        flate2::read::ZlibDecoder::new(pg.alpha.as_ref().unwrap().as_slice()).read_to_end(&mut alpha).unwrap();
        assert_eq!(alpha, vec![0; 6]);
        let bytes = pdf(&[pg]);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/MediaBox [0 0 1.500 1.000]"));
        assert!(text.contains("/SMask 6 0 R"));
        let black = document(doc, b, &json!({})).unwrap();
        let pg = page(&black, &json!({"compression":"jpeg"})).unwrap();
        let decoded = photocraft_io::import("page.jpg", &pg.rgb).unwrap().document;
        assert_eq!(decoded.size, black.size);
        assert!(photocraft_compose::flatten(&decoded).px.iter().all(|q| q[0] < 0.02 && q[1] < 0.02 && q[2] < 0.02));
    }

    #[test]
    fn variants_scale_explicit_image_and_canvas_dimensions() {
        let (s, _, b) = fixture();
        let p = json!({"width":10,"height":8,"canvasWidth":12,"canvasHeight":10});
        for (scale, size) in [(100.0, Size::new(12, 10)), (200.0, Size::new(24, 20))] {
            let opts = variant_options(&p, scale).unwrap();
            let out = document(&s.active().unwrap().doc, b, &opts).unwrap();
            assert_eq!(out.size, size);
            assert_eq!(opts["width"], json!(10.0 * scale / 100.0));
            assert_eq!(opts["height"], json!(8.0 * scale / 100.0));
        }
        assert!(validate_output_count(500, 2).is_ok());
        assert!(validate_output_count(501, 2).is_err());
        assert!(validate_output_count(usize::MAX, 32).is_err());
        assert!(variant_options(&Value::Null, 100.0).is_err());
        assert!(variant_options(&json!({"scale":1e308}), 1000.0).is_err());
        assert!(variant_options(&json!({"width":1e308}), 1000.0).is_err());
    }

    #[test]
    fn files_do_not_overwrite_existing_and_selected_variants_only() {
        let (mut s, a, _) = fixture();
        let dir = std::env::temp_dir().join(format!("pcraft-export-planning-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Card.png"), b"keep").unwrap();
        let before = (*s.active().unwrap().doc).clone();
        let p = json!({"dir":dir.to_string_lossy(),"prefix":"","artboards":[a.0],"variants":[{"scale":100,"suffix":""},{"scale":200,"suffix":"@2x"}]});
        let out = to_files(&mut s, &p).unwrap();
        assert_eq!(out["files"].as_array().unwrap().len(), 2);
        assert_eq!(std::fs::read(dir.join("Card.png")).unwrap(), b"keep");
        for (index, w, h) in [(0, 345, 360), (1, 690, 720)] {
            let bytes = std::fs::read(out["files"][index].as_str().unwrap()).unwrap();
            let doc = photocraft_io::import("board.png", &bytes).unwrap().document;
            assert_eq!(doc.size, Size::new(w, h));
        }
        assert_eq!(*s.active().unwrap().doc, before);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
