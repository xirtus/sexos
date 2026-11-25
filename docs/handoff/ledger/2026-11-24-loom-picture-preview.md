# Loom picture preview

- Gap: Loom's grid showed small picture thumbnails, but a person had to open another app to inspect a picture at window size.
- Change: F3 now decodes a selected PNG, JPEG, GIF or WebP in Loom and fits it to the window. It refuses inputs over 16 MiB, uses the bounded native decoder, blends transparent pixels over Loom's surface and returns to the folder with Escape. The window and preview use Loom branding.
- Test: The Files thumbnail QEMU gate opens the generated `IMG00.PNG` in the native preview, captures `image-preview.ppm`, and checks the selected picture's colour appears in the screenshot. The existing thumbnail gate still checks 50 picture thumbnails and cache reuse. Full builds and QEMU run only on GitHub CI.
