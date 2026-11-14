# Gaze (Image Viewer)

> **Identity Note**: **Gaze** (`apps/view`, `/bin/gaze`) is the native SexOS image viewer, named under the romance theme. In Spindle, typing `gaze` or `view` opens Gaze.

**Gaze** is a fast graphical image viewer supporting all common picture formats. It features instant zooming, high-resolution panning, orientation rotation, folder-wide slideshows, and desktop wallpaper integration.

---

## Launching Gaze

- **From Spindle**:
  ```text
  run gaze @photo.png           # View a single image with explicit custody
  run gaze @/data/pictures/     # Open Gaze in a folder gallery
  ```
- **From Loom**: Double-click any image file or select it and press **`Enter`**.
- **From Voyeur**: Press **`Super+D`**, type `gaze`, and choose an image.

---

## Shortcuts & Viewer Controls

| Shortcut | Action | Description |
|---|---|---|
| **`Left / Right`** | **Prev / Next Image** | Navigate to the previous or next picture in the current folder. |
| **`Up / Down`** | **Zoom In / Out** | Smoothly zoom in on details or zoom out. |
| **`0`** | **Actual Size (1:1)** | Reset zoom level to native 100% pixel resolution. |
| **`9`** | **Fit to Window** | Scale image so it fits comfortably within the current frame. |
| **`R`** | **Rotate Clockwise** | Rotate picture 90° clockwise. |
| **`Shift+R`** | **Rotate Counter-CW** | Rotate picture 90° counter-clockwise. |
| **`Space`** | **Slideshow** | Start or pause automatic folder slideshow. |
| **`W`** | **Set Wallpaper** | Immediately apply the current picture as your Silk desktop wallpaper. |
| **`F`** | **Fullscreen** | Expand image to full screen without window borders. |
| **`Q` / `Esc`** | **Quit** | Close Gaze. |

---

## Supported Image Formats

Gaze handles standard raster formats natively with optimized decoding:
- **PNG**: Full alpha transparency, Adam7 interlacing, 1-bit to 16-bit depths.
- **JPEG**: Baseline and progressive photographic image rendering.
- **WebP**: Lossy and lossless compressed images.
- **GIF**: Animated GIF playback with frame timing.
