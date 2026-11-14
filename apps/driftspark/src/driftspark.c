/* SPDX-License-Identifier: GPL-3.0-only */
/* Driftspark: a small arcade game for SDL2.
 *
 * The player steers a drifting craft around a wrapping field, collecting sparks
 * and dodging debris that grows as it drifts and bursts into new pieces. Each
 * eight sparks raise the level: more debris that grows faster. Three lives.
 *
 * Keys: arrows or WASD give the craft a push, Space launches from the title
 * screen and brakes in play, P pauses, R restarts, Esc (or Q) quits.
 *
 * Everything is drawn into the program's own XRGB frame (`fb`) with rectangles and
 * a 5x7 bitmap font, then copied to the SDL window surface. Sound is synthesised
 * in the SDL audio callback. The simulation is integer only and steps once per
 * frame at 60 frames a second (timed with SDL_GetTicks), so it is deterministic.
 *
 * `driftspark --demo SEED FRAMES` plays FRAMES frames with scripted input and prints
 * the final score and a checksum of the last frame:
 *   [driftspark] demo seed=S frames=F score=N level=L lives=K sparks=M
 *   [driftspark] demo checksum=XXXXXXXX
 */
#include <SDL.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 320
#define H 240
#define FP 256 /* positions are in 1/256 px */
#define HUD 12 /* the top strip for score, level and lives */
#define RATE 48000

#define MAXD 24
#define MAXS 6
#define MAXP 160

enum { TITLE, PLAY, OVER };
enum { DIR_UP, DIR_RIGHT, DIR_DOWN, DIR_LEFT };
enum { SND_START, SND_SPARK, SND_HIT, SND_LEVEL, SND_OVER, SND_PAUSE, SND_BURST };

typedef struct { int x, y, vx, vy, r, grow, alive; } Debris;
typedef struct { int x, y, vx, vy, phase, alive; } Spark;
typedef struct { int x, y, vx, vy, life; Uint32 color; } Particle;

typedef struct {
    int state, paused;
    Uint32 rng;
    int px, py, pvx, pvy, dir;
    int lives, score, best, level, collected, invuln, frame, banner, overFrames;
    int watch; /* frames until the position after a steer is reported */
    Debris d[MAXD];
    Spark s[MAXS];
    Particle p[MAXP];
} Game;

static Uint32 fb[W * H];
static Game game;
static int demo, logMoves;

/* ---------------------------------------------------------------- random */

static Uint32 rnd(Game *g)
{
    Uint32 x = g->rng;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    return g->rng = x;
}

static int range(Game *g, int lo, int hi) { return lo + (int)(rnd(g) % (Uint32)(hi - lo + 1)); }

/* ---------------------------------------------------------------- sound */

typedef struct { int wave, delay, len, pos; Uint32 phase, f0, f1; int vol; } Voice;
static Voice voices[12];
static Uint32 noise = 0x1234567u;
static SDL_AudioDeviceID audio;

static void voice(int wave, int delayMs, int ms, int hz0, int hz1, int vol)
{
    int i;
    for (i = 0; i < (int)SDL_arraysize(voices); i++) {
        Voice *v = &voices[i];
        if (v->len) continue;
        v->wave = wave;
        v->delay = delayMs * RATE / 1000;
        v->len = ms * RATE / 1000;
        v->pos = 0;
        v->phase = 0;
        /* Phase steps are 2^32 / RATE per Hz. */
        v->f0 = (Uint32)((Uint64)hz0 * 4294967296ull / RATE);
        v->f1 = (Uint32)((Uint64)hz1 * 4294967296ull / RATE);
        v->vol = vol;
        return;
    }
}

static void sound(int kind, int arg)
{
    if (!audio) return;
    SDL_LockAudioDevice(audio);
    switch (kind) {
    case SND_START: voice(1, 0, 90, 330, 330, 5000); voice(1, 90, 140, 660, 660, 5000); break;
    case SND_SPARK: voice(2, 0, 90, 880 + arg * 60, 1500 + arg * 60, 7000); break;
    case SND_HIT: voice(3, 0, 350, 0, 0, 9000); voice(1, 0, 300, 180, 50, 6000); break;
    case SND_LEVEL:
        voice(2, 0, 110, 523, 523, 7000); voice(2, 110, 110, 659, 659, 7000);
        voice(2, 220, 110, 784, 784, 7000); voice(2, 330, 220, 1047, 1047, 7000);
        break;
    case SND_OVER: voice(2, 0, 800, 440, 110, 8000); voice(1, 0, 800, 220, 55, 3000); break;
    case SND_PAUSE: voice(1, 0, 50, 600, 600, 3500); break;
    case SND_BURST: voice(3, 0, 120, 0, 0, 3500); break;
    }
    SDL_UnlockAudioDevice(audio);
}

static void SDLCALL mix(void *user, Uint8 *stream, int bytes)
{
    Sint16 *out = (Sint16 *)stream;
    int frames = bytes / 4, i, k;
    (void)user;
    for (i = 0; i < frames; i++) {
        int sum = 0;
        for (k = 0; k < (int)SDL_arraysize(voices); k++) {
            Voice *v = &voices[k];
            int s, env;
            if (!v->len) continue;
            if (v->delay) { v->delay--; continue; }
            /* Linear sweep from f0 to f1 over the voice's length. */
            v->phase += v->f0 + (Uint32)(((Sint64)v->f1 - (Sint64)v->f0) * v->pos / v->len);
            switch (v->wave) {
            case 1: s = (v->phase & 0x80000000u) ? 1 : -1; s *= 1 << 14; break;          /* square */
            case 2: s = (int)(v->phase >> 16) - 32768; s = (s < 0 ? -s : s) * 2 - 32768; s /= 2; break; /* triangle */
            default:
                noise = noise * 1103515245u + 12345u;
                s = (int)((noise >> 16) & 0x7fff) - 16384;
                break;
            }
            /* A 5 ms attack, then a linear fade. */
            env = v->len - v->pos;
            if (v->pos < RATE / 200) env = env * v->pos / (RATE / 200);
            sum += (int)((Sint64)s * v->vol / 16384 * env / v->len);
            if (++v->pos >= v->len) v->len = 0;
        }
        if (sum > 32767) sum = 32767;
        if (sum < -32768) sum = -32768;
        out[2 * i] = out[2 * i + 1] = (Sint16)sum;
    }
}

/* ---------------------------------------------------------------- drawing */

static void rect(int x, int y, int w, int h, Uint32 c)
{
    int i, j;
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > W) w = W - x;
    if (y + h > H) h = H - y;
    for (j = 0; j < h; j++)
        for (i = 0; i < w; i++) fb[(y + j) * W + x + i] = c;
}

static void dot(int x, int y, Uint32 c)
{
    if (x >= 0 && y >= 0 && x < W && y < H) fb[y * W + x] = c;
}

/* A filled circle as one span per row, wrapped around the field's edges. */
static void disc(int cx, int cy, int r, Uint32 c)
{
    int dy, dx;
    for (dy = -r; dy <= r; dy++) {
        for (dx = r; dx * dx + dy * dy > r * r + r; dx--) {}
        int y = cy + dy, x0 = cx - dx;
        if (y < HUD) y += H - HUD; else if (y >= H) y -= H - HUD;
        if (x0 < 0) { rect(x0 + W, y, -x0, 1, c); }
        if (x0 + 2 * dx + 1 > W) { rect(0, y, x0 + 2 * dx + 1 - W, 1, c); }
        rect(x0, y, 2 * dx + 1, 1, c);
    }
}

static Uint32 shade(Uint32 c, int num, int den)
{
    Uint32 r = ((c >> 16) & 255) * num / den, g = ((c >> 8) & 255) * num / den, b = (c & 255) * num / den;
    return (r > 255 ? 255 : r) << 16 | (g > 255 ? 255 : g) << 8 | (b > 255 ? 255 : b);
}

/* 5x7 glyphs, one byte per row, bit 4 is the left column. */
static const char glyphChars[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789:.-!/";
static const Uint8 glyphs[][7] = {
    {0x0E,0x11,0x11,0x1F,0x11,0x11,0x11},{0x1E,0x11,0x11,0x1E,0x11,0x11,0x1E},{0x0E,0x11,0x10,0x10,0x10,0x11,0x0E},
    {0x1C,0x12,0x11,0x11,0x11,0x12,0x1C},{0x1F,0x10,0x10,0x1E,0x10,0x10,0x1F},{0x1F,0x10,0x10,0x1E,0x10,0x10,0x10},
    {0x0E,0x11,0x10,0x17,0x11,0x11,0x0F},{0x11,0x11,0x11,0x1F,0x11,0x11,0x11},{0x0E,0x04,0x04,0x04,0x04,0x04,0x0E},
    {0x07,0x02,0x02,0x02,0x02,0x12,0x0C},{0x11,0x12,0x14,0x18,0x14,0x12,0x11},{0x10,0x10,0x10,0x10,0x10,0x10,0x1F},
    {0x11,0x1B,0x15,0x15,0x11,0x11,0x11},{0x11,0x11,0x19,0x15,0x13,0x11,0x11},{0x0E,0x11,0x11,0x11,0x11,0x11,0x0E},
    {0x1E,0x11,0x11,0x1E,0x10,0x10,0x10},{0x0E,0x11,0x11,0x11,0x15,0x12,0x0D},{0x1E,0x11,0x11,0x1E,0x14,0x12,0x11},
    {0x0F,0x10,0x10,0x0E,0x01,0x01,0x1E},{0x1F,0x04,0x04,0x04,0x04,0x04,0x04},{0x11,0x11,0x11,0x11,0x11,0x11,0x0E},
    {0x11,0x11,0x11,0x11,0x11,0x0A,0x04},{0x11,0x11,0x11,0x15,0x15,0x15,0x0A},{0x11,0x11,0x0A,0x04,0x0A,0x11,0x11},
    {0x11,0x11,0x0A,0x04,0x04,0x04,0x04},{0x1F,0x01,0x02,0x04,0x08,0x10,0x1F},
    {0x0E,0x11,0x13,0x15,0x19,0x11,0x0E},{0x04,0x0C,0x04,0x04,0x04,0x04,0x0E},{0x0E,0x11,0x01,0x02,0x04,0x08,0x1F},
    {0x1F,0x02,0x04,0x02,0x01,0x11,0x0E},{0x02,0x06,0x0A,0x12,0x1F,0x02,0x02},{0x1F,0x10,0x1E,0x01,0x01,0x11,0x0E},
    {0x06,0x08,0x10,0x1E,0x11,0x11,0x0E},{0x1F,0x01,0x02,0x04,0x08,0x08,0x08},{0x0E,0x11,0x11,0x0E,0x11,0x11,0x0E},
    {0x0E,0x11,0x11,0x0F,0x01,0x02,0x0C},
    {0x00,0x0C,0x0C,0x00,0x0C,0x0C,0x00},{0x00,0x00,0x00,0x00,0x00,0x0C,0x0C},{0x00,0x00,0x00,0x1F,0x00,0x00,0x00},
    {0x04,0x04,0x04,0x04,0x04,0x00,0x04},{0x01,0x01,0x02,0x04,0x08,0x10,0x10},
};

static int textWidth(const char *s, int scale) { return (int)strlen(s) * 6 * scale - scale; }

static void text(int x, int y, const char *s, int scale, Uint32 c)
{
    for (; *s; s++, x += 6 * scale) {
        const char *at = strchr(glyphChars, *s);
        int row, col;
        if (*s == ' ' || !at) continue;
        for (row = 0; row < 7; row++)
            for (col = 0; col < 5; col++)
                if (glyphs[at - glyphChars][row] & (0x10 >> col)) rect(x + col * scale, y + row * scale, scale, scale, c);
    }
}

static void centered(int y, const char *s, int scale, Uint32 c)
{
    /* A one-pixel shadow keeps the words readable over the field. */
    text((W - textWidth(s, scale)) / 2 + 1, y + 1, s, scale, 0x000000);
    text((W - textWidth(s, scale)) / 2, y, s, scale, c);
}

/* The craft, pointing up; the other directions are rotations of it. 1 hull, 2 cockpit, 3 wing tip. */
static const char *craftShape[7] = {
    "...1...",
    "..121..",
    "..121..",
    ".11111.",
    "1111111",
    "31.1.13",
    "3.....3",
};

static void craft(int cx, int cy, int dir, Uint32 hull)
{
    int i, j;
    for (j = 0; j < 7; j++)
        for (i = 0; i < 7; i++) {
            char c = craftShape[j][i];
            int x = i - 3, y = j - 3, rx, ry;
            if (c == '.') continue;
            switch (dir) {
            case DIR_RIGHT: rx = -y; ry = x; break;
            case DIR_DOWN: rx = -x; ry = -y; break;
            case DIR_LEFT: rx = y; ry = -x; break;
            default: rx = x; ry = y; break;
            }
            dot(cx + rx, cy + ry, c == '2' ? 0xffffff : c == '3' ? 0xff60c0 : hull);
        }
}

/* ---------------------------------------------------------------- the field */

static Uint32 starRng = 0x9e3779b9u;
static int stars[70][3];

static void initStars(void)
{
    int i;
    for (i = 0; i < 70; i++) {
        starRng ^= starRng << 13; starRng ^= starRng >> 17; starRng ^= starRng << 5;
        stars[i][0] = starRng % W;
        stars[i][1] = HUD + (starRng >> 9) % (H - HUD);
        stars[i][2] = (starRng >> 20) % 3;
    }
}

static void drawStars(int frame)
{
    int i;
    for (i = 0; i < 70; i++) {
        /* The far stars creep left, the near ones a little faster. */
        int x = ((stars[i][0] - frame * (stars[i][2] + 1) / 8) % W + W) % W;
        int tw = ((frame / 8 + i) % 23) == 0;
        dot(x, stars[i][1], tw ? 0xffffff : stars[i][2] == 2 ? 0x8890b8 : stars[i][2] == 1 ? 0x505878 : 0x30344c);
    }
}

static void edgeSpawn(Game *g, int *x, int *y, int *vx, int *vy, int speed)
{
    int side = range(g, 0, 3);
    int along = range(g, -speed, speed);
    switch (side) {
    case 0: *x = range(g, 0, W - 1) * FP; *y = HUD * FP; *vx = along; *vy = range(g, speed / 3, speed); break;
    case 1: *x = (W - 1) * FP; *y = range(g, HUD, H - 1) * FP; *vx = -range(g, speed / 3, speed); *vy = along; break;
    case 2: *x = range(g, 0, W - 1) * FP; *y = (H - 1) * FP; *vx = along; *vy = -range(g, speed / 3, speed); break;
    default: *x = 0; *y = range(g, HUD, H - 1) * FP; *vx = range(g, speed / 3, speed); *vy = along; break;
    }
}

static void wrap(int *x, int *y)
{
    if (*x < 0) *x += W * FP;
    if (*x >= W * FP) *x -= W * FP;
    if (*y < HUD * FP) *y += (H - HUD) * FP;
    if (*y >= H * FP) *y -= (H - HUD) * FP;
}

/* The shortest distance on the wrapping field, in px. */
static int delta(int a, int b, int span)
{
    int d = (a - b) / FP;
    if (d > span / 2) d -= span;
    if (d < -span / 2) d += span;
    return d;
}

static void burst(Game *g, int x, int y, int n, Uint32 c, int speed)
{
    int i;
    for (i = 0; i < MAXP && n; i++) {
        Particle *p = &g->p[i];
        if (p->life) continue;
        p->x = x; p->y = y;
        p->vx = range(g, -speed, speed);
        p->vy = range(g, -speed, speed);
        p->life = range(g, 15, 40);
        p->color = c;
        n--;
    }
}

static void spawnSpark(Game *g, Spark *s)
{
    edgeSpawn(g, &s->x, &s->y, &s->vx, &s->vy, 120);
    s->phase = range(g, 0, 63);
    s->alive = 1;
}

static int spawnDebris(Game *g, int r, int x, int y)
{
    int i, tries;
    for (i = 0; i < MAXD; i++) {
        Debris *d = &g->d[i];
        if (d->alive) continue;
        if (x < 0) {
            /* From an edge, and never right next to the craft. */
            for (tries = 0; tries < 8; tries++) {
                edgeSpawn(g, &d->x, &d->y, &d->vx, &d->vy, 100 + g->level * 25);
                if (abs(delta(d->x, g->px, W)) + abs(delta(d->y, g->py, H - HUD)) > 70) break;
            }
        } else {
            d->x = x; d->y = y;
            d->vx = range(g, -160 - g->level * 20, 160 + g->level * 20);
            d->vy = range(g, -160 - g->level * 20, 160 + g->level * 20);
        }
        d->r = r;
        d->grow = 0;
        d->alive = 1;
        return 1;
    }
    return 0;
}

static void newGame(Game *g)
{
    int i, best = g->best;
    Uint32 seed = g->rng;
    memset(g, 0, sizeof(*g));
    g->rng = seed ? seed : 1;
    g->best = best;
    g->state = PLAY;
    g->px = W / 2 * FP;
    g->py = (H + HUD) / 2 * FP;
    g->lives = 3;
    g->level = 1;
    g->dir = DIR_UP;
    for (i = 0; i < 4; i++) spawnSpark(g, &g->s[i]);
    for (i = 0; i < 3; i++) spawnDebris(g, range(g, 3, 6), -1, -1);
    g->banner = 90;
    sound(SND_START, 0);
}

static const char *dirNames[] = { "up", "right", "down", "left" };

/* One press of a direction key: a push, as the craft has no brakes but Space. */
static void steer(Game *g, int dir)
{
    const int push = 0x140, top = 0x380;
    if (g->state != PLAY || g->paused) return;
    g->dir = dir;
    if (dir == DIR_UP) g->pvy -= push;
    if (dir == DIR_DOWN) g->pvy += push;
    if (dir == DIR_LEFT) g->pvx -= push;
    if (dir == DIR_RIGHT) g->pvx += push;
    if (g->pvx > top) g->pvx = top;
    if (g->pvx < -top) g->pvx = -top;
    if (g->pvy > top) g->pvy = top;
    if (g->pvy < -top) g->pvy = -top;
    if (logMoves) {
        printf("[driftspark] steer dir=%s x=%d y=%d\n", dirNames[dir], g->px / FP, g->py / FP);
        fflush(stdout);
        g->watch = 20;
    }
}

static void togglePause(Game *g)
{
    if (g->state != PLAY) return;
    g->paused = !g->paused;
    sound(SND_PAUSE, 0);
    if (logMoves) { printf("[driftspark] pause %s\n", g->paused ? "on" : "off"); fflush(stdout); }
}

static void update(Game *g)
{
    int i, j, target;
    if (g->state == OVER) g->overFrames++;
    if (g->state != PLAY || g->paused) return;
    g->frame++;
    if (g->banner) g->banner--;
    if (g->invuln) g->invuln--;
    if (g->frame % 60 == 0) g->score += g->level;

    /* The craft drifts and slows a little each frame. */
    g->px += g->pvx; g->py += g->pvy;
    g->pvx -= g->pvx / 48; g->pvy -= g->pvy / 48;
    wrap(&g->px, &g->py);
    if (g->watch && --g->watch == 0 && logMoves) {
        printf("[driftspark] drift x=%d y=%d\n", g->px / FP, g->py / FP);
        fflush(stdout);
    }

    for (i = 0; i < MAXS; i++) {
        Spark *s = &g->s[i];
        if (!s->alive) continue;
        s->x += s->vx; s->y += s->vy; s->phase++;
        wrap(&s->x, &s->y);
        if (abs(delta(s->x, g->px, W)) <= 6 && abs(delta(s->y, g->py, H - HUD)) <= 6) {
            g->collected++;
            g->score += 10 * g->level;
            burst(g, s->x, s->y, 10, 0xffe070, 200);
            sound(SND_SPARK, g->collected % 8);
            spawnSpark(g, s);
            if (g->collected % 8 == 0) {
                g->level++;
                g->banner = 90;
                sound(SND_LEVEL, 0);
                if (g->level % 2 == 0) for (j = 0; j < MAXS; j++) if (!g->s[j].alive) { spawnSpark(g, &g->s[j]); break; }
            }
        }
    }

    for (i = 0; i < MAXD; i++) {
        Debris *d = &g->d[i];
        int dx, dy, reach;
        if (!d->alive) continue;
        d->x += d->vx; d->y += d->vy;
        wrap(&d->x, &d->y);
        /* Debris grows as it drifts, faster on later levels, and bursts at full size. */
        if (++d->grow >= 100 - (g->level < 9 ? g->level : 9) * 8) {
            d->grow = 0;
            if (++d->r > 13) {
                d->alive = 0;
                burst(g, d->x, d->y, 14, 0xb06040, 260);
                sound(SND_BURST, 0);
                spawnDebris(g, 4, d->x, d->y);
                spawnDebris(g, 4, d->x, d->y);
                continue;
            }
        }
        dx = delta(d->x, g->px, W);
        dy = delta(d->y, g->py, H - HUD);
        reach = d->r + 3;
        if (!g->invuln && dx * dx + dy * dy < reach * reach) {
            d->alive = 0;
            g->lives--;
            g->invuln = 120;
            burst(g, g->px, g->py, 30, 0x40e0ff, 320);
            burst(g, d->x, d->y, 20, 0xb06040, 260);
            sound(SND_HIT, 0);
            if (g->lives <= 0) {
                g->state = OVER;
                g->overFrames = 0;
                if (g->score > g->best) g->best = g->score;
                sound(SND_OVER, 0);
                if (logMoves) { printf("[driftspark] over score=%d level=%d\n", g->score, g->level); fflush(stdout); }
            }
        }
    }

    /* Keep the field stocked: more debris on each level. */
    target = 3 + g->level * 2;
    if (target > MAXD - 4) target = MAXD - 4;
    for (i = j = 0; i < MAXD; i++) j += g->d[i].alive;
    if (j < target && g->frame % (90 - (g->level < 8 ? g->level : 8) * 8) == 0) spawnDebris(g, 3, -1, -1);

    for (i = 0; i < MAXP; i++) {
        Particle *p = &g->p[i];
        if (!p->life) continue;
        p->x += p->vx; p->y += p->vy;
        p->life--;
    }
}

/* ---------------------------------------------------------------- screens */

static void drawField(Game *g)
{
    int i;
    char line[48];
    rect(0, 0, W, H, 0x070a18);
    drawStars(g->state == TITLE ? (int)(SDL_GetTicks() / 16) : g->frame);

    for (i = 0; i < MAXP; i++) {
        Particle *p = &g->p[i];
        if (!p->life) continue;
        dot(p->x / FP, p->y / FP, shade(p->color, p->life + 10, 50));
    }
    for (i = 0; i < MAXD; i++) {
        Debris *d = &g->d[i];
        int x = d->x / FP, y = d->y / FP;
        Uint32 c = d->r >= 11 ? 0xc04a32 : d->r >= 7 ? 0x9a6040 : 0x7a6a5c;
        if (!d->alive) continue;
        /* A full-size piece flickers just before it bursts. */
        if (d->r == 13 && (g->frame / 4) % 2) c = 0xff8040;
        disc(x, y, d->r, shade(c, 3, 5));
        disc(x - d->r / 4, y - d->r / 4, d->r * 3 / 4, c);
        if (d->r >= 6) { dot(x + d->r / 3, y, shade(c, 2, 5)); dot(x - d->r / 3, y + d->r / 3, shade(c, 2, 5)); }
    }
    for (i = 0; i < MAXS; i++) {
        Spark *s = &g->s[i];
        int x = s->x / FP, y = s->y / FP, arm = 2 + ((s->phase / 6) % 4 == 0);
        Uint32 c = (s->phase / 4) % 2 ? 0xffd040 : 0xfff0a0;
        if (!s->alive) continue;
        rect(x - arm, y, 2 * arm + 1, 1, c);
        rect(x, y - arm, 1, 2 * arm + 1, c);
        dot(x - 1, y - 1, 0xa08020); dot(x + 1, y + 1, 0xa08020); dot(x + 1, y - 1, 0xa08020); dot(x - 1, y + 1, 0xa08020);
        dot(x, y, 0xffffff);
    }
    if (g->state == PLAY || (g->state == OVER && g->overFrames < 20)) {
        int x = g->px / FP, y = g->py / FP;
        int speed = abs(g->pvx) + abs(g->pvy);
        Uint32 hull = g->invuln && (g->invuln / 4) % 2 ? 0x206878 : 0x40e0ff;
        if (g->state == PLAY) {
            /* A flame behind the craft while it moves fast. */
            if (speed > 0x100 && g->frame % 4 < 2) {
                int fx = x, fy = y;
                if (g->dir == DIR_UP) fy += 5;
                if (g->dir == DIR_DOWN) fy -= 5;
                if (g->dir == DIR_LEFT) fx += 5;
                if (g->dir == DIR_RIGHT) fx -= 5;
                rect(fx - 1, fy - 1, 3, 3, 0xff9020);
                dot(fx, fy, 0xffff80);
            }
            craft(x, y, g->dir, hull);
        }
    }

    if (g->state == TITLE) return;
    /* The top strip. */
    rect(0, 0, W, HUD, 0x141a34);
    rect(0, HUD - 1, W, 1, 0x2a3466);
    snprintf(line, sizeof line, "SCORE %06d", g->score);
    text(3, 3, line, 1, 0xe8ecff);
    snprintf(line, sizeof line, "LV %d", g->level);
    text(W / 2 - textWidth(line, 1) / 2, 3, line, 1, 0xffd040);
    for (i = 0; i < g->lives && i < 6; i++) craft(W - 8 - i * 10, 5, DIR_UP, 0x40e0ff);
}

static void dim(void)
{
    int i;
    for (i = W * HUD; i < W * H; i++) fb[i] = (fb[i] >> 1) & 0x7f7f7f;
}

static void render(Game *g, Uint32 ticks)
{
    char line[48];
    int blink = (ticks / 400) % 2 == 0;
    drawField(g);
    if (g->state == TITLE) {
        int i;
        centered(52, "DRIFTSPARK", 3, 0xffc040);
        for (i = 0; i < 9; i++) rect(W / 2 - 88 + i * 20, 80, 12, 2, i % 2 ? 0x40e0ff : 0xff60c0);
        centered(98, "STEER WITH ARROWS OR WASD", 1, 0xe8ecff);
        centered(110, "COLLECT SPARKS - DODGE DEBRIS", 1, 0xe8ecff);
        centered(122, "DEBRIS GROWS AND BURSTS", 1, 0xa0a8d0);
        centered(140, "SPACE BRAKE  P PAUSE  R RESTART", 1, 0xa0a8d0);
        centered(152, "ESC QUIT", 1, 0xa0a8d0);
        if (blink) centered(176, "PRESS SPACE TO LAUNCH", 1, 0xffd040);
        craft(W / 2, 210, DIR_UP, 0x40e0ff);
        if (g->best) { snprintf(line, sizeof line, "BEST %d", g->best); centered(224, line, 1, 0xa0a8d0); }
    } else if (g->state == OVER) {
        if (g->overFrames > 20) {
            dim();
            centered(72, "GAME OVER", 3, 0xff5040);
            snprintf(line, sizeof line, "SCORE %d", g->score);
            centered(110, line, 2, 0xe8ecff);
            snprintf(line, sizeof line, "BEST %d   LEVEL %d", g->best, g->level);
            centered(134, line, 1, 0xffd040);
            if (blink) centered(160, "SPACE OR R TO PLAY AGAIN", 1, 0xe8ecff);
            centered(174, "ESC TO QUIT", 1, 0xa0a8d0);
        }
    } else if (g->paused) {
        dim();
        centered(100, "PAUSED", 3, 0xe8ecff);
        centered(132, "P TO RESUME", 1, 0xa0a8d0);
    } else if (g->banner) {
        snprintf(line, sizeof line, "LEVEL %d", g->level);
        centered(60, line, 2, g->banner % 10 < 5 ? 0xffd040 : 0xfff0a0);
    }
}

/* ---------------------------------------------------------------- the demo pilot */

/* Scripted input for --demo: head for the nearest spark, push away from debris in
 * the way, and press a key at most every six frames, as a quick player would. */
static int pilot(Game *g, int frame)
{
    int i, best = 1 << 30, tx = 0, ty = 0, ax = 0, ay = 0;
    if (g->state == OVER) return g->overFrames == 90 ? 'r' : 0;
    if (frame == 400 || frame == 430) return 'p';
    if (frame % 6 || g->paused) return 0;
    for (i = 0; i < MAXS; i++) {
        int dx, dy;
        if (!g->s[i].alive) continue;
        dx = delta(g->s[i].x, g->px, W);
        dy = delta(g->s[i].y, g->py, H - HUD);
        if (dx * dx + dy * dy < best) { best = dx * dx + dy * dy; tx = dx; ty = dy; }
    }
    for (i = 0; i < MAXD; i++) {
        int dx, dy, near;
        if (!g->d[i].alive) continue;
        dx = delta(g->d[i].x, g->px, W);
        dy = delta(g->d[i].y, g->py, H - HUD);
        near = g->d[i].r + 22;
        if (dx * dx + dy * dy < near * near) { ax -= dx; ay -= dy; }
    }
    if (ax || ay) { tx = ax; ty = ay; }
    /* Steer on the axis that is further off, minding the craft's own speed. */
    tx -= g->pvx * 12 / FP;
    ty -= g->pvy * 12 / FP;
    if (abs(tx) < 4 && abs(ty) < 4) return 0;
    if (abs(tx) >= abs(ty)) return tx > 0 ? 'd' : 'a';
    return ty > 0 ? 's' : 'w';
}

/* ---------------------------------------------------------------- main */

static Uint32 checksum(void)
{
    /* FNV-1a over each pixel's red, green and blue. */
    Uint32 h = 2166136261u;
    int i;
    for (i = 0; i < W * H; i++) {
        h = (h ^ (fb[i] & 255)) * 16777619u;
        h = (h ^ ((fb[i] >> 8) & 255)) * 16777619u;
        h = (h ^ ((fb[i] >> 16) & 255)) * 16777619u;
    }
    return h;
}

/* Returns 1 to quit. */
static int press(Game *g, int key)
{
    switch (key) {
    case 27: case 'q': return 1;
    case 'w': steer(g, DIR_UP); break;
    case 's': steer(g, DIR_DOWN); break;
    case 'a': steer(g, DIR_LEFT); break;
    case 'd': steer(g, DIR_RIGHT); break;
    case 'p': togglePause(g); break;
    case 'r':
        if (g->state != TITLE) {
            if (logMoves) { printf("[driftspark] restart\n"); fflush(stdout); }
            newGame(g);
        }
        break;
    case ' ':
        if (g->state == TITLE || g->state == OVER) {
            if (logMoves) { printf("[driftspark] start\n"); fflush(stdout); }
            newGame(g);
        } else if (!g->paused) {
            g->pvx /= 4; g->pvy /= 4;
        }
        break;
    }
    return 0;
}

static int keyOf(SDL_Scancode sc)
{
    switch (sc) {
    case SDL_SCANCODE_UP: case SDL_SCANCODE_KP_8: case SDL_SCANCODE_W: return 'w';
    case SDL_SCANCODE_DOWN: case SDL_SCANCODE_KP_2: case SDL_SCANCODE_S: return 's';
    case SDL_SCANCODE_LEFT: case SDL_SCANCODE_KP_4: case SDL_SCANCODE_A: return 'a';
    case SDL_SCANCODE_RIGHT: case SDL_SCANCODE_KP_6: case SDL_SCANCODE_D: return 'd';
    case SDL_SCANCODE_P: return 'p';
    case SDL_SCANCODE_R: return 'r';
    case SDL_SCANCODE_SPACE: case SDL_SCANCODE_RETURN: return ' ';
    case SDL_SCANCODE_ESCAPE: return 27;
    case SDL_SCANCODE_Q: return 'q';
    default: return 0;
    }
}

static void present(SDL_Window *window)
{
    SDL_Surface *s = SDL_GetWindowSurface(window);
    if (!s) return;
    SDL_LockSurface(s);
    if (s->w >= W && s->h >= H) {
        SDL_ConvertPixels(W, H, SDL_PIXELFORMAT_RGB888, fb, W * 4, s->format->format, s->pixels, s->pitch);
    }
    SDL_UnlockSurface(s);
    SDL_UpdateWindowSurface(window);
}

int main(int argc, char **argv)
{
    SDL_Window *window;
    SDL_AudioSpec want, got;
    Game *g = &game;
    Uint32 seed = 1, frames = 0, start, ticks = 0, done = 0, quit = 0;

    if (argc >= 4 && strcmp(argv[1], "--demo") == 0) {
        demo = 1;
        seed = (Uint32)strtoul(argv[2], NULL, 10);
        frames = (Uint32)strtoul(argv[3], NULL, 10);
        if (!frames) { printf("[driftspark] usage: driftspark [--demo SEED FRAMES]\n"); return 2; }
    } else if (argc > 1) {
        printf("[driftspark] usage: driftspark [--demo SEED FRAMES]\n");
        return 2;
    }
    logMoves = !demo;

    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_TIMER) != 0) {
        printf("[driftspark] SDL_Init failed: %s\n", SDL_GetError());
        return 1;
    }
    window = SDL_CreateWindow("Driftspark", SDL_WINDOWPOS_UNDEFINED, SDL_WINDOWPOS_UNDEFINED, W, H, 0);
    if (!window) {
        printf("[driftspark] SDL_CreateWindow failed: %s\n", SDL_GetError());
        return 1;
    }
    SDL_zero(want);
    want.freq = RATE;
    want.format = AUDIO_S16SYS;
    want.channels = 2;
    want.samples = 1024;
    want.callback = mix;
    audio = SDL_OpenAudioDevice(NULL, 0, &want, &got, 0);
    if (audio) SDL_PauseAudioDevice(audio, 0);
    else printf("[driftspark] no sound: %s\n", SDL_GetError());

    initStars();
    memset(g, 0, sizeof(*g));
    g->state = TITLE;
    if (demo) {
        g->rng = seed * 2654435761u + 1;
        newGame(g);
    } else {
        g->rng = SDL_GetTicks() * 2654435761u + 7;
    }
    printf("[driftspark] ready mode=%s w=%d h=%d sound=%s\n", demo ? "demo" : "play", W, H, audio ? "on" : "off");
    fflush(stdout);

    start = SDL_GetTicks();
    while (!quit) {
        SDL_Event e;
        Uint32 now, steps = 0;
        while (SDL_PollEvent(&e)) {
            if (e.type == SDL_QUIT) quit = 1;
            else if (e.type == SDL_KEYDOWN && !demo) {
                int k = keyOf(e.key.keysym.scancode);
                if (k && press(g, k)) quit = 1;
            } else if (e.type == SDL_KEYDOWN && keyOf(e.key.keysym.scancode) == 27) quit = 1;
        }
        if (quit) break;
        if (demo) {
            int k = pilot(g, (int)done);
            if (k) press(g, k);
            update(g);
            done++;
            render(g, done * 1000 / 60);
            present(window);
            if (done == frames) break;
            /* 60 frames a second, on SDL_GetTicks. */
            now = SDL_GetTicks();
            if (now - start < done * 1000 / 60) SDL_Delay(start + done * 1000 / 60 - now);
            continue;
        }
        now = SDL_GetTicks();
        while (now - start >= ticks * 1000 / 60 && steps < 5) { update(g); ticks++; steps++; }
        if (now - start >= ticks * 1000 / 60) { start = now; ticks = 0; } /* too far behind: catch up no more */
        if (steps) {
            render(g, now);
            present(window);
        } else {
            SDL_Delay(start + ticks * 1000 / 60 - now);
        }
    }

    if (demo) {
        if (done == frames) {
            printf("[driftspark] demo seed=%u frames=%u score=%d level=%d lives=%d sparks=%d\n", (unsigned)seed,
                   (unsigned)frames, g->score, g->level, g->lives, g->collected);
            printf("[driftspark] demo checksum=%08x\n", (unsigned)checksum());
        } else {
            printf("[driftspark] demo stopped at frame %u\n", (unsigned)done);
        }
    } else {
        printf("[driftspark] quit score=%d best=%d\n", g->score, g->best);
    }
    fflush(stdout);
    if (audio) SDL_CloseAudioDevice(audio);
    SDL_DestroyWindow(window);
    SDL_Quit();
    return demo && done != frames ? 1 : 0;
}
