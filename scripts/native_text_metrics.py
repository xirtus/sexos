# SPDX-License-Identifier: GPL-3.0-only
"""Bind native pixel checks to the approved shared logical text metrics."""
from pathlib import Path
import hashlib
from functools import lru_cache
import re


def source_metrics(root=None):
    root = Path(root) if root is not None else Path(__file__).resolve().parents[1]
    source = (root / 'crates/sex-pdx/src/desktop_style.rs').read_text()
    approved = dict(title=26, inset=8, resize=8, advance=6, line=9,
                    glyph_width=5, glyph_height=7)
    result = {}
    for key, expected in approved.items():
        values = re.findall(r'pub const ' + key.upper() + r'\s*:\s*u32\s*=\s*(\d+)\s*;', source)
        if values != [str(expected)]:
            raise ValueError('approved native text metric changed: ' + key)
        result[key] = int(values[0])
    compact = re.sub(r'\s+', '', source)
    for required in ('pubconstfntitle(self)->u32{self.px(TITLE)}',
                     'pubconstfnpx(self,logical:u32)->u32{logical.saturating_mul(self.scale)}',
                     'letside=self.px(RESIZE);',
                     'lettop=self.title().saturating_add(side);',
                     'w:width.saturating_sub(side.saturating_mul(2))',
                     'h:height.saturating_sub(top.saturating_add(side))',
                     'ifwidth>=256{2}else{1}'):
        if required not in compact:
            raise ValueError('shared native text metric implementation changed')
    result.update(top=result['title'] + result['inset'], scale_threshold=256)
    return result


def font_sources(root):
    """Production font inputs and the independent host atlas generator."""
    root = Path(root)
    display = re.sub(r'\s+', '', (root / 'servers/sexdisplay/src/main.rs').read_text())
    style = re.sub(r'\s+', '', (root / 'crates/sex-pdx/src/desktop_style.rs').read_text())
    glyphs = re.sub(r'\s+', '', (root / 'servers/sexdisplay/src/glyphs.rs').read_text())
    for required in ('glyphs::mono_cell(scalar,advance*span,line_h)',
                     'coverage_blend(surf.text_color,background.unwrap_or(surf.color),alpha)',
                     'letcolor=ifsurface_id&1==0{0x00303860u32}else{0x00704890u32};',
                     'letcolor=sex_pdx::desktop_style::native_canvas(owner,surface_id).unwrap_or(color);',
                     'letmix=|shift:u32|((((fg>>shift)&0xFF)*a+((bg>>shift)&0xFF)*inv)/255)<<shift;'):
        if required not in display:
            raise ValueError('source-selected native text raster/background changed')
    if '!crate::app_catalog::is_browser(owner){Some(NATIVE_CANVAS)}else{None}' not in style:
        raise ValueError('Browser no longer owns the approved native canvas')
    if ('Font::new(GEIST_MONO)?' not in glyphs or
        'mono().is_some_and(|font|sex_font::draw_cell(font,scalar,w,h,acc(),out))' not in glyphs):
        raise ValueError('Display no longer uses the bound Mono font raster')
    names = {'Cargo.toml', 'Cargo.lock', 'crates/sex-font/Cargo.toml',
             'crates/sex-font/examples/cells.rs', 'servers/sexdisplay/src/glyphs.rs',
             'servers/sexdisplay/src/main.rs', 'crates/sex-pdx/src/desktop_style.rs',
             'crates/sex-pdx/src/app_catalog.rs',
             'assets/fonts/geist/GeistMono-Regular.ttf', 'assets/fonts/geist/Geist-Regular.ttf'}
    names.update(str(p.relative_to(root)) for p in (root / 'crates/sex-font/src').rglob('*.rs'))
    names.update(str(p.relative_to(root)) for p in (root / 'assets/fonts').rglob('*')
                 if p.is_file() and p.suffix in ('.ttf', '.otf', '.ttc'))
    return {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in sorted(names)}


def cell_rgb(contract, char, scale):
    """Every coverage pixel, including padding, with the exact integer blend."""
    from check_desktop_pixels import cell_coverage
    coverage = cell_coverage(contract['font'], char, scale)
    foreground = contract['color'].to_bytes(3, 'big')
    background = contract['background'].to_bytes(3, 'big')
    return _blend_cell(coverage, foreground, background)


@lru_cache(maxsize=2048)
def _blend_cell(coverage, foreground, background):
    return bytes((fg * alpha + bg * (255 - alpha)) // 255
                 for alpha in coverage for fg, bg in zip(foreground, background))
