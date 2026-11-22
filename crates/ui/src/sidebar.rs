use std::cell::Cell;
use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, GdiFlush, GetTextExtentPoint32W, IntersectClipRect, SelectClipRgn, SelectObject, SetBkMode,
    SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FONT_QUALITY, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT, OUT_DEFAULT_PRECIS,
    SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::IsZoomed;

pub use crate::folder_icon::draw_folder_icon;

/// Reserved `TabItem::folder_id` value: the tab lives in the Pinned Tabs section
/// (under the folders) instead of a real folder. Persisted through `folder_id`.
pub const PINNED_SECTION_ID: &str = "__pinned__";

/// Smooth in/out easing for open/close animations (input and output are 0..=1).
pub fn ease_in_out(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    if p < 0.5 {
        4.0 * p * p * p
    } else {
        1.0 - (-2.0 * p + 2.0).powi(3) / 2.0
    }
}

// ───────────────────────────── Public types ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionButton {
    None,
    Minimize,
    Maximize,
    Close,
}

/// Everything the sidebar can be hovering / clicking on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitePermissionKind {
    Notifications,
    Camera,
    Microphone,
    Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarHit {
    None,
    Caption(CaptionButton),
    Back,
    Forward,
    Reload,
    ToggleSidebar,
    CleanLinkBtn,
    SiteSettings,
    PermissionIcon(usize),
    UrlBar,
    ZoomReset,
    Shield,
    Pinned(usize),
    Folder(usize),
    FolderIcon(usize),
    Tab(usize),
    TabClose(usize),
    NewTab,
    Downloads,
    Settings,
    JumpToActiveTab,
}

#[derive(Debug, Clone)]
pub struct FolderItem {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub is_open: bool,
    pub open_progress: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineEditTarget {
    Folder(String),
    Tab(usize),
    PinnedLetter(usize),
}

#[derive(Debug, Clone)]
pub struct InlineEditState {
    pub target: InlineEditTarget,
    pub text: String,
    pub cursor: usize,
    pub selection: Option<(usize, usize)>,
    pub caret_visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Pinned(usize),
    Tab(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragSource {
    Tab(usize),
    Pinned(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragTarget {
    Favorites(usize),
    Folder(usize),
    FolderInsert { folder_idx: usize, insert_idx: usize },
    PinnedSection(usize),
    NormalTabs(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabDropDestination {
    Favorites(usize),
    Folder(String),
    FolderInsert { folder_id: String, insert_idx: usize },
    PinnedSection(usize),
    NormalTabs(usize),
}

#[derive(Debug, Clone)]
pub struct DragCarriedItem {
    pub title: String,
    pub url: String,
    pub accent: COLORREF,
    pub favicon: Option<std::sync::Arc<FaviconData>>,
    pub is_loaded: bool,
    pub custom_letter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DragState {
    pub source: DragSource,
    pub item: DragCarriedItem,
    pub start_x: i32,
    pub start_y: i32,
    pub cur_x: i32,
    pub cur_y: i32,
    pub target: Option<DragTarget>,
    pub folder_hover_start: Option<(usize, std::time::Instant)>,
}

#[derive(Debug, Clone, Copy)]
pub struct DragCandidate {
    pub source: DragSource,
    pub start_x: i32,
    pub start_y: i32,
}

#[derive(Debug, Clone)]
pub struct FaviconData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

fn extract_largest_ico_image(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 6 || bytes[0] != 0 || bytes[1] != 0 || bytes[2] != 1 || bytes[3] != 0 {
        return None;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 {
        return None;
    }

    let mut best_idx = 0;
    let mut best_score = 0usize;

    for i in 0..count {
        let entry_offset = 6 + i * 16;
        if entry_offset + 16 > bytes.len() {
            break;
        }
        let w = match bytes[entry_offset] { 0 => 256usize, v => v as usize };
        let h = match bytes[entry_offset + 1] { 0 => 256usize, v => v as usize };
        let bpp = u16::from_le_bytes([bytes[entry_offset + 6], bytes[entry_offset + 7]]) as usize;
        let score = w * h * 100 + bpp;
        if score > best_score {
            best_score = score;
            best_idx = i;
        }
    }

    let entry_offset = 6 + best_idx * 16;
    let dw_size = u32::from_le_bytes([
        bytes[entry_offset + 8], bytes[entry_offset + 9],
        bytes[entry_offset + 10], bytes[entry_offset + 11],
    ]) as usize;
    let dw_offset = u32::from_le_bytes([
        bytes[entry_offset + 12], bytes[entry_offset + 13],
        bytes[entry_offset + 14], bytes[entry_offset + 15],
    ]) as usize;

    if dw_offset + dw_size > bytes.len() {
        return None;
    }

    let sub_bytes = &bytes[dw_offset..dw_offset + dw_size];
    if sub_bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(sub_bytes.to_vec());
    }

    let mut standalone = Vec::with_capacity(6 + 16 + dw_size);
    standalone.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    let mut entry = [0u8; 16];
    entry.copy_from_slice(&bytes[entry_offset..entry_offset + 16]);
    entry[12..16].copy_from_slice(&22u32.to_le_bytes());
    standalone.extend_from_slice(&entry);
    standalone.extend_from_slice(sub_bytes);
    Some(standalone)
}

pub fn decode_favicon(bytes: &[u8]) -> Option<std::sync::Arc<FaviconData>> {
    let to_decode = extract_largest_ico_image(bytes).unwrap_or_else(|| bytes.to_vec());
    if let Ok(img) = image::load_from_memory(&to_decode).or_else(|_| image::load_from_memory(bytes)) {
        let (orig_w, orig_h) = (img.width(), img.height());
        let resized = if orig_w > 256 || orig_h > 256 {
            img.resize(256, 256, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let rgba = resized.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w > 0 && h > 0 {
            return Some(std::sync::Arc::new(FaviconData {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            }));
        }
    }
    None
}

/// Renders the official Boxicons sidebar dock SVG icon:
/// <svg viewBox="0 0 24 24"><path d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2M4 6h8v12H4zm10 12V6h6v12z"/><path d="M16 12h2v2h-2zm0-4h2v2h-2z"/></svg>
pub fn draw_sidebar_icon(cv: &Canvas, r: &RECT, color: COLORREF, a: f32, px: impl Fn(f32) -> i32) {
    let cx = (r.left + r.right) / 2;
    let cy = (r.top + r.bottom) / 2;
    let s = px(1.0) as f32;
    let total_w = (20.0 * s).round() as i32;
    let total_h = (16.0 * s).round() as i32;
    let ox = cx - total_w / 2;
    let oy = cy - total_h / 2;
    let outer_r = rc(ox, oy, ox + total_w, oy + total_h);

    let border_w = (1.8 * s).max(1.8);
    let radius = (3.5 * s).round() as i32;

    // 1. Outer rounded rectangle (window frame)
    cv.stroke_round(&outer_r, radius, color, a, border_w);

    // 2. Vertical panel divider separating left sidebar from right content
    let div_w = border_w.round().max(1.0) as i32;
    let div_x = ox + (total_w * 7 / 16);
    let top_y = oy + border_w.round() as i32;
    let bot_y = oy + total_h - border_w.round() as i32;
    cv.fill_rect(&rc(div_x, top_y, div_x + div_w.max(1), bot_y), color, a);

    // 3. Two content indicator marks inside the right pane
    let dot_w = (3.5 * s).max(3.0).round() as i32;
    let dot_h = border_w.round().max(1.0) as i32;
    let dot_x = ox + (total_w * 11 / 16);
    let dot1_y = oy + (total_h * 4 / 13);
    let dot2_y = oy + (total_h * 8 / 13);

    cv.fill_rect(&rc(dot_x, dot1_y, dot_x + dot_w, dot1_y + dot_h), color, a);
    cv.fill_rect(&rc(dot_x, dot2_y, dot_x + dot_w, dot2_y + dot_h), color, a);
}

pub const LINK_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>"##;

pub const TUNE_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="8" x2="21" y2="8"/><circle cx="9" cy="8" r="2.6" fill="#1b1b1f" stroke="__COLOR__" stroke-width="2"/><line x1="3" y1="16" x2="21" y2="16"/><circle cx="15" cy="16" r="2.6" fill="#1b1b1f" stroke="__COLOR__" stroke-width="2"/></svg>"##;

pub const BELL_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>"##;

pub const CAMERA_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z"/><circle cx="12" cy="13" r="3"/></svg>"##;

pub const MIC_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>"##;

pub const LOCATION_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0Z"/><circle cx="12" cy="10" r="3"/></svg>"##;

pub fn draw_svg_icon(cv: &Canvas, r: &RECT, svg: &str, color_hex: &str) {
    let w = (r.right - r.left).max(1) as u32;
    let h = (r.bottom - r.top).max(1) as u32;
    let colored = svg.replace("__COLOR__", color_hex);
    let opt = resvg::usvg::Options::default();
    if let Ok(rtree) = resvg::usvg::Tree::from_str(&colored, &opt) {
        if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(w, h) {
            let sx = w as f32 / rtree.size().width();
            let sy = h as f32 / rtree.size().height();
            let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);
            resvg::render(&rtree, transform, &mut pixmap.as_mut());
            cv.draw_image_scaled(r, pixmap.data(), w, h);
        }
    }
}

/// Renders the sleek settings/sliders/tune icon in the search bar matching modern browser omniboxes.
pub fn draw_tune_icon(cv: &Canvas, r: &RECT, color: COLORREF, a: f32, s: f32) {
    let cx = (r.left + r.right) as f32 / 2.0;
    let cy = (r.top + r.bottom) as f32 / 2.0;

    let dy = 3.6 * s;
    let y_top = cy - dy;
    let y_bot = cy + dy;

    let offset_x = 4.2 * s;
    let cx_top = cx - offset_x;
    let cx_bot = cx + offset_x;

    let r_out = 2.4 * s;
    let r_in = 0.9 * s;
    let r_mid = (r_out + r_in) * 0.5;
    let half_thick = (r_out - r_in) * 0.5;

    // 1. Ring for top circle (on left)
    let min_x_top = (cx_top - r_out - 1.0).floor() as i32;
    let max_x_top = (cx_top + r_out + 1.0).ceil() as i32;
    let min_y_top = (y_top - r_out - 1.0).floor() as i32;
    let max_y_top = (y_top + r_out + 1.0).ceil() as i32;

    for y in min_y_top..=max_y_top {
        for x in min_x_top..=max_x_top {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dist = ((px - cx_top) * (px - cx_top) + (py - y_top) * (py - y_top)).sqrt();
            let d_ring = (dist - r_mid).abs();
            let cov = (half_thick + 0.5 - d_ring).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(x, y, color, a * cov);
            }
        }
    }

    // 2. Horizontal line for top row (extending right from top circle)
    let line_stroke = (1.5 * s).max(1.4);
    cv.draw_line_aa(cx_top + r_out - 0.5 * s, y_top, cx + 5.8 * s, y_top, color, a, line_stroke);

    // 3. Horizontal line for bottom row (from left up to bottom circle)
    cv.draw_line_aa(cx - 5.8 * s, y_bot, cx_bot - r_out + 0.5 * s, y_bot, color, a, line_stroke);

    // 4. Ring for bottom circle (on right)
    let min_x_bot = (cx_bot - r_out - 1.0).floor() as i32;
    let max_x_bot = (cx_bot + r_out + 1.0).ceil() as i32;
    let min_y_bot = (y_bot - r_out - 1.0).floor() as i32;
    let max_y_bot = (y_bot + r_out + 1.0).ceil() as i32;

    for y in min_y_bot..=max_y_bot {
        for x in min_x_bot..=max_x_bot {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dist = ((px - cx_bot) * (px - cx_bot) + (py - y_bot) * (py - y_bot)).sqrt();
            let d_ring = (dist - r_mid).abs();
            let cov = (half_thick + 0.5 - d_ring).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(x, y, color, a * cov);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TabItem {
    pub id: String,
    pub title: String,
    pub url: String,
    /// Favicon accent colour.
    pub accent: COLORREF,
    /// Real website favicon if fetched.
    pub favicon: Option<std::sync::Arc<FaviconData>>,
    /// Whether the tab webview is loaded/running or dormant/closed.
    pub is_loaded: bool,
    /// Whether the tab audio is muted.
    pub is_muted: bool,
    /// Animation progress from 0.0 (collapsed) to 1.0 (fully expanded).
    pub anim_progress: f32,
    /// Whether this tab is currently closing (animating down to 0).
    pub is_closing: bool,
    /// Optional folder ID if this tab belongs to a folder.
    pub folder_id: Option<String>,
    /// Optional single capital custom letter assigned to the tab.
    pub custom_letter: Option<String>,
}

impl TabItem {
    pub fn new(id: String, title: String, url: String, accent: COLORREF) -> Self {
        Self {
            id,
            title,
            url,
            accent,
            favicon: None,
            is_loaded: false,
            is_muted: false,
            anim_progress: 1.0,
            is_closing: false,
            folder_id: None,
            custom_letter: None,
        }
    }
}

pub struct SidebarState {
    pub pinned: Vec<TabItem>,
    pub folders: Vec<FolderItem>,
    pub tabs: Vec<TabItem>,
    pub active: Option<ActiveTab>,
    pub hovered: SidebarHit,
    pub inline_edit: Option<InlineEditState>,
    /// Vertical scroll of the tab list, in physical pixels (>= 0).
    pub scroll: i32,
    /// Vertical scroll of the top section (folders + pinned tabs), physical pixels.
    pub scroll_top: i32,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub zoom: f64,
    pub toast: Option<String>,
    /// Regular-tab indices currently multi-selected (Ctrl+click).
    pub selected: Vec<usize>,
    /// Regular-tab indices of the tabs shown together in a split view.
    pub split_group: Option<(usize, usize)>,
    /// Whether there are closed tabs available to reopen (Ctrl+Shift+T).
    pub has_closed_tabs: bool,
    pub shield_enabled: bool,
    pub shield_blocked_count: u64,
    pub shield_paused_site: bool,
    pub pending_permissions: Vec<SitePermissionKind>,
    pub is_mouse_over_sidebar: bool,
    pub drag_state: Option<DragState>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            folders: Vec::new(),
            tabs: Vec::new(),
            active: None,
            hovered: SidebarHit::None,
            inline_edit: None,
            scroll: 0,
            scroll_top: 0,
            can_go_back: false,
            can_go_forward: false,
            zoom: 1.0,
            toast: None,
            selected: Vec::new(),
            split_group: None,
            has_closed_tabs: false,
            shield_enabled: true,
            shield_blocked_count: 0,
            shield_paused_site: false,
            pending_permissions: Vec::new(),
            is_mouse_over_sidebar: false,
            drag_state: None,
        }
    }
}

// ───────────────────────────── Theme & Web Apps ─────────────────────────────

pub fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebAppType {
    YouTube,
    Google,
    Gmail,
    GitHub,
    Zen,
    Reddit,
    Twitter,
    Generic,
}

pub fn detect_webapp(url: &str, title: &str) -> WebAppType {
    let domain = domain_of(url).to_lowercase();
    let u = url.to_lowercase();
    let t = title.to_lowercase();
    if u.contains("youtube.com") || u.contains("youtu.be") || t.contains("youtube") {
        WebAppType::YouTube
    } else if domain == "gemini.google.com" || u.contains("gemini.google.com") || t.contains("gemini") {
        WebAppType::Generic
    } else if domain == "mail.google.com" || u.contains("mail.google.com") || t.contains("gmail") {
        WebAppType::Gmail
    } else if domain == "google.com" || domain == "www.google.com" || (t == "google" || t.starts_with("google -")) {
        WebAppType::Google
    } else if u.contains("github.com") || t.contains("github") {
        WebAppType::GitHub
    } else if u.contains("zen-browser") || u.contains("zenbrowser") || t.contains("zen browser") || t.contains("zen") {
        WebAppType::Zen
    } else if u.contains("reddit.com") || t.contains("reddit") {
        WebAppType::Reddit
    } else if u.contains("x.com") || u.contains("twitter.com") || t.contains("twitter") {
        WebAppType::Twitter
    } else {
        WebAppType::Generic
    }
}

pub fn accent_for_url(url: &str) -> COLORREF {
    let app = detect_webapp(url, "");
    match app {
        WebAppType::YouTube => rgb(255, 0, 0),       // YouTube Red
        WebAppType::Google => rgb(66, 133, 244),     // Google Blue
        WebAppType::Gmail => rgb(234, 67, 53),       // Gmail Red
        WebAppType::GitHub => rgb(36, 41, 46),       // GitHub Slate
        WebAppType::Zen => rgb(235, 110, 80),        // Zen Coral
        WebAppType::Reddit => rgb(255, 69, 0),       // Reddit Orange
        WebAppType::Twitter => rgb(29, 155, 240),    // Twitter Blue
        WebAppType::Generic => {
            let lower = url.to_lowercase();
            let mut hash: u32 = 5381;
            for b in lower.bytes() {
                hash = ((hash << 5).wrapping_add(hash)).wrapping_add(b as u32);
            }
            let colors = [
                rgb(112, 134, 255), // Indigo
                rgb(72, 187, 120),  // Emerald
                rgb(236, 114, 158), // Pink
                rgb(139, 92, 246),  // Purple
                rgb(245, 158, 11),  // Amber
                rgb(14, 165, 233),  // Sky
            ];
            colors[(hash as usize) % colors.len()]
        }
    }
}

pub fn friendly_title_for_url(url: &str) -> String {
    let host = host_of(url);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() || host == "about:blank" {
        return "New Tab".to_string();
    }
    if host.contains("youtube.com") {
        "YouTube".to_string()
    } else if host.contains("google.com") {
        "Google".to_string()
    } else if host.contains("github.com") {
        "GitHub".to_string()
    } else if host.contains("zen-browser") {
        "Zen Browser".to_string()
    } else {
        let mut c = host.chars();
        match c.next() {
            None => "New Tab".to_string(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }
}

pub fn extract_palette_from_favicon(fav: &FaviconData) -> Option<[COLORREF; 4]> {
    if fav.rgba.len() < 4 || fav.width == 0 || fav.height == 0 {
        return None;
    }

    let total_pixels = (fav.width * fav.height) as usize;
    let step = (total_pixels / 512).max(1);

    struct ColorCluster {
        r: f32,
        g: f32,
        b: f32,
        count: usize,
    }

    let mut clusters: Vec<ColorCluster> = Vec::with_capacity(8);

    for i in (0..total_pixels).step_by(step) {
        let idx = i * 4;
        if idx + 3 >= fav.rgba.len() {
            break;
        }
        let a = fav.rgba[idx + 3];
        if a < 100 {
            continue;
        }

        let r = fav.rgba[idx];
        let g = fav.rgba[idx + 1];
        let b = fav.rgba[idx + 2];

        let max_val = r.max(g).max(b);
        let min_val = r.min(g).min(b);
        let chroma = max_val - min_val;

        // Skip near-monochrome pixels (pure white, gray, pure dark) to isolate vibrant colors
        if chroma < 24 {
            continue;
        }

        let rf = r as f32;
        let gf = g as f32;
        let bf = b as f32;

        let mut nearest_idx = None;
        let mut min_dist_sq = 55.0 * 55.0;

        for (ci, cluster) in clusters.iter().enumerate() {
            let dr = cluster.r - rf;
            let dg = cluster.g - gf;
            let db = cluster.b - bf;
            let dist_sq = dr * dr + dg * dg + db * db;
            if dist_sq < min_dist_sq {
                min_dist_sq = dist_sq;
                nearest_idx = Some(ci);
            }
        }

        if let Some(ci) = nearest_idx {
            let c = &mut clusters[ci];
            let n = c.count as f32;
            c.r = (c.r * n + rf) / (n + 1.0);
            c.g = (c.g * n + gf) / (n + 1.0);
            c.b = (c.b * n + bf) / (n + 1.0);
            c.count += 1;
        } else if clusters.len() < 8 {
            clusters.push(ColorCluster {
                r: rf,
                g: gf,
                b: bf,
                count: 1,
            });
        }
    }

    if clusters.is_empty() {
        return None;
    }

    clusters.sort_by(|a, b| b.count.cmp(&a.count));

    let to_col = |c: &ColorCluster| rgb(c.r.round() as u8, c.g.round() as u8, c.b.round() as u8);

    match clusters.len() {
        1 => {
            let c = to_col(&clusters[0]);
            Some([c, c, c, c])
        }
        2 => {
            let c0 = to_col(&clusters[0]);
            let c1 = to_col(&clusters[1]);
            Some([c0, c1, c0, c1])
        }
        3 => {
            let c0 = to_col(&clusters[0]);
            let c1 = to_col(&clusters[1]);
            let c2 = to_col(&clusters[2]);
            Some([c0, c1, c2, c0])
        }
        _ => {
            let c0 = to_col(&clusters[0]);
            let c1 = to_col(&clusters[1]);
            let c2 = to_col(&clusters[2]);
            let c3 = to_col(&clusters[3]);
            Some([c0, c1, c2, c3])
        }
    }
}

pub fn get_pinned_tab_palette(item: &TabItem) -> [COLORREF; 4] {
    let domain = domain_of(&item.url);
    let u = item.url.to_lowercase();
    let t = item.title.to_lowercase();

    // 1. Google (Red -> Blue -> Green -> Yellow clockwise perimeter matching Zen Browser)
    if domain.contains("google.") || domain == "google" || u.contains("google.com") || t.contains("google") {
        return [
            rgb(234, 67, 53),  // Red (Top)
            rgb(66, 133, 244), // Blue (Right)
            rgb(52, 168, 83),  // Green (Bottom)
            rgb(251, 188, 5),  // Yellow (Left)
        ];
    }

    // 2. YouTube (Dominant vibrant red matching Zen Browser)
    if domain.contains("youtube.com") || domain.contains("youtu.be") || u.contains("youtube.com") || t.contains("youtube") {
        let yt_red = rgb(255, 0, 0);
        return [yt_red, yt_red, yt_red, yt_red];
    }

    // 3. Reddit (Vibrant Reddit Orange)
    if domain.contains("reddit.com") || u.contains("reddit.com") || t.contains("reddit") {
        let reddit_orange = rgb(255, 69, 0);
        return [reddit_orange, rgb(255, 110, 30), reddit_orange, rgb(255, 140, 0)];
    }

    // 4. Twitter / X (Twitter Electric Blue)
    if domain.contains("twitter.com") || domain.contains("x.com") || u.contains("x.com") || t.contains("twitter") {
        let tw_blue = rgb(29, 155, 240);
        return [tw_blue, tw_blue, tw_blue, tw_blue];
    }

    // 5. Discord (Discord Blurple)
    if domain.contains("discord.com") || domain.contains("discord.gg") || u.contains("discord.com") {
        let disc_blurple = rgb(88, 101, 242);
        return [disc_blurple, rgb(114, 137, 218), disc_blurple, rgb(71, 82, 196)];
    }

    // 6. Spotify (Spotify Electric Green)
    if domain.contains("spotify.com") || u.contains("spotify.com") {
        let spot_green = rgb(30, 215, 96);
        return [spot_green, spot_green, spot_green, spot_green];
    }

    // 7. GitHub (Vibrant Electric Purple & Cyan Gradient)
    if domain.contains("github.com") || u.contains("github.com") {
        return [
            rgb(137, 87, 229), // Electric Purple
            rgb(88, 166, 255), // Cyan Blue
            rgb(63, 185, 80),  // Green
            rgb(188, 140, 255),// Violet
        ];
    }

    // 8. Instagram (Vibrant sunset gradient)
    if domain.contains("instagram.com") || u.contains("instagram.com") {
        return [
            rgb(225, 48, 108), // Magenta
            rgb(131, 58, 180), // Purple
            rgb(253, 29, 29),  // Red
            rgb(247, 119, 55), // Orange
        ];
    }

    // 9. Twitch (Twitch Purple)
    if domain.contains("twitch.tv") || u.contains("twitch.tv") {
        let tw_purp = rgb(145, 71, 255);
        return [tw_purp, tw_purp, tw_purp, tw_purp];
    }

    // 10. Netflix (Netflix Red)
    if domain.contains("netflix.com") || u.contains("netflix.com") {
        let nf_red = rgb(229, 9, 20);
        return [nf_red, nf_red, nf_red, nf_red];
    }

    // 11. Dynamic Favicon extraction for any arbitrary site
    if let Some(ref fav) = item.favicon {
        if let Some(palette) = extract_palette_from_favicon(fav) {
            return palette;
        }
    }

    // 12. Fallback to tab accent color
    let acc = if item.accent.0 != 0 { item.accent } else { accent_for_url(&item.url) };
    [acc, acc, acc, acc]
}

/// Modern look = translucent white overlays on a single dark base.
#[derive(Clone)]
pub struct Theme {
    pub bg: COLORREF,
    pub white: COLORREF,
    pub surface_a: f32, // url pill, pinned tiles
    pub hover_a: f32,
    pub active_a: f32,
    pub stroke_a: f32,  // hairline borders
    pub divider_a: f32,
    pub text: COLORREF,
    pub text_dim: COLORREF,
    pub accent: COLORREF,
    pub caption_hover: COLORREF,
    pub caption_press: COLORREF,
    pub close_hover: COLORREF,
    pub close_press: COLORREF,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(18, 18, 21),
            white: rgb(255, 255, 255),
            surface_a: 0.050,
            hover_a: 0.070,
            active_a: 0.115,
            stroke_a: 0.060,
            divider_a: 0.070,
            text: rgb(236, 236, 242),
            text_dim: rgb(140, 142, 154),
            accent: rgb(112, 134, 255),
            caption_hover: rgb(42, 42, 48),
            caption_press: rgb(58, 58, 66),
            close_hover: rgb(196, 43, 28),
            close_press: rgb(175, 30, 18),
        }
    }
}

// ───────────────────────────── Layout ─────────────────────────────

#[derive(Clone)]
pub struct Layout {
    pub back: RECT,
    pub forward: RECT,
    pub reload: RECT,
    pub toggle_sidebar: RECT,
    pub url: RECT,
    pub clean_link_btn: RECT,
    pub site_settings: RECT,
    pub pending_permissions: Vec<(SitePermissionKind, RECT)>,
    pub pinned: Vec<RECT>,
    pub ghost_pinned: Option<RECT>,
    pub divider_y: i32,
    pub list_clip: RECT,
    pub folders: Vec<RECT>,
    pub folder_icons: Vec<RECT>,
    /// Divider between the top section (folders + pinned tabs) and the normal tabs.
    pub folder_divider: Option<RECT>,
    /// Clip rect of the top section (folders + pinned tabs); empty when there is none.
    pub top_clip: RECT,
    pub top_content_h: i32,
    pub tabs: Vec<RECT>,
    pub new_tab: RECT,
    pub footer_top: i32,
    pub downloads: RECT,
    pub settings: RECT,
    pub footer_toggle: RECT,
    pub zoom_pill: RECT,
    pub shield_pill: RECT,
    pub jump_to_active_pill: Option<RECT>,
    pub jump_target_scroll: i32,
    pub jump_active_favicon: Option<std::sync::Arc<FaviconData>>,
    pub list_content_h: i32,
}

pub fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

pub fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub fn host_of(url: &str) -> &str {
    let s = url.split("://").nth(1).unwrap_or(url);
    s.split('/').next().unwrap_or(s)
}

pub fn domain_of(url: &str) -> String {
    let host = host_of(url);
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host).trim().to_lowercase()
}

pub struct SidebarRenderer {
    pub theme: Theme,
    pub last_layout: std::cell::RefCell<Option<Layout>>,
}

impl Default for SidebarRenderer {
    fn default() -> Self {
        Self {
            theme: Theme::dark(),
            last_layout: std::cell::RefCell::new(None),
        }
    }
}

impl SidebarRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scale(hwnd: HWND) -> f32 {
        unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
    }

    pub fn compute_layout(&self, s: f32, width: i32, height: i32, st: &SidebarState) -> Layout {
        let px = |v: f32| (v * s).round() as i32;
        let lp = px(4.0);  // left inner pad (content gap of 8px adds up to 12px)
        let rp = px(12.0); // right inner pad (window edge)
        let cap_h = px(32.0);

        // Nav buttons: Back, Forward, Toggle Sidebar
        // Reload is removed per request.
        // If neither back nor forward is possible: no nav button is shown, starts directly with toggle_sidebar.
        // If can_go_back: shows back button '<-'.
        // If can_go_forward: shows forward button '->'.
        let nb = px(28.0);
        let ny = (cap_h - nb) / 2;
        let mut cur_x = lp;

        let back = if st.can_go_back {
            let r = rc(cur_x, ny, cur_x + nb, ny + nb);
            cur_x += nb + px(4.0);
            r
        } else {
            rc(0, 0, 0, 0)
        };

        let forward = if st.can_go_forward {
            let r = rc(cur_x, ny, cur_x + nb, ny + nb);
            cur_x += nb + px(4.0);
            r
        } else {
            rc(0, 0, 0, 0)
        };

        let reload = rc(0, 0, 0, 0);

        let toggle_sidebar = rc(cur_x, ny, cur_x + nb, ny + nb);

        // URL pill
        let url_top = cap_h + px(8.0);
        let url = rc(lp, url_top, width - rp, url_top + px(36.0));

        let btn_sz = px(24.0);
        let btn_y = url.top + (px(36.0) - btn_sz) / 2;
        let mut cur_right = url.right - px(6.0);

        let mut pending_permissions = Vec::new();
        for perm in &st.pending_permissions {
            let r = rc(cur_right - btn_sz, btn_y, cur_right, btn_y + btn_sz);
            cur_right = r.left - px(4.0);
            pending_permissions.push((*perm, r));
        }

        let (site_settings, clean_link_btn) = if st.is_mouse_over_sidebar {
            let s_r = rc(cur_right - btn_sz, btn_y, cur_right, btn_y + btn_sz);
            cur_right = s_r.left - px(4.0);
            let c_r = rc(cur_right - btn_sz, btn_y, cur_right, btn_y + btn_sz);
            cur_right = c_r.left - px(4.0);
            (s_r, c_r)
        } else {
            (rc(0, 0, 0, 0), rc(0, 0, 0, 0))
        };

        let zoom_pill = if (st.zoom - 1.0).abs() > 0.01 {
            let pill_w = px(50.0);
            let pill_h = px(22.0);
            let py = url.top + (px(36.0) - pill_h) / 2;
            let pr = cur_right - px(4.0);
            cur_right = pr - pill_w;
            rc(pr - pill_w, py, pr, py + pill_h)
        } else {
            rc(0, 0, 0, 0)
        };

        let shield_pill = if st.shield_blocked_count > 0 || st.shield_paused_site || !st.shield_enabled {
            let pill_w = if !st.shield_enabled || st.shield_paused_site { px(46.0) } else { px(40.0) };
            let pill_h = px(22.0);
            let py = url.top + (px(36.0) - pill_h) / 2;
            let pr = cur_right - px(4.0);
            rc(pr - pill_w, py, pr, py + pill_h)
        } else {
            rc(0, 0, 0, 0)
        };

        // Pinned grid (Dynamic columns: up to 4 tabs fit in 1 row; 5+ tabs split into 3-column rows)
        let total_pinned = st.pinned.len();
        let gap = px(8.0);
        let tile_h = px(44.0);
        let pinned_top = url.bottom + px(12.0);

        let calc_pinned_cells = |count: usize| -> Vec<RECT> {
            if count == 0 {
                return Vec::new();
            }
            let cols = if count <= 4 {
                count.max(1)
            } else {
                3
            };
            let tile_w = ((width - lp - rp - gap * (cols as i32 - 1)) / (cols as i32)).max(20);
            (0..count as i32)
                .map(|i| {
                    let (c, r) = if count <= 4 {
                        (i, 0)
                    } else {
                        (i % 3, i / 3)
                    };
                    let l = lp + c * (tile_w + gap);
                    let t = pinned_top + r * (tile_h + gap);
                    rc(l, t, l + tile_w, t + tile_h)
                })
                .collect::<Vec<RECT>>()
        };

        let mut ghost_pinned: Option<RECT> = None;
        let mut pinned: Vec<RECT> = vec![rc(0, 0, 0, 0); total_pinned];

        // Check if there is an active drag targeting the Favorites grid -> shift existing tabs sideways
        if let Some(ref drag) = st.drag_state {
            if let Some(DragTarget::Favorites(target_slot)) = drag.target {
                match drag.source {
                    DragSource::Pinned(fav_idx) => {
                        let all_cells = calc_pinned_cells(total_pinned);
                        let s = target_slot.min(total_pinned.saturating_sub(1));
                        let other_indices: Vec<usize> = (0..total_pinned).filter(|&j| j != fav_idx).collect();
                        for (k, &orig_i) in other_indices.iter().enumerate() {
                            let cell_idx = if k < s { k } else { k + 1 };
                            if cell_idx < all_cells.len() {
                                pinned[orig_i] = all_cells[cell_idx];
                            }
                        }
                        if s < all_cells.len() {
                            pinned[fav_idx] = all_cells[s];
                            ghost_pinned = Some(all_cells[s]);
                        }
                    }
                    DragSource::Tab(_) => {
                        let all_cells = calc_pinned_cells(total_pinned + 1);
                        let s = target_slot.min(total_pinned);
                        for i in 0..total_pinned {
                            let cell_idx = if i < s { i } else { i + 1 };
                            if cell_idx < all_cells.len() {
                                pinned[i] = all_cells[cell_idx];
                            }
                        }
                        if s < all_cells.len() {
                            ghost_pinned = Some(all_cells[s]);
                        }
                    }
                }
            } else {
                pinned = calc_pinned_cells(total_pinned);
            }
        } else {
            pinned = calc_pinned_cells(total_pinned);
        }

        let pinned_bottom = pinned.iter().chain(ghost_pinned.as_ref()).map(|r| r.bottom).max().unwrap_or(url.bottom);

        let divider_y = if st.pinned.is_empty() && ghost_pinned.is_none() {
            url.bottom
        } else {
            pinned_bottom + px(14.0)
        };

        // Footer
        let footer_h = px(48.0);
        let footer_top = height - footer_h;
        let fb = px(36.0);
        let fy = footer_top + (footer_h - fb) / 2;
        let downloads = rc(lp, fy, lp + fb, fy + fb);
        let settings = rc(width - rp - fb, fy, width - rp, fy + fb);
        let footer_toggle = rc(0, 0, 0, 0);

        // Unified scrollable area below the fixed Favorites grid:
        // Folders, Pinned tabs, "+ New Tab", and Normal tabs all scroll together!
        let list_top = if st.pinned.is_empty() && ghost_pinned.is_none() {
            url.bottom + px(12.0)
        } else {
            pinned_bottom + px(10.0)
        };

        let row_l = lp - px(4.0);
        let row_r = width - rp + px(4.0);

        let folder_row_h = px(38.0);
        let folder_gap = px(2.0);
        let full_row_h = px(34.0);
        let full_row_gap = px(2.0);

        let mut folders = Vec::with_capacity(st.folders.len());
        let mut folder_icons = Vec::with_capacity(st.folders.len());
        let mut tabs = vec![rc(0, 0, 0, 0); st.tabs.len()];

        let list_clip = rc(0, list_top, width, footer_top);
        let mut y = list_top - st.scroll;

        // 1. Folders (accordion)
        for f in &st.folders {
            let r = rc(row_l, y, row_r, y + folder_row_h);
            let icon_w = px(28.0);
            let icon_h = px(28.0);
            let icon_x = row_l + px(8.0);
            let icon_y = y + (folder_row_h - icon_h) / 2;
            let icon_r = rc(icon_x, icon_y, icon_x + icon_w, icon_y + icon_h);

            folders.push(r);
            folder_icons.push(icon_r);
            y += folder_row_h + folder_gap;

            // Sites belonging to this folder (accordion)
            if f.open_progress > 0.001 {
                let ease = ease_in_out(f.open_progress);
                let row_h = (full_row_h as f32 * ease).round() as i32;
                let row_gap = (full_row_gap as f32 * ease).round() as i32;
                let indent_l = row_l + px(16.0);

                for (tab_idx, tab) in st.tabs.iter().enumerate() {
                    if tab.folder_id.as_deref() == Some(&f.id) {
                        if row_h > 0 {
                            tabs[tab_idx] = rc(indent_l, y, row_r, y + row_h);
                            y += row_h + row_gap;
                        }
                    }
                }
            }
        }

        // 2. Pinned tabs section (under the folders)
        let mut any_pinned_tab = false;
        for (i, tab_item) in st.tabs.iter().enumerate() {
            if tab_item.folder_id.as_deref() != Some(PINNED_SECTION_ID) {
                continue;
            }
            if !any_pinned_tab {
                any_pinned_tab = true;
                if !st.folders.is_empty() {
                    y += px(6.0);
                }
            }
            let progress = tab_item.anim_progress.clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - progress).powi(3);
            let row_h = (full_row_h as f32 * ease).round() as i32;
            let row_gap = (full_row_gap as f32 * ease).round() as i32;
            tabs[i] = rc(row_l, y, row_r, y + row_h);
            y += row_h + row_gap;
        }

        // 3. Subtle separator divider before "+ New Tab" if there are folders or pinned tabs
        let (folder_divider, b_top) = if !st.folders.is_empty() || any_pinned_tab {
            let dy = y + px(6.0);
            let dh = px(1.0).max(1);
            let r = rc(row_l, dy, row_r, dy + dh);
            y = dy + dh + px(8.0);
            (Some(r), y)
        } else {
            (None, y)
        };

        // 4. "+ New Tab" then normal tabs
        let new_tab = rc(lp - px(4.0), y, width - rp + px(4.0), y + full_row_h);
        y += full_row_h + full_row_gap + px(4.0);

        // 5. Normal open tabs (UNDER "+ New Tab" button)
        let group = st.split_group.filter(|g| g.1 == g.0 + 1);
        for (i, tab_item) in st.tabs.iter().enumerate() {
            if tab_item.folder_id.is_none() {
                let mid = (row_l + row_r) / 2;
                let progress = tab_item.anim_progress.clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - progress).powi(3);
                let row_h = (full_row_h as f32 * ease).round() as i32;
                let row_gap = (full_row_gap as f32 * ease).round() as i32;

                match group {
                    Some((a, _)) if i == a => {
                        tabs[i] = rc(row_l, y, mid - px(1.0), y + row_h);
                    }
                    Some((_, b)) if i == b => {
                        tabs[i] = rc(mid + px(1.0), y, row_r, y + row_h);
                        y += row_h + row_gap;
                    }
                    _ => {
                        tabs[i] = rc(row_l, y, row_r, y + row_h);
                        y += row_h + row_gap;
                    }
                }
            }
        }

        let list_content_h = ((y + st.scroll) - list_top).max(0);
        let max_scroll = (list_content_h - (list_clip.bottom - list_clip.top)).max(0);

        let top_clip = list_clip;
        let top_content_h = list_content_h;

        let mut jump_to_active_pill = None;
        let mut jump_target_scroll = 0;
        let mut jump_active_favicon = None;

        if let Some(ActiveTab::Tab(active_i)) = st.active {
            if active_i < tabs.len() && active_i < st.tabs.len() && st.tabs[active_i].folder_id.is_none() {
                let tab_r = tabs[active_i];
                if tab_r.bottom > tab_r.top && tab_r.bottom > list_clip.bottom {
                    let pill_w = px(58.0);
                    let pill_h = px(28.0);
                    let pill_x = (width - pill_w) / 2;
                    let pill_y = footer_top - pill_h - px(8.0);
                    jump_to_active_pill = Some(rc(pill_x, pill_y, pill_x + pill_w, pill_y + pill_h));

                    let tab_unscrolled_top = tab_r.top + st.scroll;
                    let view_h = list_clip.bottom - list_clip.top;
                    jump_target_scroll = (tab_unscrolled_top - b_top - view_h / 2).clamp(0, max_scroll);
                    jump_active_favicon = st.tabs[active_i].favicon.clone();
                }
            }
        }

        let layout = Layout {
            back, forward, reload, toggle_sidebar, url, clean_link_btn, site_settings, pending_permissions, zoom_pill, shield_pill, pinned, ghost_pinned, divider_y, list_clip,
            folders, folder_icons, folder_divider, top_clip, top_content_h, tabs, new_tab,
            footer_top, downloads, settings, footer_toggle,
            jump_to_active_pill, jump_target_scroll, jump_active_favicon,
            list_content_h,
        };
        self.last_layout.replace(Some(layout.clone()));
        layout
    }

    /// Max valid value for `SidebarState::scroll`.
    pub fn max_scroll(&self, hwnd: HWND, height: i32, width: i32, st: &SidebarState) -> i32 {
        let l = self.compute_layout(Self::scale(hwnd), width, height, st);
        (l.list_content_h - (l.list_clip.bottom - l.list_clip.top)).max(0)
    }

    /// Max valid value for `SidebarState::scroll_top` (folders + pinned tabs section).
    pub fn max_scroll_top(&self, hwnd: HWND, height: i32, width: i32, st: &SidebarState) -> i32 {
        let l = self.compute_layout(Self::scale(hwnd), width, height, st);
        (l.top_content_h - (l.top_clip.bottom - l.top_clip.top)).max(0)
    }

    // ───────────────────────── Hit testing ─────────────────────────

    pub fn hit_test_button(&self, hwnd: HWND, x: i32, y: i32, client_width: i32) -> CaptionButton {
        let s = Self::scale(hwnd);
        let bw = (46.0 * s).round() as i32;
        let bh = (32.0 * s).round() as i32;
        if y < 0 || y >= bh {
            return CaptionButton::None;
        }
        if x >= client_width - bw && x < client_width {
            CaptionButton::Close
        } else if x >= client_width - bw * 2 && x < client_width - bw {
            CaptionButton::Maximize
        } else if x >= client_width - bw * 3 && x < client_width - bw * 2 {
            CaptionButton::Minimize
        } else {
            CaptionButton::None
        }
    }

    pub fn get_caption_buttons_rect(&self, hwnd: HWND, client_width: i32) -> RECT {
        let s = Self::scale(hwnd);
        let bw = (46.0 * s).round() as i32;
        let bh = (32.0 * s).round() as i32;
        rc(client_width - bw * 3, 0, client_width, bh)
    }

    /// `x`,`y` are in sidebar-local coordinates (0,0 = sidebar top-left).
    pub fn hit_test(&self, hwnd: HWND, x: i32, y: i32, width: i32, height: i32, st: &SidebarState) -> SidebarHit {
        let s = Self::scale(hwnd);
        let l = self.compute_layout(s, width, height, st);

        // Check interactive navigation buttons FIRST before caption buttons.
        let cap_h = (32.0 * s).round() as i32;
        if y >= 0 && y < cap_h {
            if l.back.right > l.back.left && contains(&l.back, x, y) {
                return SidebarHit::Back;
            }
            if l.forward.right > l.forward.left && contains(&l.forward, x, y) {
                return SidebarHit::Forward;
            }
            if contains(&l.toggle_sidebar, x, y) {
                return SidebarHit::ToggleSidebar;
            }
        }

        let c = self.hit_test_button(hwnd, x, y, width);
        if c != CaptionButton::None {
            return SidebarHit::Caption(c);
        }

        for (i, (_perm, r)) in l.pending_permissions.iter().enumerate() {
            if contains(r, x, y) { return SidebarHit::PermissionIcon(i); }
        }
        if l.shield_pill.right > l.shield_pill.left && contains(&l.shield_pill, x, y) { return SidebarHit::Shield; }
        if l.zoom_pill.right > l.zoom_pill.left && contains(&l.zoom_pill, x, y) { return SidebarHit::ZoomReset; }
        if contains(&l.site_settings, x, y) { return SidebarHit::SiteSettings; }
        if contains(&l.clean_link_btn, x, y) { return SidebarHit::CleanLinkBtn; }
        if contains(&l.url, x, y) { return SidebarHit::UrlBar; }
        if contains(&l.downloads, x, y) { return SidebarHit::Downloads; }
        if contains(&l.settings, x, y) { return SidebarHit::Settings; }
        if contains(&l.footer_toggle, x, y) { return SidebarHit::ToggleSidebar; }
        if let Some(ref pill) = l.jump_to_active_pill {
            if contains(pill, x, y) {
                return SidebarHit::JumpToActiveTab;
            }
        }
        for (i, r) in l.pinned.iter().enumerate() {
            if contains(r, x, y) { return SidebarHit::Pinned(i); }
        }
        let cb = (20.0 * s).round() as i32;
        // `top_section` = folder/pinned-section tabs (top clip); otherwise normal tabs (bottom clip).
        let hit_tab = |top_section: bool| -> Option<SidebarHit> {
            for (i, r) in l.tabs.iter().enumerate() {
                if r.bottom <= r.top {
                    continue;
                }
                let tab_item = match st.tabs.get(i) {
                    Some(t) => t,
                    None => continue,
                };
                if tab_item.folder_id.is_some() != top_section {
                    continue;
                }
                if tab_item.is_closing || tab_item.anim_progress < 0.1 {
                    continue;
                }
                if contains(r, x, y) {
                    let close = rc(r.right - cb - (8.0 * s) as i32, r.top + (r.bottom - r.top - cb) / 2,
                                   r.right - (8.0 * s) as i32, r.top + (r.bottom - r.top - cb) / 2 + cb);
                    return Some(if contains(&close, x, y) { SidebarHit::TabClose(i) } else { SidebarHit::Tab(i) });
                }
            }
            None
        };

        if contains(&l.list_clip, x, y) {
            for (i, r) in l.folders.iter().enumerate() {
                if contains(r, x, y) {
                    return SidebarHit::Folder(i);
                }
            }
            if let Some(h) = hit_tab(true) {
                return h;
            }
            if contains(&l.new_tab, x, y) { return SidebarHit::NewTab; }
            if let Some(h) = hit_tab(false) {
                return h;
            }
        }
        SidebarHit::None
    }

    pub fn compute_drag_target(
        &self,
        hwnd: HWND,
        st: &SidebarState,
        cur_x: i32,
        cur_y: i32,
        sidebar_rect: RECT,
    ) -> Option<DragTarget> {
        let width = sidebar_rect.right - sidebar_rect.left;
        let height = sidebar_rect.bottom - sidebar_rect.top;
        if width <= 0 || height <= 0 {
            return None;
        }
        let s = Self::scale(hwnd);
        let px = |v: f32| (v * s).round() as i32;
        let l = self.compute_layout(s, width, height, st);

        let sx = cur_x - sidebar_rect.left;
        let sy = cur_y - sidebar_rect.top;

        // 1. Top region: Favorites grid
        let pinned_bottom = l.pinned.iter().chain(l.ghost_pinned.as_ref()).map(|r| r.bottom).max().unwrap_or(l.url.bottom);
        let fav_zone_bottom = if st.pinned.is_empty() && l.ghost_pinned.is_none() {
            l.url.bottom + px(24.0)
        } else {
            pinned_bottom + px(12.0)
        };

        if sy <= fav_zone_bottom {
            let total = st.pinned.len();
            if total == 0 {
                return Some(DragTarget::Favorites(0));
            }
            let ghost_total = if matches!(st.drag_state.as_ref().map(|d| d.source), Some(DragSource::Tab(_))) {
                total + 1
            } else {
                total
            };
            let lp = px(4.0);
            let rp = px(12.0);
            let gap = px(8.0);
            let tile_h = px(44.0);
            let pinned_top = l.url.bottom + px(12.0);
            let cols = if ghost_total <= 4 { ghost_total.max(1) } else { 3 };
            let tile_w = ((width - lp - rp - gap * (cols as i32 - 1)) / (cols as i32)).max(20);
            let all_cells: Vec<RECT> = (0..ghost_total as i32)
                .map(|i| {
                    let (c, r) = if ghost_total <= 4 { (i, 0) } else { (i % 3, i / 3) };
                    let cell_l = lp + c * (tile_w + gap);
                    let cell_t = pinned_top + r * (tile_h + gap);
                    rc(cell_l, cell_t, cell_l + tile_w, cell_t + tile_h)
                })
                .collect();

            let best = all_cells.iter().enumerate().min_by_key(|(_, r)| {
                let mx = (r.left + r.right) / 2;
                let my = (r.top + r.bottom) / 2;
                (mx - sx).pow(2) + (my - sy).pow(2)
            });
            if let Some((i, _)) = best {
                return Some(DragTarget::Favorites(i));
            }
            return Some(DragTarget::Favorites(0));
        }

        // 2. Middle region: Folders & Pinned section
        if sy < l.new_tab.top {
            // Over a folder header?
            for (i, r) in l.folders.iter().enumerate() {
                if sy >= r.top && sy < r.bottom {
                    return Some(DragTarget::Folder(i));
                }
            }

            // Inside an open folder's items?
            for (f_idx, f) in st.folders.iter().enumerate() {
                if f.open_progress > 0.001 {
                    let folder_tabs: Vec<usize> = st.tabs.iter().enumerate()
                        .filter(|(_, t)| t.folder_id.as_deref() == Some(&f.id))
                        .map(|(idx, _)| idx)
                        .collect();
                    if !folder_tabs.is_empty() {
                        let first_top = l.tabs[folder_tabs[0]].top;
                        let last_bottom = l.tabs[*folder_tabs.last().unwrap()].bottom;
                        if sy >= first_top - px(4.0) && sy <= last_bottom + px(4.0) {
                            let mut slot = folder_tabs.len();
                            for (insert_idx, &t_idx) in folder_tabs.iter().enumerate() {
                                let tr = l.tabs[t_idx];
                                if sy < (tr.top + tr.bottom) / 2 {
                                    slot = insert_idx;
                                    break;
                                }
                            }
                            return Some(DragTarget::FolderInsert {
                                folder_idx: f_idx,
                                insert_idx: slot,
                            });
                        }
                    }
                }
            }

            // In the pinned tabs section (under folders)
            let pinned_tabs: Vec<usize> = st.tabs.iter().enumerate()
                .filter(|(_, t)| t.folder_id.as_deref() == Some(PINNED_SECTION_ID))
                .map(|(idx, _)| idx)
                .collect();
            if pinned_tabs.is_empty() {
                return Some(DragTarget::PinnedSection(0));
            }
            let mut slot = pinned_tabs.len();
            for (insert_idx, &t_idx) in pinned_tabs.iter().enumerate() {
                let tr = l.tabs[t_idx];
                if sy < (tr.top + tr.bottom) / 2 {
                    slot = insert_idx;
                    break;
                }
            }
            return Some(DragTarget::PinnedSection(slot));
        }

        // 3. Bottom region: Normal tabs
        let normal_tabs: Vec<usize> = st.tabs.iter().enumerate()
            .filter(|(_, t)| t.folder_id.is_none())
            .map(|(idx, _)| idx)
            .collect();

        if sy < l.new_tab.bottom + px(4.0) {
            return Some(DragTarget::NormalTabs(0));
        }
        if normal_tabs.is_empty() {
            return Some(DragTarget::NormalTabs(0));
        }
        let mut slot = normal_tabs.len();
        for (insert_idx, &t_idx) in normal_tabs.iter().enumerate() {
            let tr = l.tabs[t_idx];
            if tr.bottom > tr.top && sy < (tr.top + tr.bottom) / 2 {
                slot = insert_idx;
                break;
            }
        }
        Some(DragTarget::NormalTabs(slot))
    }

    // ───────────────────────── Rendering ─────────────────────────

    pub fn render(
        &self,
        hwnd: HWND,
        hdc: HDC,
        sidebar_rect: RECT,
        full_width: i32,
        st: &SidebarState,
        hovered_button: CaptionButton,
        pressed_button: CaptionButton,
    ) {
        let width = sidebar_rect.right - sidebar_rect.left;
        let height = sidebar_rect.bottom - sidebar_rect.top;
        if width <= 0 || height <= 0 {
            return;
        }
        let layout_w = full_width.max(width);
        let t = &self.theme;
        let s = Self::scale(hwnd);
        let px = |v: f32| (v * s).round() as i32;
        let l = self.compute_layout(s, layout_w, height, st);

        unsafe {
            // 32-bit top-down DIB so we can rasterize anti-aliased shapes ourselves.
            let mem_dc = CreateCompatibleDC(Some(hdc));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: layout_w,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let mem_bmp = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    let _ = DeleteDC(mem_dc);
                    return;
                }
            };
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());
            let _ = SetBkMode(mem_dc, TRANSPARENT);

            let cv = Canvas {
                dc: mem_dc,
                bits: bits as *mut u32,
                w: layout_w,
                h: height,
                clip: Cell::new(rc(0, 0, layout_w, height)),
            };

            // Fonts: Segoe Fluent Icons for symbols, Segoe UI for text.
            let ui = make_font(w!("Segoe UI"), px(13.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
            let ui_semi = make_font(w!("Segoe UI"), px(13.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
            let icon = make_font(w!("Segoe Fluent Icons"), px(13.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
            let icon_cap = make_font(w!("Segoe Fluent Icons"), px(10.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
            let old_font = SelectObject(mem_dc, ui.into());

            // Background
            cv.fill_rect(&rc(0, 0, width, height), t.bg, 1.0);

            // ── Caption row: nav buttons ──
            if l.back.right > l.back.left {
                if st.hovered == SidebarHit::Back {
                    cv.fill_round(&l.back, px(8.0), t.white, t.hover_a);
                }
                SelectObject(mem_dc, icon.into());
                text(mem_dc, "\u{E72B}", l.back, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
                     if st.hovered == SidebarHit::Back { t.text } else { t.text_dim });
            }
            if l.forward.right > l.forward.left {
                if st.hovered == SidebarHit::Forward {
                    cv.fill_round(&l.forward, px(8.0), t.white, t.hover_a);
                }
                SelectObject(mem_dc, icon.into());
                text(mem_dc, "\u{E72A}", l.forward, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
                     if st.hovered == SidebarHit::Forward { t.text } else { t.text_dim });
            }

            // Sidebar toggle button with Boxicons SVG icon
            if st.hovered == SidebarHit::ToggleSidebar {
                cv.fill_round(&l.toggle_sidebar, px(8.0), t.white, t.hover_a);
            }
            let tog_col = if st.hovered == SidebarHit::ToggleSidebar { t.text } else { t.text_dim };
            draw_sidebar_icon(&cv, &l.toggle_sidebar, tog_col, 1.0, px);

            // ── URL pill ──
            let url_hover = st.hovered == SidebarHit::UrlBar;
            cv.fill_round(&l.url, px(5.0), t.white, if url_hover { t.hover_a } else { t.surface_a });
            cv.stroke_round(&l.url, px(5.0), t.white, t.stroke_a, 1.0);
            let active_item = match st.active {
                Some(ActiveTab::Pinned(i)) => st.pinned.get(i),
                Some(ActiveTab::Tab(i)) => st.tabs.get(i),
                None => None,
            };
            let active_url = active_item.map(|a| a.url.as_str()).unwrap_or("");
            let is_insecure = !active_url.is_empty()
                && !active_url.starts_with("about:")
                && !active_url.starts_with("browser:")
                && !active_url.starts_with("https://");

            // ── Pending Permission Buttons (e.g. Notifications Bell, Camera, Mic, Pin) ──
            for (i, (perm, perm_r)) in l.pending_permissions.iter().enumerate() {
                let is_perm_hov = st.hovered == SidebarHit::PermissionIcon(i);
                if is_perm_hov {
                    cv.fill_round(perm_r, px(5.0), t.white, 0.14);
                }
                let perm_hex = if is_perm_hov { "#ffffff" } else { "#e2e8f0" };
                let svg_data = match perm {
                    SitePermissionKind::Notifications => BELL_ICON_SVG,
                    SitePermissionKind::Camera => CAMERA_ICON_SVG,
                    SitePermissionKind::Microphone => MIC_ICON_SVG,
                    SitePermissionKind::Location => LOCATION_ICON_SVG,
                };
                let perm_pad = px(4.0);
                let icon_r = rc(perm_r.left + perm_pad, perm_r.top + perm_pad, perm_r.right - perm_pad, perm_r.bottom - perm_pad);
                draw_svg_icon(&cv, &icon_r, svg_data, perm_hex);
            }

            // ── Clean Link Button (🔗) ──
            if l.clean_link_btn.right > l.clean_link_btn.left {
                let is_link_hov = st.hovered == SidebarHit::CleanLinkBtn;
                if is_link_hov {
                    cv.fill_round(&l.clean_link_btn, px(5.0), t.white, 0.14);
                }
                let link_hex = if is_link_hov { "#ffffff" } else { "#959bb0" };
                let link_pad = px(4.0);
                let link_r = rc(l.clean_link_btn.left + link_pad, l.clean_link_btn.top + link_pad, l.clean_link_btn.right - link_pad, l.clean_link_btn.bottom - link_pad);
                draw_svg_icon(&cv, &link_r, LINK_ICON_SVG, link_hex);
            }

            // ── Settings / Site Info Tune Button (🎛️, borderless matching Zen) ──
            if l.site_settings.right > l.site_settings.left {
                let is_btn_hover = st.hovered == SidebarHit::SiteSettings;
                if is_btn_hover {
                    cv.fill_round(&l.site_settings, px(5.0), t.white, 0.14);
                }
                let tune_hex = if is_btn_hover { "#ffffff" } else if is_insecure { "#ef4444" } else { "#959bb0" };
                let tune_pad = px(4.0);
                let tune_r = rc(l.site_settings.left + tune_pad, l.site_settings.top + tune_pad, l.site_settings.right - tune_pad, l.site_settings.bottom - tune_pad);
                draw_svg_icon(&cv, &tune_r, TUNE_ICON_SVG, tune_hex);
            }

            SelectObject(mem_dc, ui.into());
            let label = active_item.map(|a| host_of(&a.url)).filter(|h| !h.is_empty()).unwrap_or("Search or enter address");
            let right_limit = if l.shield_pill.right > l.shield_pill.left {
                l.shield_pill.left - px(6.0)
            } else if l.zoom_pill.right > l.zoom_pill.left {
                l.zoom_pill.left - px(6.0)
            } else if l.clean_link_btn.right > l.clean_link_btn.left {
                l.clean_link_btn.left - px(6.0)
            } else if l.site_settings.right > l.site_settings.left {
                l.site_settings.left - px(6.0)
            } else if let Some((_, last_r)) = l.pending_permissions.last() {
                last_r.left - px(6.0)
            } else {
                l.url.right - px(8.0)
            };
            let label_rect = rc(l.url.left + px(12.0), l.url.top, right_limit, l.url.bottom);
            text(mem_dc, label, label_rect,
                 DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                 if active_item.is_some() { t.text } else { t.text_dim });

            // If zoomed, render sleek pill button
            if l.zoom_pill.right > l.zoom_pill.left {
                let hov = st.hovered == SidebarHit::ZoomReset;
                cv.fill_round(&l.zoom_pill, px(5.0), t.white, if hov { 0.22 } else { 0.12 });
                cv.stroke_round(&l.zoom_pill, px(5.0), t.white, 0.18, 1.0);
                let zoom_str = format!("{}%", (st.zoom * 100.0).round() as i32);
                SelectObject(mem_dc, ui_semi.into());
                text(mem_dc, &zoom_str, l.zoom_pill,
                     DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
                     if hov { t.text } else { rgb(200, 205, 220) });
            }

            // Shield status badge
            if l.shield_pill.right > l.shield_pill.left {
                let hov = st.hovered == SidebarHit::Shield;
                let (bg_alpha, border_alpha) = if hov { (0.24, 0.35) } else { (0.12, 0.20) };
                let is_off = !st.shield_enabled || st.shield_paused_site;
                let accent_col = if is_off {
                    rgb(239, 68, 68)
                } else {
                    rgb(56, 189, 248)
                };
                cv.fill_round(&l.shield_pill, px(5.0), accent_col, bg_alpha);
                cv.stroke_round(&l.shield_pill, px(5.0), accent_col, border_alpha, 1.0);

                let icon_rc = rc(l.shield_pill.left + px(3.0), l.shield_pill.top, l.shield_pill.left + px(18.0), l.shield_pill.bottom);
                SelectObject(mem_dc, icon.into());
                text(mem_dc, "\u{EA18}", icon_rc, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, accent_col);

                let label_rc = rc(l.shield_pill.left + px(16.0), l.shield_pill.top, l.shield_pill.right - px(3.0), l.shield_pill.bottom);
                let text_str = if is_off {
                    "Off".to_string()
                } else {
                    format!("{}", st.shield_blocked_count)
                };
                SelectObject(mem_dc, ui_semi.into());
                text(mem_dc, &text_str, label_rc, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, if hov { t.text } else { rgb(210, 215, 230) });
            }

            // ── Pinned rectangular tiles with authentic website favicons ──
            let tile_rad = px(8.0);
            let border_w = (1.5 * s).round().max(1.0);

            for (i, r) in l.pinned.iter().enumerate() {
                let is_drag_source = matches!(st.drag_state.as_ref(), Some(d) if d.source == DragSource::Pinned(i));
                let is_active = st.active == Some(ActiveTab::Pinned(i));
                let is_hover = st.hovered == SidebarHit::Pinned(i);
                let is_loaded = st.pinned.get(i).map_or(false, |p| p.is_loaded);

                let mut a = if is_active {
                    if is_hover {
                        (t.active_a + 0.035).min(0.20)
                    } else {
                        (t.active_a + 0.02).min(0.18)
                    }
                } else if is_loaded {
                    if is_hover {
                        t.hover_a
                    } else {
                        t.surface_a
                    }
                } else {
                    // Closed / inactive pinned tab (dimmed)
                    if is_hover {
                        0.065
                    } else {
                        0.02
                    }
                };

                if is_drag_source {
                    a *= 0.25;
                }

                let is_editing = match st.inline_edit {
                    Some(ref ed) => ed.target == InlineEditTarget::PinnedLetter(i),
                    None => false,
                };

                if is_editing {
                    let ed = st.inline_edit.as_ref().unwrap();
                    cv.fill_round(r, tile_rad, t.white, 0.15);
                    cv.stroke_round(r, tile_rad, rgb(110, 165, 255), 0.95, 2.0);

                    let fav = px(22.0);
                    let cx = (r.left + r.right) / 2;
                    let cy = (r.top + r.bottom) / 2;
                    let fav_rect = rc(cx - fav / 2, cy - fav / 2, cx + fav / 2, cy + fav / 2);

                    let letter_font = make_font(w!("Segoe UI"), px(18.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
                    let old = SelectObject(mem_dc, letter_font.into());
                    text(mem_dc, &ed.text, fav_rect, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(255, 255, 255));
                    if ed.caret_visible {
                        let tw = text_w(mem_dc, &ed.text);
                        let caret_x = cx + tw / 2 + px(1.0);
                        let caret_r = rc(caret_x, fav_rect.top + px(2.0), caret_x + px(1.5), fav_rect.bottom - px(2.0));
                        cv.fill_rect(&caret_r, rgb(255, 255, 255), 0.95);
                    }
                    SelectObject(mem_dc, old);
                    let _ = DeleteObject(letter_font.into());
                    continue;
                }

                if is_active {
                    let palette = get_pinned_tab_palette(&st.pinned[i]);
                    // 1. Soft ambient glow radiating outward matching Zen Browser
                    cv.glow_round_gradient(r, tile_rad, &palette, 0.35, (3.5 * s).round());
                    // 2. Elevated tile background
                    cv.fill_round(r, tile_rad, t.white, a);
                    // 3. Subtle accent tint wash inside the tile body matching the border colors
                    cv.fill_round_gradient(r, tile_rad, &palette, 0.08);
                    // 4. Refined crisp gradient border
                    cv.stroke_round_gradient(r, tile_rad, &palette, 0.95, border_w);
                } else if is_loaded {
                    cv.fill_round(r, tile_rad, t.white, a);
                    cv.stroke_round(r, tile_rad, t.white, t.stroke_a, 1.0);
                } else {
                    cv.fill_round(r, tile_rad, t.white, a);
                    cv.stroke_round(r, tile_rad, t.white, 0.03, 1.0);
                }

                let fav = px(18.0);
                let cx = (r.left + r.right) / 2;
                let cy = (r.top + r.bottom) / 2;
                let fav_rect = rc(cx - fav / 2, cy - fav / 2, cx + fav / 2, cy + fav / 2);
                draw_webapp_icon(&cv, &fav_rect, &st.pinned[i], tile_rad, px);
            }

            // Floating tooltip pill when a pinned tile is hovered (Image 2)
            if let SidebarHit::Pinned(hover_idx) = st.hovered {
                if let Some(pitem) = st.pinned.get(hover_idx) {
                    if let Some(tile_r) = l.pinned.get(hover_idx) {
                        SelectObject(mem_dc, ui.into());
                        let tw = text_w(mem_dc, &pitem.title);
                        let pill_w = tw + px(16.0);
                        let pill_h = px(22.0);
                        let cx = (tile_r.left + tile_r.right) / 2;
                        let pill_left = (cx - pill_w / 2).max(px(6.0)).min(width - pill_w - px(6.0));
                        let pill_top = tile_r.bottom + px(5.0);
                        let pill_rect = rc(pill_left, pill_top, pill_left + pill_w, pill_top + pill_h);

                        cv.fill_round(&pill_rect, px(5.0), rgb(30, 31, 35), 0.98);
                        cv.stroke_round(&pill_rect, px(5.0), rgb(255, 255, 255), 0.22, 1.0);
                        text(
                            mem_dc,
                            &pitem.title,
                            pill_rect,
                            DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                            rgb(240, 240, 245),
                        );
                    }
                }
            }

            // ── Unified scrollable area below the fixed Favorites grid ──
            cv.reset_clip();
            if l.list_clip.bottom > l.list_clip.top {
                cv.set_clip(l.list_clip);

                // 1. Folders list (accordion)
                for (i, r) in l.folders.iter().enumerate() {
                    if r.bottom < l.list_clip.top || r.top > l.list_clip.bottom {
                        continue;
                    }
                    if let Some(folder) = st.folders.get(i) {
                        let is_hover = matches!(st.hovered, SidebarHit::Folder(idx) | SidebarHit::FolderIcon(idx) if idx == i);
                        if is_hover {
                            cv.fill_round(r, px(6.0), t.white, t.hover_a);
                        }

                        // Folder icon badge (Image 1, 4)
                        if let Some(ir) = l.folder_icons.get(i) {
                            draw_folder_icon(&cv, mem_dc, ir, folder.is_open, folder.open_progress, &folder.icon, is_hover, px);
                        }

                        let is_editing = match st.inline_edit {
                            Some(ref ed) => ed.target == InlineEditTarget::Folder(folder.id.clone()),
                            None => false,
                        };

                        let text_x = r.left + px(44.0);
                        let text_r = rc(text_x, r.top, r.right - px(8.0), r.bottom);

                        if is_editing {
                            let ed = st.inline_edit.as_ref().unwrap();
                            SelectObject(mem_dc, ui_semi.into());
                            if let Some((sel_s, sel_e)) = ed.selection {
                                let s_idx = sel_s.min(ed.text.len());
                                let e_idx = sel_e.min(ed.text.len());
                                let (min_i, max_i) = if s_idx <= e_idx { (s_idx, e_idx) } else { (e_idx, s_idx) };
                                let prefix = &ed.text[..min_i];
                                let sel_text = &ed.text[min_i..max_i];
                                let prefix_w = text_w(mem_dc, prefix);
                                let sel_w = text_w(mem_dc, sel_text);
                                let max_r = text_r.right - px(4.0);
                                let box_l = (text_x + prefix_w).min(max_r);
                                let box_r = (text_x + prefix_w + sel_w).min(max_r);
                                if box_r > box_l {
                                    let sel_box = rc(box_l, r.top + px(6.0), box_r, r.bottom - px(6.0));
                                    cv.fill_round(&sel_box, px(2.0), rgb(43, 108, 176), 0.95);
                                }
                            }
                            text(
                                mem_dc,
                                &ed.text,
                                text_r,
                                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                                rgb(255, 255, 255),
                            );
                            if ed.selection.is_none() && ed.caret_visible {
                                let cursor_idx = ed.cursor.min(ed.text.len());
                                let text_up_to_cursor = &ed.text[..cursor_idx];
                                let cw = text_w(mem_dc, text_up_to_cursor);
                                let caret_x = (text_x + cw).min(text_r.right - px(4.0));
                                let caret_y0 = r.top + px(8.0);
                                let caret_y1 = r.bottom - px(8.0);
                                cv.fill_rect(&rc(caret_x, caret_y0, caret_x + px(1.5).max(1), caret_y1), rgb(255, 255, 255), 1.0);
                            }
                        } else {
                            SelectObject(mem_dc, ui_semi.into());
                            text(
                                mem_dc,
                                &folder.name,
                                text_r,
                                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0 | DT_END_ELLIPSIS.0,
                                if is_hover { rgb(245, 245, 250) } else { rgb(220, 222, 232) },
                            );
                        }
                    }
                }

                // 2. Divider between folders/pinned tabs and "+ New Tab"
                if let Some(ref div) = l.folder_divider {
                    if div.bottom >= l.list_clip.top && div.top <= l.list_clip.bottom {
                        cv.fill_rect(div, rgb(255, 255, 255), 0.12);
                    }
                }

                // 3. "+ New Tab" row
                let nt = l.new_tab;
                if nt.bottom >= l.list_clip.top && nt.top <= l.list_clip.bottom {
                    let is_new_hover = st.hovered == SidebarHit::NewTab;
                    if is_new_hover {
                        cv.fill_round(&nt, px(7.0), t.white, t.hover_a);
                    }
                    SelectObject(mem_dc, icon.into());
                    text(mem_dc, "\u{E710}", rc(nt.left + px(8.0), nt.top, nt.left + px(34.0), nt.bottom),
                         DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text_dim);
                    SelectObject(mem_dc, ui.into());
                    text(mem_dc, "New Tab", rc(nt.left + px(42.0), nt.top, nt.right, nt.bottom),
                         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text_dim);
                }

                // 4. Stitched background for tabs that are split together
                if let Some((ga, gb)) = st.split_group.filter(|g| g.1 == g.0 + 1) {
                    if let (Some(ra), Some(rb)) = (l.tabs.get(ga), l.tabs.get(gb)) {
                        let combined = rc(ra.left, ra.top, rb.right, rb.bottom);
                        cv.fill_round(&combined, px(7.0), t.white, t.surface_a);
                        cv.stroke_round(&combined, px(7.0), t.white, t.stroke_a, 1.0);
                        let mx = (ra.right + rb.left) / 2;
                        cv.fill_rect(&rc(mx, ra.top + px(8.0), mx + 1, ra.bottom - px(8.0)), t.white, 0.12);
                    }
                }

                // 5. Tabs list (all tabs: folder children, pinned section tabs, normal tabs)
                for (i, r) in l.tabs.iter().enumerate() {
                    if r.bottom < l.list_clip.top || r.top > l.list_clip.bottom {
                        continue;
                    }
                    let tab_item = &st.tabs[i];
                    let progress = tab_item.anim_progress.clamp(0.0, 1.0);
                    if progress <= 0.001 || (r.bottom - r.top) < px(6.0) {
                        continue;
                    }
                    let ease = 1.0 - (1.0 - progress).powi(3);

                    let is_drag_source = matches!(st.drag_state.as_ref(), Some(d) if d.source == DragSource::Tab(i));
                    let alpha_mult = if is_drag_source { 0.25 } else { 1.0 };
                    let is_active = st.active == Some(ActiveTab::Tab(i));
                    let is_hover = matches!(st.hovered, SidebarHit::Tab(n) | SidebarHit::TabClose(n) if n == i);
                    if st.selected.contains(&i) {
                        cv.fill_round(r, px(7.0), t.accent, 0.28 * ease * alpha_mult);
                        cv.stroke_round(r, px(7.0), t.accent, 0.8 * ease * alpha_mult, 1.0);
                    }
                    if is_active {
                        cv.fill_round(r, px(7.0), t.white, t.active_a * ease * alpha_mult);
                        cv.stroke_round(r, px(7.0), t.white, t.stroke_a * ease * alpha_mult, 1.0);
                    } else if is_hover {
                        cv.fill_round(r, px(7.0), t.white, t.hover_a * ease * alpha_mult);
                    }

                    if (r.bottom - r.top) < px(14.0) {
                        continue;
                    }

                    let fav = px(15.0);
                    let fx = r.left + px(10.0);
                    let fy = r.top + (r.bottom - r.top - fav) / 2;
                    let fav_rect = rc(fx, fy, fx + fav, fy + fav);
                    draw_webapp_icon(&cv, &fav_rect, &st.tabs[i], px(7.0), px);

                    let show_close = is_hover || is_active;
                    let cb = px(20.0);
                    let is_muted = st.tabs[i].is_muted;
                    let right_limit = if show_close && is_muted {
                        r.right - cb * 2 - px(14.0)
                    } else if show_close || is_muted {
                        r.right - cb - px(12.0)
                    } else {
                        r.right - px(10.0)
                    };
                    let is_loaded = st.tabs[i].is_loaded;
                    let text_color = if is_active {
                        t.text
                    } else if is_loaded {
                        t.text_dim
                    } else {
                        rgb(125, 130, 142) // Sleeping tab title color
                    };

                    let is_editing = match st.inline_edit {
                        Some(ref ed) => ed.target == InlineEditTarget::Tab(i),
                        None => false,
                    };

                    let text_left = fx + fav + px(10.0);
                    let title_rect = rc(text_left, r.top, right_limit, r.bottom);

                    if is_editing {
                        let ed = st.inline_edit.as_ref().unwrap();
                        SelectObject(mem_dc, ui.into());
                        if let Some((sel_s, sel_e)) = ed.selection {
                            let s_idx = sel_s.min(ed.text.len());
                            let e_idx = sel_e.min(ed.text.len());
                            let (min_i, max_i) = if s_idx <= e_idx { (s_idx, e_idx) } else { (e_idx, s_idx) };
                            let prefix = &ed.text[..min_i];
                            let sel_text = &ed.text[min_i..max_i];
                            let prefix_w = text_w(mem_dc, prefix);
                            let sel_w = text_w(mem_dc, sel_text);
                            let max_r = right_limit - px(4.0);
                            let box_l = (text_left + prefix_w).min(max_r);
                            let box_r = (text_left + prefix_w + sel_w).min(max_r);
                            if box_r > box_l {
                                let sel_box = rc(box_l, r.top + px(6.0), box_r, r.bottom - px(6.0));
                                cv.fill_round(&sel_box, px(2.0), rgb(43, 108, 176), 0.95);
                            }
                        }
                        text(
                            mem_dc,
                            &ed.text,
                            title_rect,
                            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                            rgb(255, 255, 255),
                        );
                        if ed.selection.is_none() && ed.caret_visible {
                            let cursor_idx = ed.cursor.min(ed.text.len());
                            let text_up_to_cursor = &ed.text[..cursor_idx];
                            let cw = text_w(mem_dc, text_up_to_cursor);
                            let caret_x = (text_left + cw).min(right_limit - px(4.0));
                            let caret_y0 = r.top + px(8.0);
                            let caret_y1 = r.bottom - px(8.0);
                            cv.fill_rect(&rc(caret_x, caret_y0, caret_x + px(1.5).max(1), caret_y1), rgb(255, 255, 255), 1.0);
                        }
                    } else {
                        SelectObject(mem_dc, ui.into());
                        text(
                            mem_dc,
                            &st.tabs[i].title,
                            title_rect,
                            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                            text_color,
                        );
                    }

                    let cy = r.top + (r.bottom - r.top - cb) / 2;

                    if is_muted {
                        let mx = if show_close { r.right - cb * 2 - px(10.0) } else { r.right - cb - px(8.0) };
                        let mute_r = rc(mx, cy, mx + cb, cy + cb);
                        SelectObject(mem_dc, icon_cap.into());
                        text(mem_dc, "\u{E74F}", mute_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, rgb(255, 130, 130));
                    }

                    if show_close {
                        let close = rc(r.right - cb - px(8.0), cy, r.right - px(8.0), cy + cb);
                        if st.hovered == SidebarHit::TabClose(i) {
                            cv.fill_round(&close, px(6.0), t.white, t.active_a);
                        }
                        SelectObject(mem_dc, icon_cap.into());
                        text(mem_dc, "\u{E711}", close, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text);
                    }
                }
            }
            cv.reset_clip();

            // Floating Jump to Active Tab pill
            if let Some(ref pill) = l.jump_to_active_pill {
                let is_hover = st.hovered == SidebarHit::JumpToActiveTab;
                let radius = (pill.bottom - pill.top) / 2;
                cv.fill_round(pill, radius, rgb(30, 33, 42), 0.96);
                cv.stroke_round(pill, radius, rgb(255, 255, 255), if is_hover { 0.38 } else { 0.20 }, 1.0);

                let fav_size = px(16.0);
                let fy = pill.top + (pill.bottom - pill.top - fav_size) / 2;
                let fx = pill.left + px(9.0);
                let fr = rc(fx, fy, fx + fav_size, fy + fav_size);

                if let Some(ref fav) = l.jump_active_favicon {
                    draw_favicon_to_canvas(&cv, fav, &fr);
                } else {
                    cv.fill_round(&fr, fav_size / 2, rgb(235, 60, 60), 0.9);
                }

                let arrow_r = rc(fx + fav_size + px(4.0), pill.top, pill.right - px(6.0), pill.bottom);
                let arrow_font = make_font(w!("Segoe UI Symbol"), px(12.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
                let old_af = SelectObject(mem_dc, arrow_font.into());
                text(
                    mem_dc,
                    "\u{2193}", // Down arrow ↓
                    arrow_r,
                    DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                    if is_hover { rgb(255, 255, 255) } else { rgb(215, 218, 228) },
                );
                let _ = SelectObject(mem_dc, old_af);
                let _ = DeleteObject(arrow_font.into());
            }

            // ── Footer ── (top border removed, settings placed at right where sidebar button was, buttons enlarged)
            let icon_footer = make_font(w!("Segoe Fluent Icons"), px(17.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
            let old_footer_font = SelectObject(mem_dc, icon_footer.into());
            for (r, glyph, id) in [
                (l.downloads, "\u{E896}", SidebarHit::Downloads),
                (l.settings, "\u{E713}", SidebarHit::Settings),
            ] {
                if st.hovered == id {
                    cv.fill_round(&r, px(8.0), t.white, t.hover_a);
                }
                text(mem_dc, glyph, r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
                     if st.hovered == id { t.text } else { t.text_dim });
            }
            SelectObject(mem_dc, old_footer_font);
            let _ = DeleteObject(icon_footer.into());

            // ── Caption buttons (Minimize, Maximize, Close) ──
            let bw = px(46.0);
            let bh = px(32.0);
            let close_r = rc(width - bw, 0, width, bh);
            let max_r = rc(width - bw * 2, 0, width - bw, bh);
            let min_r = rc(width - bw * 3, 0, width - bw * 2, bh);

            let state_bg = |b: CaptionButton, hover: COLORREF, press: COLORREF, r: &RECT| {
                if pressed_button == b {
                    cv.fill_rect(r, press, 1.0);
                } else if hovered_button == b {
                    cv.fill_rect(r, hover, 1.0);
                }
            };
            state_bg(CaptionButton::Minimize, t.caption_hover, t.caption_press, &min_r);
            state_bg(CaptionButton::Maximize, t.caption_hover, t.caption_press, &max_r);
            state_bg(CaptionButton::Close, t.close_hover, t.close_press, &close_r);

            SelectObject(mem_dc, icon_cap.into());
            let fmt = DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0;
            let glyph_col = rgb(208, 208, 208);
            text(mem_dc, "\u{E921}", min_r, fmt, glyph_col);
            let max_glyph = if IsZoomed(hwnd).as_bool() { "\u{E923}" } else { "\u{E922}" };
            text(mem_dc, max_glyph, max_r, fmt, glyph_col);
            let close_active = hovered_button == CaptionButton::Close || pressed_button == CaptionButton::Close;
            text(mem_dc, "\u{E8BB}", close_r, fmt, if close_active { rgb(255, 255, 255) } else { glyph_col });

            // ── Drag & Drop Overlay ──
            if let Some(ref drag) = st.drag_state {
                cv.reset_clip();

                let lp = px(14.0);
                let rp = px(14.0);
                let row_l = lp - px(4.0);
                let row_r = width - rp + px(4.0);

                // 1. Draw destination indicator
                match drag.target {
                    Some(DragTarget::Favorites(_)) => {
                        if let Some(ref ghost_r) = l.ghost_pinned {
                            let tile_rad = px(8.0);
                            cv.fill_round(ghost_r, tile_rad, t.white, 0.08);
                            cv.stroke_round(ghost_r, tile_rad, rgb(180, 190, 255), 0.45, 1.2);
                            if let Some(ref letter) = drag.item.custom_letter {
                                let letter_font = make_font(w!("Segoe UI"), px(16.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
                                let old = SelectObject(mem_dc, letter_font.into());
                                text(mem_dc, letter, *ghost_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(180, 190, 220));
                                SelectObject(mem_dc, old);
                                let _ = DeleteObject(letter_font.into());
                            } else if let Some(ref fav) = drag.item.favicon {
                                let fav_sz = px(18.0);
                                let fx = (ghost_r.left + ghost_r.right - fav_sz) / 2;
                                let fy = (ghost_r.top + ghost_r.bottom - fav_sz) / 2;
                                let fr = rc(fx, fy, fx + fav_sz, fy + fav_sz);
                                cv.draw_image_scaled_custom(&fr, &fav.rgba, fav.width, fav.height, 0.45, false);
                            }
                        }
                    }
                    Some(DragTarget::Folder(f_idx)) => {
                        if let Some(fr) = l.folders.get(f_idx) {
                            let rad = px(8.0);
                            cv.fill_round(fr, rad, rgb(130, 140, 230), 0.30);
                            cv.stroke_round(fr, rad, rgb(180, 195, 255), 0.85, 1.5);
                        }
                    }
                    Some(DragTarget::FolderInsert { folder_idx, insert_idx }) => {
                        let folder_tabs: Vec<usize> = st.tabs.iter().enumerate()
                            .filter(|(_, t)| t.folder_id.as_deref() == Some(&st.folders[folder_idx].id))
                            .map(|(idx, _)| idx)
                            .collect();
                        let indent_l = row_l + px(16.0);
                        let y_insert = if folder_tabs.is_empty() {
                            l.folders[folder_idx].bottom + px(4.0)
                        } else if insert_idx == 0 {
                            l.tabs[folder_tabs[0]].top - px(2.0)
                        } else if insert_idx < folder_tabs.len() {
                            (l.tabs[folder_tabs[insert_idx - 1]].bottom + l.tabs[folder_tabs[insert_idx]].top) / 2
                        } else {
                            l.tabs[*folder_tabs.last().unwrap()].bottom + px(2.0)
                        };
                        draw_drag_insertion_line(&cv, indent_l, row_r, y_insert, px);
                    }
                    Some(DragTarget::PinnedSection(slot)) => {
                        let pinned_tabs: Vec<usize> = st.tabs.iter().enumerate()
                            .filter(|(_, t)| t.folder_id.as_deref() == Some(PINNED_SECTION_ID))
                            .map(|(idx, _)| idx)
                            .collect();
                        let y_insert = if pinned_tabs.is_empty() {
                            let top = l.folders.last().map(|r| r.bottom).unwrap_or(l.list_clip.top);
                            let bottom = l.folder_divider.map(|d| d.top).unwrap_or(l.new_tab.top);
                            (top + bottom) / 2
                        } else if slot == 0 {
                            let top_bound = l.folders.last().map(|r| r.bottom).unwrap_or(l.list_clip.top);
                            (top_bound + l.tabs[pinned_tabs[0]].top) / 2
                        } else if slot < pinned_tabs.len() {
                            (l.tabs[pinned_tabs[slot - 1]].bottom + l.tabs[pinned_tabs[slot]].top) / 2
                        } else {
                            let bot_bound = l.folder_divider.map(|d| d.top).unwrap_or(l.new_tab.top);
                            (l.tabs[*pinned_tabs.last().unwrap()].bottom + bot_bound) / 2
                        };
                        draw_drag_insertion_line(&cv, row_l, row_r, y_insert, px);
                    }
                    Some(DragTarget::NormalTabs(slot)) => {
                        let normal_tabs: Vec<usize> = st.tabs.iter().enumerate()
                            .filter(|(_, t)| t.folder_id.is_none())
                            .map(|(idx, _)| idx)
                            .collect();
                        let y_insert = if normal_tabs.is_empty() {
                            l.new_tab.bottom + px(6.0)
                        } else if slot == 0 {
                            (l.new_tab.bottom + l.tabs[normal_tabs[0]].top) / 2
                        } else if slot < normal_tabs.len() {
                            (l.tabs[normal_tabs[slot - 1]].bottom + l.tabs[normal_tabs[slot]].top) / 2
                        } else {
                            l.tabs[*normal_tabs.last().unwrap()].bottom + px(2.0)
                        };
                        draw_drag_insertion_line(&cv, row_l, row_r, y_insert, px);
                    }
                    None => {}
                }

                // 2. Draw floating carried item preview following cursor
                let sx = drag.cur_x - sidebar_rect.left;
                let sy = drag.cur_y - sidebar_rect.top;
                let is_fav_preview = matches!(drag.target, Some(DragTarget::Favorites(_)));

                if is_fav_preview {
                    // Square Favorite Tile (Image 0)
                    let sz = px(44.0);
                    let left = sx - sz / 2;
                    let top = sy - sz / 2;
                    let r = rc(left, top, left + sz, top + sz);
                    let rad = px(8.0);

                    // Soft drop shadow
                    let shadow_r = rc(left - px(3.0), top - px(1.0), left + sz + px(3.0), top + sz + px(5.0));
                    cv.fill_round(&shadow_r, rad + px(2.0), rgb(0, 0, 0), 0.45);

                    // Elevated white / light grey card
                    cv.fill_round(&r, rad, rgb(228, 229, 235), 0.98);
                    cv.stroke_round(&r, rad, rgb(180, 182, 195), 0.90, 1.2);

                    // Centered Favicon or Letter
                    if let Some(ref letter) = drag.item.custom_letter {
                        let font = make_font(w!("Segoe UI"), px(18.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
                        let old_f = SelectObject(mem_dc, font.into());
                        text(mem_dc, letter, r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(20, 20, 25));
                        SelectObject(mem_dc, old_f);
                        let _ = DeleteObject(font.into());
                    } else if let Some(ref fav) = drag.item.favicon {
                        let fav_sz = px(22.0);
                        let fx = left + (sz - fav_sz) / 2;
                        let fy = top + (sz - fav_sz) / 2;
                        let fav_r = rc(fx, fy, fx + fav_sz, fy + fav_sz);
                        draw_favicon_to_canvas(&cv, fav, &fav_r);
                    } else {
                        let letter = drag.item.title
                            .chars()
                            .find(|c| c.is_alphanumeric())
                            .map(|c| c.to_uppercase().to_string())
                            .unwrap_or_else(|| "W".to_string());
                        let font = make_font(w!("Segoe UI"), px(18.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
                        let old_f = SelectObject(mem_dc, font.into());
                        text(mem_dc, &letter, r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(20, 20, 25));
                        SelectObject(mem_dc, old_f);
                        let _ = DeleteObject(font.into());
                    }
                } else {
                    // Carried Tab Card (Image 1 & Image 2)
                    let card_w = (width - px(24.0)).min(px(240.0));
                    let card_h = px(34.0);
                    let left = (sx - px(40.0)).clamp(px(8.0), (width - card_w - px(8.0)).max(px(8.0)));
                    let top = (sy - card_h - px(8.0)).max(px(8.0));
                    let r = rc(left, top, left + card_w, top + card_h);
                    let rad = px(8.0);

                    // Soft shadow
                    let shadow_r = rc(left - px(4.0), top - px(1.0), left + card_w + px(4.0), top + card_h + px(5.0));
                    cv.fill_round(&shadow_r, rad + px(2.0), rgb(0, 0, 0), 0.40);

                    // Elevated translucent card (matching Arc style: semi-transparent, frosted white outline)
                    cv.fill_round(&r, rad, rgb(48, 52, 64), 0.88);
                    cv.stroke_round(&r, rad, rgb(255, 255, 255), 0.35, 1.2);

                    // Favicon on left
                    let fav_sz = px(16.0);
                    let fx = left + px(10.0);
                    let fy = top + (card_h - fav_sz) / 2;
                    let fav_r = rc(fx, fy, fx + fav_sz, fy + fav_sz);
                    if let Some(ref fav) = drag.item.favicon {
                        draw_favicon_to_canvas(&cv, fav, &fav_r);
                    } else {
                        let icon_font = make_font(w!("Segoe Fluent Icons"), px(12.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
                        let old_f = SelectObject(mem_dc, icon_font.into());
                        text(mem_dc, "\u{E774}", fav_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, rgb(140, 144, 156));
                        SelectObject(mem_dc, old_f);
                        let _ = DeleteObject(icon_font.into());
                    }

                    // Title
                    let title_r = rc(left + px(32.0), top, left + card_w - px(26.0), top + card_h);
                    SelectObject(mem_dc, ui.into());
                    text(mem_dc, &drag.item.title, title_r, DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0, rgb(235, 238, 248));

                    // Close 'x' icon
                    let close_r = rc(left + card_w - px(24.0), top, left + card_w - px(8.0), top + card_h);
                    SelectObject(mem_dc, icon_cap.into());
                    text(mem_dc, "\u{E8BB}", close_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, rgb(160, 165, 180));
                }
            }

            // Blit + cleanup
            let _ = GdiFlush();
            let _ = SelectObject(mem_dc, old_font);
            for f in [ui, ui_semi, icon, icon_cap] {
                let _ = DeleteObject(f.into());
            }
            let _ = BitBlt(hdc, sidebar_rect.left, sidebar_rect.top, width, height, Some(mem_dc), 0, 0, SRCCOPY);
            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);
        }
    }

    /// Renders the minimal top bar when sidebar is collapsed.
    pub fn render_collapsed_header(
        &self,
        hwnd: HWND,
        hdc: HDC,
        rect: RECT,
        hovered_hit: SidebarHit,
        pressed_button: CaptionButton,
    ) {
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return;
        }
        let t = &self.theme;
        let s = Self::scale(hwnd);
        let px = |v: f32| (v * s).round() as i32;

        unsafe {
            let mem_dc = CreateCompatibleDC(Some(hdc));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0 as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let mem_bmp = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    let _ = DeleteDC(mem_dc);
                    return;
                }
            };
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());
            let _ = SetBkMode(mem_dc, TRANSPARENT);

            let cv = Canvas {
                dc: mem_dc,
                bits: bits as *mut u32,
                w: width,
                h: height,
                clip: Cell::new(rc(0, 0, width, height)),
            };

            // Dark caption bar background
            cv.fill_rect(&rc(0, 0, width, height), t.bg, 1.0);

            let icon = make_font(w!("Segoe Fluent Icons"), px(13.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
            let icon_cap = make_font(w!("Segoe Fluent Icons"), px(10.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
            let old_font = SelectObject(mem_dc, icon.into());

            let bw = px(46.0);
            let bh = px(32.0);

            // Toggle sidebar button at right beside minimize (using Boxicons sidebar dock SVG)
            let toggle_r = rc(width - bw * 4, 0, width - bw * 3, bh);
            if hovered_hit == SidebarHit::ToggleSidebar {
                cv.fill_rect(&toggle_r, t.caption_hover, 1.0);
            }
            let tog_col = if hovered_hit == SidebarHit::ToggleSidebar { t.text } else { t.text_dim };
            draw_sidebar_icon(&cv, &toggle_r, tog_col, 1.0, px);

            // Caption buttons
            let min_r = rc(width - bw * 3, 0, width - bw * 2, bh);
            let max_r = rc(width - bw * 2, 0, width - bw, bh);
            let close_r = rc(width - bw, 0, width, bh);

            let hov_btn = match hovered_hit {
                SidebarHit::Caption(b) => b,
                _ => CaptionButton::None,
            };

            let state_bg = |b: CaptionButton, hover: COLORREF, press: COLORREF, r: &RECT| {
                if pressed_button == b {
                    cv.fill_rect(r, press, 1.0);
                } else if hov_btn == b {
                    cv.fill_rect(r, hover, 1.0);
                }
            };
            state_bg(CaptionButton::Minimize, t.caption_hover, t.caption_press, &min_r);
            state_bg(CaptionButton::Maximize, t.caption_hover, t.caption_press, &max_r);
            state_bg(CaptionButton::Close, t.close_hover, t.close_press, &close_r);

            SelectObject(mem_dc, icon_cap.into());
            let fmt = DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0;
            let glyph_col = rgb(208, 208, 208);
            text(mem_dc, "\u{E921}", min_r, fmt, glyph_col);
            let max_glyph = if IsZoomed(hwnd).as_bool() { "\u{E923}" } else { "\u{E922}" };
            text(mem_dc, max_glyph, max_r, fmt, glyph_col);
            let close_active = hov_btn == CaptionButton::Close || pressed_button == CaptionButton::Close;
            text(mem_dc, "\u{E8BB}", close_r, fmt, if close_active { rgb(255, 255, 255) } else { glyph_col });

            let _ = GdiFlush();
            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(icon.into());
            let _ = DeleteObject(icon_cap.into());

            let _ = BitBlt(hdc, rect.left, rect.top, width, height, Some(mem_dc), 0, 0, SRCCOPY);
            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);
        }
    }
}

// ───────────────────── Anti-aliased software canvas ─────────────────────

pub struct Canvas {
    pub dc: HDC,
    pub bits: *mut u32, // 0xAARRGGBB, top-down
    pub w: i32,
    pub h: i32,
    pub clip: Cell<RECT>,
}

impl Canvas {
    pub fn set_clip(&self, r: RECT) {
        self.clip.set(r);
        unsafe {
            let _ = IntersectClipRect(self.dc, r.left, r.top, r.right, r.bottom);
        }
    }

    pub fn reset_clip(&self) {
        self.clip.set(rc(0, 0, self.w, self.h));
        unsafe {
            let _ = SelectClipRgn(self.dc, None);
        }
    }

    pub fn sync(&self) {
        unsafe {
            let _ = GdiFlush();
        }
    }

    #[inline]
    pub fn blend(&self, x: i32, y: i32, color: COLORREF, a: f32) {
        let c = self.clip.get();
        if a <= 0.0 || x < c.left || x >= c.right || y < c.top || y >= c.bottom {
            return;
        }
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        unsafe {
            let p = self.bits.add((y * self.w + x) as usize);
            let dst = *p;
            let (sr, sg, sb) = (color.0 & 0xff, (color.0 >> 8) & 0xff, (color.0 >> 16) & 0xff);
            let (dr, dg, db) = ((dst >> 16) & 0xff, (dst >> 8) & 0xff, dst & 0xff);
            let mix = |s: u32, d: u32| -> u32 {
                (d as f32 + (s as f32 - d as f32) * a).round().clamp(0.0, 255.0) as u32
            };
            *p = 0xFF00_0000 | (mix(sr, dr) << 16) | (mix(sg, dg) << 8) | mix(sb, db);
        }
    }

    pub fn fill_rect(&self, r: &RECT, color: COLORREF, a: f32) {
        self.sync();
        for y in r.top..r.bottom {
            for x in r.left..r.right {
                self.blend(x, y, color, a);
            }
        }
    }

    pub fn fill_round(&self, r: &RECT, radius: i32, color: COLORREF, a: f32) {
        self.sync();
        let g = RoundGeom::new(r, radius);
        for y in (r.top - 1)..(r.bottom + 1) {
            for x in (r.left - 1)..(r.right + 1) {
                let cov = (0.5 - g.dist(x, y)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, color, a * cov);
                }
            }
        }
    }

    pub fn draw_line_aa(
        &self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: COLORREF,
        a: f32,
        width: f32,
    ) {
        if a <= 0.0 || width <= 0.0 {
            return;
        }
        self.sync();
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len_sq = dx * dx + dy * dy;
        if len_sq <= 0.001 {
            return;
        }
        let half_w = width * 0.5;
        let min_x = (x0.min(x1) - half_w - 1.0).floor() as i32;
        let max_x = (x0.max(x1) + half_w + 1.0).ceil() as i32;
        let min_y = (y0.min(y1) - half_w - 1.0).floor() as i32;
        let max_y = (y0.max(y1) + half_w + 1.0).ceil() as i32;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let t = (((px - x0) * dx + (py - y0) * dy) / len_sq).clamp(0.0, 1.0);
                let qx = x0 + t * dx;
                let qy = y0 + t * dy;
                let dist = ((px - qx) * (px - qx) + (py - qy) * (py - qy)).sqrt();
                let cov = (half_w + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, color, a * cov);
                }
            }
        }
    }

    pub fn stroke_round(&self, r: &RECT, radius: i32, color: COLORREF, a: f32, width: f32) {
        self.sync();
        let g = RoundGeom::new(r, radius);
        for y in (r.top - 1)..(r.bottom + 1) {
            for x in (r.left - 1)..(r.right + 1) {
                let d = g.dist(x, y);
                let outer = (0.5 - d).clamp(0.0, 1.0);
                let inner = (0.5 - (d + width)).clamp(0.0, 1.0);
                let ring = (outer - inner).max(0.0);
                if ring > 0.0 {
                    self.blend(x, y, color, a * ring);
                }
            }
        }
    }

    pub fn stroke_round_gradient(
        &self,
        r: &RECT,
        radius: i32,
        palette: &[COLORREF; 4],
        a: f32,
        width: f32,
    ) {
        if a <= 0.0 || width <= 0.0 {
            return;
        }
        self.sync();
        let g = RoundGeom::new(r, radius);
        let (cx, cy) = g.center();

        let pad = (width + 2.0).ceil() as i32;
        let y_min = r.top - pad;
        let y_max = r.bottom + pad;
        let x_min = r.left - pad;
        let x_max = r.right + pad;

        let c_same = palette[0] == palette[1] && palette[1] == palette[2] && palette[2] == palette[3];

        for y in y_min..y_max {
            for x in x_min..x_max {
                let d = g.dist(x, y);
                let outer = (0.5 - d).clamp(0.0, 1.0);
                let inner = (0.5 - (d + width)).clamp(0.0, 1.0);
                let ring = (outer - inner).max(0.0);
                if ring <= 0.0 {
                    continue;
                }

                let col = if c_same {
                    palette[0]
                } else {
                    let angle = (y as f32 + 0.5 - cy).atan2(x as f32 + 0.5 - cx);
                    let norm = (angle + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
                    let seg = norm * 4.0;
                    let (c1, c2, t) = if seg < 1.0 {
                        (palette[0], palette[1], seg)
                    } else if seg < 2.0 {
                        (palette[1], palette[2], seg - 1.0)
                    } else if seg < 3.0 {
                        (palette[2], palette[3], seg - 2.0)
                    } else {
                        (palette[3], palette[0], (seg - 3.0).min(1.0))
                    };

                    let (r1, g1, b1) = (c1.0 & 0xff, (c1.0 >> 8) & 0xff, (c1.0 >> 16) & 0xff);
                    let (r2, g2, b2) = (c2.0 & 0xff, (c2.0 >> 8) & 0xff, (c2.0 >> 16) & 0xff);
                    let cr = (r1 as f32 + (r2 as f32 - r1 as f32) * t).round() as u8;
                    let cg = (g1 as f32 + (g2 as f32 - g1 as f32) * t).round() as u8;
                    let cb = (b1 as f32 + (b2 as f32 - b1 as f32) * t).round() as u8;
                    rgb(cr, cg, cb)
                };

                self.blend(x, y, col, a * ring);
            }
        }
    }

    pub fn glow_round_gradient(
        &self,
        r: &RECT,
        radius: i32,
        palette: &[COLORREF; 4],
        max_alpha: f32,
        glow_spread: f32,
    ) {
        if glow_spread <= 0.0 || max_alpha <= 0.0 {
            return;
        }
        self.sync();
        let g = RoundGeom::new(r, radius);
        let (cx, cy) = g.center();

        let pad = (glow_spread + 2.0).ceil() as i32;
        let y_min = r.top - pad;
        let y_max = r.bottom + pad;
        let x_min = r.left - pad;
        let x_max = r.right + pad;

        let c_same = palette[0] == palette[1] && palette[1] == palette[2] && palette[2] == palette[3];

        for y in y_min..y_max {
            for x in x_min..x_max {
                let d = g.dist(x, y);
                if d < -2.0 || d > glow_spread {
                    continue;
                }

                let cov = if d <= 0.0 {
                    1.0
                } else {
                    let t = 1.0 - (d / glow_spread);
                    t * t
                };

                let col = if c_same {
                    palette[0]
                } else {
                    let angle = (y as f32 + 0.5 - cy).atan2(x as f32 + 0.5 - cx);
                    let norm = (angle + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
                    let seg = norm * 4.0;
                    let (c1, c2, t) = if seg < 1.0 {
                        (palette[0], palette[1], seg)
                    } else if seg < 2.0 {
                        (palette[1], palette[2], seg - 1.0)
                    } else if seg < 3.0 {
                        (palette[2], palette[3], seg - 2.0)
                    } else {
                        (palette[3], palette[0], (seg - 3.0).min(1.0))
                    };

                    let (r1, g1, b1) = (c1.0 & 0xff, (c1.0 >> 8) & 0xff, (c1.0 >> 16) & 0xff);
                    let (r2, g2, b2) = (c2.0 & 0xff, (c2.0 >> 8) & 0xff, (c2.0 >> 16) & 0xff);
                    let cr = (r1 as f32 + (r2 as f32 - r1 as f32) * t).round() as u8;
                    let cg = (g1 as f32 + (g2 as f32 - g1 as f32) * t).round() as u8;
                    let cb = (b1 as f32 + (b2 as f32 - b1 as f32) * t).round() as u8;
                    rgb(cr, cg, cb)
                };

                self.blend(x, y, col, max_alpha * cov);
            }
        }
    }

    pub fn fill_round_gradient(
        &self,
        r: &RECT,
        radius: i32,
        palette: &[COLORREF; 4],
        a: f32,
    ) {
        if a <= 0.0 {
            return;
        }
        self.sync();
        let g = RoundGeom::new(r, radius);
        let (cx, cy) = g.center();

        let c_same = palette[0] == palette[1] && palette[1] == palette[2] && palette[2] == palette[3];

        for y in (r.top - 1)..(r.bottom + 1) {
            for x in (r.left - 1)..(r.right + 1) {
                let cov = (0.5 - g.dist(x, y)).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }

                let col = if c_same {
                    palette[0]
                } else {
                    let angle = (y as f32 + 0.5 - cy).atan2(x as f32 + 0.5 - cx);
                    let norm = (angle + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
                    let seg = norm * 4.0;
                    let (c1, c2, t) = if seg < 1.0 {
                        (palette[0], palette[1], seg)
                    } else if seg < 2.0 {
                        (palette[1], palette[2], seg - 1.0)
                    } else if seg < 3.0 {
                        (palette[2], palette[3], seg - 2.0)
                    } else {
                        (palette[3], palette[0], (seg - 3.0).min(1.0))
                    };

                    let (r1, g1, b1) = (c1.0 & 0xff, (c1.0 >> 8) & 0xff, (c1.0 >> 16) & 0xff);
                    let (r2, g2, b2) = (c2.0 & 0xff, (c2.0 >> 8) & 0xff, (c2.0 >> 16) & 0xff);
                    let cr = (r1 as f32 + (r2 as f32 - r1 as f32) * t).round() as u8;
                    let cg = (g1 as f32 + (g2 as f32 - g1 as f32) * t).round() as u8;
                    let cb = (b1 as f32 + (b2 as f32 - b1 as f32) * t).round() as u8;
                    rgb(cr, cg, cb)
                };

                self.blend(x, y, col, a * cov);
            }
        }
    }

    pub fn draw_image_scaled(&self, r: &RECT, img_rgba: &[u8], img_w: u32, img_h: u32) {
        self.draw_image_scaled_custom(r, img_rgba, img_w, img_h, 1.0, false);
    }

    pub fn draw_image_scaled_custom(&self, r: &RECT, img_rgba: &[u8], img_w: u32, img_h: u32, opacity: f32, grayscale: bool) {
        if img_w == 0 || img_h == 0 {
            return;
        }
        self.sync();
        let dst_w_orig = r.right - r.left;
        let dst_h_orig = r.bottom - r.top;
        if dst_w_orig <= 0 || dst_h_orig <= 0 {
            return;
        }

        // If image is smaller than or equal to target rect, do NOT stretch (prevents blurriness on 16x16 images)
        let (fit_w, fit_h) = if (img_w as i32) <= dst_w_orig && (img_h as i32) <= dst_h_orig {
            (img_w as i32, img_h as i32)
        } else if img_w == img_h {
            (dst_w_orig, dst_h_orig)
        } else {
            let aspect = img_w as f32 / img_h as f32;
            if aspect > 1.0 {
                let fw = dst_w_orig;
                let fh = ((dst_w_orig as f32 / aspect).round() as i32).max(1);
                (fw, fh)
            } else {
                let fh = dst_h_orig;
                let fw = ((dst_h_orig as f32 * aspect).round() as i32).max(1);
                (fw, fh)
            }
        };

        let off_x = (dst_w_orig - fit_w) / 2;
        let off_y = (dst_h_orig - fit_h) / 2;
        let target_r = RECT {
            left: r.left + off_x,
            top: r.top + off_y,
            right: r.left + off_x + fit_w,
            bottom: r.top + off_y + fit_h,
        };
        let dst_w = fit_w;
        let dst_h = fit_h;

        // 1:1 Pixel-exact rendering: zero interpolation, 100% crisp
        if img_w as i32 == dst_w && img_h as i32 == dst_h {
            for dy in 0..dst_h {
                let y = target_r.top + dy;
                let row_offset = (dy as usize) * (img_w as usize);
                for dx in 0..dst_w {
                    let x = target_r.left + dx;
                    let idx = (row_offset + (dx as usize)) * 4;
                    if idx + 3 < img_rgba.len() {
                        let a = (img_rgba[idx + 3] as f32 / 255.0) * opacity;
                        if a > 0.005 {
                            let mut cr = img_rgba[idx];
                            let mut cg = img_rgba[idx + 1];
                            let mut cb = img_rgba[idx + 2];
                            if grayscale {
                                let gray = (0.299 * cr as f32 + 0.587 * cg as f32 + 0.114 * cb as f32).round() as u8;
                                cr = gray;
                                cg = gray;
                                cb = gray;
                            }
                            let col = rgb(cr, cg, cb);
                            self.blend(x, y, col, a);
                        }
                    }
                }
            }
            return;
        }

        let scale_x = img_w as f32 / dst_w as f32;
        let scale_y = img_h as f32 / dst_h as f32;

        if scale_x >= 1.0 && scale_y >= 1.0 {
            // High-quality box area averaging with premultiplied alpha (anti-aliased downsampling)
            for dy in 0..dst_h {
                let y = target_r.top + dy;
                let sy0 = (dy as f32 * scale_y).clamp(0.0, img_h as f32);
                let sy1 = ((dy + 1) as f32 * scale_y).clamp(0.0, img_h as f32);

                for dx in 0..dst_w {
                    let x = target_r.left + dx;
                    let sx0 = (dx as f32 * scale_x).clamp(0.0, img_w as f32);
                    let sx1 = ((dx + 1) as f32 * scale_x).clamp(0.0, img_w as f32);

                    let sy_start = sy0.floor() as usize;
                    let sy_end = (sy1.ceil() as usize).min(img_h as usize);
                    let sx_start = sx0.floor() as usize;
                    let sx_end = (sx1.ceil() as usize).min(img_w as usize);

                    let mut sum_r = 0.0f32;
                    let mut sum_g = 0.0f32;
                    let mut sum_b = 0.0f32;
                    let mut sum_a = 0.0f32;
                    let mut total_weight = 0.0f32;

                    for py in sy_start..sy_end {
                        let y_top = py as f32;
                        let y_bot = (py + 1) as f32;
                        let y_overlap = (sy1.min(y_bot) - sy0.max(y_top)).max(0.0);
                        if y_overlap <= 0.0 {
                            continue;
                        }

                        let row_offset = py * img_w as usize;
                        for px in sx_start..sx_end {
                            let x_left = px as f32;
                            let x_right = (px + 1) as f32;
                            let x_overlap = (sx1.min(x_right) - sx0.max(x_left)).max(0.0);
                            if x_overlap <= 0.0 {
                                continue;
                            }

                            let w = x_overlap * y_overlap;
                            let idx = (row_offset + px) * 4;
                            if idx + 3 < img_rgba.len() {
                                let a = img_rgba[idx + 3] as f32 / 255.0;
                                let r = img_rgba[idx] as f32;
                                let g = img_rgba[idx + 1] as f32;
                                let b = img_rgba[idx + 2] as f32;

                                sum_r += r * a * w;
                                sum_g += g * a * w;
                                sum_b += b * a * w;
                                sum_a += a * w;
                                total_weight += w;
                            }
                        }
                    }

                    if total_weight > 0.0 && sum_a > 0.001 {
                        let final_a = ((sum_a / total_weight).clamp(0.0, 1.0)) * opacity;
                        let mut final_r = (sum_r / sum_a).round().clamp(0.0, 255.0) as u8;
                        let mut final_g = (sum_g / sum_a).round().clamp(0.0, 255.0) as u8;
                        let mut final_b = (sum_b / sum_a).round().clamp(0.0, 255.0) as u8;
                        if grayscale {
                            let gray = (0.299 * final_r as f32 + 0.587 * final_g as f32 + 0.114 * final_b as f32).round() as u8;
                            final_r = gray;
                            final_g = gray;
                            final_b = gray;
                        }

                        let col = rgb(final_r, final_g, final_b);
                        self.blend(x, y, col, final_a);
                    }
                }
            }
        } else {
            // Bilinear interpolation for upscaling
            for dy in 0..dst_h {
                let y = target_r.top + dy;
                let sy = ((dy as f32 + 0.5) * scale_y - 0.5).clamp(0.0, (img_h - 1) as f32);
                let sy0 = sy.floor() as usize;
                let sy1 = (sy0 + 1).min(img_h as usize - 1);
                let ty = sy - sy0 as f32;

                for dx in 0..dst_w {
                    let x = target_r.left + dx;
                    let sx = ((dx as f32 + 0.5) * scale_x - 0.5).clamp(0.0, (img_w - 1) as f32);
                    let sx0 = sx.floor() as usize;
                    let sx1 = (sx0 + 1).min(img_w as usize - 1);
                    let tx = sx - sx0 as f32;

                    let sample = |ix: usize, iy: usize| -> (f32, f32, f32, f32) {
                        let idx = (iy * img_w as usize + ix) * 4;
                        if idx + 3 < img_rgba.len() {
                            let a = img_rgba[idx + 3] as f32 / 255.0;
                            (img_rgba[idx] as f32 * a, img_rgba[idx + 1] as f32 * a, img_rgba[idx + 2] as f32 * a, a)
                        } else {
                            (0.0, 0.0, 0.0, 0.0)
                        }
                    };

                    let (r00, g00, b00, a00) = sample(sx0, sy0);
                    let (r10, g10, b10, a10) = sample(sx1, sy0);
                    let (r01, g01, b01, a01) = sample(sx0, sy1);
                    let (r11, g11, b11, a11) = sample(sx1, sy1);

                    let top_r = r00 * (1.0 - tx) + r10 * tx;
                    let top_g = g00 * (1.0 - tx) + g10 * tx;
                    let top_b = b00 * (1.0 - tx) + b10 * tx;
                    let top_a = a00 * (1.0 - tx) + a10 * tx;

                    let bot_r = r01 * (1.0 - tx) + r11 * tx;
                    let bot_g = g01 * (1.0 - tx) + g11 * tx;
                    let bot_b = b01 * (1.0 - tx) + b11 * tx;
                    let bot_a = a01 * (1.0 - tx) + a11 * tx;

                    let fa = (top_a * (1.0 - ty) + bot_a * ty).clamp(0.0, 1.0) * opacity;
                    if fa > 0.005 {
                        let mut fr = ((top_r * (1.0 - ty) + bot_r * ty) / fa).round().clamp(0.0, 255.0) as u8;
                        let mut fg = ((top_g * (1.0 - ty) + bot_g * ty) / fa).round().clamp(0.0, 255.0) as u8;
                        let mut fb = ((top_b * (1.0 - ty) + bot_b * ty) / fa).round().clamp(0.0, 255.0) as u8;
                        if grayscale {
                            let gray = (0.299 * fr as f32 + 0.587 * fg as f32 + 0.114 * fb as f32).round() as u8;
                            fr = gray;
                            fg = gray;
                            fb = gray;
                        }

                        self.blend(x, y, rgb(fr, fg, fb), fa);
                    }
                }
            }
        }
    }
}

pub struct RoundGeom {
    cx: f32,
    cy: f32,
    hx: f32,
    hy: f32,
    rad: f32,
}

impl RoundGeom {
    pub fn new(r: &RECT, radius: i32) -> Self {
        let hx = (r.right - r.left) as f32 / 2.0;
        let hy = (r.bottom - r.top) as f32 / 2.0;
        Self {
            cx: r.left as f32 + hx,
            cy: r.top as f32 + hy,
            hx,
            hy,
            rad: (radius as f32).min(hx).min(hy),
        }
    }

    #[inline]
    pub fn center(&self) -> (f32, f32) {
        (self.cx, self.cy)
    }

    #[inline]
    pub fn dist(&self, x: i32, y: i32) -> f32 {
        let qx = ((x as f32 + 0.5) - self.cx).abs() - (self.hx - self.rad);
        let qy = ((y as f32 + 0.5) - self.cy).abs() - (self.hy - self.rad);
        let (ox, oy) = (qx.max(0.0), qy.max(0.0));
        (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - self.rad
    }
}

// ───────────────────────────── GDI helpers & Web App Favicons ─────────────────────────────

pub unsafe fn make_font(name: PCWSTR, height_px: i32, weight: i32, quality: FONT_QUALITY) -> HFONT {
    CreateFontW(
        -height_px, 0, 0, 0, weight, 0, 0, 0,
        DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, quality, 0, name,
    )
}

pub unsafe fn text(dc: HDC, s: &str, mut r: RECT, fmt: u32, color: COLORREF) {
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    let _ = SetBkMode(dc, TRANSPARENT);
    let _ = SetTextColor(dc, color);
    DrawTextW(dc, &mut buf, &mut r, DRAW_TEXT_FORMAT(fmt));
}

pub unsafe fn text_w(dc: HDC, s: &str) -> i32 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &wide, &mut sz);
    sz.cx
}

/// Renders authentic website favicons or custom letter glyphs.
pub unsafe fn draw_webapp_icon<F>(cv: &Canvas, r: &RECT, item: &TabItem, _radius: i32, px: F)
where
    F: Fn(f32) -> i32,
{
    let (opacity, grayscale) = if item.is_loaded {
        (1.0f32, false)
    } else {
        (0.35f32, true)
    };

    // 1. If a custom letter is assigned, render that capital letter instead of the favicon!
    if let Some(ref letter) = item.custom_letter {
        let letter_font = make_font(w!("Segoe UI"), px(17.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
        let old = SelectObject(cv.dc, letter_font.into());
        let color = if item.is_loaded { rgb(240, 240, 245) } else { rgb(150, 154, 166) };
        text(cv.dc, letter, *r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, color);
        SelectObject(cv.dc, old);
        let _ = DeleteObject(letter_font.into());
        return;
    }

    // 2. If a real website favicon is available, render it directly with no background square!
    if let Some(ref fav) = item.favicon {
        cv.draw_image_scaled_custom(r, &fav.rgba, fav.width, fav.height, opacity, grayscale);
        return;
    }

    // 3. Fallback: single capital letter from title if available
    let letter = item.title.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string());
    if let Some(ref l) = letter {
        let letter_font = make_font(w!("Segoe UI"), px(15.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
        let old = SelectObject(cv.dc, letter_font.into());
        let color = if item.is_loaded { rgb(180, 184, 196) } else { rgb(110, 114, 126) };
        text(cv.dc, l, *r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, color);
        SelectObject(cv.dc, old);
        let _ = DeleteObject(letter_font.into());
        return;
    }

    // 4. Default clean fallback: subtle muted globe icon
    let icon_font = make_font(w!("Segoe Fluent Icons"), px(12.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
    let old = SelectObject(cv.dc, icon_font.into());
    let color = if item.is_loaded { rgb(140, 144, 156) } else { rgb(75, 78, 86) };
    text(cv.dc, "\u{E774}", *r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, color);
    SelectObject(cv.dc, old);
    let _ = DeleteObject(icon_font.into());
}

pub fn draw_favicon_to_canvas(cv: &Canvas, fav: &FaviconData, r: &RECT) {
    cv.draw_image_scaled_custom(r, &fav.rgba, fav.width, fav.height, 1.0, false);
}

pub fn draw_drag_insertion_line<F>(cv: &Canvas, left: i32, right: i32, y: i32, px: F)
where
    F: Fn(f32) -> i32,
{
    let dot_r = px(4.0).max(4);
    let line_h = px(2.0).max(2);
    let cx = left + dot_r + px(2.0);
    let cy = y;

    // Outer glow for extra visibility
    let glow_r = rc(cx - dot_r - px(2.0), cy - line_h / 2 - px(3.0), right, cy + line_h / 2 + px(3.0));
    cv.fill_round(&glow_r, px(4.0), COLORREF(0x00ffffff), 0.12);

    // Circle ring on left: open circle (dark background fill, crisp white 1.8px border)
    let dot_rect = rc(cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r);
    cv.fill_round(&dot_rect, dot_r, rgb(28, 30, 36), 1.0);
    cv.stroke_round(&dot_rect, dot_r, rgb(255, 255, 255), 1.0, 1.8);

    // Horizontal line extending across the row to the right edge
    let line_rect = rc(cx + dot_r, cy - line_h / 2, right - px(4.0), cy + line_h / 2);
    cv.fill_round(&line_rect, line_h / 2, rgb(255, 255, 255), 0.95);
}

const CLOSED_FOLDER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><path fill="#1c1f28" stroke="#OUTLINE_COLOR#" stroke-width="1.4" stroke-linejoin="round" d="M21 15.8v-4.79c0-.683 0-1.22-.03-1.66c-.03-.444-.093-.817-.237-1.16a3.5 3.5 0 0 0-1.89-1.89c-.347-.144-.72-.206-1.16-.237c-.435-.03-.974-.03-1.66-.03h-3.81c-.657 0-.882-.005-1.08-.064a1.5 1.5 0 0 1-.517-.276c-.158-.13-.287-.314-.652-.861l-.14-.212c-.309-.462-.52-.781-.809-1.02a2.5 2.5 0 0 0-.86-.461c-.357-.108-.74-.108-1.29-.108h-1.58c-.822 0-1.47 0-1.99.042c-.532.044-.976.134-1.38.34a3.52 3.52 0 0 0-1.53 1.53c-.206.402-.297.846-.34 1.38c-.042.52-.042 1.17-.042 1.99v7.44c0 .82 0 1.47.042 1.99c.044.53.134.974.34 1.38c.335.658.87 1.19 1.53 1.53c.402.204.846.295 1.38.338c.52.043 1.17.043 1.99.043h10.5c.797 0 1.43 0 1.93-.04c.516-.042.948-.127 1.34-.322c.69-.34 1.25-.9 1.59-1.59c.193-.394.28-.826.32-1.34c.04-.506.04-1.14.04-1.93z"/></svg>"##;

const OPEN_FOLDER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><path fill="#1c1f28" stroke="#OUTLINE_COLOR#" stroke-width="1.4" stroke-linejoin="round" d="M7.87 4.06c-.196-.06-.421-.065-1.08-.065H5.3c-.848 0-1.45 0-1.93.04c-.469.038-.767.111-1.01.232a2.5 2.5 0 0 0-1.09 1.09c-.122.24-.195.536-.233 1.01c-.039.475-.04 1.08-.04 1.93v7.4c0 .849.001 1.45.04 1.93c.038.47.112.768.233 1.01c.24.47.622.854 1.09 1.09c.1.052.21.094.339.129l2.84-7.72a2.51 2.51 0 0 1 2.35-1.64h14.2c1.04 0 1.77 1.04 1.41 2.02l-2.28 6.19a3.495 3.495 0 0 1-3.28 2.29h-12.6c-.821 0-1.47 0-1.99-.043c-.531-.043-.975-.134-1.38-.34a3.52 3.52 0 0 1-1.53-1.53c-.205-.402-.296-.846-.339-1.38c-.043-.52-.043-1.17-.043-1.99v-7.44c0-.82 0-1.47.043-1.99c.043-.53.134-.974.339-1.38a3.52 3.52 0 0 1 1.53-1.53c.403-.204.847-.295 1.38-.338c.521-.043 1.17-.043 1.99-.043h1.58c.555 0 .938 0 1.29.108a2.5 2.5 0 0 1 .861.461c.288.237.5.556.808 1.02l.141.212c.365.547.493.731.652.861c.152.125.328.22.517.276c.196.06.421.065 1.08.065h3.81c.683 0 1.22 0 1.66.03c.444.03.817.093 1.16.237a3.5 3.5 0 0 1 1.89 1.89q.056.135.096.277c.085.293-.159.563-.464.563c-.25 0-.452-.187-.542-.421l-.014-.036a2.5 2.5 0 0 0-1.35-1.35c-.205-.085-.456-.136-.849-.163c-.397-.027-.902-.027-1.61-.027h-3.86c-.555 0-.938 0-1.29-.108a2.5 2.5 0 0 1-.86-.461c-.289-.237-.5-.556-.809-1.02l-.14-.212c-.366-.547-.494-.731-.653-.861a1.5 1.5 0 0 0-.517-.276z"/></svg>"##;

fn render_folder_svg(raw_svg: &str, outline_color: &str, size: u32) -> Vec<u8> {
    let svg = raw_svg.replace("#OUTLINE_COLOR#", outline_color);
    let opt = resvg::usvg::Options::default();
    if let Ok(rtree) = resvg::usvg::Tree::from_str(&svg, &opt) {
        if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(size, size) {
            let sx = size as f32 / rtree.size().width();
            let sy = size as f32 / rtree.size().height();
            let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);
            resvg::render(&rtree, transform, &mut pixmap.as_mut());
            return pixmap.take();
        }
    }
    Vec::new()
}

#[allow(dead_code)]
fn draw_folder_icon_legacy(
    cv: &Canvas,
    dc: HDC,
    r: &RECT,
    is_open: bool,
    open_progress: f32,
    icon_str: &str,
    is_hover: bool,
    px: impl Fn(f32) -> i32,
) {
    use std::sync::OnceLock;
    static ICONS: OnceLock<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> = OnceLock::new();
    let (closed_norm, closed_hov, open_norm, open_hov) = ICONS.get_or_init(|| {
        (
            render_folder_svg(CLOSED_FOLDER_SVG, "#b0b5c4", 48),
            render_folder_svg(CLOSED_FOLDER_SVG, "#ffffff", 48),
            render_folder_svg(OPEN_FOLDER_SVG, "#b0b5c4", 48),
            render_folder_svg(OPEN_FOLDER_SVG, "#ffffff", 48),
        )
    });

    let use_open = open_progress >= 0.5 || is_open;
    let rgba = match (use_open, is_hover) {
        (false, false) => closed_norm,
        (false, true) => closed_hov,
        (true, false) => open_norm,
        (true, true) => open_hov,
    };

    if !rgba.is_empty() {
        cv.draw_image_scaled(r, rgba, 48, 48);
    }

    // Centered glyph/symbol inside folder icon
    let trimmed = icon_str.trim();
    if !trimmed.is_empty() && trimmed != "📁" && trimmed != "folder" {
        let badge_size = px(13.0);
        let bx = r.left + (r.right - r.left - badge_size) / 2;
        let by = r.top + px(3.0) + (r.bottom - (r.top + px(3.0)) - badge_size) / 2;
        let badge_r = rc(bx, by, bx + badge_size, by + badge_size);

        if let Some(v_rgba) = crate::icon_picker::get_rendered_icon(trimmed, if is_hover { "#ffffff" } else { "#e2e6f0" }, badge_size as u32) {
            cv.draw_image_scaled(&badge_r, &v_rgba, badge_size as u32, badge_size as u32);
        } else {
            let inner_font = unsafe { make_font(w!("Segoe UI Symbol"), px(9.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY) };
            unsafe {
                let old_f = SelectObject(dc, inner_font.into());
                text(
                    dc,
                    trimmed,
                    rc(r.left, r.top + px(3.0), r.right, r.bottom),
                    DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                    if is_hover { rgb(255, 255, 255) } else { rgb(220, 222, 232) },
                );
                let _ = SelectObject(dc, old_f);
                let _ = DeleteObject(inner_font.into());
            }
        }
    }
}