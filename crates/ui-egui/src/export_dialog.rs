//! File › Export › Export As… (and Quick Export as PNG): format, quality, transparency and scale,
//! with a preview and an estimated file size. The estimate encodes a small proxy and scales by
//! pixel count, so the dialog stays instant on 36 MP documents.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Map, Value, json};

use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::{ExportSettings, PhotocraftApp};

const FORMATS: [(&str, &str); 5] = [("png", "PNG"), ("jpg", "JPG"), ("webp", "WebP"), ("tif", "TIFF"), ("tga", "TGA")];
const BOARD_OPTIONS: &[&str] = &[
    "format",
    "quality",
    "lossless",
    "transparency",
    "scale",
    "metadata",
    "convertToSrgb",
    "embedIcc",
    "width",
    "height",
    "resample",
    "canvasWidth",
    "canvasHeight",
    "smaller8Bit",
    "customCanvas",
];

fn board_options(f: &Map<String, Value>, id: u64) -> Map<String, Value> {
    let mut out = f.clone();
    if f.get("__preview").and_then(Value::as_u64) != Some(id) {
        let options = f.get("__boardOptions").and_then(|v| v.get(id.to_string())).or_else(|| f.get("__defaults"));
        for key in BOARD_OPTIONS {
            out.remove(*key);
            if let Some(value) = options.and_then(|v| v.get(*key)) {
                out.insert((*key).into(), value.clone());
            }
        }
    }
    out
}

fn activate_board(f: &mut Map<String, Value>, id: u64) {
    if let Some(old) = f.get("__preview").and_then(Value::as_u64) {
        let values: Map<String, Value> = BOARD_OPTIONS.iter().filter_map(|key| f.get(*key).cloned().map(|value| ((*key).into(), value))).collect();
        let mut options = f.get("__boardOptions").and_then(Value::as_object).cloned().unwrap_or_default();
        options.insert(old.to_string(), Value::Object(values));
        f.insert("__boardOptions".into(), Value::Object(options));
    }
    let values = board_options(f, id);
    for key in BOARD_OPTIONS {
        f.remove(*key);
        if let Some(value) = values.get(*key) {
            f.insert((*key).into(), value.clone());
        }
    }
    f.insert("__preview".into(), json!(id));
    f.insert("__previewOverview".into(), json!(false));
}

/// Checked export targets and the explicitly focused preview are independent. The overview
/// shows checked boards; a row click temporarily focuses that board without changing checks.
fn preview_boards(doc: &Document, f: &Map<String, Value>) -> Vec<photocraft_doc::LayerId> {
    let checked: Vec<_> = f.get("__artboards").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let boards = doc.artboards();
    let chosen: Vec<_> = boards.iter().rev().filter(|b| checked.contains(&b.0.0)).map(|b| b.0).collect();
    if !f.get("__previewOverview").and_then(Value::as_bool).unwrap_or(true)
        && let Some(focus) = f.get("__preview").and_then(Value::as_u64).map(photocraft_doc::LayerId).filter(|id| boards.iter().any(|b| b.0 == *id))
    {
        return vec![focus];
    }
    chosen
}

fn show_overview(doc: &Document, f: &mut Map<String, Value>) {
    f.insert("__previewOverview".into(), json!(true));
    let chosen = preview_boards(doc, f);
    let focus = f.get("__preview").and_then(Value::as_u64).map(photocraft_doc::LayerId);
    let target = if chosen.len() == 1 || !focus.is_some_and(|id| chosen.contains(&id)) { chosen.first().copied() } else { focus };
    if let Some(id) = target {
        if focus != Some(id) {
            if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) {
                f.insert("__preview".into(), json!(id.0));
            } else {
                activate_board(f, id.0);
            }
        }
        if let Some(board) = doc.layer(id).and_then(|l| l.artboard()) {
            f.insert("__w".into(), json!(board.rect.width()));
            f.insert("__h".into(), json!(board.rect.height()));
        }
    }
    f.insert("__previewOverview".into(), json!(true));
}

fn preview_fields(f: &Map<String, Value>, id: photocraft_doc::LayerId) -> Map<String, Value> {
    let mut options = if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) { f.clone() } else { board_options(f, id.0) };
    options.insert("__preview".into(), json!(id.0));
    options
}

pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__export")) {
        return Ok(d.id);
    }
    let st = app.session.active().ok_or("no document")?;
    let mut f = Map::new();
    f.insert("__export".into(), json!(true));
    f.insert("__label".into(), json!("Export As"));
    f.insert("format".into(), json!("png"));
    f.insert("quality".into(), json!(app.session.prefs().export.jpeg_quality));
    f.insert("lossless".into(), json!(app.session.prefs().export.webp_lossless));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("metadata".into(), json!("none"));
    f.insert("tgaBits".into(), json!("auto"));
    f.insert("convertToSrgb".into(), json!(true));
    f.insert("embedIcc".into(), json!(true));
    if st.doc.has_artboards() {
        let boards: Vec<u64> = st.doc.artboards().into_iter().rev().map(|b| b.0.0).collect();
        f.insert("__artboards".into(), json!(boards));
        f.insert("__previewOverview".into(), json!(true));
        if let Some(id) = boards.first() {
            f.insert("__preview".into(), json!(id));
            if let Some(a) = st.doc.layer(photocraft_doc::LayerId(*id)).and_then(|l| l.artboard()) {
                f.insert("__w".into(), json!(a.rect.width()));
                f.insert("__h".into(), json!(a.rect.height()));
            }
        }
        f.insert("__variants".into(), json!([{"scale":100,"suffix":""}]));
    }
    if !f.contains_key("__w") {
        f.insert("__w".into(), json!(st.doc.size.width));
    }
    if !f.contains_key("__h") {
        f.insert("__h".into(), json!(st.doc.size.height));
    }
    if f.contains_key("__artboards") {
        let defaults: Map<String, Value> = BOARD_OPTIONS.iter().filter_map(|key| f.get(*key).cloned().map(|value| ((*key).into(), value))).collect();
        f.insert("__defaults".into(), Value::Object(defaults));
    }
    Ok(app.ui.open_dialog(DialogKind::Command, f))
}

/// The dedicated Files/PDF entries reuse the same independently checked board list.
pub fn open_artboards(app: &mut PhotocraftApp, pdf: bool) -> Result<u64, String> {
    if !app.session.active().is_some_and(|s| s.doc.has_artboards()) {
        return Err("the document has no artboards".into());
    }
    let id = open(app)?;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.insert("__artboardPdf".into(), json!(pdf));
        d.fields.insert("__label".into(), json!(if pdf { "Artboards to PDF" } else { "Artboards to Files" }));
        d.fields.insert("includeBackground".into(), json!(true));
        d.fields.insert("includeOverlapping".into(), json!(false));
        d.fields.insert("reverseOrder".into(), json!(false));
        d.fields.insert("__nameLabel".into(), json!(false));
        if pdf {
            d.fields.insert("quality".into(), json!(10));
            d.fields.insert("compression".into(), json!("jpeg"));
        }
    }
    Ok(id)
}

/// Layer › Export As…: the same dialog for just the active layer (trimmed to its pixels).
pub fn open_layer(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let ldoc = photocraft_engine::layer_menu_cmds::layer_document(&st.doc, layer).map_err(|e| e.to_string())?;
    let id = open(app)?;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.remove("__artboards");
        d.fields.remove("__preview");
        d.fields.remove("__variants");
        d.fields.insert("__layer".into(), json!(layer.0));
        d.fields.insert("__label".into(), json!(format!("Export As: {}", ldoc.name)));
        d.fields.insert("__w".into(), json!(ldoc.size.width));
        d.fields.insert("__h".into(), json!(ldoc.size.height));
    }
    Ok(id)
}

/// Layer › Quick Export as PNG.
pub fn quick_export_layer_png(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<Value, String> {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("__layer".into(), json!(layer.0));
    confirm(app, &f)
}

/// Export each selected layer separately, with one destination prompt for the batch.
/// The chosen path names the first file; subsequent files have numbered sibling names.
pub fn quick_export_selected_layers_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layers = st.selected_layers();
    if layers.len() == 1 {
        return quick_export_layer_png(app, layers[0]);
    }
    if layers.is_empty() {
        return Err("no selected layers".into());
    }
    // Capture all sources before showing the dialog: changing selection or document while it
    // is open cannot change which layers get exported.
    let docs: Vec<_> =
        layers.into_iter().map(|id| photocraft_engine::layer_menu_cmds::layer_document(&st.doc, id).map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    let suggested = format!("{}.png", export_name(&st.doc.name));
    app.pick_save(&suggested, move |app, path| {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err("Quick Export as PNG writes .png files".into());
        }
        let paths = batch_paths(&path, &docs);
        let mut exported = Vec::new();
        for (doc, path) in docs.iter().zip(paths) {
            let export = app.services.export.as_ref().ok_or("no exporter configured")?;
            let (bytes, warnings) = export(doc, &path, &ExportSettings { xmp_all: false, ..Default::default() })?;
            let write = app.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
            crate::notices::io_warnings(app, &format!("Exported {}", crate::file_open::display_name(&path)), &warnings);
            exported.push(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}));
        }
        app.ui.status = format!("Exported {} PNG files", exported.len());
        app.ui.status_error = false;
        Ok(json!({"files": exported}))
    })
}

fn export_name(name: &str) -> String {
    let name: String = name.chars().take(80).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() { "layer".into() } else { name.into() }
}

fn batch_paths(path: &str, docs: &[Document]) -> Vec<String> {
    let first = std::path::Path::new(path);
    let parent = first.parent().unwrap_or_else(|| std::path::Path::new(""));
    let stem = first.file_stem().and_then(|s| s.to_str()).unwrap_or("layers");
    docs.iter()
        .enumerate()
        .map(|(i, doc)| {
            if i == 0 {
                return path.to_string();
            }
            let name = export_name(&doc.name);
            let candidate = parent.join(format!("{stem}-{}-{name}.png", i + 1));
            // The first file's overwrite is handled by the Save dialog; never overwrite sibling
            // files silently on desktop. Downloads on the web are named by the browser.
            #[cfg(not(target_arch = "wasm32"))]
            let candidate = {
                let mut candidate = candidate;
                let mut suffix = 1u64;
                while candidate.exists() {
                    candidate = parent.join(format!("{stem}-{}-{name}-{suffix}.png", i + 1));
                    suffix = suffix.saturating_add(1);
                }
                candidate
            };
            candidate.to_string_lossy().into_owned()
        })
        .collect()
}

/// The document a dialog exports: the whole image, or one layer (`__layer`).
fn source_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    let st = app.session.active().ok_or("no document")?;
    match f.get("__layer").and_then(Value::as_u64) {
        Some(id) => photocraft_engine::layer_menu_cmds::layer_document(&st.doc, photocraft_doc::LayerId(id)).map(Arc::new).map_err(|e| e.to_string()),
        None => match f.get("__preview").and_then(Value::as_u64) {
            Some(id) => photocraft_engine::artboard_export::document(&st.doc, photocraft_doc::LayerId(id),&json!({"includeBackground":f.get("includeBackground").cloned().unwrap_or(json!(true)),"includeOverlapping":f.get("includeOverlapping").cloned().unwrap_or(json!(false))})).map(Arc::new).map_err(|e|e.to_string()),
            None => Ok(st.doc.clone()),
        },
    }
}

fn s(f: &Map<String, Value>, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn n(f: &Map<String, Value>, k: &str, d: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// The document as exported: scaled (engine Image Size, bicubic) and flattened over white when
/// transparency is off or the format has no alpha.
fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>) -> Result<Document, String> {
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc.clone(), None);
    if f.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(false) {
        if doc.mode != photocraft_doc::ColorMode::Rgb {
            s.execute("image.mode.rgb", json!({})).map_err(|e| e.to_string())?;
        }
        if doc.icc_profile.is_some() {
            s.execute("edit.convertToProfile", json!({"profile":"srgb"})).map_err(|e| e.to_string())?;
        }
    }
    let scale = n(f, "scale", 100.0) / 100.0;
    let (w, h) = (n(f, "width", f64::from(doc.size.width) * scale), n(f, "height", f64::from(doc.size.height) * scale));
    let canvas = f.contains_key("canvasWidth") || f.contains_key("canvasHeight");
    let (cw, ch) = (n(f, "canvasWidth", w), n(f, "canvasHeight", h));
    let proxy_scale = max_side.map_or(1.0, |m| (f64::from(m) / w.max(h).max(cw).max(ch)).min(1.0));
    // Apply the preview limit before rounding: rounding an intermediate export width would
    // distort the proxy's aspect ratio (13 × 20 at 50%, limited to 8 px, is 5 × 8).
    // Both dimensions are sent: a scale can leave a 1 px width unchanged while halving the
    // height, and the engine would otherwise keep the aspect ratio from the width alone.
    let (w, h) = scaled_size(w, h, proxy_scale);
    if (w - doc.size.width as f64).abs() >= 1.0 || (h - doc.size.height as f64).abs() >= 1.0 {
        s.execute("image.imageSize", json!({"width": w, "height": h, "resample": if max_side.is_some() { "bilinear" } else { f.get("resample").and_then(Value::as_str).unwrap_or("bicubic") }}))
            .map_err(|e| e.to_string())?;
    }
    if canvas {
        let (cw, ch) = scaled_size(cw, ch, proxy_scale);
        s.execute("image.canvasSize", json!({"width":cw,"height":ch,"anchor":"center","extensionColor":"transparent"})).map_err(|e| e.to_string())?;
    }
    if f.get("smaller8Bit").and_then(Value::as_bool).unwrap_or(false) {
        s.execute("image.mode.bits8", json!({})).map_err(|e| e.to_string())?;
    }
    let fmt = s_fmt(f);
    if !f.get("transparency").and_then(Value::as_bool).unwrap_or(true) || fmt == "jpg" {
        s.execute("layer.flattenImage", json!({})).map_err(|e| e.to_string())?;
    }
    let mut out = s.active().map(|d| (*d.doc).clone()).ok_or_else(|| "export failed".to_string())?;
    if !f.get("embedIcc").and_then(Value::as_bool).unwrap_or(true) {
        out.icc_profile = None;
    }
    let policy = match f.get("metadata").and_then(Value::as_str).unwrap_or("all") {
        "none" => photocraft_engine::web_cmds::WebMetadata::None,
        "copyrightAndContact" => photocraft_engine::web_cmds::WebMetadata::CopyrightAndContact,
        _ => photocraft_engine::web_cmds::WebMetadata::All,
    };
    out.metadata.xmp = photocraft_engine::web_cmds::web_xmp(doc, policy);
    if !matches!(policy, photocraft_engine::web_cmds::WebMetadata::All) {
        out.metadata.text.clear();
    }
    Ok(out)
}

/// The exported pixel size: each side scaled and rounded on its own, never below 1 px. The
/// dialog's size label and `export_document` share it so the label names the file's real size.
fn scaled_size(w: f64, h: f64, scale: f64) -> (f64, f64) {
    ((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// Export As already supports lossy WebP; reuse the Quick Export defaults when
/// choosing that format so both flows expose the same quality and lossless options.
fn set_format_defaults(f: &mut Map<String, Value>, fmt: &str, prefs: &photocraft_engine::prefs::Export) {
    f.insert("format".into(), json!(fmt));
    match fmt {
        "jpg" => {
            f.insert("quality".into(), json!(prefs.jpeg_quality));
        }
        "webp" => {
            f.insert("quality".into(), json!(prefs.webp_quality));
            f.insert("lossless".into(), json!(prefs.webp_lossless));
        }
        _ => {}
    }
}

fn s_fmt(f: &Map<String, Value>) -> String {
    let v = s(f, "format");
    if v.is_empty() { "png".into() } else { v }
}

fn lossless(f: &Map<String, Value>) -> bool {
    f.get("lossless").and_then(Value::as_bool).unwrap_or(false)
}

/// TGA bits per pixel: `"24"` or `"32"`; `"auto"` (the default) is 32 with an alpha channel or
/// transparency, else 24, as Export As wrote before it offered the choice.
fn tga_bits(f: &Map<String, Value>) -> Option<photocraft_io::TgaBits> {
    s(f, "tgaBits").parse().ok().and_then(photocraft_io::TgaBits::from_bits)
}

fn settings(f: &Map<String, Value>) -> ExportSettings {
    let fmt = s_fmt(f);
    let quality = n(f, "quality", 85.0).clamp(1.0, 100.0) as u8;
    ExportSettings {
        jpeg_quality: (fmt == "jpg").then_some(quality),
        webp_lossless: fmt != "webp" || lossless(f),
        webp_quality: (fmt == "webp" && !lossless(f)).then_some(quality),
        xmp_all: s(f, "metadata") == "all",
        tga_bits: if fmt == "tga" { tga_bits(f) } else { None },
        ..Default::default()
    }
}

/// Estimated size (bytes) from a ≤512 px proxy encode, scaled by pixel count.
fn estimate(app: &PhotocraftApp, doc: &Document, f: &Map<String, Value>) -> Option<u64> {
    let export = app.services.export.as_ref()?;
    let proxy = export_document(doc, f, Some(512)).ok()?;
    let (bytes, _) = export(&proxy, &format!("estimate.{}", s_fmt(f)), &settings(f)).ok()?;
    let scale = n(f, "scale", 100.0) / 100.0;
    let full = doc.size.width as f64 * scale * doc.size.height as f64 * scale;
    let small = (proxy.size.width as f64 * proxy.size.height as f64).max(1.0);
    Some((bytes.len() as f64 * full / small) as u64)
}

type PreviewTexture = (Option<u64>, Arc<egui::TextureHandle>);

fn cached_preview(app: &PhotocraftApp, ui: &egui::Ui, doc: &Document, f: &Map<String, Value>) -> Result<PreviewTexture, String> {
    let layer = f.get("__layer").or_else(|| f.get("__preview")).and_then(Value::as_u64);
    let key = egui::Id::new(("export-preview", doc.id.0, app.session.active().map_or(0, |s| s.revision), layer));
    let signature: Map<String, Value> = f
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "__artboards" | "__boardOptions" | "__defaults" | "__zoom" | "__twoUp" | "__previewOverview"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let sig = serde_json::to_string(&signature).unwrap_or_default();
    let cached: Option<(String, Option<u64>, Arc<egui::TextureHandle>)> = ui.data(|d| d.get_temp(key));
    if let Some((_, size, tex)) = cached.filter(|c| c.0 == sig) {
        return Ok((size, tex));
    }
    let src = source_document(app, f)?;
    let size = estimate(app, &src, f);
    let img = photocraft_compose::thumbnail(&export_document(&src, f, Some(360))?, 360);
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    let tex = Arc::new(ui.ctx().load_texture("export-preview", color, egui::TextureOptions::LINEAR));
    ui.data_mut(|d| d.insert_temp(key, (sig, size, tex.clone())));
    Ok((size, tex))
}

fn preview_image(ui: &mut egui::Ui, tex: &egui::TextureHandle, side: f32, zoom: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    crate::widgets::checker(ui.painter(), r, 8.0);
    let ts = tex.size_vec2();
    let k = (side / ts.max_elem()).min(1.0e3) * zoom / 100.0;
    let ir = egui::Rect::from_center_size(r.center(), ts * k);
    ui.painter().with_clip_rect(r).image(tex.id(), ir, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
}

fn preview_size(ui: &mut egui::Ui, f: &Map<String, Value>, size: Option<u64>) {
    let est = size.map_or("—".to_string(), |b| crate::sizing::human_bytes(b as f64));
    let format = if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) { "PDF".into() } else { s_fmt(f).to_uppercase() };
    ui.label(egui::RichText::new(format!("{format}  ≈ {est}")).color(Tokens::get(ui.ctx()).text_dim).size(11.5));
}

fn board_grid(app: &PhotocraftApp, ui: &mut egui::Ui, doc: &Document, f: &Map<String, Value>, boards: &[photocraft_doc::LayerId]) {
    let gap = 12.0;
    let side = ((ui.available_width() - gap) / 2.0).max(1.0);
    egui::ScrollArea::vertical().id_salt("export-board-overview").max_height(360.0).show(ui, |ui| {
        for pair in boards.chunks(2) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for id in pair {
                    ui.push_id(id.0, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(side);
                            let Some(board) = doc.layer(*id) else { return };
                            let label = ui
                                .add(egui::Label::new(egui::RichText::new(&board.name).font(crate::theme::semibold(12.0))).truncate())
                                .on_hover_text(&board.name);
                            label.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{}: {}", tl!("Preview"), board.name)));
                            let options = preview_fields(f, *id);
                            // Long lists retain their geometry without rendering offscreen boards.
                            if !ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, egui::vec2(side, side))) {
                                ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                                preview_size(ui, &options, None);
                                return;
                            }
                            match cached_preview(app, ui, doc, &options) {
                                Ok((size, tex)) => {
                                    preview_image(ui, &tex, side, 100.0);
                                    preview_size(ui, &options, size);
                                }
                                Err(error) => {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                                    crate::widgets::checker(ui.painter(), rect, 8.0);
                                    let mut message = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)));
                                    message.set_clip_rect(rect.intersect(ui.clip_rect()));
                                    message.colored_label(Tokens::get(ui.ctx()).danger, error);
                                    preview_size(ui, &options, None);
                                }
                            }
                        });
                    });
                }
            });
            ui.add_space(gap);
        }
    });
}

fn single_preview(app: &PhotocraftApp, ui: &mut egui::Ui, doc: &Document, f: &mut Map<String, Value>) {
    let layer = f.get("__layer").or_else(|| f.get("__preview")).and_then(Value::as_u64);
    let (size, tex) = match cached_preview(app, ui, doc, f) {
        Ok(preview) => preview,
        Err(error) => {
            ui.colored_label(Tokens::get(ui.ctx()).danger, error);
            return;
        }
    };
    let mut two_up = f.get("__twoUp").and_then(Value::as_bool).unwrap_or(false);
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut two_up, tl!("2-Up"));
    });
    f.insert("__twoUp".into(), json!(two_up));
    let side = if two_up { 145.0 } else { 300.0 };
    if two_up {
        let original_key = egui::Id::new((
            "export-original",
            doc.id.0,
            app.session.active().map_or(0, |s| s.revision),
            layer,
            f.get("includeBackground").and_then(Value::as_bool).unwrap_or(true),
            f.get("includeOverlapping").and_then(Value::as_bool).unwrap_or(false),
        ));
        let texture: Option<Arc<egui::TextureHandle>> = ui.data(|d| d.get_temp(original_key));
        let texture = texture.unwrap_or_else(|| {
            let src = source_document(app, f).unwrap_or_else(|_| Arc::new(doc.clone()));
            let img = photocraft_compose::thumbnail(&src, 150);
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
            let tex = Arc::new(ui.ctx().load_texture("export-original", color, egui::TextureOptions::LINEAR));
            ui.data_mut(|d| d.insert_temp(original_key, tex.clone()));
            tex
        });
        ui.label(tl!("Original"));
        ui.add(egui::Image::new((texture.id(), texture.size_vec2())).max_size(egui::vec2(side, side)));
        ui.label(tl!("Preview"));
    }
    let mut zoom = n(f, "__zoom", 100.0) as f32;
    preview_image(ui, &tex, side, zoom);
    preview_size(ui, f, size);
    ui.horizontal(|ui| {
        if ui.small_button("−").clicked() {
            zoom = (zoom / 1.25).max(25.0);
        }
        crate::widgets::value_field(ui, &mut zoom, 25.0..=400.0, "%", 70.0);
        if ui.small_button("+").clicked() {
            zoom = (zoom * 1.25).min(400.0);
        }
    });
    f.insert("__zoom".into(), json!(zoom));
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|s| s.doc.clone()) else { return };
    if f.contains_key("__artboards") && f.get("__previewOverview").and_then(Value::as_bool).unwrap_or(true) {
        show_overview(&doc, f);
    }
    ui.horizontal_top(|ui| {
        if f.contains_key("__artboards") {
            ui.vertical(|ui| {
                ui.set_width(190.0);
                ui.label(egui::RichText::new(tl!("Artboards")).font(crate::theme::semibold(12.0)));
                let boards = doc.artboards();
                let mut chosen: Vec<u64> =
                    f.get("__artboards").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                let mut overview_changed = false;
                let mut all = chosen.len() == boards.len();
                if crate::widgets::checkbox(ui, &mut all, tl!("Select All")).changed() {
                    chosen = if all { boards.iter().map(|b| b.0.0).collect() } else { Vec::new() };
                    f.insert("__previewOverview".into(), json!(true));
                    overview_changed = true;
                }
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (id, name, board) in boards.into_iter().rev() {
                        ui.horizontal(|ui| {
                            let mut checked = chosen.contains(&id.0);
                            let checkbox = crate::widgets::checkbox(ui, &mut checked, "");
                            checkbox
                                .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, checked, format!("{}: {name}", tl!("Artboards"))));
                            if checkbox.changed() {
                                f.insert("__previewOverview".into(), json!(true));
                                overview_changed = true;
                                if checked {
                                    chosen.push(id.0);
                                } else {
                                    chosen.retain(|v| *v != id.0);
                                }
                            }
                            let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                            crate::widgets::checker(ui.painter(), r, 4.0);
                            if ui.is_rect_visible(r) {
                                let key = egui::Id::new(("artboard-export-thumb", doc.id.0, app.session.active().map_or(0, |s| s.revision), id.0));
                                let cached: Option<Arc<egui::TextureHandle>> = ui.data(|d| d.get_temp(key));
                                let tex = cached.or_else(|| {
                                    let one = photocraft_engine::artboard_cmds::artboard_document(&doc, id).ok()??;
                                    let img = photocraft_compose::thumbnail(&one, 32);
                                    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
                                    let tex = Arc::new(ui.ctx().load_texture("artboard-export-thumb", color, egui::TextureOptions::LINEAR));
                                    ui.data_mut(|d| d.insert_temp(key, tex.clone()));
                                    Some(tex)
                                });
                                if let Some(tex) = tex {
                                    let size = tex.size_vec2();
                                    let size = size * (32.0 / size.max_elem());
                                    ui.painter().image(
                                        tex.id(),
                                        egui::Rect::from_center_size(r.center(), size),
                                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                        egui::Color32::WHITE,
                                    );
                                }
                            }
                            let active = !f.get("__previewOverview").and_then(Value::as_bool).unwrap_or(true)
                                && f.get("__preview").and_then(Value::as_u64) == Some(id.0);
                            let options = board_options(f, id.0);
                            let fmt = if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) {
                                "PDF".into()
                            } else {
                                s_fmt(&options).to_ascii_uppercase()
                            };
                            if ui.selectable_label(active, format!("{name}\n{fmt}  {} × {} px", board.rect.width(), board.rect.height())).clicked() {
                                if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) {
                                    f.insert("__preview".into(), json!(id.0));
                                    f.insert("__previewOverview".into(), json!(false));
                                } else {
                                    activate_board(f, id.0);
                                }
                                f.insert("__w".into(), json!(board.rect.width()));
                                f.insert("__h".into(), json!(board.rect.height()));
                            }
                        });
                    }
                });
                f.insert("__artboards".into(), json!(chosen));
                if overview_changed {
                    show_overview(&doc, f);
                }
                ui.separator();
                if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) {
                    return;
                }
                ui.label(tl!("Scale All"));
                let mut variants = f.get("__variants").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut remove = None;
                for (i, variant) in variants.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            let mut scale = variant.get("scale").and_then(Value::as_f64).unwrap_or(100.0) as f32;
                            crate::widgets::value_field(ui, &mut scale, 1.0..=1000.0, "%", 50.0);
                            let mut suffix = variant.get("suffix").and_then(Value::as_str).unwrap_or_default().to_string();
                            ui.add_sized([60.0, 20.0], egui::TextEdit::singleline(&mut suffix).hint_text(tl!("Suffix")));
                            *variant = json!({"scale":scale,"suffix":suffix});
                            if ui.small_button("−").clicked() {
                                remove = Some(i);
                            }
                        })
                    });
                }
                if let Some(i) = remove.filter(|_| variants.len() > 1) {
                    variants.remove(i);
                }
                if variants.len() < 32 && ui.small_button("+").clicked() {
                    variants.push(json!({"scale":200,"suffix":"@2x"}));
                }
                f.insert("__variants".into(), json!(variants));
            });
            ui.add_space(12.0);
        }
        // Left: settings.
        ui.vertical(|ui| {
            ui.set_width(220.0);
            let pdf = f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false);
            ui.label(egui::RichText::new(tl!("File Settings")).font(crate::theme::semibold(12.0)).color(t.text));
            if f.contains_key("__artboards")
                && let Some(board) = f.get("__preview").and_then(Value::as_u64).and_then(|id| doc.layer(photocraft_doc::LayerId(id)))
            {
                ui.label(egui::RichText::new(&board.name).color(t.text_dim)).on_hover_text(&board.name);
            }
            if pdf {
                let mut compression = s(f, "compression");
                let opts = vec![("jpeg".into(), "JPEG"), ("zip".into(), "ZIP")];
                crate::widgets::dropdown(ui, "artboard-pdf-compression", &mut compression, &opts, 130.0);
                f.insert("compression".into(), json!(compression));
                let mut quality = n(f, "quality", 10.0) as f32;
                if s(f, "compression") == "jpeg" {
                    crate::widgets::slider_row(ui, tl!("Quality"), &mut quality, 0.0..=12.0, "", None);
                    f.insert("quality".into(), json!(quality.round()));
                }
                let mut separate = f.get("separateFiles").and_then(Value::as_bool).unwrap_or(false);
                crate::widgets::checkbox(ui, &mut separate, tl!("Separate Files"));
                f.insert("separateFiles".into(), json!(separate));
            } else {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Format")).color(t.text_dim));
                    let mut fmt = s_fmt(f);
                    let opts: Vec<(String, &str)> = FORMATS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
                    if crate::widgets::dropdown(ui, "export-format", &mut fmt, &opts, 130.0) {
                        set_format_defaults(f, &fmt, &app.session.prefs().export);
                    }
                });
            }
            let fmt = s_fmt(f);
            if fmt == "webp" {
                let mut ll = lossless(f);
                crate::widgets::checkbox(ui, &mut ll, tl!("Lossless"));
                f.insert("lossless".into(), json!(ll));
            }
            if fmt == "jpg" || (fmt == "webp" && !lossless(f)) {
                let mut q = n(f, "quality", 85.0) as f32;
                crate::widgets::slider_row(ui, tl!("Quality"), &mut q, 1.0..=100.0, "%", None);
                f.insert("quality".into(), json!(q.round()));
            }
            if fmt == "tga" {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Resolution")).color(t.text_dim));
                    let mut bits = s(f, "tgaBits");
                    let opts: Vec<(String, &str)> =
                        vec![("auto".into(), tl!("Auto")), ("24".into(), tl!("24 bits/pixel")), ("32".into(), tl!("32 bits/pixel"))];
                    if crate::widgets::dropdown(ui, "export-tga-bits", &mut bits, &opts, 130.0) {
                        f.insert("tgaBits".into(), json!(bits));
                    }
                });
            }
            if fmt != "jpg" {
                let mut tr = f.get("transparency").and_then(Value::as_bool).unwrap_or(true);
                crate::widgets::checkbox(ui, &mut tr, tl!("Transparency"));
                f.insert("transparency".into(), json!(tr));
            }
            if f.contains_key("__artboards") {
                for (key, label, default) in [
                    ("includeBackground", tl!("Include Background"), true),
                    ("includeOverlapping", tl!("Include Overlapping Areas"), false),
                    ("reverseOrder", tl!("Reverse Order"), false),
                    ("__nameLabel", tl!("Include Artboard Name"), false),
                ] {
                    let mut value = f.get(key).and_then(Value::as_bool).unwrap_or(default);
                    crate::widgets::checkbox(ui, &mut value, label);
                    f.insert(key.into(), json!(value));
                }
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Metadata")).color(t.text_dim));
                let mut m = s(f, "metadata");
                let opts: Vec<(String, &str)> =
                    vec![("none".into(), tl!("None")), ("copyrightAndContact".into(), tl!("Copyright and Contact Info")), ("all".into(), tl!("All"))];
                if crate::widgets::dropdown(ui, "export-metadata", &mut m, &opts, 130.0) {
                    f.insert("metadata".into(), json!(m));
                }
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tl!("Image Size")).font(crate::theme::semibold(12.0)).color(t.text));
            let mut sc = n(f, "scale", 100.0) as f32;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Scale")).color(t.text_dim));
                if crate::widgets::value_field(ui, &mut sc, 1.0..=1000.0, "%", 70.0).changed() {
                    f.remove("width");
                    f.remove("height");
                }
            });
            f.insert("scale".into(), json!(sc.round()));
            let (bw, bh) = (n(f, "__w", f64::from(doc.size.width)), n(f, "__h", f64::from(doc.size.height)));
            let (w, h) = scaled_size(bw, bh, f64::from(sc) / 100.0);
            if f.contains_key("__artboards") {
                let mut width = n(f, "width", w) as f32;
                let mut height = n(f, "height", h) as f32;
                ui.horizontal(|ui| {
                    ui.label(tl!("Width"));
                    if crate::widgets::value_field(ui, &mut width, 1.0..=300000.0, "px", 70.0).changed() {
                        f.insert("width".into(), json!(width.round()));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tl!("Height"));
                    if crate::widgets::value_field(ui, &mut height, 1.0..=300000.0, "px", 70.0).changed() {
                        f.insert("height".into(), json!(height.round()));
                    }
                });
                let mut resample = f.get("resample").and_then(Value::as_str).unwrap_or("bicubic").to_string();
                let opts = vec![
                    ("bicubic".into(), tl!("Bicubic")),
                    ("bilinear".into(), tl!("Bilinear")),
                    ("nearest".into(), tl!("Nearest Neighbor")),
                    ("lanczos".into(), "Lanczos"),
                ];
                crate::widgets::dropdown(ui, "artboard-export-resample", &mut resample, &opts, 150.0);
                f.insert("resample".into(), json!(resample));
                let mut smaller = f.get("smaller8Bit").and_then(Value::as_bool).unwrap_or(false);
                if fmt == "png" && !pdf {
                    crate::widgets::checkbox(ui, &mut smaller, tl!("Smaller File (8-bit)"));
                    f.insert("smaller8Bit".into(), json!(smaller));
                }
                let mut custom = f.get("customCanvas").and_then(Value::as_bool).unwrap_or(false);
                crate::widgets::checkbox(ui, &mut custom, tl!("Canvas Size"));
                f.insert("customCanvas".into(), json!(custom));
                if custom {
                    let mut cw = n(f, "canvasWidth", f64::from(width)) as f32;
                    let mut ch = n(f, "canvasHeight", f64::from(height)) as f32;
                    ui.horizontal(|ui| {
                        crate::widgets::value_field(ui, &mut cw, 1.0..=300000.0, "px", 70.0);
                        crate::widgets::value_field(ui, &mut ch, 1.0..=300000.0, "px", 70.0);
                    });
                    f.insert("canvasWidth".into(), json!(cw.round()));
                    f.insert("canvasHeight".into(), json!(ch.round()));
                    if ui.small_button(tl!("Reset")).clicked() {
                        f.remove("canvasWidth");
                        f.remove("canvasHeight");
                        f.insert("customCanvas".into(), json!(false));
                    }
                } else {
                    f.remove("canvasWidth");
                    f.remove("canvasHeight");
                }
            }
            ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text_dim).size(11.5));
            let mut srgb = f.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(true);
            crate::widgets::checkbox(ui, &mut srgb, tl!("Convert to sRGB"));
            f.insert("convertToSrgb".into(), json!(srgb));
            let mut icc = f.get("embedIcc").and_then(Value::as_bool).unwrap_or(true);
            crate::widgets::checkbox(ui, &mut icc, tl!("Embed Color Profile"));
            f.insert("embedIcc".into(), json!(icc));
        });
        ui.add_space(12.0);
        // Right: checked-board overview or the explicitly focused board.
        ui.vertical(|ui| {
            ui.set_width(300.0);
            if f.contains_key("__artboards") {
                ui.horizontal(|ui| {
                    ui.label(tl!("Preview"));
                    let overview = f.get("__previewOverview").and_then(Value::as_bool).unwrap_or(true);
                    if ui.selectable_label(overview, tl!("Artboards")).clicked() {
                        show_overview(&doc, f);
                    }
                });
                let boards = preview_boards(&doc, f);
                if boards.len() > 1 {
                    board_grid(app, ui, &doc, f, &boards);
                } else if let Some(id) = boards.first() {
                    let mut options = preview_fields(f, *id);
                    if let Some(board) = doc.layer(*id) {
                        let label = ui.label(&board.name);
                        label.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{}: {}", tl!("Preview"), board.name)));
                    }
                    single_preview(app, ui, &doc, &mut options);
                    for key in ["__zoom", "__twoUp"] {
                        if let Some(value) = options.get(key) {
                            f.insert(key.into(), value.clone());
                        }
                    }
                } else {
                    ui.label(tl!("Artboards"));
                    ui.label("—");
                }
            } else {
                single_preview(app, ui, &doc, f);
            }
        });
    });
}

/// Export with the dialog's settings: choose a path, render, encode, write.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key("__artboards") {
        return confirm_artboards(app, f);
    }
    let doc = source_document(app, f)?;
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a).to_string();
    let ext = s_fmt(f);
    let suggested = format!("{stem}.{ext}");
    let (f, settings) = (f.clone(), settings(f));
    app.pick_save(&suggested, move |app, path| {
        let out = export_document(&doc, &f, None)?;
        let export = app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&out, &path, &settings)?;
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let template = "Exported {path} ({size})";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("path", &path), ("size", &crate::sizing::human_bytes(bytes.len() as f64))]);
        app.ui.status_error = false;
        let template = "Exported {path}";
        let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(&path))]);
        crate::notices::io_warnings(app, &what, &warnings);
        Ok(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}))
    })
}

fn confirm_artboards(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let source = app.session.active().ok_or("no document")?.doc.clone();
    let mut params = Value::Object(f.clone());
    params["artboards"] = f.get("__artboards").cloned().unwrap_or(json!([]));
    params["variants"] = f.get("__variants").cloned().unwrap_or(json!([{"scale":100,"suffix":""}]));
    params["nameLabel"] = json!({"enabled":f.get("__nameLabel").and_then(Value::as_bool).unwrap_or(false)});
    let ids = photocraft_engine::artboard_export::selected(&source, &params).map_err(|e| e.to_string())?;
    let variants = photocraft_engine::artboard_export::variants(&params).map_err(|e| e.to_string())?;
    if f.get("__artboardPdf").and_then(Value::as_bool).unwrap_or(false) {
        let suggested = format!("{}.pdf", source.name.rsplit_once('.').map_or(source.name.as_str(), |(a, _)| a));
        let doc = source.id;
        return app.pick_save(&suggested, move |app, path| {
            app.refocus(doc)?;
            params["path"] = json!(path);
            app.run("file.export.artboardsToPdf", params)
        });
    }
    photocraft_engine::artboard_export::validate_output_count(ids.len(), variants.len()).map_err(|e| e.to_string())?;
    let prepared: Vec<_> = ids.iter().map(|id| (*id, board_options(f, id.0))).collect();
    let ext = prepared.first().map(|(_, f)| s_fmt(f)).unwrap_or_else(|| s_fmt(f));
    let multiple = ids.len() * variants.len() > 1;
    let name = ids.first().and_then(|id| source.layer(*id)).map(|l| l.name.as_str()).unwrap_or("Artboard");
    let suggested = format!("{}.{}", name.replace(['/', '\\'], "_"), ext);
    app.pick_save(&suggested, move |app, path| {
        let dir = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let mut used =
            if multiple { photocraft_engine::artboard_export::existing_names(&dir).map_err(|e| e.to_string())? } else { std::collections::HashSet::new() };
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        for (id, fields) in &prepared {
            for (scale, suffix) in &variants {
                let mut opts = photocraft_engine::artboard_export::variant_options(&Value::Object(fields.clone()), *scale).map_err(|e| e.to_string())?;
                opts["nameLabel"] = params["nameLabel"].clone();
                let out =
                    photocraft_engine::artboard_export::document(&source, *id, &opts).map_err(|e| format!("{e}; already exported: {}", files.join(", ")))?;
                let ext = s_fmt(fields);
                let target = if multiple {
                    let name = photocraft_engine::artboard_export::unique_name(&format!("{}{suffix}", out.name), &ext, &mut used);
                    std::path::Path::new(&dir).join(name).to_string_lossy().into_owned()
                } else {
                    path.clone()
                };
                let export = app.services.export.as_ref().ok_or("no exporter configured")?;
                let (bytes, w) = export(&out, &target, &settings(fields))?;
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&target, &bytes).map_err(|e| format!("{e}; already exported: {}", files.join(", ")))?;
                files.push(target);
                warnings.extend(w);
            }
        }
        app.ui.status = format!("Exported {} files", files.len());
        app.ui.status_error = false;
        crate::notices::io_warnings(app, "Artboard export", &warnings);
        Ok(json!({"files":files,"warnings":warnings}))
    })
}

/// File › Export › Quick Export as PNG: the format, quality, metadata, colour space and location
/// from File › Export › Export Preferences (engine `file.export.quickExport`). On the web (no
/// file system) it falls back to a PNG download through the export service.
pub fn quick_export_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    if !cfg!(target_arch = "wasm32") {
        let prefs = app.session.prefs().export.clone();
        let fmt = serde_json::to_value(prefs.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into());
        let same = serde_json::to_value(prefs.quick_export_location).ok().is_some_and(|v| v == "sameFolder");
        let saved = app.session.active().is_some_and(|d| d.path.is_some());
        if same && saved {
            return quick_export(app, json!({}));
        }
        let st = app.session.active().ok_or("no document")?;
        let stem = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(a, _)| a);
        let (suggested, doc) = (format!("{stem}.{fmt}"), st.doc.id);
        return app.pick_save(&suggested, move |app, path| {
            app.refocus(doc)?;
            if app.session.active().is_some_and(|s| s.doc.has_artboards()) {
                let dir = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                quick_export(app, json!({"dir": dir}))
            } else {
                quick_export(app, json!({"path": path}))
            }
        });
    }
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    confirm(app, &f)
}

/// Runs Quick Export with `p` (an explicit `path`, or none for the document's folder).
fn quick_export(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("file.export.quickExport", p)?;
    app.ui.status = match r.get("files").and_then(Value::as_array) {
        Some(files) => format!("Exported {} files", files.len()),
        None => format!("Exported {}", r["path"].as_str().unwrap_or_default()),
    };
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn overview_app(count: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":20,"height":20})).unwrap();
        for i in 0..count {
            app.run("layer.new.artboard", json!({"name":format!("Board {}",i+1),"rect":[i as i32*1000,-(i as i32)*200,20+i,15+i],"background":"white"}))
                .unwrap();
        }
        open(&mut app).unwrap();
        app
    }

    fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut initialized = false;
        let mut h = Harness::builder().with_size(egui::vec2(840.0, 650.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !initialized {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    initialized = true;
                    return;
                }
                let mut fields = app.ui.dialogs.last().unwrap().fields.clone();
                body(app, ui, &mut fields);
                app.ui.dialogs.last_mut().unwrap().fields = fields;
            },
            app,
        );
        h.run_steps(4);
        h
    }

    fn click(h: &mut Harness<'_, PhotocraftApp>, point: egui::Pos2) {
        h.hover_at(point);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
    }

    #[test]
    fn checked_board_overview_uses_two_columns_and_scrolls_odd_final_rows() {
        for count in [1, 2, 3, 5] {
            let mut h = harness(overview_app(count));
            let labels: Vec<_> = (1..=count).rev().map(|i| format!("Preview: Board {i}")).collect();
            let rects: Vec<_> = labels.iter().map(|label| h.get_by_label(label).rect()).collect();
            for (i, r) in rects.iter().enumerate() {
                if i % 2 == 1 {
                    assert!((r.top() - rects[i - 1].top()).abs() < 1.0);
                    assert!(r.left() > rects[i - 1].right(), "distinct columns have space between board names");
                } else if i > 0 {
                    assert!((r.left() - rects[0].left()).abs() < 1.0);
                    assert!(r.top() > rects[i - 2].bottom(), "each pair starts a new row");
                }
            }
            if count == 5 {
                h.get_by_label(labels.last().unwrap()).scroll_to_me();
                h.run_steps(4);
                assert!(h.get_by_label(labels.last().unwrap()).rect().top() < rects.last().unwrap().top());
            }
        }
    }

    #[test]
    fn row_focus_and_checkbox_overview_keep_export_checks_independent() {
        let mut h = harness(overview_app(3));
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        let checked = fields["__artboards"].clone();
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(preview_boards(doc, fields).len(), 3);
        let board = doc.artboards().into_iter().find(|b| b.1 == "Board 2").unwrap();
        let row_label = format!("{}\nPNG  {} × {} px", board.1, board.2.rect.width(), board.2.rect.height());
        let focus = board.0;
        let row = h.get_by_label(&row_label).rect().center();
        click(&mut h, row);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__artboards"], checked);
        assert_eq!(preview_boards(&h.state().session.active().unwrap().doc, fields), [focus]);
        let checkbox = h.get_by_label("Artboards: Board 2").rect().center();
        click(&mut h, checkbox);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_ne!(fields["__preview"], focus.0);
        assert_eq!(preview_boards(&h.state().session.active().unwrap().doc, fields).len(), 2);
        let checkbox = h.get_by_label("Artboards: Board 1").rect().center();
        click(&mut h, checkbox);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        let shown = preview_boards(&h.state().session.active().unwrap().doc, fields);
        assert_eq!(shown.len(), 1);
        assert_eq!(h.state().session.active().unwrap().doc.layer(shown[0]).unwrap().name, "Board 3");
        let all = h.get_by_label("Select All").rect().center();
        click(&mut h, all);
        assert_eq!(preview_boards(&h.state().session.active().unwrap().doc, &h.state().ui.dialogs.last().unwrap().fields).len(), 3);
        click(&mut h, all);
        assert!(preview_boards(&h.state().session.active().unwrap().doc, &h.state().ui.dialogs.last().unwrap().fields).is_empty());
    }

    #[test]
    fn sole_checked_board_owns_settings_edits_and_explicit_row_focus_remains_independent() {
        let mut h = harness(overview_app(2));
        let ids = |h: &Harness<'_, PhotocraftApp>, name: &str| h.state().session.active().unwrap().doc.artboards().into_iter().find(|b| b.1 == name).unwrap().0;
        let a = ids(&h, "Board 1");
        let b = ids(&h, "Board 2");
        let row = h.get_by_label("Board 1\nPNG  20 × 15 px").rect().center();
        click(&mut h, row);
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        fields.insert("format".into(), json!("jpg"));
        fields.insert("width".into(), json!(7));
        fields.insert("height".into(), json!(9));
        h.run_steps(2);
        let checkbox = h.get_by_label("Artboards: Board 1").rect().center();
        click(&mut h, checkbox);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__preview"], b.0);
        assert_eq!((fields["__w"].as_u64(), fields["__h"].as_u64()), (Some(21), Some(16)));
        assert_eq!(fields["format"], "png");
        assert!(!fields.contains_key("width"));
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        fields.insert("format".into(), json!("webp"));
        fields.insert("width".into(), json!(11));
        fields.insert("height".into(), json!(8));
        h.run_steps(2);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        let options = board_options(fields, b.0);
        assert_eq!(options["format"], "webp");
        let source = source_document(h.state(), &preview_fields(fields, b)).unwrap();
        assert_eq!(export_document(&source, &options, None).unwrap().size, photocraft_doc::Size::new(11, 8));
        assert_eq!(board_options(fields, a.0)["format"], "jpg");
        assert_eq!(board_options(fields, a.0)["width"], 7);
        let row = h.get_by_label("Board 1\nJPG  20 × 15 px").rect().center();
        click(&mut h, row);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__artboards"], json!([b.0]));
        assert_eq!(preview_boards(&h.state().session.active().unwrap().doc, fields), [a]);
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__artboards".into(), json!([]));
        click(&mut h, row);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__artboards"], json!([]));
        assert_eq!(preview_boards(&h.state().session.active().unwrap().doc, fields), [a]);
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__artboards".into(), json!([b.0]));
        // Automation can update overview fields directly: the next render normalizes settings.
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__previewOverview".into(), json!(true));
        h.run_steps(2);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__preview"], b.0);
        assert_eq!(fields["format"], "webp");
        assert_eq!(fields["width"], 11);
        assert_eq!(fields["__w"], 21);
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__artboards".into(), json!([a.0]));
        h.run_steps(2);
        assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__preview"], a.0);
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__artboards".into(), json!([b.0]));
        h.run_steps(2);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__preview"], b.0);
        assert_eq!(fields["format"], "webp");
        assert_eq!(fields["width"], 11);
    }

    #[test]
    fn failed_grid_previews_keep_visible_and_offscreen_tile_geometry() {
        let mut good = harness(overview_app(5));
        let labels: Vec<_> = (1..=5).rev().map(|i| format!("Preview: Board {i}")).collect();
        let expected: Vec<_> = labels.iter().map(|label| good.get_by_label(label).rect()).collect();
        let mut app = overview_app(5);
        app.session
            .edit("large empty preview fixtures", |doc, _| {
                for layer in &mut doc.layers {
                    if let Some(board) = layer.artboard_mut() {
                        board.rect = photocraft_geom::Rect::from_xywh(board.rect.x0, board.rect.y0, 20000, 20000);
                    }
                }
                Ok(())
            })
            .unwrap();
        assert!(source_document(&app, &app.ui.dialogs.last().unwrap().fields).is_err());
        let mut failed = harness(app);
        let actual: Vec<_> = labels.iter().map(|label| failed.get_by_label(label).rect()).collect();
        // Larger dimensions can widen the board list, translating the whole preview column.
        // Compare tile geometry within that column rather than its absolute window position.
        for ((label, expected_rect), actual_rect) in labels.iter().zip(&expected).zip(&actual) {
            let expected_offset = expected_rect.min - expected[0].min;
            let actual_offset = actual_rect.min - actual[0].min;
            assert!(
                (actual_offset - expected_offset).length() < 1.0,
                "failed tiles retain their row and column offsets: {label}, expected {expected_rect:?}, actual {actual_rect:?}"
            );
            assert!((actual_rect.size() - expected_rect.size()).length() < 1.0);
        }
        good.get_by_label(labels.last().unwrap()).scroll_to_me();
        failed.get_by_label(labels.last().unwrap()).scroll_to_me();
        good.run_steps(4);
        failed.run_steps(4);
        let expected_scroll = expected.last().unwrap().top() - good.get_by_label(labels.last().unwrap()).rect().top();
        let actual_scroll = actual.last().unwrap().top() - failed.get_by_label(labels.last().unwrap()).rect().top();
        assert!(expected_scroll > 1.0 && actual_scroll > 1.0, "both final rows scroll into view");
        assert!((actual_scroll - expected_scroll).abs() < 1.0, "visible error tiles preserve the offscreen scroll geometry");
    }

    #[test]
    fn board_overview_respects_each_boards_options_and_reuses_preview_cache() {
        let (mut app, first, second) = board_app();
        let id = open(&mut app).unwrap();
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        fields.insert("scale".into(), json!(50));
        activate_board(fields, first);
        fields.insert("width".into(), json!(200));
        fields.insert("height".into(), json!(100));
        fields.insert("format".into(), json!("webp"));
        fields.insert("__previewOverview".into(), json!(true));
        let fields = fields.clone();
        let doc = app.session.active().unwrap().doc.clone();
        let mut textures = Vec::new();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                for (board, expected) in [(first, photocraft_doc::Size::new(200, 100)), (second, photocraft_doc::Size::new(173, 180))] {
                    let options = preview_fields(&fields, photocraft_doc::LayerId(board));
                    let source = source_document(&app, &options).unwrap();
                    let out = export_document(&source, &options, None).unwrap();
                    assert_eq!(out.size, expected);
                    let (_, texture) = cached_preview(&app, ui, &doc, &options).unwrap();
                    textures.push(texture);
                }
            });
            output.textures_delta.clear();
        }
        assert!(Arc::ptr_eq(&textures[0], &textures[2]));
        assert!(Arc::ptr_eq(&textures[1], &textures[3]));
        assert_ne!(textures[0].id(), textures[1].id());
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(fields["__artboards"], json!([second, first]));
    }

    fn board_app() -> (PhotocraftApp, u64, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":100,"height":100})).unwrap();
        let first = app.run("layer.new.artboard", json!({"name":"White","width":426,"height":302})).unwrap()["layer"].as_u64().unwrap();
        let second =
            app.run("layer.new.artboard", json!({"name":"Empty","rect":[334,-131,345,360],"background":"transparent"})).unwrap()["layer"].as_u64().unwrap();
        (app, first, second)
    }

    #[test]
    fn board_preview_does_not_change_checks_or_other_board_settings() {
        let (mut app, first, second) = board_app();
        let id = open(&mut app).unwrap();
        assert_eq!(open(&mut app).unwrap(), id);
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(fields["__artboards"], json!([second, first]));
        assert_eq!(fields["__w"], 345);
        assert_eq!(fields["__h"], 360);
        fields.insert("scale".into(), json!(50));
        fields.insert("format".into(), json!("webp"));
        let checked = fields["__artboards"].clone();
        activate_board(fields, first);
        assert_eq!(fields["format"], "png");
        assert_eq!(fields["scale"], 100);
        fields.insert("width".into(), json!(200));
        activate_board(fields, second);
        assert_eq!(fields["format"], "webp");
        assert_eq!(fields["scale"], 50);
        assert!(!fields.contains_key("width"));
        assert_eq!(fields["__artboards"], checked);
        assert_eq!(board_options(fields, first)["width"], 200);
    }

    #[test]
    fn selected_empty_board_exports_one_full_size_png_through_platform_services() {
        use std::sync::Mutex;
        let (mut app, _, second) = board_app();
        let outputs = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let captured = outputs.clone();
        app.services.file_dialog = Some(Box::new(|_, _, reply| reply.send(Some(crate::FileDialogAnswer::SaveTo("empty.png".into())))));
        app.services.export =
            Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(move |path, bytes| {
            captured.lock().unwrap().push((path.into(), bytes.into()));
            Ok(())
        }));
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__artboards".into(), json!([second]));
        let before = (*app.session.active().unwrap().doc).clone();
        confirm(&mut app, &fields).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        let outputs = outputs.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        let decoded = photocraft_io::import("empty.png", &outputs[0].1).unwrap().document;
        assert_eq!(decoded.size, photocraft_doc::Size::new(345, 360));
        assert!(photocraft_compose::flatten(&decoded).px.iter().all(|q| q[3] == 0.0));
        assert_eq!(*app.session.active().unwrap().doc, before);
    }

    #[test]
    fn too_many_outputs_does_not_open_picker() {
        let (mut app, _, _) = board_app();
        app.session
            .edit("Export fixtures", |doc, _| {
                let board = doc.layers.iter().find(|l| l.artboard().is_some()).unwrap().clone();
                for _ in 0..32 {
                    let next = board.duplicate();
                    doc.layers.push(next);
                }
                Ok(())
            })
            .unwrap();
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__variants".into(), json!((0..32).map(|_| json!({"scale":100})).collect::<Vec<_>>()));
        assert!(confirm(&mut app, &fields).is_err());
        assert!(!app.file_dialog_open());
    }

    #[test]
    fn no_checked_boards_does_not_open_picker_or_export() {
        let (mut app, _, _) = board_app();
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__artboards".into(), json!([]));
        assert!(confirm(&mut app, &fields).is_err());
        assert!(!app.file_dialog_open());
    }

    #[test]
    fn export_document_scales_height_when_rounded_width_is_unchanged() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};

        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (width, height, scale, expected) in [
                (1, 100, 50, (1, 50)),
                (1, 200, 50, (1, 100)),
                (10, 100, 99, (10, 99)),
                (10, 100, 101, (10, 101)),
                (400, 300, 50, (200, 150)),
                (400, 300, 100, (400, 300)),
            ] {
                let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, depth, Color::WHITE);
                let before = doc.clone();
                let mut f = Map::new();
                f.insert("format".into(), json!("png"));
                f.insert("scale".into(), json!(scale));
                let out = export_document(&doc, &f, None).unwrap();
                assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, {depth:?}");
                assert_eq!(out.depth, depth, "export preserves the document's depth");
                assert_eq!(doc, before, "export leaves the source document unchanged");
            }
        }
    }

    #[test]
    fn export_document_proxy_limits_both_dimensions_before_rounding() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};

        for (width, height, scale, max_side, expected) in
            [(1, 1000, 100, 360, (1, 360)), (1, 1000, 50, 360, (1, 360)), (1, 1000, 25, 360, (1, 250)), (13, 20, 50, 8, (5, 8)), (20, 13, 50, 8, (8, 5))]
        {
            let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, SampleType::U8, Color::WHITE);
            let before = doc.clone();
            let mut f = Map::new();
            f.insert("format".into(), json!("png"));
            f.insert("scale".into(), json!(scale));
            let out = export_document(&doc, &f, Some(max_side)).unwrap();
            assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, proxy limit {max_side}");
            assert!(out.size.width.max(out.size.height) <= max_side, "proxy respects its longest-side limit");
            assert_eq!(doc, before, "preview leaves the source document unchanged");
        }
    }

    #[test]
    fn export_document_scales_and_flattens() {
        let doc = Document::with_background(
            "x",
            photocraft_doc::Size::new(200, 100),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let mut f = Map::new();
        f.insert("format".into(), json!("jpg"));
        f.insert("scale".into(), json!(50));
        let out = export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (100, 50));
        assert_eq!(out.layers.len(), 1);
        let proxy = export_document(&doc, &f, Some(40)).unwrap();
        assert_eq!(proxy.size.width, 40);
        assert_eq!(settings(&f).jpeg_quality, Some(85));
    }

    /// #1353: a scale that changes only the height (the width rounds back to the original)
    /// used to be skipped, so the exported file kept the original size while the dialog
    /// labelled it as scaled.
    #[test]
    fn export_document_scales_when_only_the_height_changes() {
        let mut f = Map::new();
        f.insert("format".into(), json!("png"));
        f.insert("scale".into(), json!(50));
        for (src_h, want_h) in [(100u32, 50u32), (200, 100)] {
            let doc = Document::with_background(
                "x",
                photocraft_doc::Size::new(1, src_h),
                photocraft_doc::ColorMode::Rgb,
                photocraft_doc::SampleType::U8,
                photocraft_doc::Color::WHITE,
            );
            let out = export_document(&doc, &f, None).unwrap();
            assert_eq!((out.size.width, out.size.height), (1, want_h), "1 × {src_h} at 50%");
            // The preview proxy takes the same path.
            let proxy = export_document(&doc, &f, Some(40)).unwrap();
            assert_eq!((proxy.size.width, proxy.size.height), (1, 40), "proxy of 1 × {src_h}");
        }
        // The label and the export agree, and neither side drops below 1 px.
        assert_eq!(scaled_size(1.0, 100.0, 0.5), (1.0, 50.0));
        assert_eq!(scaled_size(10.0, 300.0, 0.01), (1.0, 3.0));
    }

    #[test]
    fn switching_export_formats_uses_independent_quality_preferences() {
        let prefs = photocraft_engine::prefs::Export { jpeg_quality: 43, webp_quality: 72, webp_lossless: false, ..Default::default() };
        let mut fields = Map::new();
        set_format_defaults(&mut fields, "webp", &prefs);
        assert_eq!(fields["quality"], 72);
        assert_eq!(fields["lossless"], false);
        let s = settings(&fields);
        assert!(!s.webp_lossless);
        assert_eq!(s.webp_quality, Some(72));
        assert_eq!(s.jpeg_quality, None);

        set_format_defaults(&mut fields, "jpg", &prefs);
        assert_eq!(fields["quality"], 43);
        assert_eq!(settings(&fields).jpeg_quality, Some(43));
        assert_eq!(settings(&fields).webp_quality, None);

        set_format_defaults(&mut fields, "webp", &photocraft_engine::prefs::Export::default());
        assert!(settings(&fields).webp_lossless, "default WebP Quick Export mode is lossless");
        assert_eq!(settings(&fields).webp_quality, None);
    }

    #[test]
    fn webp_settings_follow_the_lossless_switch() {
        let mut f = Map::new();
        f.insert("format".into(), json!("webp"));
        f.insert("quality".into(), json!(70));
        let s = settings(&f);
        assert!(!s.webp_lossless, "Export As writes lossy WebP unless asked");
        assert_eq!(s.webp_quality, Some(70));
        assert_eq!(s.jpeg_quality, None);
        f.insert("lossless".into(), json!(true));
        let s = settings(&f);
        assert!(s.webp_lossless);
        assert_eq!(s.webp_quality, None);
        f.insert("format".into(), json!("png"));
        assert!(settings(&f).webp_lossless, "other formats leave the WebP default alone");
    }

    /// TGA's Resolution (bits per pixel): Auto leaves the choice to the export; 24 and 32 force it, and only
    /// for TGA.
    #[test]
    fn tga_settings_follow_the_bits_per_pixel() {
        use photocraft_io::TgaBits;
        let mut f = Map::new();
        f.insert("format".into(), json!("tga"));
        assert_eq!(settings(&f).tga_bits, None, "no choice made");
        for (v, want) in [("auto", None), ("24", Some(TgaBits::Bits24)), ("32", Some(TgaBits::Bits32)), ("16", None)] {
            f.insert("tgaBits".into(), json!(v));
            assert_eq!(settings(&f).tga_bits, want, "{v}");
        }
        f.insert("format".into(), json!("png"));
        assert_eq!(settings(&f).tga_bits, None, "only TGA has bits per pixel");
    }
}
