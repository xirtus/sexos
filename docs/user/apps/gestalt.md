# Gestalt (vector drawing)

**Gestalt** (`apps/gestalt`, `/bin/gestalt`) is the SexOS vector editor, in the spirit of
Illustrator and Inkscape. It runs VectorCraft's engine (storytold/vectorcraft): the document,
the tools, Smart Guides, Pathfinder, undo, the renderer and the file formats are VectorCraft's
own, under a canvas Gestalt draws itself. In Spindle, `shapes` still opens it.

## Opening a drawing

```text
run gestalt                # opens welcome.svg, writing it first if it is missing
run gestalt poster.svg     # opens poster.svg, or starts an 800 x 600 pt artboard saved there
```

Gestalt opens SVG, PDF, Illustrator `.ai`, EPS and `.vectorcraft` files. It saves SVG, SVGZ,
PDF and `.vectorcraft` in place; an `.ai` or EPS drawing saves beside itself as SVG (`art.eps`
saves to `art.svg`), and the original is left as it was.

## Tools

The strip on the left shows the tools; click one or press its key.

| Key | Tool |
|---|---|
| V | Selection: click, drag to move, drag on the desk for a marquee |
| A | Direct Selection: anchors and handles |
| P | Pen |
| N | Pencil |
| M | Rectangle |
| L | Ellipse |
| \ | Line Segment |
| T | Type (Esc ends the text) |
| I | Eyedropper |
| H | Hand (the middle button pans with any tool) |
| Shift+M | Shape Builder |

## Keys

| Keys | Does |
|---|---|
| Arrows (Shift ×10) | Nudge the selection one point |
| Delete, Backspace | Delete the selection |
| Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y | Undo, redo |
| Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D | Copy, cut, paste, duplicate |
| Ctrl+A | Select all |
| Ctrl+G, Ctrl+Shift+G | Group, ungroup |
| Ctrl+Shift+1 … 4 | Pathfinder: unite, minus front, intersect, exclude |
| Ctrl+=, Ctrl+-, Ctrl+0, Ctrl+1 | Zoom in, out, fit the artboard, 100% |
| Wheel | Scroll |
| Shift-drag, Alt-drag | Constrain; copy (Selection) or draw from the centre (shapes) |
| Ctrl+S | Save |
| Ctrl+E, Ctrl+Shift+E | Export PNG, PDF beside the drawing |

## Not yet

VectorCraft's panels (Layers, Appearance, Swatches, Properties), its menus and Image Trace come
with its egui interface.
