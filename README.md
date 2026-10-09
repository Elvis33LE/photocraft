# Indexed Color review evidence

Review screenshots for PhotoCraft: integer color counts, linear slider markers/snapping and native asynchronous previews. These files are hosted separately from the source PR.

- before.png: the existing generic Range control rendered with the unchanged Indexed Color parameter schema supplied as __spec and a fractional count (64.5). This uses the current renderer; it is not a historical installation screenshot.
- after.png: the actual new Indexed Color control, 64 colors, linear scaling.
- after-*.png: all five themes at 1440x900 and 900x700, captured with the offscreen Rust snapshot example using the CPU canvas.
- visual-overview.png: contact sheet of those ten captures.
- timings.json: sanitized release timings, no input pixels, palettes, metadata or input hashes. The private source image stays local. One warmup and three measured samples per count; p50. The complete CPU preview excludes GPU upload and presentation. Earlier 256-color baseline batch measured about 2017 ms; final consecutive before/after batch measured 3018/1061 ms. Absolute times vary with system state.

All visible artwork is an original procedural RGB gradient, not the private timing image. Original test artwork and screenshot contributions by @Elvis33LE are MIT OR Apache-2.0 (license files included). Existing application UI/assets retain their upstream licenses; branding is covered by LICENSE-brand.txt. No font files or third-party source assets are distributed here.
