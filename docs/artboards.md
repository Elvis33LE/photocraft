# Artboards

Artboards are top-level layer groups with their own non-destructive frame and background. Each group contains ordinary raster, text, shape, fill, adjustment, smart-object and nested group layers. All boards share the document's resolution, colour model, bit depth and profile. Artboards cannot be nested inside groups or other artboards.

## Create and edit

Enable **Artboards** beside the orientation controls in File > New to start with one board. The option is retained in saved document presets. Alternatively, open the Move tool flyout and choose **Artboard Tool**. Both tools use the V shortcut group; Shift+V cycles according to the tool-switch preference.

Drag an empty pasteboard area to create a board. Click inside a board or its name to select and move it. Eight handles resize the frame without scaling or deleting its contents. A resize from the left or top leaves child pixels at their global positions. Moving a board moves its contents and owned guides together. Escape cancels an in-progress gesture; release commits one undo step. Changing tools or switching documents cancels the gesture.

The Move tool also shows the four plus controls and eight size handles when the artboard container itself is selected. Click its name or frame to select the container. Side handles change width or height; corner handles change both. Selecting an ordinary child keeps the usual Move behaviour. These board handles do not require Show Transform Controls.

Circular plus controls create an adjacent empty board of the same size and background. A direction is offered only when its immediate board-sized destination, 100 document pixels from the current frame, is free; hidden boards also reserve their space. A control is also hidden if its circle would cover another visible board. Alt on Windows/Linux or Option on macOS copies the contents and owned guides as well. Engine/automation placement can still advance outward past existing frames to avoid a collision. This placement policy is PhotoCraft's explicit contract; detailed Photoshop collision behaviour has not been measured.

After successfully creating or automatically copying a board, the canvas pans smoothly toward its centre over 0.3 seconds while preserving zoom and rotation. The transition clock starts after the canvas has prepared the new image; initial copying and rendering can still pause on large boards. Manual navigation or a new canvas gesture interrupts the motion, including during preparation; another creation starts from the currently displayed position. With Overscroll disabled, the destination respects the visible artboard workspace, including negative origins; workspace growth does not snap the starting position. Explicit Alt/Option dragging keeps its pointer-controlled view behaviour. View changes do not add document undo steps, and unfinished transitions are not restored from settings.

Layer > Duplicate Layer and Ctrl+J on Windows/Linux or Command+J on macOS place a copied artboard to its right at the original X plus its width plus 100 document pixels, retaining Y. An existing board in that space moves placement further right to avoid overlap. Ordinary layer copies keep their original position.

The Properties panel offers dimensions, position, units, proportional linking, orientation, presets and background colour. Units convert through the document resolution. The Artboard tool options allow renaming and hiding board names. The Layers panel shows compact artboard headers with visibility, disclosure and a bold name, and provides an Artboards kind filter that retains their contents. Mask thumbnails remain interactive; clicking the header name returns from mask editing to the board's contents.

Fit on Screen includes all visible board frames, including negative positions. Selecting a board or one of its children gives fills, selections and colour-range sampling that board's editing bounds. Creating an ordinary layer while a board is selected places it inside that board. Document channels continue to use the document canvas.

Moving ordinary layers across boards assigns them to the topmost board under the release point, preserving global positions. Releasing outside all boards moves them to the document root. The existing **Prevent auto-nesting** lock retains their current parent. Explicit layer-panel moves remain available. A board's position lock protects its frame while allowing unlocked children to move; ordinary group locks and the all-lock still propagate.

Layer > Ungroup Layers dissolves a board. Dissolving the last board returns to an ordinary document, expands its canvas to retain contents and translates negative content into that canvas. Undo restores the original board, identities and coordinates. Operations that would exceed the existing 300,000-pixel dimension limit fail before changing the document.

## Guides and persistence

New Guide and New Guide Layout offer an explicit target: Document, an individual board, or all boards for a layout. Ruler-created guides belong to the selected board; without a selected board they belong to the document. Board guides draw inside their frame, move and copy with it, and can be cleared independently. Document guides remain stationary. A coincident document and board guide remain separate objects.

Native `.pcraft` stores guide ownership in the artboard. Older native files without this additive field load with empty owned guides; their document guides remain document guides. PSD/PSB maps ownership through `guideIndeces` and resource 1032. Unknown descriptor properties and unrelated layer blocks are retained. Invalid guide indices report import warnings.

Negative origins are supported. The canvas renders visible artboard overflow outside the original document bounds with bounded textures; empty spaces between distant boards do not require a full-resolution combined raster image.

## Export

File > Export > Export As opens with **Ctrl+Alt+Shift+W** on Windows/Linux and **Command+Option+Shift+W** on macOS. Custom shortcut preferences continue to override defaults. Native Linux/macOS shortcut delivery still needs testing on those operating systems.

The board list has individual checkboxes and Select All. Multiple checked boards appear in a two-column preview grid with their names and individual aspect ratios. Checking a single board shows that board alone. Clicking a row switches to its single preview without changing the checked export set; changing checkboxes or returning to the overview restores the checked-board preview. Each board retains its own format, dimensions, scale, canvas padding, transparency and profile options. Scaling variants add a scale and suffix. The preview has zoom controls. Empty transparent boards export their full dimensions, as do Layer Export As and Quick Export.

Artboards to Files and Artboards to PDF reuse the checked board list. Their options include background, overlapping document content, name labels and reversed Layers-panel order. PDF offers JPEG or ZIP compression, profile embedding and a single multi-page file or separate files. Labels use the existing text engine and add a header to the export canvas. Export uses copies and leaves document selection, IDs and history unchanged.

Filenames are sanitized and disambiguated, including case-insensitive collisions with existing output names. Each raster output is limited to 100 million pixels and 300,000 pixels per side; one export selects at most 1,000 boards, 32 scaling variants and 1,000 outputs. Multi-page PDF is limited to 100 million total pixels. Browser builds retain platform-service file-output restrictions. PDF remains raster-based; vector PDF and Content Credentials are outside this feature.

## Engine and automation

Existing command IDs remain stable. Examples use document pixel coordinates:

```json
{"command":"file.new","params":{"width":800,"height":600,"depth":16,"background":"transparent","artboards":true}}
{"command":"layer.new.artboard","params":{"x":-400,"y":-200,"width":320,"height":240}}
{"command":"layer.new.artboard","params":{"adjacentTo":42,"direction":"left","duplicate":true}}
{"command":"layer.artboard.set","params":{"layer":42,"x":10,"width":300,"moveContents":false}}
{"command":"layer.translate","params":{"dx":900,"dy":0,"autoNest":true,"drop":[950,200]}}
{"command":"view.newGuide","params":{"artboard":42,"orientation":"vertical","position":100}}
{"command":"view.newGuideLayout","params":{"artboards":[42,43],"columns":4,"gutter":20,"clearExisting":true}}
{"command":"file.export.artboardsToFiles","params":{"artboards":[42,43],"dir":"output","prefix":"design","format":"png","variants":[{"scale":100,"suffix":""},{"scale":200,"suffix":"@2x"}]}}
{"command":"file.export.artboardsToPdf","params":{"artboards":[42,43],"path":"output/design.pdf","compression":"zip","separateFiles":false,"reverseOrder":true}}
```

`ui.set` accepts `tool:"artboard"`, `artboardProportions`, `artboardUnit` (`px`, `in`, `cm`, `mm`, `pt`, `pica`) and `hideArtboardNames`. Pointer input uses the same tool state machine as native interaction. The serialized artboard gesture is observable but is never restored as a held pointer.

This workflow implements documented core behaviour and explicit PhotoCraft rules. It does not claim fully measured Photoshop parity for automatic nesting, overlapping-board exports, collision placement, guide behaviour or modifier details.
