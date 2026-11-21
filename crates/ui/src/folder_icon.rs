//! Premium folder icon: back panel + front flap that swings open with
//! `open_progress`, with the assigned glyph sitting on the flap like a cover.
//! The whole thing is built as one SVG per (glyph, animation step, hover)
//! and cached, so animation frames after the first pass are just blits.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use windows::core::w;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, SelectObject, CLEARTYPE_QUALITY, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_SEMIBOLD, HDC,
};

use crate::icon_picker::icon_body;
use crate::sidebar::{ease_in_out, make_font, rc, rgb, text, Canvas};

/// Quantisation of the open/close animation (cached frames per glyph).
const STEPS: u32 = 12;
/// Raster size of the cached folder image (downsampled with box filtering).
const RENDER_PX: u32 = 64;

type CacheKey = (String, u32, bool);
static CACHE: OnceLock<RwLock<HashMap<CacheKey, Arc<Vec<u8>>>>> = OnceLock::new();

/// Polygon with quadratic-rounded corners.
fn rounded_poly(pts: &[(f32, f32)], r: f32) -> String {
    let n = pts.len();
    let mut d = String::new();
    for i in 0..n {
        let p = pts[i];
        let a = pts[(i + n - 1) % n];
        let b = pts[(i + 1) % n];
        let (v1x, v1y) = (a.0 - p.0, a.1 - p.1);
        let (v2x, v2y) = (b.0 - p.0, b.1 - p.1);
        let l1 = (v1x * v1x + v1y * v1y).sqrt().max(0.001);
        let l2 = (v2x * v2x + v2y * v2y).sqrt().max(0.001);
        let d1 = r.min(l1 / 2.0);
        let d2 = r.min(l2 / 2.0);
        let s = (p.0 + v1x / l1 * d1, p.1 + v1y / l1 * d1);
        let e = (p.0 + v2x / l2 * d2, p.1 + v2y / l2 * d2);
        if i == 0 {
            d.push_str(&format!("M{:.2} {:.2}", s.0, s.1));
        } else {
            d.push_str(&format!("L{:.2} {:.2}", s.0, s.1));
        }
        d.push_str(&format!("Q{:.2} {:.2} {:.2} {:.2}", p.0, p.1, e.0, e.1));
    }
    d.push('Z');
    d
}

fn build_svg(step: u32, glyph_body: Option<&str>, hover: bool) -> String {
    let p = step as f32 / STEPS as f32;

    let (stroke, back_a, back_b, flap_a, flap_b, glyph_col) = if hover {
        ("#ffffff", "#363b4e", "#2a2e3e", "#59607a", "#3f455b", "#ffffff")
    } else {
        ("#aab1c6", "#2d3141", "#222633", "#454c62", "#323748", "#e8ebf4")
    };

    // Back panel with tab (static).
    let back = "M5.5 3H9a2 2 0 0 1 1.5.68L12 5.5h6.5A2.5 2.5 0 0 1 21 8v10.5a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 18.5v-13A2.5 2.5 0 0 1 5.5 3z";

    // Front flap: rectangle when closed, perspective open flap swinging forward towards viewer.
    // Bottom stays aligned with back panel base (bl = 2.8, br = 21.2) so back panel NEVER protrudes.
    let tl = (3.0 - 1.2 * p, 9.0 + 2.0 * p);
    let tr = (21.0 + 1.2 * p, 9.0 + 2.0 * p);
    let br = (21.0 + 0.2 * p, 21.0);
    let bl = (3.0 - 0.2 * p, 21.0);
    let flap = rounded_poly(&[tl, tr, br, bl], 2.5);

    // Glyph rides on the flap and scales with it.
    let glyph = match glyph_body {
        Some(body) => {
            let cx = (tl.0 + tr.0 + br.0 + bl.0) / 4.0;
            let cy = (tl.1 + bl.1) / 2.0;
            let sc = 0.46 - 0.04 * p;
            let body = body.replace("__COLOR__", glyph_col);
            format!(
                r#"<g transform="translate({cx:.2} {cy:.2}) scale({sc:.3}) translate(-12 -12)">{body}</g>"#
            )
        }
        None => String::new(),
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{px}" height="{px}" viewBox="0 0 24 24"><defs><linearGradient id="b" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{back_a}"/><stop offset="1" stop-color="{back_b}"/></linearGradient><linearGradient id="f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{flap_a}"/><stop offset="1" stop-color="{flap_b}"/></linearGradient></defs><path d="{back}" fill="url(#b)" stroke="{stroke}" stroke-width="1.3" stroke-linejoin="round"/><path d="{flap}" fill="url(#f)" stroke="{stroke}" stroke-width="1.3" stroke-linejoin="round"/>{glyph}</svg>"##,
        px = RENDER_PX
    )
}

fn render(svg: &str) -> Vec<u8> {
    let opt = resvg::usvg::Options::default();
    let Ok(tree) = resvg::usvg::Tree::from_str(svg, &opt) else {
        return Vec::new();
    };
    let Some(mut pm) = resvg::tiny_skia::Pixmap::new(RENDER_PX, RENDER_PX) else {
        return Vec::new();
    };
    let sx = RENDER_PX as f32 / tree.size().width();
    let sy = RENDER_PX as f32 / tree.size().height();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(sx, sy),
        &mut pm.as_mut(),
    );
    crate::icons::straight_rgba(&pm)
}

fn cached_folder(glyph_key: &str, glyph_body: Option<&str>, step: u32, hover: bool) -> Arc<Vec<u8>> {
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let key: CacheKey = (glyph_key.to_string(), step, hover);
    if let Ok(c) = cache.read() {
        if let Some(v) = c.get(&key) {
            return v.clone();
        }
    }
    let img = Arc::new(render(&build_svg(step, glyph_body, hover)));
    if let Ok(mut c) = cache.write() {
        c.insert(key, img.clone());
    }
    img
}

pub fn draw_folder_icon(
    cv: &Canvas,
    dc: HDC,
    r: &RECT,
    _is_open: bool,
    open_progress: f32,
    icon_str: &str,
    is_hover: bool,
    px: impl Fn(f32) -> i32,
) {
    let eased = ease_in_out(open_progress);
    let step = ((eased * STEPS as f32).round() as u32).min(STEPS);

    let trimmed = icon_str.trim();
    let has_glyph = !trimmed.is_empty() && trimmed != "📁" && trimmed != "folder";
    let body = if has_glyph { icon_body(trimmed) } else { None };
    let glyph_key = if body.is_some() { trimmed } else { "" };

    let img = cached_folder(glyph_key, body, step, is_hover);
    if !img.is_empty() {
        cv.draw_image_scaled(r, &img, RENDER_PX, RENDER_PX);
    }

    // Unknown symbol / emoji with no vector glyph: draw as text on the flap.
    if has_glyph && body.is_none() {
        let h = r.bottom - r.top;
        let area = rc(r.left, r.top + h * 4 / 10, r.right, r.bottom);
        unsafe {
            let f = make_font(w!("Segoe UI Symbol"), px(10.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
            let old = SelectObject(dc, f.into());
            text(
                dc,
                trimmed,
                area,
                DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                if is_hover { rgb(255, 255, 255) } else { rgb(232, 235, 244) },
            );
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(f.into());
        }
    }
}
