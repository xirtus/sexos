# Gestalt (Vector Canvas & Graphics)

> **Identity Note**: **Gestalt** (`apps/shapes`, `/bin/gestalt`) is the SexOS 2D vector graphics and canvas rendering showcase, named under Andreas's design philosophy ("the whole from its shapes"). In Spindle, typing `gestalt` or `shapes` opens Gestalt.

**Gestalt** demonstrates the high-performance 2D rasterization capabilities of the SexOS graphics subsystem (`sex-canvas` and tiny-skia). It showcases real-time subpixel anti-aliasing, rounded geometry, soft Gaussian drop shadows, linear/radial gradients, and parametric Bézier curves rendered directly on the Silk desktop.

---

## Launching Gestalt

- **From Spindle**:
  ```text
  run gestalt
  ```
- **From Voyeur**: Press **`Super+D`**, type `gestalt` or `shapes`, and press **`Enter`**.

---

## Interactive Features

### 1. Vector Primitives & Geometry
Gestalt renders smooth, resolution-independent vector shapes:
- Rounded rectangles with continuous curvature.
- Circles, ellipses, polygons, and complex stars.
- Cubic and quadratic Bézier splines.

### 2. High-Fidelity Shadows & Gradients
- Demonstrates real-time box blur and Gaussian shadow algorithms used across the **Silk** window manager for window elevation and active frame focus highlights.
- Multi-stop color gradients with high-precision dithered transitions.

### 3. Mouse & Pointer Interaction
- **Click & Drag**: Move shapes freely across the canvas to observe live anti-aliasing re-rasterization without screen tearing or frame stutter.
- **Scroll Wheel**: Scale vector shapes dynamically.
- **Space**: Toggle animation mode to demonstrate continuous 60 FPS procedural rotation and translation.
- **Q / Esc**: Close Gestalt.
