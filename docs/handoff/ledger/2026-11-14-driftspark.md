# Driftspark: one SDL game (slice I8, second half)

**`apps/driftspark`.** An original arcade game written in C against SDL2 (crates/sex-sdl). It is built and started the way apps/sdltest is.

## The game

The player steers a small craft around a field that wraps at the edges. Sparks drift across it, and the craft collects them for 10 points times the level. Debris drifts too and grows as it goes. At full size it flickers and bursts into two small pieces. Touching debris costs one of three lives, followed by two seconds of blinking safety.

Every eight sparks raise the level, which brings more debris, faster debris that grows sooner, and a fanfare. Surviving earns points as well: the level's number each second.

Screens:
- a title screen
- play, with a top strip for score, level and lives
- a pause overlay
- a game-over screen with score and best

Keys: arrows or WASD push the craft, and it drifts with a little drag. Each press is one push, because Shell delivers presses only. Space launches the game, and in play it brakes. P pauses, R restarts, and Esc (or Q) quits.

- **Drawing:** everything is drawn with filled rects into the program's own 320x240 XRGB frame:
  - discs for debris
  - a 7x7 craft rotated four ways
  - cross-shaped sparks
  - a scrolling starfield
  - particles
  - a 5x7 bitmap font made for the game

  `SDL_ConvertPixels` copies the frame to the window surface, and `SDL_UpdateWindowSurface` shows it.
- **Sound:** synthesised in the SDL audio callback at 48 kHz: square, triangle and noise voices with pitch sweeps and a linear fade. There are sounds for launch, spark, hit, debris burst, level up, pause and game over.
- **Timing:** 60 updates a second on an `SDL_GetTicks` schedule. When the game falls behind it catches up at most five steps, then resyncs. The simulation is integer only (1/256 px fixed point, a xorshift generator).

## Demo mode

`run driftspark --demo SEED FRAMES` runs exactly one update per frame, still paced at 60 fps, with scripted input. The pilot heads for the nearest spark and pushes away from close debris. It also pauses and resumes at frames 400 and 430, and presses R 90 frames after a game over. At the end it prints:

```
[driftspark] demo seed=S frames=F score=N level=L lives=K sparks=M
[driftspark] demo checksum=XXXXXXXX   (FNV-1a over the last frame's RGB)
```

and exits 0. `--demo 11 900` gives score 417, level 3, two lives, 21 sparks and checksum `7cee900e`. A host build of the same C against a stub SDL header gives the same values.

## Arrow keys in the SDL backend

The game showed that Shell passes the grey movement keys as their set-1 codes without the E0 prefix: Right arrives as `0x4d`. SDL's Linux table reads those codes as the keypad, so Right became `SDL_SCANCODE_KP_6`.

`send_key` in `crates/sex-sdl/c/SDL_sexosvideo.c` now reports these set-1 codes as the movement keys:
- `0x48`, `0x4b`, `0x4d` and `0x50` as the arrows
- `0x47`, `0x4f`, `0x49` and `0x51` as Home, End, PgUp and PgDn
- `0x52` and `0x53` as Ins and Del

Shell does not tell the keypad apart from these keys. The game also takes keypad 8, 4, 6 and 2 for steering.

## Registration

Additions only:
- the workspace `Cargo.toml` members and `Cargo.lock`
- `sexos_build_spec.toml`: the whitelist entry and the `build_driftspark` stage, which writes `iso_root/programs/driftspark`
- `.github/workflows/ci.yml`: the step "SDL2 game (demo score, window, sound, keys)", right after the SDL2 step

## Gate (`scripts/driftspark_gate.py`)

The gate uses the same boot as `sdl_gate.py`, with QEMU's Intel HD Audio recorded to `sound.wav`.

1. It types `run driftspark --demo 11 900` and checks the exact score line and checksum, `[sdl.present]`, exit status 0, and `[sdl.audio]` with at least 4800 non-silent frames.
2. It types `run driftspark` and takes a screendump of the title, which must show the amber title letters on the dark field.
3. It presses Space and gets `start`. It presses Right and gets `steer dir=right x y`, then 20 frames later `drift x y`, which must be at least 12 px to the right. A screendump must show the craft's cyan hull right of where the push began.
4. P pauses and P resumes. Esc quits, the program exits 0 and its window retires.
5. The WAV must hold at least four separate sounds.

## Left out

- No high-score file: the best score lasts only for the session.
- No key-hold steering, because Shell sends presses only, and no gamepad (SDL joystick is off).
- Single window size, 320x240, unscaled. In the gate's default tiling the window's pixel area is 550x144, so only the top 144 rows show; the game does not scale to the area (SDL reports no resize).
- No music, only effects.

Gate result: `{'ok': True, 'demo_score': 417, 'checksum': '7cee900e', 'nonsilent': 174453, 'sounds': 37, 'title_amber': 1350, 'moved': (160, 180), 'drawn_x': 220, 'area': (550, 144)}`; `sdl_gate.py` still passes.
