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
pub enum SidebarHit {
    None,
    Caption(CaptionButton),
    Back,
    Forward,
    Reload,
    ToggleSidebar,
    UrlBar,
    ZoomReset,
    Pinned(usize),
    Tab(usize),
    TabClose(usize),
    NewTab,
    Downloads,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Pinned(usize),
    Tab(usize),
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

    let border_w = (1.0 * s).max(1.0);
    let radius = (3.0 * s).round() as i32;

    // 1. Outer rounded rectangle (window frame)
    cv.stroke_round(&outer_r, radius, color, a, border_w);

    // 2. Vertical panel divider separating left sidebar from right content
    let div_w = border_w.round() as i32;
    let div_x = ox + (total_w * 7 / 16);
    let top_y = oy + border_w.round() as i32;
    let bot_y = oy + total_h - border_w.round() as i32;
    cv.fill_rect(&rc(div_x, top_y, div_x + div_w.max(1), bot_y), color, a);

    // 3. Two content indicator marks inside the right pane
    let dot_w = (2.5 * s).max(2.0).round() as i32;
    let dot_h = (1.0 * s).max(1.0).round() as i32;
    let dot_x = ox + (total_w * 11 / 16);
    let dot1_y = oy + (total_h * 4 / 13);
    let dot2_y = oy + (total_h * 8 / 13);

    cv.fill_rect(&rc(dot_x, dot1_y, dot_x + dot_w, dot1_y + dot_h), color, a);
    cv.fill_rect(&rc(dot_x, dot2_y, dot_x + dot_w, dot2_y + dot_h), color, a);
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
}

pub struct SidebarState {
    pub pinned: Vec<TabItem>,
    pub tabs: Vec<TabItem>,
    pub active: Option<ActiveTab>,
    pub hovered: SidebarHit,
    /// Vertical scroll of the tab list, in physical pixels (>= 0).
    pub scroll: i32,
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
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            tabs: Vec::new(),
            active: None,
            hovered: SidebarHit::None,
            scroll: 0,
            can_go_back: false,
            can_go_forward: false,
            zoom: 1.0,
            toast: None,
            selected: Vec::new(),
            split_group: None,
            has_closed_tabs: false,
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
            bg: rgb(22, 22, 26),
            white: rgb(255, 255, 255),
            surface_a: 0.050,
            hover_a: 0.070,
            active_a: 0.115,
            stroke_a: 0.060,
            divider_a: 0.070,
            text: rgb(236, 236, 242),
            text_dim: rgb(140, 142, 154),
            accent: rgb(112, 134, 255),
            caption_hover: rgb(45, 45, 52),
            caption_press: rgb(60, 60, 70),
            close_hover: rgb(196, 43, 28),
            close_press: rgb(175, 30, 18),
        }
    }
}

// ───────────────────────────── Layout ─────────────────────────────

pub struct Layout {
    pub back: RECT,
    pub forward: RECT,
    pub reload: RECT,
    pub toggle_sidebar: RECT,
    pub url: RECT,
    pub pinned: Vec<RECT>,
    pub divider_y: i32,
    pub list_clip: RECT,
    pub tabs: Vec<RECT>,
    pub new_tab: RECT,
    pub footer_top: i32,
    pub downloads: RECT,
    pub settings: RECT,
    pub footer_toggle: RECT,
    pub zoom_pill: RECT,
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
}

impl Default for SidebarRenderer {
    fn default() -> Self {
        Self { theme: Theme::dark() }
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

        let zoom_pill = if (st.zoom - 1.0).abs() > 0.01 {
            let pill_w = px(54.0);
            let pill_h = px(22.0);
            let py = url.top + (px(36.0) - pill_h) / 2;
            rc(url.right - pill_w - px(8.0), py, url.right - px(8.0), py + pill_h)
        } else {
            rc(0, 0, 0, 0)
        };

        // Pinned grid (Dynamic columns matching Zen: 1 pinned -> full width, 2 pinned -> 2 columns, >=3 pinned -> 3 columns)
        let total_pinned = st.pinned.len();
        let cols = match total_pinned {
            0 | 1 => 1,
            2 => 2,
            _ => 3,
        };
        let gap = px(8.0);
        let tile_w = if cols == 1 {
            width - lp - rp
        } else {
            (width - lp - rp - gap * (cols - 1)) / cols
        };
        let tile_h = px(44.0);
        let pinned_top = url.bottom + px(12.0);
        let pinned: Vec<RECT> = (0..total_pinned as i32)
            .map(|i| {
                let (c, r) = (i % cols, i / cols);
                let l = lp + c * (tile_w + gap);
                let t = pinned_top + r * (tile_h + gap);
                rc(l, t, l + tile_w, t + tile_h)
            })
            .collect();
        let pinned_bottom = pinned.last().map(|r| r.bottom).unwrap_or(url.bottom);

        let divider_y = if st.pinned.is_empty() {
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

        // Scrollable list
        let list_top = if st.pinned.is_empty() {
            url.bottom + px(12.0)
        } else {
            divider_y + px(10.0)
        };
        let list_clip = rc(0, list_top, width, footer_top);
        let row_h = px(36.0);
        let row_gap = px(2.0);
        let mut y = list_top - st.scroll;
        let mut tabs = Vec::with_capacity(st.tabs.len());
        let group = st.split_group.filter(|g| g.1 == g.0 + 1);
        for i in 0..st.tabs.len() {
            let row_l = lp - px(4.0);
            let row_r = width - rp + px(4.0);
            let mid = (row_l + row_r) / 2;
            match group {
                Some((a, _)) if i == a => {
                    // first half of a split row (y not advanced)
                    tabs.push(rc(row_l, y, mid - px(1.0), y + row_h));
                }
                Some((_, b)) if i == b => {
                    tabs.push(rc(mid + px(1.0), y, row_r, y + row_h));
                    y += row_h + row_gap;
                }
                _ => {
                    tabs.push(rc(row_l, y, row_r, y + row_h));
                    y += row_h + row_gap;
                }
            }
        }
        let new_tab = rc(lp - px(4.0), y, width - rp + px(4.0), y + row_h);
        let list_content_h = (y + row_h + st.scroll) - list_top;

        Layout {
            back, forward, reload, toggle_sidebar, url, zoom_pill, pinned, divider_y, list_clip, tabs, new_tab,
            footer_top, downloads, settings, footer_toggle, list_content_h,
        }
    }

    /// Max valid value for `SidebarState::scroll`.
    pub fn max_scroll(&self, hwnd: HWND, height: i32, width: i32, st: &SidebarState) -> i32 {
        let l = self.compute_layout(Self::scale(hwnd), width, height, st);
        (l.list_content_h - (l.list_clip.bottom - l.list_clip.top)).max(0)
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

        if l.zoom_pill.right > l.zoom_pill.left && contains(&l.zoom_pill, x, y) { return SidebarHit::ZoomReset; }
        if contains(&l.url, x, y) { return SidebarHit::UrlBar; }
        if contains(&l.downloads, x, y) { return SidebarHit::Downloads; }
        if contains(&l.settings, x, y) { return SidebarHit::Settings; }
        if contains(&l.footer_toggle, x, y) { return SidebarHit::ToggleSidebar; }
        for (i, r) in l.pinned.iter().enumerate() {
            if contains(r, x, y) { return SidebarHit::Pinned(i); }
        }
        if contains(&l.list_clip, x, y) {
            let cb = (20.0 * s).round() as i32;
            for (i, r) in l.tabs.iter().enumerate() {
                if contains(r, x, y) {
                    let close = rc(r.right - cb - (8.0 * s) as i32, r.top + (r.bottom - r.top - cb) / 2,
                                   r.right - (8.0 * s) as i32, r.top + (r.bottom - r.top - cb) / 2 + cb);
                    return if contains(&close, x, y) { SidebarHit::TabClose(i) } else { SidebarHit::Tab(i) };
                }
            }
            if contains(&l.new_tab, x, y) { return SidebarHit::NewTab; }
        }
        SidebarHit::None
    }

    // ───────────────────────── Rendering ─────────────────────────

    pub fn render(
        &self,
        hwnd: HWND,
        hdc: HDC,
        sidebar_rect: RECT,
        st: &SidebarState,
        hovered_button: CaptionButton,
        pressed_button: CaptionButton,
    ) {
        let width = sidebar_rect.right - sidebar_rect.left;
        let height = sidebar_rect.bottom - sidebar_rect.top;
        if width <= 0 || height <= 0 {
            return;
        }
        let t = &self.theme;
        let s = Self::scale(hwnd);
        let px = |v: f32| (v * s).round() as i32;
        let l = self.compute_layout(s, width, height, st);

        unsafe {
            // 32-bit top-down DIB so we can rasterize anti-aliased shapes ourselves.
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
            SelectObject(mem_dc, icon.into());
            let lock = rc(l.url.left + px(13.0), l.url.top, l.url.left + px(31.0), l.url.bottom);
            text(mem_dc, "\u{E72E}", lock, DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text_dim);
            SelectObject(mem_dc, ui.into());
            let label = active_item.map(|a| host_of(&a.url)).filter(|h| !h.is_empty()).unwrap_or("Search or enter address");
            let right_limit = if l.zoom_pill.right > l.zoom_pill.left {
                l.zoom_pill.left - px(6.0)
            } else {
                l.url.right - px(12.0)
            };
            let label_rect = rc(l.url.left + px(36.0), l.url.top, right_limit, l.url.bottom);
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

            // ── Pinned rectangular tiles with authentic website favicons ──
            for (i, r) in l.pinned.iter().enumerate() {
                let is_active = st.active == Some(ActiveTab::Pinned(i));
                let is_hover = st.hovered == SidebarHit::Pinned(i);
                let is_loaded = st.pinned.get(i).map_or(false, |p| p.is_loaded);

                let a = if is_active {
                    if is_hover {
                        (t.active_a - 0.035).max(0.06)
                    } else {
                        t.active_a
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

                cv.fill_round(r, px(7.0), t.white, a);
                if is_active {
                    cv.stroke_round(r, px(7.0), t.accent, 0.85, 1.0);
                } else if is_loaded {
                    cv.stroke_round(r, px(7.0), t.white, t.stroke_a, 1.0);
                } else {
                    cv.stroke_round(r, px(7.0), t.white, 0.03, 1.0);
                }
                let fav = px(13.0);
                let cx = (r.left + r.right) / 2;
                let cy = (r.top + r.bottom) / 2;
                let fav_rect = rc(cx - fav / 2, cy - fav / 2, cx + fav / 2, cy + fav / 2);
                draw_webapp_icon(&cv, &fav_rect, &st.pinned[i], px(7.0), px);
            }

            // Floating tooltip pill when a pinned tile is hovered (Image 2)
            if let SidebarHit::Pinned(hover_idx) = st.hovered {
                if let Some(pitem) = st.pinned.get(hover_idx) {
                    if let Some(tile_r) = l.pinned.get(hover_idx) {
                        SelectObject(mem_dc, ui_semi.into());
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

            // Divider (soft hairline) - only shown when there are pinned tabs
            if !st.pinned.is_empty() {
                cv.fill_rect(&rc(px(4.0), l.divider_y, width - px(12.0), l.divider_y + 1), t.white, t.divider_a);
            }

            // ── Scrollable tab list with authentic web app icons ──
            cv.set_clip(l.list_clip);
            // Stitched background for tabs that are split together
            if let Some((ga, gb)) = st.split_group.filter(|g| g.1 == g.0 + 1) {
                if let (Some(ra), Some(rb)) = (l.tabs.get(ga), l.tabs.get(gb)) {
                    let combined = rc(ra.left, ra.top, rb.right, rb.bottom);
                    cv.fill_round(&combined, px(7.0), t.white, t.surface_a);
                    cv.stroke_round(&combined, px(7.0), t.white, t.stroke_a, 1.0);
                    let mx = (ra.right + rb.left) / 2;
                    cv.fill_rect(&rc(mx, ra.top + px(8.0), mx + 1, ra.bottom - px(8.0)), t.white, 0.12);
                }
            }
            for (i, r) in l.tabs.iter().enumerate() {
                if r.bottom < l.list_clip.top || r.top > l.list_clip.bottom {
                    continue;
                }
                let is_active = st.active == Some(ActiveTab::Tab(i));
                let is_hover = matches!(st.hovered, SidebarHit::Tab(n) | SidebarHit::TabClose(n) if n == i);
                if st.selected.contains(&i) {
                    cv.fill_round(r, px(7.0), t.accent, 0.28);
                    cv.stroke_round(r, px(7.0), t.accent, 0.8, 1.0);
                }
                if is_active {
                    cv.fill_round(r, px(7.0), t.white, t.active_a);
                    cv.stroke_round(r, px(7.0), t.white, t.stroke_a, 1.0);
                } else if is_hover {
                    cv.fill_round(r, px(7.0), t.white, t.hover_a);
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
                    rgb(125, 130, 142) // Sleeping tab title color (subtly dimmer)
                };
                SelectObject(mem_dc, ui.into());
                text(mem_dc, &st.tabs[i].title,
                     rc(fx + fav + px(10.0), r.top, right_limit, r.bottom),
                     DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                     text_color);

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

            // New tab row
            let nt = l.new_tab;
            if st.hovered == SidebarHit::NewTab {
                cv.fill_round(&nt, px(7.0), t.white, t.hover_a);
            }
            SelectObject(mem_dc, icon.into());
            text(mem_dc, "\u{E710}", rc(nt.left + px(8.0), nt.top, nt.left + px(34.0), nt.bottom),
                 DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text_dim);
            SelectObject(mem_dc, ui.into());
            text(mem_dc, "New Tab", rc(nt.left + px(42.0), nt.top, nt.right, nt.bottom),
                 DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0, t.text_dim);
            cv.reset_clip();

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

            // ── Floating Toast Notification (e.g. "Copied current URL!") matching user's image ──
            if let Some(ref msg) = st.toast {
                SelectObject(mem_dc, ui.into());
                let tw = text_w(mem_dc, msg);
                let toast_w = (tw + px(44.0)).min(width - px(24.0));
                let toast_h = px(32.0);
                let tx = px(4.0);
                let cap_h = px(32.0);
                let ty = cap_h + px(8.0);
                let toast_r = rc(tx, ty, tx + toast_w, ty + toast_h);

                // Subtle shadow
                cv.fill_round(&rc(toast_r.left - 1, toast_r.top + 2, toast_r.right + 1, toast_r.bottom + 4), px(8.0), rgb(0, 0, 0), 0.40);
                // Background card (Dark slate / translucent card matching image)
                cv.fill_round(&toast_r, px(8.0), rgb(48, 52, 66), 0.98);
                // Subtle border
                cv.stroke_round(&toast_r, px(8.0), rgb(255, 255, 255), 0.22, 1.0);

                // Share / Tray up-arrow icon (\u{E72D} in Segoe Fluent Icons)
                SelectObject(mem_dc, icon_cap.into());
                text(mem_dc, "\u{E72D}", rc(toast_r.left + px(8.0), toast_r.top, toast_r.left + px(28.0), toast_r.bottom),
                     DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, rgb(200, 205, 220));

                // Notification text
                SelectObject(mem_dc, ui.into());
                text(mem_dc, msg, rc(toast_r.left + px(28.0), toast_r.top, toast_r.right - px(8.0), toast_r.bottom),
                     DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(240, 240, 245));
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

/// Renders authentic website favicons with crisp alpha downsampling.
pub unsafe fn draw_webapp_icon<F>(cv: &Canvas, r: &RECT, item: &TabItem, _radius: i32, px: F)
where
    F: Fn(f32) -> i32,
{
    let (opacity, grayscale) = if item.is_loaded {
        (1.0f32, false)
    } else {
        (0.35f32, true)
    };

    // 1. If a real website favicon is available, render it directly with no background square!
    if let Some(ref fav) = item.favicon {
        cv.draw_image_scaled_custom(r, &fav.rgba, fav.width, fav.height, opacity, grayscale);
        return;
    }

    // 2. Default clean fallback: subtle muted globe icon with NO background square, NO letter
    let icon_font = make_font(w!("Segoe Fluent Icons"), px(12.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
    let old = SelectObject(cv.dc, icon_font.into());
    let color = if item.is_loaded { rgb(140, 144, 156) } else { rgb(75, 78, 86) };
    text(cv.dc, "\u{E774}", *r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, color);
    SelectObject(cv.dc, old);
    let _ = DeleteObject(icon_font.into());
}