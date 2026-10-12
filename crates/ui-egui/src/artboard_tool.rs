//! Artboard tool. Gestures preview a frame and commit one engine command on release.
use egui::{Pos2, Rect, Stroke, StrokeKind};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    PhotocraftApp,
    canvas::{ToolEvent, ViewXform},
    state::{Tool, View},
    theme::Tokens,
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Draw,
    Move { layer: u64 },
    Resize { layer: u64, handle: usize },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gesture {
    #[serde(default = "default_gesture_tool")]
    pub tool: Tool,
    pub document: u64,
    pub revision: u64,
    pub action: Action,
    pub start: [f64; 2],
    pub original: [f64; 4],
    pub rect: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Focus {
    document: photocraft_doc::DocId,
    layer: photocraft_doc::LayerId,
    rect: photocraft_geom::Rect,
    from: [f32; 2],
    prepared: bool,
    started: Option<f64>,
    expected: View,
}

const FOCUS_SECONDS: f64 = 0.3;

fn default_gesture_tool() -> Tool {
    Tool::Artboard
}

fn enabled(app: &PhotocraftApp) -> bool {
    matches!(app.active_tool(), Tool::Artboard | Tool::Move) && app.ui.transform.is_none()
}

fn active(app: &PhotocraftApp) -> Option<(u64, [f64; 4])> {
    let st = app.session.active()?;
    let selected = st.active_layer?;
    let id = if app.active_tool() == Tool::Move { st.doc.layer(selected)?.artboard().map(|_| selected)? } else { st.doc.artboard_of(selected)? };
    let layer = st.doc.layer(id)?;
    if !layer.visible {
        return None;
    }
    let a = layer.artboard()?;
    Some((id.0, [a.rect.x0 as f64, a.rect.y0 as f64, a.rect.x1 as f64, a.rect.y1 as f64]))
}

fn handles(r: [f64; 4]) -> [[f64; 2]; 8] {
    let [l, t, r, b] = r;
    let (x, y) = ((l + r) / 2.0, (t + b) / 2.0);
    [[l, t], [x, t], [r, t], [r, y], [r, b], [x, b], [l, b], [l, y]]
}

fn title_rect(xf: &ViewXform, rect: photocraft_geom::Rect) -> Rect {
    let screen = xf.doc_rect(rect);
    Rect::from_min_max(screen.left_top() - egui::vec2(0.0, 24.0), screen.left_top() + egui::vec2(screen.width().max(80.0), 0.0))
}

pub(crate) fn frame_at(app: &PhotocraftApp, point: [f64; 2]) -> Option<photocraft_doc::LayerId> {
    if app.active_tool() != Tool::Move || !enabled(app) {
        return None;
    }
    let xf = ViewXform::active(app)?;
    let p = xf.to_screen(point[0] as f32, point[1] as f32);
    app.session.active().and_then(|st| {
        st.doc.artboards().into_iter().rev().find_map(|(id, _, a)| {
            if st.doc.layer(id).is_none_or(|l| !l.visible) {
                return None;
            }
            let frame = xf.doc_rect(a.rect);
            let title = !app.ui.hide_artboard_names && title_rect(&xf, a.rect).contains(p);
            (title || (frame.expand(4.0).contains(p) && !frame.shrink(4.0).contains(p))).then_some(id)
        })
    })
}

/// The Move tool selects a board by its name or frame; its interior keeps normal layer picking.
pub(crate) fn select_frame_at(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(id) = frame_at(app, point) else { return false };
    run(app, "layer.select", json!({"layer":id.0}));
    true
}

fn plus(xf: &ViewXform, r: [f64; 4]) -> [(&'static str, Pos2); 4] {
    let h = handles(r);
    let center = xf.to_screen(((r[0] + r[2]) / 2.0) as f32, ((r[1] + r[3]) / 2.0) as f32);
    [("up", h[1]), ("right", h[3]), ("down", h[5]), ("left", h[7])].map(|(dir, p)| {
        let p = xf.to_screen(p[0] as f32, p[1] as f32);
        let v = p - center;
        let offset = if v.length() > 0.01 { v.normalized() * 36.0 } else { egui::vec2(0.0, -36.0) };
        (dir, p + offset)
    })
}

/// The immediate neighbour, before the engine's collision avoidance slides it farther out.
fn immediate_neighbour(r: [f64; 4], direction: &str) -> Option<photocraft_geom::Rect> {
    if !r.iter().all(|v| v.is_finite() && *v >= f64::from(i32::MIN) && *v <= f64::from(i32::MAX)) {
        return None;
    }
    let [l, t, r, b] = r.map(|v| v.round() as i64);
    let (w, h) = (r - l, b - t);
    if !(1..=300_000).contains(&w) || !(1..=300_000).contains(&h) {
        return None;
    }
    // Same document-pixel gap as engine::artboard_cmds::GAP; independent of view zoom.
    let gap = 100;
    let (x, y) = match direction {
        "left" => (l - w - gap, t),
        "right" => (r + gap, t),
        "up" => (l, t - h - gap),
        "down" => (l, b + gap),
        _ => return None,
    };
    Some(photocraft_geom::Rect::new(i32::try_from(x).ok()?, i32::try_from(y).ok()?, i32::try_from(x + w).ok()?, i32::try_from(y + h).ok()?))
}

fn available_pluses(app: &PhotocraftApp, xf: &ViewXform, r: [f64; 4]) -> Vec<(&'static str, Pos2)> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let boards = st.doc.artboards();
    let source = active(app).map(|(id, _)| id);
    if !xf.zoom.is_finite() || xf.zoom <= 0.0 {
        return Vec::new();
    }
    plus(xf, r)
        .into_iter()
        .filter(|(direction, p)| {
            let point = xf.to_doc(*p);
            let radius = 10.0 / f64::from(xf.zoom);
            immediate_neighbour(r, direction).is_some_and(|candidate| {
                boards.iter().all(|(id, _, a)| {
                    if !candidate.intersect(&a.rect).is_empty() {
                        return false;
                    }
                    if a.rect.is_empty() || source == Some(id.0) || st.doc.layer(*id).is_none_or(|l| !l.visible) {
                        return true;
                    }
                    let closest = [point[0].clamp(f64::from(a.rect.x0), f64::from(a.rect.x1)), point[1].clamp(f64::from(a.rect.y0), f64::from(a.rect.y1))];
                    (point[0] - closest[0]).hypot(point[1] - closest[1]) > radius
                })
            })
        })
        .collect()
}

pub(crate) fn creation_snapshot(
    app: &PhotocraftApp,
    command: &str,
    params: &serde_json::Value,
) -> Option<(photocraft_doc::DocId, Vec<photocraft_doc::LayerId>)> {
    if !matches!(
        command,
        "layer.new.artboard"
            | "layer.new.artboardFromGroup"
            | "layer.new.artboardFromLayers"
            | "layer.duplicate"
            | "layer.new.layerViaCopy"
            | "layer.new.layerViaCut"
    ) || params.get("inPlace").and_then(serde_json::Value::as_bool) == Some(true)
    {
        return None;
    }
    let st = app.session.active()?;
    Some((st.doc.id, st.doc.artboards().into_iter().map(|(id, _, _)| id).collect()))
}

/// Only new, selected boards pan the camera; history and the existing zoom stay untouched.
pub(crate) fn focus_created_board(app: &mut PhotocraftApp, before: &(photocraft_doc::DocId, Vec<photocraft_doc::LayerId>)) {
    let Some(st) = app.session.active() else { return };
    let Some(id) = st.active_layer else { return };
    if st.doc.id != before.0 || before.1.contains(&id) {
        return;
    }
    let Some(board) = st.doc.layer(id).and_then(photocraft_doc::Layer::artboard) else { return };
    if board.rect.is_empty() {
        return;
    }
    let rect = board.rect;
    let size = [st.doc.size.width, st.doc.size.height];
    let Some(view) = app.session.active_index().and_then(|idx| app.ui.views.get_mut(idx)) else { return };
    if !view.center.iter().all(|v| v.is_finite()) || !view.zoom.is_finite() || view.zoom <= 0.0 || !view.rotation.is_finite() {
        app.ui.artboard_focus = None;
        return;
    }
    view.doc_size = size;
    view.fit_pending = false;
    view.fill_pending = false;
    app.ui.artboard_focus = Some(Focus { document: before.0, layer: id, rect, from: view.center, prepared: false, started: None, expected: view.clone() });
}

pub(crate) fn cancel_stale_focus(app: &mut PhotocraftApp) {
    if app.ui.artboard_focus.as_ref().is_some_and(|focus| {
        app.session.active().is_none_or(|st| {
            st.doc.id != focus.document || st.doc.layer(focus.layer).is_none_or(|l| !l.visible || l.artboard().is_none_or(|a| a.rect != focus.rect))
        })
    }) {
        app.ui.artboard_focus = None;
    }
}

/// Only an image prepared for this pending focus can arm its next frame's animation clock.
pub(crate) fn focus_image_prepared(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, view: &View) {
    if let Some(focus) = app.ui.artboard_focus.as_mut()
        && focus.document == doc.id
        && focus.expected == *view
        && doc.layer(focus.layer).is_some_and(|layer| layer.visible && layer.artboard().is_some_and(|board| board.rect == focus.rect))
    {
        focus.prepared = true;
    }
}

/// Runs before pointer coordinates are mapped, so navigation can interrupt at the shown position.
pub(crate) fn advance_focus(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, doc: &photocraft_doc::Document, view: &mut View, canvas: Rect) -> bool {
    cancel_stale_focus(app);
    if app.session.active_index() != Some(idx) {
        return false;
    }
    let Some(mut focus) = app.ui.artboard_focus.take() else { return false };
    let interrupted = ctx.input(|i| {
        i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pos, pressed: true, .. } if canvas.contains(*pos)))
            || (i.pointer.press_origin().is_some_and(|p| canvas.contains(p))
                && (i.pointer.primary_down() || i.pointer.middle_down() || i.pointer.secondary_down())
                && (i.pointer.delta() != egui::Vec2::ZERO
                    || i.pointer.middle_down()
                    || (i.pointer.primary_down() && (i.key_down(egui::Key::Space) || matches!(app.ui.tool, Tool::Hand | Tool::Zoom | Tool::RotateView)))))
            || (i.pointer.hover_pos().is_some_and(|p| canvas.contains(p))
                && (i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))
                    || i.smooth_scroll_delta != egui::Vec2::ZERO
                    || i.zoom_delta() != 1.0
                    || i.rotation_delta() != 0.0))
    });
    let now = ctx.input(|i| i.time);
    if interrupted || *view != focus.expected || doc.id != focus.document || !now.is_finite() {
        return false;
    }
    if focus.started.is_some_and(|started| now < started) || !canvas.is_finite() || canvas.width() <= 0.0 || canvas.height() <= 0.0 {
        return false;
    }
    // Copying a large board may refresh the whole image. Keep the shown camera until that
    // preparation frame finishes; its cold render must not consume the 300 ms pan.
    if !focus.prepared {
        app.ui.artboard_focus = Some(focus);
        ctx.request_repaint();
        return true;
    }
    let started = *focus.started.get_or_insert(now);
    let mut target = view.clone();
    target.center =
        [((f64::from(focus.rect.x0) + f64::from(focus.rect.x1)) / 2.0) as f32, ((f64::from(focus.rect.y0) + f64::from(focus.rect.y1)) / 2.0) as f32];
    if !app.session.prefs().tools.overscroll {
        let bounds = crate::scrollbars::document_bounds(doc);
        crate::scrollbars::clamp_view_in_bounds(&mut target, canvas.size(), ctx.pixels_per_point(), bounds);
    }
    let elapsed = now - started;
    let t = if elapsed >= FOCUS_SECONDS - 1e-9 { 1.0 } else { (elapsed / FOCUS_SECONDS).clamp(0.0, 1.0) as f32 };
    let eased = t * t * (3.0 - 2.0 * t);
    view.center = if t == 1.0 { target.center } else { std::array::from_fn(|axis| focus.from[axis] + (target.center[axis] - focus.from[axis]) * eased) };
    if let Some(current) = app.ui.views.get_mut(idx) {
        *current = view.clone();
    }
    if t < 1.0 {
        focus.expected = view.clone();
        app.ui.artboard_focus = Some(focus);
        ctx.request_repaint();
    }
    t < 1.0
}

fn run(app: &mut PhotocraftApp, id: &str, p: serde_json::Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    let tool = app.active_tool();
    if !enabled(app) || app.ui.artboard_gesture.as_ref().is_some_and(|g| g.tool != tool) {
        if app.ui.artboard_gesture.is_some() {
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
        }
        app.ui.artboard_gesture = None;
        app.artboard_preview = None;
        return false;
    }
    let Some(doc_id) = app.session.active().map(|st| st.doc.id.0) else { return tool == Tool::Artboard };
    if tool == Tool::Move && !matches!(ev, ToolEvent::Down { .. }) && app.ui.artboard_gesture.is_none() {
        return false;
    }
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if !x.is_finite() || !y.is_finite() {
                return true;
            }
            let xf = ViewXform::active(app);
            let mut action = Action::Draw;
            let mut original = [x, y, x, y];
            if let Some((id, r)) = active(app)
                && let Some(xf) = xf
            {
                let p = xf.to_screen(x as f32, y as f32);
                if let Some((dir, _)) = available_pluses(app, &xf, r).into_iter().find(|(_, c)| c.distance(p) <= 13.0) {
                    run(app, "layer.new.artboard", json!({"adjacentTo":id,"direction":dir,"duplicate":mods.alt}));
                    return true;
                }
                if let Some(i) = handles(r).iter().position(|h| xf.to_screen(h[0] as f32, h[1] as f32).distance(p) <= 7.0) {
                    action = Action::Resize { layer: id, handle: i };
                    original = r;
                }
            }
            if tool == Tool::Move && action == Action::Draw {
                return false;
            }
            if action == Action::Draw {
                let picked = app.session.active().and_then(|st| {
                    st.doc.artboards().into_iter().rev().find_map(|(id, _, a)| {
                        if st.doc.layer(id).is_none_or(|l| !l.visible) {
                            return None;
                        }
                        let r = a.rect;
                        let title = xf.is_some_and(|xf| {
                            let p = xf.to_screen(x as f32, y as f32);
                            !app.ui.hide_artboard_names && title_rect(&xf, r).contains(p)
                        });
                        if title || (x >= r.x0 as f64 && x < r.x1 as f64 && y >= r.y0 as f64 && y < r.y1 as f64) {
                            Some((id.0, [r.x0 as f64, r.y0 as f64, r.x1 as f64, r.y1 as f64]))
                        } else {
                            None
                        }
                    })
                });
                if let Some((id, r)) = picked {
                    run(app, "layer.select", json!({"layer":id}));
                    action = Action::Move { layer: id };
                    original = r;
                }
            }
            if let Action::Move { layer } | Action::Resize { layer, .. } = action {
                let locked = app.session.active().is_some_and(|st| {
                    let l = st.doc.effective_locks(photocraft_doc::LayerId(layer));
                    l.position || l.all
                });
                if locked {
                    app.ui.status = tl!("The artboard position is locked.").to_string();
                    app.ui.status_error = true;
                    return true;
                }
            }
            let revision = app.session.active().map_or(0, |st| st.revision);
            app.ui.artboard_gesture = Some(Gesture { tool, document: doc_id, revision, action, start: [x, y], original, rect: original });
            let layer = match action {
                Action::Draw => None,
                Action::Move { layer } | Action::Resize { layer, .. } => Some(photocraft_doc::LayerId(layer)),
            };
            crate::snap_ui::begin_artboard(app, [x, y], matches!(action, Action::Move { .. }).then_some(original), layer, mods);
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(g) = &mut app.ui.artboard_gesture {
                update(g, [x, y], mods.shift);
            }
        }
        ToolEvent::Up { x, y } => {
            app.artboard_preview = None;
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            if let Some(mut g) = app.ui.artboard_gesture.take() {
                if g.document != doc_id || app.session.active().is_none_or(|st| st.revision != g.revision) {
                    return true;
                }
                if !x.is_finite() || !y.is_finite() {
                    return true;
                }
                update(&mut g, [x, y], mods.shift);
                if (x - g.start[0]).abs() + (y - g.start[1]).abs() < 1.0 {
                    return true;
                }
                let [l, t, r, b] = g.rect.map(f64::round);
                if !(1.0..=300_000.0).contains(&(r - l)) || !(1.0..=300_000.0).contains(&(b - t)) {
                    return true;
                }
                let mut p = json!({"x":l,"y":t,"width":r-l,"height":b-t});
                match g.action {
                    Action::Draw => run(app, "layer.new.artboard", p),
                    Action::Move { layer } | Action::Resize { layer, .. } => {
                        p["layer"] = json!(layer);
                        p["moveContents"] = json!(matches!(g.action, Action::Move { .. }));
                        run(app, "layer.artboard.set", p);
                    }
                }
            }
        }
    }
    true
}

pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<egui::CursorIcon> {
    let xf = ViewXform::active(app)?;
    let doc = app.session.active()?;
    let selected = doc.active_layer?;
    let layer = doc.doc.layer(selected)?;
    let board = layer.artboard()?;
    let point = xf.to_screen(p[0] as f32, p[1] as f32);
    let rect = board.rect;
    let index = handles([f64::from(rect.x0), f64::from(rect.y0), f64::from(rect.x1), f64::from(rect.y1)])
        .iter()
        .position(|h| xf.to_screen(h[0] as f32, h[1] as f32).distance(point) <= 7.0)?;
    let (hx, hy) = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)][index];
    Some(match (hx, hy) {
        (0, _) => egui::CursorIcon::ResizeVertical,
        (_, 0) => egui::CursorIcon::ResizeHorizontal,
        _ if hx == hy => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::ResizeNeSw,
    })
}

fn update(g: &mut Gesture, p: [f64; 2], shift: bool) {
    if !p.iter().all(|v| v.is_finite()) {
        return;
    }
    let (mut dx, mut dy) = (p[0] - g.start[0], p[1] - g.start[1]);
    let mut r = g.original;
    match g.action {
        Action::Draw => {
            if shift {
                let n = dx.abs().max(dy.abs());
                dx = n * if dx < 0.0 { -1.0 } else { 1.0 };
                dy = n * if dy < 0.0 { -1.0 } else { 1.0 };
            }
            r = [g.start[0].min(g.start[0] + dx), g.start[1].min(g.start[1] + dy), g.start[0].max(g.start[0] + dx), g.start[1].max(g.start[1] + dy)];
        }
        Action::Move { .. } => {
            if shift {
                if dx.abs() > dy.abs() {
                    dy = 0.0;
                } else {
                    dx = 0.0;
                }
            }
            r = [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy];
        }
        Action::Resize { handle, .. } => {
            if matches!(handle, 0 | 6 | 7) {
                r[0] = (r[0] + dx).min(r[2] - 1.0);
            }
            if matches!(handle, 0..=2) {
                r[1] = (r[1] + dy).min(r[3] - 1.0);
            }
            if matches!(handle, 2..=4) {
                r[2] = (r[2] + dx).max(r[0] + 1.0);
            }
            if matches!(handle, 4..=6) {
                r[3] = (r[3] + dy).max(r[1] + 1.0);
            }
            if shift {
                let (ow, oh) = ((g.original[2] - g.original[0]).max(1.0), (g.original[3] - g.original[1]).max(1.0));
                let ratio = ow / oh;
                let (w, h) = (r[2] - r[0], r[3] - r[1]);
                if matches!(handle, 1 | 5) {
                    let center = (r[0] + r[2]) / 2.0;
                    let w = h * ratio;
                    r[0] = center - w / 2.0;
                    r[2] = center + w / 2.0;
                } else if matches!(handle, 3 | 7) {
                    let center = (r[1] + r[3]) / 2.0;
                    let h = w / ratio;
                    r[1] = center - h / 2.0;
                    r[3] = center + h / 2.0;
                } else if (w - ow).abs() / ow >= (h - oh).abs() / oh {
                    let h = w / ratio;
                    if matches!(handle, 0 | 2) {
                        r[1] = r[3] - h;
                    } else {
                        r[3] = r[1] + h;
                    }
                } else {
                    let w = h * ratio;
                    if matches!(handle, 0 | 6) {
                        r[0] = r[2] - w;
                    } else {
                        r[2] = r[0] + w;
                    }
                }
            }
        }
    }
    g.rect = r;
}

#[derive(Clone)]
pub(crate) struct Preview {
    snapshot: std::sync::Weak<photocraft_doc::Document>,
    gesture: Gesture,
    doc: std::sync::Arc<photocraft_doc::Document>,
    key: u64,
}

/// Cache one pure engine preview per source snapshot and pointer rectangle.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(std::sync::Arc<photocraft_doc::Document>, u64)> {
    if app.ui.artboard_gesture.as_ref().is_some_and(|g| {
        !enabled(app) || g.tool != app.active_tool() || app.session.active().is_none_or(|st| st.doc.id.0 != g.document || st.revision != g.revision)
    }) {
        app.ui.artboard_gesture = None;
        app.artboard_preview = None;
        app.prefs_rt.snap = None;
        app.prefs_rt.snap_lines.clear();
    }
    let g = app.ui.artboard_gesture.as_ref()?;
    if !enabled(app) || app.session.active_index() != Some(idx) {
        return None;
    }
    let (layer, moving) = match g.action {
        Action::Move { layer } => (layer, true),
        Action::Resize { layer, .. } => (layer, false),
        Action::Draw => return None,
    };
    let st = app.session.documents().get(idx)?;
    if st.doc.id.0 != g.document || st.revision != g.revision || g.rect == g.original {
        return None;
    }
    let snapshot = std::sync::Arc::downgrade(&st.doc);
    if let Some(p) = app.artboard_preview.as_ref().filter(|p| p.snapshot.ptr_eq(&snapshot) && p.gesture == *g) {
        return Some((p.doc.clone(), p.key));
    }
    let [l, t, r, b] = g.rect.map(|v| v.round());
    if ![l, t, r, b].iter().all(|v| v.is_finite() && *v >= i32::MIN as f64 && *v <= i32::MAX as f64) {
        return None;
    }
    let doc = std::sync::Arc::new(
        photocraft_engine::artboard_cmds::preview_rect(
            &st.doc,
            photocraft_doc::LayerId(layer),
            photocraft_geom::Rect::new(l as i32, t as i32, r as i32, b as i32),
            moving,
        )
        .ok()?,
    );
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (layer, moving, [l as i32, t as i32, r as i32, b as i32]).hash(&mut h);
    let key = h.finish() | (1 << 62);
    app.artboard_preview = Some(Preview { snapshot, gesture: g.clone(), doc: doc.clone(), key });
    Some((doc, key))
}

pub fn paint(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !enabled(app) {
        return;
    }
    let t = Tokens::get(painter.ctx());
    let r = app.ui.artboard_gesture.as_ref().filter(|g| g.tool == app.active_tool()).map(|g| g.rect).or_else(|| {
        let (id, r) = active(app)?;
        let (dx, dy) =
            app.session.active().and_then(|st| crate::move_ui::shown_offset(app, st.doc.id, st.revision, photocraft_doc::LayerId(id))).unwrap_or((0, 0));
        Some([r[0] + f64::from(dx), r[1] + f64::from(dy), r[2] + f64::from(dx), r[3] + f64::from(dy)])
    });
    let Some(r) = r else { return };
    let pts = handles(r).map(|p| xf.to_screen(p[0] as f32, p[1] as f32));
    for (a, b) in [(0, 2), (2, 4), (4, 6), (6, 0)] {
        painter.line_segment([pts[a], pts[b]], Stroke::new(1.0, t.accent));
    }
    for p in pts {
        painter.rect_filled(Rect::from_center_size(p, egui::vec2(6.0, 6.0)), 0.0, t.text);
        painter.rect_stroke(Rect::from_center_size(p, egui::vec2(6.0, 6.0)), 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    if app.ui.artboard_gesture.is_none() && app.drag.is_none() {
        let ppp = painter.ctx().pixels_per_point();
        let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
        let stroke = 1.0 / ppp;
        for (_, p) in available_pluses(app, xf, r) {
            let p = Pos2::new(((p.x * ppp).floor() + 0.5) / ppp, ((p.y * ppp).floor() + 0.5) / ppp);
            painter.circle_stroke(p, 10.0, Stroke::new(stroke, t.text_dim));
            painter.line_segment([p - egui::vec2(5.0, 0.0), p + egui::vec2(5.0, 0.0)], Stroke::new(stroke, t.text));
            painter.line_segment([p - egui::vec2(0.0, 5.0), p + egui::vec2(0.0, 5.0)], Stroke::new(stroke, t.text));
        }
    }
}

/// Compatibility entry point used by the responsive-artboard canvas integration.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    paint(app, painter, xf);
}

pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::widgets::checkbox(ui, &mut app.ui.hide_artboard_names, tl!("Hide Artboard Names"));
    if let Some(id) = active(app).map(|a| a.0)
        && let Some(l) = app.session.active().and_then(|st| st.doc.layer(photocraft_doc::LayerId(id))).cloned()
    {
        let key = egui::Id::new(("artboard-tool-name", id));
        let mut name =
            ui.data(|d| d.get_temp::<(String, String)>(key)).filter(|(original, _)| *original == l.name).map_or_else(|| l.name.clone(), |(_, edited)| edited);
        let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
        if response.lost_focus() && name != l.name {
            run(app, "layer.rename", json!({"layer":id,"name":name}));
            ui.data_mut(|d| d.remove::<(String, String)>(key));
        } else {
            ui.data_mut(|d| d.insert_temp(key, (l.name.clone(), name)));
        }
        if let Some(a) = l.artboard() {
            let mut preset = a.preset.clone();
            let mut choices = vec![(String::new(), tl!("Custom"))];
            choices.extend(photocraft_engine::artboard_cmds::PRESETS.iter().map(|(n, _, _)| (n.to_string(), *n)));
            if crate::widgets::dropdown(ui, "artboard-tool-preset", &mut preset, &choices, 125.0) && !preset.is_empty() {
                run(app, "layer.artboard.set", json!({"layer":id,"preset":preset}));
            }
            for (icon, portrait) in [("rectangle-vertical", true), ("rectangle-horizontal", false)] {
                if crate::icons::button(
                    ui,
                    icon,
                    24.0,
                    (a.rect.height() > a.rect.width()) == portrait,
                    if portrait { tl!("Portrait") } else { tl!("Landscape") },
                )
                .clicked()
                    && (a.rect.height() > a.rect.width()) != portrait
                {
                    run(app, "layer.artboard.set", json!({"layer":id,"width":a.rect.height(),"height":a.rect.width()}));
                }
            }
            let mut bg = a.background.name().to_string();
            let choices = [
                ("white".to_string(), tl!("White")),
                ("black".to_string(), tl!("Black")),
                ("transparent".to_string(), tl!("Transparent")),
                ("custom".to_string(), tl!("Other…")),
            ];
            if crate::widgets::dropdown(ui, "artboard-tool-background", &mut bg, &choices, 110.0) {
                if bg == "custom" {
                    let rgb = match a.background {
                        photocraft_doc::ArtboardBackground::Custom(c) => c.to_rgb(),
                        _ => [1.0, 1.0, 1.0],
                    };
                    crate::color_picker_ui::open_for_command(
                        app,
                        tl!("Artboard Background Color"),
                        rgb,
                        "layer.artboard.set",
                        json!({"layer":id,"background":"custom"}),
                    );
                } else {
                    run(app, "layer.artboard.set", json!({"layer":id,"background":bg}));
                }
            }
        }
    }
    ui.label(tl!("Drag to create, move or resize; Alt/Option + plus copies contents"));
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Artboard {
        return false;
    }
    options(app, ui);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_hit_matches_drawn_frame_in_rotated_and_flipped_views() {
        let board = photocraft_geom::Rect::new(-30, -20, 170, 80);
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            for flip in [false, true] {
                let xf = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)), zoom: 1.0, center: [70.0, 30.0], flip, rotation };
                let frame = xf.doc_rect(board);
                assert!(title_rect(&xf, board).contains(frame.left_top() + egui::vec2(10.0, -10.0)));
            }
        }
    }
    fn app() -> PhotocraftApp {
        let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        a.run("file.new", json!({"width":100,"height":100,"background":"transparent"})).unwrap();
        a.sync_views();
        a.ui.tool = Tool::Artboard;
        a
    }
    fn move_board() -> (PhotocraftApp, photocraft_doc::LayerId, photocraft_doc::LayerId) {
        let mut a = app();
        a.run("file.new", json!({"width":100,"height":80,"background":"transparent","artboards":true})).unwrap();
        a.sync_views();
        let st = a.session.active().unwrap();
        let board = st.doc.layers[0].id;
        let child = st.doc.layers[0].children().unwrap()[0].id;
        a.run("layer.select", json!({"layer":child.0})).unwrap();
        a.run("select.rect", json!({"x":16,"y":16,"width":8,"height":8})).unwrap();
        a.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        a.run("select.deselect", json!({})).unwrap();
        a.run("layer.select", json!({"layer":board.0})).unwrap();
        a.ui.tool = Tool::Move;
        a.ui.extras.snap = false;
        a.ui.view.show.smart_guides = false;
        a.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 500.0));
        let view = a.ui.views.get_mut(a.session.active_index().unwrap()).unwrap();
        view.zoom = 1.0;
        view.center = [50.0, 40.0];
        view.fit_pending = false;
        (a, board, child)
    }
    fn event(a: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
        crate::canvas::tool_event(a, ev, mods);
    }
    fn controls(a: &PhotocraftApp) -> (usize, usize) {
        let ctx = egui::Context::default();
        let xf = ViewXform::active(a).unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| paint(a, ui.painter(), &xf));
        output.textures_delta.clear();
        let handles = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect.size() == egui::vec2(6.0,6.0) && r.fill != egui::Color32::TRANSPARENT))
            .count();
        let pluses = output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.radius == 10.0)).count();
        (handles, pluses)
    }
    #[test]
    fn plus_strokes_share_the_circle_center_at_each_display_scale() {
        for ppp in [1.0, 1.25, 2.0] {
            let (mut a, _, _) = move_board();
            let idx = a.session.active_index().unwrap();
            a.ui.views[idx].zoom = 1.37;
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            let xf = ViewXform::active(&a).unwrap();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| paint(&a, ui.painter(), &xf));
            output.textures_delta.clear();
            let circles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if c.radius == 10.0 => Some(c),
                    _ => None,
                })
                .collect();
            assert_eq!(circles.len(), 4);
            for circle in circles {
                let lines: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::LineSegment { points, stroke } if (points[0] + (points[1] - points[0]) / 2.0).distance(circle.center) < 0.001 => {
                            Some((points, stroke))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(lines.len(), 2, "one centered horizontal and vertical stroke");
                assert!(lines.iter().any(|(p, _)| p[0].y == p[1].y && (p[1].x - p[0].x - 10.0).abs() < 0.001));
                assert!(lines.iter().any(|(p, _)| p[0].x == p[1].x && (p[1].y - p[0].y - 10.0).abs() < 0.001));
                for (_, stroke) in lines {
                    assert!((stroke.width * ctx.pixels_per_point() - 1.0).abs() < 0.001);
                }
                for coordinate in [circle.center.x, circle.center.y] {
                    assert!(((coordinate * ctx.pixels_per_point()).fract() - 0.5).abs() < 0.001);
                }
            }
            assert!(!output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Text(_))), "plus controls have no font-baseline offset");
        }
    }
    #[test]
    fn occupied_neighbour_rects_hide_and_disable_plus_even_for_hidden_boards() {
        for direction in ["left", "right", "up", "down"] {
            for hidden in [false, true] {
                for origin in [[0, 0], [-500, -300]] {
                    for zoom in [0.5, 1.75] {
                        let (mut a, board, _) = move_board();
                        a.run("layer.artboard.set", json!({"layer":board.0,"x":origin[0],"y":origin[1]})).unwrap();
                        let r = [f64::from(origin[0]), f64::from(origin[1]), f64::from(origin[0] + 100), f64::from(origin[1] + 80)];
                        let candidate = immediate_neighbour(r, direction).unwrap();
                        // Only a distant corner is occupied: checking the button center is insufficient.
                        let obstacle = a.run("layer.new.artboard", json!({"x":candidate.x1-10,"y":candidate.y1-10,"width":20,"height":20})).unwrap()["layer"]
                            .as_u64()
                            .unwrap();
                        if hidden {
                            a.session
                                .edit("hide obstacle", |doc, _| {
                                    doc.layer_mut(photocraft_doc::LayerId(obstacle)).unwrap().visible = false;
                                    Ok(())
                                })
                                .unwrap();
                        }
                        a.run("layer.select", json!({"layer":board.0})).unwrap();
                        let idx = a.session.active_index().unwrap();
                        a.ui.views[idx].center = [(origin[0] + 50) as f32, (origin[1] + 40) as f32];
                        a.ui.views[idx].zoom = zoom;
                        let xf = ViewXform::active(&a).unwrap();
                        assert!(!available_pluses(&a, &xf, r).iter().any(|(d, _)| *d == direction));
                        assert_eq!(controls(&a).1, 3);
                        let point = xf.to_doc(plus(&xf, r).into_iter().find(|(d, _)| *d == direction).unwrap().1);
                        let before = a.session.active().unwrap().doc.clone();
                        let steps = a.session.active().unwrap().history.past_len();
                        for tool in [Tool::Move, Tool::Artboard] {
                            a.ui.tool = tool;
                            event(&mut a, ToolEvent::Down { x: point[0], y: point[1], pressure: 1.0 }, egui::Modifiers::ALT);
                            event(&mut a, ToolEvent::Up { x: point[0], y: point[1] }, egui::Modifiers::ALT);
                            assert_eq!(*a.session.active().unwrap().doc, *before);
                            assert_eq!(a.session.active().unwrap().history.past_len(), steps);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn free_destination_still_hides_a_plus_that_covers_another_board() {
        let (mut a, board, _) = move_board();
        a.run("layer.artboard.set", json!({"layer":board.0,"width":20})).unwrap();
        a.run("layer.new.artboard", json!({"x":145,"y":20,"width":35,"height":40})).unwrap();
        a.run("layer.select", json!({"layer":board.0})).unwrap();
        let idx = a.session.active_index().unwrap();
        a.ui.views[idx].zoom = 0.25;
        a.ui.views[idx].center = [50.0, 40.0];
        let xf = ViewXform::active(&a).unwrap();
        let r = [0.0, 0.0, 20.0, 80.0];
        // At 25%, the button center is x164 although the free destination is x120..140.
        let doc = &a.session.active().unwrap().doc;
        assert!(doc.artboards().iter().all(|(_, _, b)| immediate_neighbour(r, "right").unwrap().intersect(&b.rect).is_empty()));
        assert!(!available_pluses(&a, &xf, r).iter().any(|(d, _)| *d == "right"));
    }
    #[test]
    fn new_board_commands_focus_without_zoom_or_extra_history_and_failures_do_not_pan() {
        for command in ["layer.new.artboard", "layer.duplicate", "layer.new.layerViaCopy"] {
            let (mut a, _, _) = move_board();
            let idx = a.session.active_index().unwrap();
            a.ui.views[idx].zoom = 1.75;
            a.ui.views[idx].center = [-20.0, -30.0];
            a.ui.views[idx].doc_size = [100, 80];
            a.ui.views[idx].fill_pending = true;
            let steps = a.session.active().unwrap().history.past_len();
            a.run(command, json!({})).unwrap();
            let st = a.session.active().unwrap();
            let r = st.doc.layer(st.active_layer.unwrap()).unwrap().artboard().unwrap().rect;
            let view = &a.ui.views[idx];
            assert_eq!(view.center, [-20.0, -30.0], "focus starts at the displayed camera instead of jumping");
            assert_eq!(a.ui.artboard_focus.as_ref().unwrap().rect, r);
            assert_eq!(view.zoom, 1.75);
            assert_eq!(view.doc_size, [st.doc.size.width, st.doc.size.height]);
            assert!(!view.fit_pending && !view.fill_pending);
            assert_eq!(st.history.past_len(), steps + 1);
        }
        let (mut a, board, child) = move_board();
        let idx = a.session.active_index().unwrap();
        let before = a.ui.views[idx].clone();
        assert!(a.run("layer.new.artboard", json!({"width":0})).is_err());
        assert_eq!(a.ui.views[idx], before);
        a.run("layer.duplicate", json!({"inPlace":true})).unwrap();
        assert_eq!(a.ui.views[idx], before, "Alt-drag duplication never pans mid-gesture");
        a.run("layer.select", json!({"layer":child.0})).unwrap();
        a.run("layer.new.layerViaCopy", json!({})).unwrap();
        assert_eq!(a.ui.views[idx], before, "an ordinary layer copy does not focus its owner");
        a.run("layer.select", json!({"layer":board.0})).unwrap();
        a.run("layer.artboard.set", json!({"width":100,"moveContents":false})).unwrap();
        assert_eq!(a.ui.views[idx], before, "editing an existing board does not pan");
    }
    #[test]
    fn focus_rejects_nonfinite_cameras_and_invalid_animation_frames() {
        for (center, zoom, rotation) in [([f32::NAN, 40.0], 1.0, 0.0), ([50.0, 40.0], f32::INFINITY, 0.0), ([50.0, 40.0], 1.0, f32::NAN)] {
            let (mut a, _, _) = move_board();
            let idx = a.session.active_index().unwrap();
            let view = &mut a.ui.views[idx];
            view.center = center;
            view.zoom = zoom;
            view.rotation = rotation;
            a.run("layer.duplicate", json!({})).unwrap();
            assert!(a.ui.artboard_focus.is_none());
        }
        for backwards in [false, true] {
            let (mut a, _, _) = move_board();
            let idx = a.session.active_index().unwrap();
            a.run("layer.duplicate", json!({})).unwrap();
            let doc = a.session.active().unwrap().doc.clone();
            let mut view = a.ui.views[idx].clone();
            let before = view.clone();
            let rect = if backwards {
                a.ui.artboard_focus.as_mut().unwrap().started = Some(1.0);
                a.last_canvas_rect
            } else {
                Rect::ZERO
            };
            advance_focus(&mut a, &egui::Context::default(), idx, &doc, &mut view, rect);
            assert!(a.ui.artboard_focus.is_none());
            assert_eq!(view, before);
        }
    }
    #[test]
    fn focus_preparation_requires_the_pending_document_target_and_displayed_view() {
        let (mut a, _, _) = move_board();
        a.run("layer.duplicate", json!({})).unwrap();
        let idx = a.session.active_index().unwrap();
        let doc = a.session.active().unwrap().doc.clone();
        let view = a.ui.views[idx].clone();
        let wrong_doc = a.session.documents()[0].doc.clone();
        focus_image_prepared(&mut a, &wrong_doc, &view);
        assert!(!a.ui.artboard_focus.as_ref().unwrap().prepared);
        let mut changed = view.clone();
        changed.center[0] += 1.0;
        focus_image_prepared(&mut a, &doc, &changed);
        assert!(!a.ui.artboard_focus.as_ref().unwrap().prepared);
        let mut hidden = doc.clone();
        let target = a.ui.artboard_focus.as_ref().unwrap().layer;
        std::sync::Arc::make_mut(&mut hidden).layer_mut(target).unwrap().visible = false;
        focus_image_prepared(&mut a, &hidden, &view);
        assert!(!a.ui.artboard_focus.as_ref().unwrap().prepared);
        focus_image_prepared(&mut a, &doc, &view);
        let focus = a.ui.artboard_focus.as_ref().unwrap();
        assert!(focus.prepared);
        assert!(focus.started.is_none(), "preparation cannot start a clock in the cold frame");
    }
    #[test]
    fn move_controls_only_show_for_a_directly_selected_board() {
        let (mut a, board, child) = move_board();
        assert_eq!(controls(&a), (8, 4));
        a.run("layer.select", json!({"layer":child.0})).unwrap();
        assert_eq!(controls(&a), (0, 0));
        assert!(!pointer(&mut a, ToolEvent::Down { x: 100.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE));
        a.ui.tool = Tool::Artboard;
        assert_eq!(controls(&a), (8, 4), "the Artboard tool still resolves a child's owner");
        a.run("layer.select", json!({"layer":board.0})).unwrap();
        a.ui.tool = Tool::Brush;
        assert_eq!(controls(&a), (0, 0));
    }
    #[test]
    fn move_resizes_all_eight_board_handles_without_scaling_children() {
        let expected =
            [[10, 10, 100, 80], [0, 10, 100, 80], [0, 10, 110, 80], [0, 0, 110, 80], [0, 0, 110, 90], [0, 0, 100, 90], [10, 0, 100, 90], [10, 0, 100, 80]];
        for (i, h) in handles([0.0, 0.0, 100.0, 80.0]).into_iter().enumerate() {
            let (mut a, board, child) = move_board();
            a.ui.tool_options.move_show_transform = true;
            let before = a.session.active().unwrap().doc.clone();
            let steps = a.session.active().unwrap().history.past_len();
            event(&mut a, ToolEvent::Down { x: h[0], y: h[1], pressure: 1.0 }, egui::Modifiers::NONE);
            assert_eq!(a.ui.artboard_gesture.as_ref().unwrap().action, Action::Resize { layer: board.0, handle: i });
            event(&mut a, ToolEvent::Move { x: h[0] + 10.0, y: h[1] + 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
            let idx = a.session.active_index().unwrap();
            let shown = display_doc(&mut a, idx).unwrap().0;
            let [l, t, r, b] = expected[i];
            let rect = photocraft_geom::Rect::new(l, t, r, b);
            assert_eq!(shown.layer(board).unwrap().artboard().unwrap().rect, rect);
            assert_eq!(shown.layer(child).unwrap().surface(), before.layer(child).unwrap().surface());
            assert_eq!(a.session.active().unwrap().history.past_len(), steps);
            assert!(a.drag.is_none() && a.ui.transform.is_none());
            event(&mut a, ToolEvent::Up { x: h[0] + 10.0, y: h[1] + 10.0 }, egui::Modifiers::NONE);
            assert_eq!(a.session.active().unwrap().doc.layer(board).unwrap().artboard().unwrap().rect, rect);
            assert_eq!(a.session.active().unwrap().history.past_len(), steps + 1);
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(*a.session.active().unwrap().doc, *before);
        }
    }
    #[test]
    fn move_plus_controls_create_or_copy_all_four_neighbours_in_one_step() {
        for index in 0..4 {
            for duplicate in [false, true] {
                let (mut a, board, _) = move_board();
                let before = a.session.active().unwrap().doc.clone();
                let steps = a.session.active().unwrap().history.past_len();
                let xf = ViewXform::active(&a).unwrap();
                let p = xf.to_doc(plus(&xf, [0.0, 0.0, 100.0, 80.0])[index].1);
                let mods = if duplicate { egui::Modifiers::ALT } else { egui::Modifiers::NONE };
                event(&mut a, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods);
                event(&mut a, ToolEvent::Up { x: p[0], y: p[1] }, mods);
                let st = a.session.active().unwrap();
                assert_eq!(st.doc.artboards().len(), 2);
                let copy = st.doc.layer(st.active_layer.unwrap()).unwrap();
                let expected = [
                    photocraft_geom::Rect::new(0, -180, 100, -100),
                    photocraft_geom::Rect::new(200, 0, 300, 80),
                    photocraft_geom::Rect::new(0, 180, 100, 260),
                    photocraft_geom::Rect::new(-200, 0, -100, 80),
                ][index];
                assert_eq!(copy.artboard().unwrap().rect, expected);
                assert_eq!(copy.children().unwrap().len(), usize::from(duplicate));
                assert_eq!(st.doc.layer(board).unwrap(), before.layer(board).unwrap());
                assert_eq!(st.history.past_len(), steps + 1);
                assert!(a.ui.artboard_gesture.is_none() && a.drag.is_none() && a.ui.transform.is_none());
                a.run("edit.undo", json!({})).unwrap();
                assert_eq!(*a.session.active().unwrap().doc, *before);
            }
        }
    }
    #[test]
    fn move_name_and_frame_select_the_board_but_interior_picks_children() {
        for point in [[10.0, -10.0], [100.0, 20.0]] {
            let (mut a, board, child) = move_board();
            a.run("layer.select", json!({"layer":child.0})).unwrap();
            a.ui.tool_options.move_auto_select = true;
            event(&mut a, ToolEvent::Down { x: point[0], y: point[1], pressure: 1.0 }, egui::Modifiers::NONE);
            event(&mut a, ToolEvent::Up { x: point[0], y: point[1] }, egui::Modifiers::NONE);
            assert_eq!(a.session.active().unwrap().active_layer, Some(board));
            assert_eq!(controls(&a), (8, 4));
            event(&mut a, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
            event(&mut a, ToolEvent::Up { x: 20.0, y: 20.0 }, egui::Modifiers::NONE);
            assert_eq!(a.session.active().unwrap().active_layer, Some(child));
            assert_eq!(controls(&a), (0, 0));
        }
    }
    #[test]
    fn move_board_name_keeps_alt_duplicate_drag_and_child_interior_keeps_normal_move() {
        let (mut a, board, child) = move_board();
        let before = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        for ev in [ToolEvent::Down { x: 10.0, y: -10.0, pressure: 1.0 }, ToolEvent::Move { x: 25.0, y: 5.0, pressure: 1.0 }, ToolEvent::Up { x: 25.0, y: 5.0 }]
        {
            event(&mut a, ev, egui::Modifiers::ALT);
        }
        let st = a.session.active().unwrap();
        let copy = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(st.doc.artboards().len(), 2);
        assert_eq!(copy.artboard().unwrap().rect, photocraft_geom::Rect::new(15, 15, 115, 95));
        assert_eq!(st.doc.layer(board).unwrap(), before.layer(board).unwrap());
        assert_eq!(st.history.past_len(), steps + 1);
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(*a.session.active().unwrap().doc, *before);
        a.ui.tool_options.move_auto_select = true;
        for ev in [ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, ToolEvent::Move { x: 30.0, y: 25.0, pressure: 1.0 }, ToolEvent::Up { x: 30.0, y: 25.0 }]
        {
            event(&mut a, ev, egui::Modifiers::NONE);
        }
        let st = a.session.active().unwrap();
        assert_eq!(st.active_layer, Some(child));
        assert_eq!(st.doc.layer(board).unwrap().artboard(), before.layer(board).unwrap().artboard());
        let pixels = st.doc.layer(child).unwrap().surface().unwrap();
        assert_eq!(pixels.rgba(20, 20)[3], 0.0);
        assert_eq!(pixels.rgba(30, 25)[3], 1.0);
        assert!(a.ui.artboard_gesture.is_none() && a.ui.transform.is_none());
    }
    #[test]
    fn move_board_controls_cancel_on_escape_tool_change_and_position_lock() {
        for escape in [false, true] {
            let (mut a, _, _) = move_board();
            let before = a.session.active().unwrap().doc.clone();
            event(&mut a, ToolEvent::Down { x: 100.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
            event(&mut a, ToolEvent::Move { x: 120.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
            let idx = a.session.active_index().unwrap();
            assert!(display_doc(&mut a, idx).is_some());
            if escape {
                let ctx = egui::Context::default();
                let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
                ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| crate::shortcuts::handle(&mut a, ui.ctx())).textures_delta.clear();
            } else {
                a.ui.tool = Tool::Artboard;
                assert!(display_doc(&mut a, idx).is_none());
            }
            assert!(a.ui.artboard_gesture.is_none() && a.artboard_preview.is_none() && a.prefs_rt.snap.is_none());
            event(&mut a, ToolEvent::Up { x: 120.0, y: 40.0 }, egui::Modifiers::NONE);
            assert_eq!(*a.session.active().unwrap().doc, *before);
        }
        let (mut a, _, _) = move_board();
        a.run("layer.lockLayers", json!({"position":true})).unwrap();
        let before = a.session.active().unwrap().doc.clone();
        let view = a.ui.views[a.session.active_index().unwrap()].clone();
        event(&mut a, ToolEvent::Down { x: 100.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(a.ui.artboard_gesture.is_none() && a.ui.status_error);
        event(&mut a, ToolEvent::Up { x: 120.0, y: 40.0 }, egui::Modifiers::NONE);
        assert_eq!(*a.session.active().unwrap().doc, *before);
        assert_eq!(a.ui.views[a.session.active_index().unwrap()], view);
    }
    fn drag(a: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
        pointer(a, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(a, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(a, ToolEvent::Up { x: to[0], y: to[1] }, egui::Modifiers::NONE);
    }
    #[test]
    fn draw_preview_commit_and_undo() {
        let mut a = app();
        pointer(&mut a, ToolEvent::Down { x: -40.0, y: -30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut a, ToolEvent::Move { x: 60.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(!a.session.active().unwrap().doc.has_artboards());
        pointer(&mut a, ToolEvent::Up { x: 60.0, y: 50.0 }, egui::Modifiers::NONE);
        assert_eq!(a.session.active().unwrap().doc.artboards()[0].2.rect, photocraft_geom::Rect::new(-40, -30, 60, 50));
        a.run("edit.undo", json!({})).unwrap();
        assert!(!a.session.active().unwrap().doc.has_artboards());
    }
    #[test]
    fn tool_switch_cancels_without_edit() {
        let mut a = app();
        pointer(&mut a, ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, egui::Modifiers::NONE);
        a.ui.tool = Tool::Move;
        assert!(!pointer(&mut a, ToolEvent::Up { x: 60.0, y: 50.0 }, egui::Modifiers::NONE));
        assert!(a.ui.artboard_gesture.is_none());
        assert!(!a.session.active().unwrap().doc.has_artboards());
    }
    #[test]
    fn moving_board_keeps_dimensions() {
        let mut a = app();
        a.run("layer.new.artboard", json!({"x":0,"y":0,"width":50,"height":50})).unwrap();
        drag(&mut a, [25.0, 25.0], [35.0, 45.0]);
        assert_eq!(a.session.active().unwrap().doc.artboards()[0].2.rect, photocraft_geom::Rect::new(10, 20, 60, 70));
    }
    #[test]
    fn left_handle_keeps_child_pixels_and_single_undo() {
        let mut a = app();
        let id = a.run("layer.new.artboard", json!({"x":0,"y":0,"width":50,"height":50})).unwrap()["layer"].as_u64().unwrap();
        a.run("layer.new.layer", json!({})).unwrap();
        let child = a.session.active().unwrap().active_layer.unwrap();
        a.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        a.run("layer.select", json!({"layer":id})).unwrap();
        let before = a.session.active().unwrap().doc.clone();
        a.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0));
        let v = a.ui.views.get_mut(0).unwrap();
        v.zoom = 1.0;
        v.center = [50.0, 50.0];
        v.fit_pending = false;
        drag(&mut a, [0.0, 25.0], [10.0, 25.0]);
        assert_eq!(a.session.active().unwrap().doc.artboards()[0].2.rect.x0, 10);
        assert_eq!(a.session.active().unwrap().doc.layer(child).unwrap().surface(), before.layer(child).unwrap().surface());
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(a.session.active().unwrap().doc.artboards()[0].2.rect.x0, 0);
    }
    #[test]
    fn directional_plus_and_alt_copy_use_current_board() {
        let mut a = app();
        let id = a.run("layer.new.artboard", json!({"width":50,"height":50})).unwrap()["layer"].as_u64().unwrap();
        a.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 500.0));
        let v = a.ui.views.get_mut(0).unwrap();
        v.zoom = 1.0;
        v.center = [25.0, 25.0];
        v.fit_pending = false;
        let xf = ViewXform::active(&a).unwrap();
        let p = xf.to_doc(plus(&xf, [0.0, 0.0, 50.0, 50.0])[1].1);
        pointer(&mut a, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::ALT);
        assert_eq!(a.session.active().unwrap().doc.artboards().len(), 2);
        assert!(a.ui.artboard_gesture.is_none());
        let old = a.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().artboard().unwrap().rect;
        let current = active(&a).unwrap();
        assert!(current.1[0] >= old.x1 as f64);
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(a.session.active().unwrap().doc.artboards().len(), 1);
    }
    #[test]
    fn stale_document_edit_cancels_gesture() {
        let mut a = app();
        pointer(&mut a, ToolEvent::Down { x: -10.0, y: -10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        a.run("layer.new.layer", json!({})).unwrap();
        pointer(&mut a, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
        assert!(!a.session.active().unwrap().doc.has_artboards());
    }
}
