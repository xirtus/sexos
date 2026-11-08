# Using SexOS

What you can do on SexOS today, and how. This is an alpha: rough edges are expected, and
the [feature ledger](handoff/SEXOS_FEATURE_LEDGER_2026_11_02.md) says what each part can't do yet.

## Starting it

- **From the ISO.** Boot `sexos-v1.0.0.iso` from a USB stick or in QEMU. The desktop opens
  with Quil (editor), Linen, the Browser and Spindle (terminal).
- **Installing to the disk.** In Spindle, type `install`. It writes the system you booted to
  the disk's boot slot and makes the disk bootable under UEFI. Take the stick out and the
  machine starts from the disk, with your files. `install FILE` writes a boot image you
  fetched instead. Install replaces the disk's partition table.

## The desktop

| Key | Does |
|---|---|
| backtick | Command palette: open apps and programs |
| Alt+Tab | Next window |
| Alt+= / Alt+- | Larger / smaller text in the focused window |
| Alt+0 | Text back to its usual size |
| F11 | Close the focused window. A program that doesn't close within five seconds is ended |
| Scroll Lock | Show or hide Spindle |

Windows tile themselves. Drag a title bar to move a window, or an edge or corner to resize it; either way the tiling then leaves it where you put it, until you zoom it. Up to eight programs can be open at once, beside the built-in apps.

## Spindle, the terminal

Type `help` for the full list.

| Command | Does |
|---|---|
| `ls`, `cd`, `pwd`, `mkdir`, `rm`, `mv` | Folders and files |
| `cat FILE` | Show a file |
| `write FILE TEXT`, `append FILE TEXT`, `touch FILE` | Make or change a file |
| `df` | Disk space |
| `fetch URL FILE` | Download over HTTP or HTTPS into a file |
| `edit FILE` | Open a file in Jot |
| `run NAME [ARGS]` | Start a program from `/bin`, or `run PATH` for any program file |
| `run +net NAME ...` | The same, and let that run use the network |
| `run NAME @FILE` | Give the program that one file of yours |
| `spawn NAME COUNT`, `records`, `stopall [force]` | Background programs, thousands at once |
| `install [FILE]` | Make this disk start SexOS |
| `beep [HZ] [MS] [VOL]` | Play a note on the sound card (virtio sound in QEMU) |

## Programs

A program is a file. Install one by downloading it into `/bin`, then run it by name:

```
fetch https://example.org/jot /bin/jot
run jot
```

A program can only reach its own folder, `/data/NAME`, plus any file you hand it with
`@FILE`. It reaches the network only when you start it with `+net`. These are the person's
choices at launch, not settings inside the program.

- **Jot** is a notepad: arrows move, Ctrl+S saves, `edit FILE` opens a file in it.
- **Peek** shows a web page's text: `run +net peek https://example.com/`.
- **Wasm** runs WebAssembly programs, including Rust or C built for `wasm32-wasip1`:
  `run wasm @/hello.wasm`. They print to the window and read typed lines. `run wasm`
  alone shows a small built-in one.
- **View** shows a picture: `run view @photo.png`. PNG up to 800x600 for now.
- **Shapes** shows anti-aliased drawing (rounded windows, soft shadows): `run shapes`.
- **Web** is the graphical browser: pages with their CSS, pictures and links, drawn in Geist.
  `run web` shows its welcome page; `run +net web https://example.com` opens a site (fetch it to
  `/bin/web` first for runs with arguments); `run web @page.html` opens a file. Up/Down/Space
  scroll, Right/Left pick a link or button, Enter opens it, Backspace goes back, `/` types an
  address. Page JavaScript runs (QuickJS in a Wasm sandbox): scripts can change the page,
  answer clicks and keys, animate with timers, and draw on a `<canvas>` (Canvas 2D).
  The arrow keys move the front window, Home puts it back.

Writing your own: see `crates/sex-app` (a window, keys, files, the network, pixels) and `apps/jot`.

## The Browser

Ctrl+L opens the address bar. Type an address and Enter; a bare name means HTTPS. Type a
link's number and Enter to follow it, Backspace goes back, Esc closes. Pages show as text:
there's no layout, CSS, images, forms or JavaScript yet.

## When something breaks

If the network service or Files crashes, the kernel starts it again: a page or file request
in flight is lost, the rest carries on. A program that hangs is ended when you close it.
