//! Release benchmark for 24 MP artboard gestures. Synthetic dense 16-bit raster; no file I/O.
//! Run: cargo run --release -p photocraft-engine --example artboard_bench
use std::time::Instant;

use photocraft_engine::{Session, artboard_cmds};
use photocraft_geom::Rect;
use serde_json::json;

fn stats(samples: &mut [f64]) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let median = samples.get(samples.len() / 2).copied().unwrap_or_default();
    json!({"samples_ms":samples,"p50_ms":median,"max_ms":samples.last().copied().unwrap_or_default()})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":6000,"height":4000,"depth":16,"artboards":true,"background":"transparent"}))?;
    let state = session.active().ok_or("no document")?;
    let board = state.doc.layers.first().ok_or("no board")?.id;
    let child = state.doc.layer(board).and_then(|l| l.children()).and_then(|c| c.first()).ok_or("no child")?.id;
    session.edit("Dense benchmark image", |doc, _| {
        let surface = doc.layer_mut(child).and_then(|l| l.surface_mut()).ok_or(photocraft_engine::EngineError::NoLayer(child))?;
        surface.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.8, 0.1, 0.2, 1.0]);
        surface.fill_rect(Rect::new(0, 0, 3000, 2000), &[0.1, 0.2, 0.8, 1.0]);
        Ok(())
    })?;
    let source = session.active().ok_or("no document")?.doc.clone();
    let mut rows = serde_json::Map::new();
    for (name, rect, move_contents) in [
        ("resize frame", Rect::new(-1, -1, 5999, 3999), false),
        ("move one pixel", Rect::new(1, 1, 6001, 4001), true),
        ("move one tile", Rect::new(256, 256, 6256, 4256), true),
    ] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            let preview = artboard_cmds::preview_rect(&source, board, rect, move_contents)?;
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            if preview.layer(board).and_then(|l| l.artboard()).map(|a| a.rect) != Some(rect) {
                return Err("preview frame mismatch".into());
            }
            let offset = if move_contents { (rect.x0, rect.y0) } else { (0, 0) };
            for (x, y) in [(0, 0), (2999, 1999), (3000, 2000), (5999, 3999)] {
                let expected = source.layer(child).and_then(|l| l.surface()).ok_or("source surface missing")?.pixel(x, y);
                let actual = preview.layer(child).and_then(|l| l.surface()).ok_or("preview surface missing")?.pixel(x + offset.0, y + offset.1);
                if actual != expected {
                    return Err("preview pixel mismatch".into());
                }
            }
        }
        rows.insert(name.into(), stats(&mut samples));
    }
    let started = Instant::now();
    session.execute("layer.artboard.set", json!({"layer":board.0,"x":-256,"y":-256}))?;
    let commit_ms = started.elapsed().as_secs_f64() * 1000.0;
    if !session.undo() || session.active().is_none_or(|s| *s.doc != *source) {
        return Err("undo did not restore source".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"size":[6000,4000],"depth":16,"synthetic":true,"rows":rows,"commit_ms":commit_ms,"pixel_equality":"exact at four boundary probes per preview","undo_equality":"complete document","scope":"engine preview/commit only; excludes UI presentation, codecs and Photoshop comparison"})
        )?
    );
    Ok(())
}
