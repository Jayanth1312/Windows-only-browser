use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    EndPaint, GdiFlush, GetTextExtentPoint32W, SelectObject, SetBkMode, AC_SRC_ALPHA,
    AC_SRC_OVER, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    CLEARTYPE_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HDC, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_BACK, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, IsWindowVisible, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, SWP_NOACTIVATE, SW_HIDE, SW_SHOW, ULW_ALPHA,
    WM_CHAR, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WNDCLASSW, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::sidebar::{accent_for_url, make_font, Canvas};
use storage::Storage;

// Logical (96-dpi) metrics
const MARGIN: f32 = 48.0; // transparent space around the card, holds the soft spread shadow
const CARD_W: f32 = 620.0;
const INPUT_H: f32 = 56.0;
const ROW_H: f32 = 42.0;
const ROW_GAP: f32 = 2.0;
const LIST_PAD: f32 = 6.0;
const CARD_RADIUS: f32 = 12.0;
const MAX_ROWS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandBarMode {
    NewTab,
    NavigateCurrent,
    RenameTab(usize),
}

#[derive(Debug, Clone)]
pub enum CommandAction {
    OpenInNewTab(String),
    Navigate(String),
    RenameTab(usize, String),
    NewTab,
    CloseTab,
    ToggleSidebar,
}

#[derive(Debug, Clone)]
pub struct CommandItem {
    pub title: String,
    pub subtitle: String,
    pub url: String,
    pub favicon: Option<std::sync::Arc<crate::sidebar::FaviconData>>,
    pub icon_glyph: u16,
    pub icon_accent: COLORREF,
    pub action: CommandAction,
}

pub type CommandCallback = Rc<dyn Fn(CommandAction)>;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

// ───────────────────────────── Theme ─────────────────────────────

struct Theme {
    card: COLORREF,
    border: COLORREF,
    border_a: f32,
    divider: COLORREF,
    divider_a: f32,
    text: COLORREF,
    text_dim: COLORREF,
    placeholder: COLORREF,
    caret: COLORREF,
    hover: COLORREF,
    hover_a: f32,
    selected: COLORREF,
    selected_text: COLORREF,
    selected_sub: COLORREF,
    arrow_bg: COLORREF,
    arrow_bg_a: f32,
    arrow_fg: COLORREF,
    chip: COLORREF,
    chip_a: f32,
    badge_a: f32,
    shadow_a: f32,
}

impl Theme {
    fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    fn light() -> Self {
        Self {
            card: rgb(251, 252, 254),
            border: rgb(0, 0, 0),
            border_a: 0.10,
            divider: rgb(0, 0, 0),
            divider_a: 0.08,
            text: rgb(30, 32, 40),
            text_dim: rgb(122, 128, 142),
            placeholder: rgb(165, 170, 182),
            caret: rgb(88, 172, 204),
            hover: rgb(0, 0, 0),
            hover_a: 0.045,
            selected: rgb(89, 173, 204),
            selected_text: rgb(255, 255, 255),
            selected_sub: rgb(228, 244, 250),
            arrow_bg: rgb(255, 255, 255),
            arrow_bg_a: 0.90,
            arrow_fg: rgb(24, 100, 132),
            chip: rgb(0, 0, 0),
            chip_a: 0.07,
            badge_a: 0.14,
            shadow_a: 0.28,
        }
    }

    fn dark() -> Self {
        Self {
            // Same family as the sidebar (bg 22,22,26 / accent 112,134,255),
            // a touch lighter so the card separates from the dark page.
            card: rgb(26, 26, 32),
            border: rgb(255, 255, 255),
            border_a: 0.09,
            divider: rgb(255, 255, 255),
            divider_a: 0.07,
            text: rgb(236, 236, 242),
            text_dim: rgb(140, 142, 154),
            placeholder: rgb(104, 106, 120),
            caret: rgb(112, 134, 255),
            hover: rgb(255, 255, 255),
            hover_a: 0.07,
            selected: rgb(84, 102, 222),
            selected_text: rgb(255, 255, 255),
            selected_sub: rgb(214, 220, 250),
            arrow_bg: rgb(255, 255, 255),
            arrow_bg_a: 0.20,
            arrow_fg: rgb(255, 255, 255),
            chip: rgb(255, 255, 255),
            chip_a: 0.10,
            badge_a: 0.20,
            shadow_a: 0.28,
        }
    }
}

// ───────────────────────────── Geometry ─────────────────────────────

struct Geo {
    m: i32,
    card_w: i32,
    input_h: i32,
    pad: i32,
    row_h: i32,
    gap: i32,
    stride: i32,
    radius: i32,
}

impl Geo {
    fn new(s: f32) -> Self {
        let px = |v: f32| (v * s).round() as i32;
        let row_h = px(ROW_H);
        let gap = px(ROW_GAP);
        Self {
            m: px(MARGIN),
            card_w: px(CARD_W),
            input_h: px(INPUT_H),
            pad: px(LIST_PAD),
            row_h,
            gap,
            stride: row_h + gap,
            radius: px(CARD_RADIUS),
        }
    }

    fn win_w(&self) -> i32 {
        self.card_w + 2 * self.m
    }

    fn card_h(&self, visible_rows: usize) -> i32 {
        if visible_rows == 0 {
            self.input_h
        } else {
            self.input_h + 1 + self.pad + visible_rows as i32 * self.stride - self.gap + self.pad
        }
    }

    /// Client-space y of the first row.
    fn list_top(&self) -> i32 {
        self.m + self.input_h + 1 + self.pad
    }
}

// ───────────────────────────── State ─────────────────────────────

struct CommandBarState {
    query: String,
    items: Vec<CommandItem>,
    selected_index: usize,
    hovered_index: Option<usize>,
    scroll: usize,
    mode: CommandBarMode,
    on_execute: Option<CommandCallback>,
    storage: Option<Rc<RefCell<Storage>>>,
    favicon_cache: std::collections::HashMap<String, std::sync::Arc<crate::sidebar::FaviconData>>,
    main_hwnd: HWND,
    is_all_selected: bool,
    /// Follows the app theme (the app is dark, so this defaults to true).
    dark: bool,
}

pub struct CommandBar {
    hwnd: HWND,
    state: Rc<RefCell<CommandBarState>>,
}

impl CommandBar {
    pub fn new<F>(
        owner: HWND,
        storage: Option<Rc<RefCell<Storage>>>,
        on_execute: F,
    ) -> Result<Self, windows::core::Error>
    where
        F: Fn(CommandAction) + 'static,
    {
        let class_name = w!("ArcCommandBarWindowClass");
        let hinstance = unsafe { GetModuleHandleW(None)?.into() };

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(cmd_wndproc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(CommandBarState {
            query: String::new(),
            items: Vec::new(),
            selected_index: 0,
            hovered_index: None,
            scroll: 0,
            mode: CommandBarMode::NewTab,
            on_execute: Some(Rc::new(on_execute)),
            storage,
            favicon_cache: std::collections::HashMap::new(),
            main_hwnd: owner,
            is_all_selected: false,
            dark: true,
        }));

        let raw_state = Rc::into_raw(state.clone()) as *mut _;

        // WS_EX_LAYERED: per-pixel alpha gives smooth rounded corners and a
        // soft drop shadow, which a normal GDI window cannot do.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Command Bar"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                800,
                200,
                Some(owner),
                None,
                Some(hinstance),
                Some(raw_state),
            )?
        };

        Ok(Self { hwnd, state })
    }

    pub fn update_favicon(&self, domain: &str, fav: std::sync::Arc<crate::sidebar::FaviconData>) {
        let domain_clean = domain.trim().trim_start_matches("www.").to_lowercase();
        self.state.borrow_mut().favicon_cache.insert(domain_clean, fav);
    }

    /// Call this whenever the app theme changes (true = dark).
    pub fn set_dark_mode(&self, dark: bool) {
        self.state.borrow_mut().dark = dark;
        if self.is_visible() {
            unsafe { repaint(self.hwnd, &self.state) };
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn toggle(&self, content_rect: RECT, mode: CommandBarMode, initial_query: Option<&str>) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show(content_rect, mode, initial_query);
        }
    }

    pub fn show(&self, content_rect: RECT, mode: CommandBarMode, initial_query: Option<&str>) {
        {
            let mut s = self.state.borrow_mut();
            if let Some(q) = initial_query {
                s.query = q.to_string();
                s.is_all_selected = true;
            } else {
                s.query.clear();
                s.is_all_selected = false;
            }
            s.hovered_index = None;
            s.mode = mode;
        }
        refresh_items(&self.state);
        self.recenter(content_rect);
        unsafe {
            repaint(self.hwnd, &self.state);
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetFocus(Some(self.hwnd));
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn recenter(&self, content_rect: RECT) {
        let mut pt_top_left = POINT { x: content_rect.left, y: content_rect.top };
        let mut pt_bottom_right = POINT { x: content_rect.right, y: content_rect.bottom };

        let owner = self.state.borrow().main_hwnd;
        let g = Geo::new(unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0);

        unsafe {
            let _ = ClientToScreen(owner, &mut pt_top_left);
            let _ = ClientToScreen(owner, &mut pt_bottom_right);
        }

        let content_w = pt_bottom_right.x - pt_top_left.x;
        let content_h = pt_bottom_right.y - pt_top_left.y;

        // Horizontally centered; vertically positioned in upper portion (modern Arc / Spotlight style)
        let card_x = pt_top_left.x + (content_w - g.card_w) / 2;
        let card_y = (pt_top_left.y + ((content_h - g.input_h) as f32 * 0.20).round() as i32)
            .max(pt_top_left.y + (16.0 * (unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0)).round() as i32);

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                card_x - g.m,
                card_y - g.m,
                g.win_w(),
                g.m * 2 + g.input_h,
                SWP_NOACTIVATE,
            );
        }
    }
}

// ───────────────────────────── Items ─────────────────────────────

pub fn is_url_query(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("file://") || t.starts_with("about:") {
        return true;
    }
    // Any spaces mean it's a search phrase, not a direct URL
    if t.contains(' ') {
        return false;
    }
    if t.eq_ignore_ascii_case("localhost") || t.starts_with("localhost:") {
        return true;
    }
    let host = if let Some((h, _)) = t.split_once('/') { h } else { t };
    let host = if let Some((h, _)) = host.split_once(':') { h } else { host };
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') && host.split('.').count() == 4 {
        return true;
    }
    if let Some(dot_pos) = host.rfind('.') {
        let tld = &host[dot_pos + 1..];
        if tld.len() >= 2 && tld.chars().all(|c| c.is_alphabetic()) && host.len() > tld.len() + 1 {
            return true;
        }
    }
    false
}

fn wrap_action(target_url: String, mode: CommandBarMode) -> CommandAction {
    match mode {
        CommandBarMode::NewTab => CommandAction::OpenInNewTab(target_url),
        CommandBarMode::NavigateCurrent => CommandAction::Navigate(target_url),
        CommandBarMode::RenameTab(idx) => CommandAction::RenameTab(idx, target_url),
    }
}

fn get_action_items(
    query: &str,
    mode: CommandBarMode,
    storage: &Option<Rc<RefCell<Storage>>>,
    favicon_cache: &mut std::collections::HashMap<String, std::sync::Arc<crate::sidebar::FaviconData>>,
) -> Vec<CommandItem> {
    let mut items = Vec::new();
    let q = query.trim();
    let q_lower = q.to_lowercase();

    if let CommandBarMode::RenameTab(tab_idx) = mode {
        let display_title = if q.is_empty() { "Keep current title" } else { q };
        items.push(CommandItem {
            title: format!("Rename tab to: \"{}\"", display_title),
            subtitle: "Press Enter to save changes (Esc to cancel)".to_string(),
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE70F, // edit / pencil icon
            icon_accent: rgb(80, 160, 255),
            action: CommandAction::RenameTab(tab_idx, q.to_string()),
        });
        return items;
    }

    let popular_sites: [(&str, &str, u16, COLORREF); 6] = [
        ("YouTube", "https://www.youtube.com", 0xE714u16, rgb(255, 0, 0)),
        ("Zen Browser", "https://zen-browser.app", 0xE774u16, rgb(235, 110, 80)),
        ("Google", "https://www.google.com", 0xE774u16, rgb(66, 133, 244)),
        ("GitHub", "https://github.com", 0xEB51u16, rgb(80, 80, 94)),
        ("Reddit", "https://reddit.com", 0xE8BDu16, rgb(255, 69, 0)),
        ("Twitter / X", "https://x.com", 0xE774u16, rgb(29, 155, 240)),
    ];

    let mut resolve_fav = |url: &str| -> Option<std::sync::Arc<crate::sidebar::FaviconData>> {
        let domain = crate::sidebar::domain_of(url);
        if domain.is_empty() {
            return None;
        }
        if let Some(fav) = favicon_cache.get(&domain) {
            return Some(fav.clone());
        }
        if let Some(ref st_cell) = storage {
            if let Ok(st) = st_cell.try_borrow() {
                if let Ok(Some(bytes)) = st.get_favicon(&domain) {
                    if let Some(fav) = crate::sidebar::decode_favicon(&bytes) {
                        favicon_cache.insert(domain, fav.clone());
                        return Some(fav);
                    }
                }
            }
        }
        None
    };

    if !q.is_empty() {
        let mut added_urls = std::collections::HashSet::new();
        let is_url = is_url_query(q);

        if is_url {
            // ── A. URL Query: Direct site navigation is Priority #1 ──
            let clean_q = q_lower
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');

            let matched_pop = popular_sites.iter().find(|(name, url, _, _)| {
                let pop_domain = crate::sidebar::domain_of(url);
                clean_q == pop_domain
                    || clean_q == name.to_lowercase()
                    || pop_domain.contains(&clean_q)
            });

            let (root_title, root_url, glyph, accent) = if let Some(&(name, url, glyph, accent)) = matched_pop {
                (name.to_string(), url.to_string(), glyph, accent)
            } else {
                let target = if q.starts_with("http://") || q.starts_with("https://") {
                    q.to_string()
                } else {
                    format!("https://{}", clean_q)
                };
                (clean_q.to_string(), target.clone(), 0xE774u16, accent_for_url(&target))
            };

            let sub = match mode {
                CommandBarMode::NewTab => format!("Open {} in new tab", root_title),
                CommandBarMode::NavigateCurrent => format!("Navigate to {}", root_url),
                _ => String::new(),
            };

            added_urls.insert(root_url.to_lowercase());
            added_urls.insert(format!("{}/", root_url.to_lowercase()));
            added_urls.insert(clean_q.to_string());
            let fav = resolve_fav(&root_url);

            items.push(CommandItem {
                title: root_title,
                subtitle: sub,
                url: root_url.clone(),
                favicon: fav,
                icon_glyph: glyph,
                icon_accent: accent,
                action: wrap_action(root_url, mode),
            });

            // History Matches (specific subpages matching URL)
            if let Some(ref st_cell) = storage {
                if let Ok(st) = st_cell.try_borrow() {
                    if let Ok(history) = st.query_history(q, 5) {
                        for h in history {
                            let h_url_lower = h.url.to_lowercase();
                            let h_trimmed = h_url_lower.trim_end_matches('/').to_string();
                            if !added_urls.contains(&h_url_lower) && !added_urls.contains(&h_trimmed) {
                                added_urls.insert(h_url_lower.clone());
                                added_urls.insert(h_trimmed);
                                let action = wrap_action(h.url.clone(), mode);
                                let fav = resolve_fav(&h.url);
                                items.push(CommandItem {
                                    title: if h.title.is_empty() { h.url.clone() } else { h.title },
                                    subtitle: h.url.clone(),
                                    url: h.url.clone(),
                                    favicon: fav,
                                    icon_glyph: 0xE81C,
                                    icon_accent: accent_for_url(&h.url),
                                    action,
                                });
                            }
                        }
                    }
                }
            }

            // Fallback Search Google for the URL string
            let search_url = format!("https://www.google.com/search?q={}", urlencoding(q));
            let sub = match mode {
                CommandBarMode::NewTab => "Search web in new tab".to_string(),
                CommandBarMode::NavigateCurrent => "Search web".to_string(),
                _ => String::new(),
            };
            items.push(CommandItem {
                title: format!("Search Google for \"{}\"", q),
                subtitle: sub,
                url: search_url.clone(),
                favicon: resolve_fav("https://www.google.com"),
                icon_glyph: 0xE721,
                icon_accent: rgb(66, 133, 244),
                action: wrap_action(search_url, mode),
            });
        } else {
            // ── B. Normal Search Query (e.g. "hello", "news"): Exact Search is Priority #1! ──
            let search_url = format!("https://www.google.com/search?q={}", urlencoding(q));
            let sub = match mode {
                CommandBarMode::NewTab => "Search Google in new tab".to_string(),
                CommandBarMode::NavigateCurrent => "Search Google".to_string(),
                _ => String::new(),
            };
            added_urls.insert(search_url.to_lowercase());

            items.push(CommandItem {
                title: format!("Search Google for \"{}\"", q),
                subtitle: sub,
                url: search_url.clone(),
                favicon: resolve_fav("https://www.google.com"),
                icon_glyph: 0xE721,
                icon_accent: rgb(66, 133, 244),
                action: wrap_action(search_url, mode),
            });

            // History Matches (shown below the search query; selectable via down arrow, won't hijack Enter)
            if let Some(ref st_cell) = storage {
                if let Ok(st) = st_cell.try_borrow() {
                    if let Ok(history) = st.query_history(q, 6) {
                        for h in history {
                            let h_url_lower = h.url.to_lowercase();
                            let h_trimmed = h_url_lower.trim_end_matches('/').to_string();
                            if !added_urls.contains(&h_url_lower) && !added_urls.contains(&h_trimmed) {
                                added_urls.insert(h_url_lower.clone());
                                added_urls.insert(h_trimmed);
                                let action = wrap_action(h.url.clone(), mode);
                                let fav = resolve_fav(&h.url);
                                items.push(CommandItem {
                                    title: if h.title.is_empty() { h.url.clone() } else { h.title },
                                    subtitle: h.url.clone(),
                                    url: h.url.clone(),
                                    favicon: fav,
                                    icon_glyph: 0xE81C,
                                    icon_accent: accent_for_url(&h.url),
                                    action,
                                });
                            }
                        }
                    }
                }
            }

            // Popular Sites Autocomplete
            for (name, url, glyph, col) in popular_sites {
                let u_lower = url.to_lowercase();
                let u_trimmed = u_lower.trim_end_matches('/').to_string();
                if (name.to_lowercase().contains(&q_lower) || u_lower.contains(&q_lower))
                    && !added_urls.contains(&u_lower)
                    && !added_urls.contains(&u_trimmed)
                {
                    added_urls.insert(u_lower.clone());
                    added_urls.insert(u_trimmed);
                    let sub = match mode {
                        CommandBarMode::NewTab => format!("Open {} in new tab", url),
                        CommandBarMode::NavigateCurrent => format!("Navigate to {}", url),
                        _ => String::new(),
                    };
                    let fav = resolve_fav(url);
                    items.push(CommandItem {
                        title: name.to_string(),
                        subtitle: sub,
                        url: url.to_string(),
                        favicon: fav,
                        icon_glyph: glyph,
                        icon_accent: col,
                        action: wrap_action(url.to_string(), mode),
                    });
                }
            }
        }

        // ── Commands Autocomplete ──
        if "new tab".contains(&q_lower) || "create tab".contains(&q_lower) {
            items.push(CommandItem {
                title: "New Tab".to_string(),
                subtitle: "Create empty tab (Ctrl+T)".to_string(),
                url: String::new(),
                favicon: None,
                icon_glyph: 0xE710,
                icon_accent: rgb(72, 187, 120),
                action: CommandAction::NewTab,
            });
        }
        if "toggle sidebar".contains(&q_lower) || "sidebar".contains(&q_lower) {
            items.push(CommandItem {
                title: "Toggle Sidebar".to_string(),
                subtitle: "Hide / Show sidebar (Ctrl+S)".to_string(),
                url: String::new(),
                favicon: None,
                icon_glyph: 0xE700,
                icon_accent: rgb(235, 110, 80),
                action: CommandAction::ToggleSidebar,
            });
        }
    } else {
        // Query is empty: initially show single clean bar without history results underneath
        // (results only appear when the user starts typing)
    }

    items
}

/// Right-hand label of a row: either a keyboard-shortcut chip or dim text.
/// Returns (text, is_chip).
fn right_meta(item: &CommandItem) -> (String, bool) {
    let sub = item.subtitle.trim();
    if sub.ends_with(')') {
        if let Some(a) = sub.rfind('(') {
            return (sub[a + 1..sub.len() - 1].to_string(), true);
        }
    }
    let s = sub.strip_prefix("Recent: ").unwrap_or(sub);
    if let Some(rest) = s.split("://").nth(1) {
        let host = rest.split('/').next().unwrap_or(rest);
        return (host.trim_start_matches("www.").to_string(), false);
    }
    (s.to_string(), false)
}

fn urlencoding(input: &str) -> String {
    let mut encoded = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            encoded.push(b as char);
        } else if b == b' ' {
            encoded.push('+');
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}

/// Recompute the result list (call whenever the query changes).
fn refresh_items(state: &RefCell<CommandBarState>) {
    let mut s = state.borrow_mut();
    let query = s.query.clone();
    let mode = s.mode;
    let storage = s.storage.clone();
    let items = get_action_items(&query, mode, &storage, &mut s.favicon_cache);
    s.items = items;
    s.selected_index = 0;
    s.scroll = 0;
}

fn ensure_visible(s: &mut CommandBarState) {
    let sel = s.selected_index;
    if sel < s.scroll {
        s.scroll = sel;
    } else if sel >= s.scroll + MAX_ROWS {
        s.scroll = sel + 1 - MAX_ROWS;
    }
}

// ───────────────────────────── Rendering ─────────────────────────────

pub(crate) unsafe fn text_w(dc: HDC, s: &str) -> i32 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &wide, &mut sz);
    sz.cx
}

/// Signed distance from (x, y) to a rounded rect (negative inside).
#[inline]
pub(crate) fn rr_dist(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, rad: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - rad);
    let qy = (y - cy).abs() - (hy - rad);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - rad
}

/// Turns the opaque, square-cornered card bitmap into a premultiplied-alpha
/// image: smooth rounded corners + a soft drop shadow in the margin.
pub(crate) unsafe fn finalize_alpha(
    bits: *mut u32,
    w: i32,
    h: i32,
    card: &RECT,
    radius: f32,
    shadow_dy: f32,
    blur: f32,
    shadow_a: f32,
) {
    let hx = (card.right - card.left) as f32 / 2.0;
    let hy = (card.bottom - card.top) as f32 / 2.0;
    let cx = card.left as f32 + hx;
    let cy = card.top as f32 + hy;
    let rad = radius.min(hx).min(hy);

    for y in 0..h {
        for x in 0..w {
            let p = bits.add((y * w + x) as usize);
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let d = rr_dist(fx, fy, cx, cy, hx, hy, rad);

            if d <= -1.0 {
                *p = (*p & 0x00FF_FFFF) | 0xFF00_0000;
                continue;
            }

            let cov = (0.5 - d).clamp(0.0, 1.0);
            let ds = rr_dist(fx, fy - shadow_dy, cx, cy, hx, hy, rad);
            let sa = if ds >= blur * 2.0 {
                0.0
            } else {
                let t = ds.max(0.0) / blur;
                shadow_a * (-1.4 * t * t).exp()
            };

            let out_a = cov + sa * (1.0 - cov);
            if out_a <= 0.0 {
                *p = 0;
                continue;
            }

            let px = *p;
            let r = (((px >> 16) & 0xff) as f32 * cov).round() as u32;
            let g = (((px >> 8) & 0xff) as f32 * cov).round() as u32;
            let b = ((px & 0xff) as f32 * cov).round() as u32;
            let a = (out_a * 255.0).round() as u32;
            *p = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
}

/// Renders the whole bar into a DIB and pushes it with UpdateLayeredWindow.
unsafe fn repaint(hwnd: HWND, state: &RefCell<CommandBarState>) {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;
    let th = Theme::for_mode(state.borrow().dark);
    let g = Geo::new(s);

    let (query, selected_idx, hovered_idx, mode, items, scroll) = {
        let st = state.borrow();
        (st.query.clone(), st.selected_index, st.hovered_index, st.mode, st.items.clone(), st.scroll)
    };

    let n_vis = items.len().saturating_sub(scroll).min(MAX_ROWS);
    let card_h = g.card_h(n_vis);
    let width = g.win_w();
    let height = card_h + 2 * g.m;
    let card = rc(g.m, g.m, g.m + g.card_w, g.m + card_h);

    let mem_dc = CreateCompatibleDC(None);
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
    let mem_bmp = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
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

    let icon_lg = make_font(w!("Segoe Fluent Icons"), px(14.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
    let icon = make_font(w!("Segoe Fluent Icons"), px(11.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
    let input_font = make_font(w!("Segoe UI"), px(16.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let title_font = make_font(w!("Segoe UI"), px(12.5), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let title_bold = make_font(w!("Segoe UI"), px(12.5), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
    let sub_font = make_font(w!("Segoe UI"), px(11.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let chip_font = make_font(w!("Segoe UI"), px(10.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
    let old_font = SelectObject(mem_dc, title_font.into());

    // Card: filled square here; the rounded mask is applied in finalize_alpha.
    cv.fill_rect(&card, th.card, 1.0);
    cv.stroke_round(&card, g.radius, th.border, th.border_a, 1.0);

    // ── Search input (no pill: big text on the card, like Arc) ──
    let in_top = card.top;
    let in_mid = in_top + g.input_h / 2;
    SelectObject(mem_dc, icon_lg.into());
    let icon_r = rc(card.left + px(16.0), in_top, card.left + px(38.0), in_top + g.input_h);
    let glyph = match mode {
        CommandBarMode::RenameTab(_) => "\u{E70F}",
        _ => "\u{E721}",
    };
    crate::sidebar::text(mem_dc, glyph, icon_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, th.text);

    SelectObject(mem_dc, input_font.into());
    let text_r = rc(card.left + px(46.0), in_top, card.right - px(18.0), in_top + g.input_h);
    let caret_h = px(20.0);
    let mut caret_x = text_r.left;
    let is_all_selected = state.borrow().is_all_selected;
    if query.is_empty() {
        let placeholder = match mode {
            CommandBarMode::NewTab => "Search or Enter URL (opens new tab)...",
            CommandBarMode::NavigateCurrent => "Search or Enter URL...",
            CommandBarMode::RenameTab(_) => "Enter new tab title...",
        };
        let pr = rc(text_r.left + px(2.0), text_r.top, text_r.right, text_r.bottom);
        crate::sidebar::text(mem_dc, placeholder, pr,
            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0, th.placeholder);
    } else {
        if is_all_selected {
            let tw = text_w(mem_dc, &query);
            let sel_r = rc(text_r.left, in_mid - caret_h / 2 - px(2.0), (text_r.left + tw + px(4.0)).min(text_r.right), in_mid + caret_h / 2 + px(2.0));
            cv.fill_round(&sel_r, px(4.0), th.selected, 0.45);
        }
        crate::sidebar::text(mem_dc, &query, text_r,
            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0, th.text);
        caret_x = (text_r.left + text_w(mem_dc, &query) + px(1.0)).min(text_r.right - px(2.0));
    }
    cv.fill_rect(&rc(caret_x, in_mid - caret_h / 2, caret_x + px(2.0).max(1), in_mid + caret_h / 2), th.caret, 1.0);

    // Divider under the input, full card width (only when there are rows below)
    if n_vis > 0 {
        let div_y = in_top + g.input_h;
        cv.fill_rect(&rc(card.left, div_y, card.right, div_y + 1), th.divider, th.divider_a);
    }

    // ── Rows ──
    let pad_x = px(6.0);
    for row_i in 0..n_vis {
        let idx = scroll + row_i;
        let item = &items[idx];
        let top = g.list_top() + row_i as i32 * g.stride;
        let row = rc(card.left + pad_x, top, card.right - pad_x, top + g.row_h);
        let mid = top + g.row_h / 2;
        let is_sel = idx == selected_idx;
        let is_hov = hovered_idx == Some(idx);

        if is_sel {
            cv.fill_round(&row, px(9.0), th.selected, 1.0);
        } else if is_hov {
            cv.fill_round(&row, px(9.0), th.hover, th.hover_a);
        }

        // ── Icon badge ──
        let bs = px(22.0);
        let bl = row.left + px(8.0);
        let badge = rc(bl, mid - bs / 2, bl + bs, mid - bs / 2 + bs);

        if let Some(ref fav) = item.favicon {
            cv.draw_image_scaled(&badge, &fav.rgba, fav.width, fav.height);
        } else if !item.url.is_empty() {
            let tab_item = crate::sidebar::TabItem {
                id: String::new(),
                title: item.title.clone(),
                url: item.url.clone(),
                accent: item.icon_accent,
                favicon: None,
                is_loaded: true,
                is_muted: false,
            };
            crate::sidebar::draw_webapp_icon(&cv, &badge, &tab_item, px(5.0), px);
        } else if matches!(item.action, CommandAction::ToggleSidebar) {
            if is_sel {
                cv.fill_round(&badge, px(6.0), rgb(255, 255, 255), 0.26);
            } else {
                cv.fill_round(&badge, px(6.0), item.icon_accent, th.badge_a);
            }
            crate::sidebar::draw_sidebar_icon(&cv, &badge, if is_sel { th.selected_text } else { item.icon_accent }, 1.0, px);
        } else {
            if is_sel {
                cv.fill_round(&badge, px(6.0), rgb(255, 255, 255), 0.26);
            } else {
                cv.fill_round(&badge, px(6.0), item.icon_accent, th.badge_a);
            }
            SelectObject(mem_dc, icon.into());
            let glyph = char::from_u32(item.icon_glyph as u32).unwrap_or('\u{E774}').to_string();
            crate::sidebar::text(mem_dc, &glyph, badge, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
                if is_sel { th.selected_text } else { item.icon_accent });
        }

        // Right side: arrow button (selected) + chip / dim label
        let mut content_right = row.right - px(10.0);
        if is_sel {
            let ab = px(22.0);
            let ar = rc(row.right - px(8.0) - ab, mid - ab / 2, row.right - px(8.0), mid - ab / 2 + ab);
            cv.fill_round(&ar, px(6.0), th.arrow_bg, th.arrow_bg_a);
            SelectObject(mem_dc, icon.into());
            crate::sidebar::text(mem_dc, "\u{E72A}", ar, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, th.arrow_fg);
            content_right = ar.left - px(8.0);
        }

        let mut title_right = content_right - px(8.0);
        let (label, is_chip) = right_meta(item);
        if !label.is_empty() {
            if is_chip {
                SelectObject(mem_dc, chip_font.into());
                let cw = text_w(mem_dc, &label) + px(12.0);
                let ch = px(18.0);
                let chip = rc(content_right - cw, mid - ch / 2, content_right, mid - ch / 2 + ch);
                if is_sel {
                    cv.fill_round(&chip, px(5.0), rgb(255, 255, 255), 0.22);
                } else {
                    cv.fill_round(&chip, px(5.0), th.chip, th.chip_a);
                }
                crate::sidebar::text(mem_dc, &label, chip, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                    if is_sel { th.selected_text } else { th.text_dim });
                title_right = chip.left - px(10.0);
            } else {
                SelectObject(mem_dc, sub_font.into());
                let max_w = (row.right - row.left) * 38 / 100;
                let lw = text_w(mem_dc, &label).min(max_w);
                let lr = rc(content_right - lw, row.top, content_right, row.bottom);
                crate::sidebar::text(mem_dc, &label, lr,
                    DT_RIGHT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                    if is_sel { th.selected_sub } else { th.text_dim });
                title_right = lr.left - px(10.0);
            }
        }

        // Title
        SelectObject(mem_dc, if is_sel { title_bold.into() } else { title_font.into() });
        let tr = rc(badge.right + px(10.0), row.top, title_right, row.bottom);
        crate::sidebar::text(mem_dc, &item.title, tr,
            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
            if is_sel { th.selected_text } else { th.text });
    }

    // ── Alpha mask + shadow, then present ──
    let _ = GdiFlush();
    finalize_alpha(
        bits as *mut u32,
        width,
        height,
        &card,
        g.radius as f32,
        px(12.0) as f32,
        px(28.0) as f32,
        th.shadow_a,
    );

    let size = SIZE { cx: width, cy: height };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        None,
        None,
        Some(&size),
        Some(mem_dc),
        Some(&src_pt),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SelectObject(mem_dc, old_font);
    for f in [icon_lg, icon, input_font, title_font, title_bold, sub_font, chip_font] {
        let _ = DeleteObject(f.into());
    }
    let _ = SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(mem_bmp.into());
    let _ = DeleteDC(mem_dc);
}

// ───────────────────────────── Input helpers ─────────────────────────────

/// Item index under a client-space point, if any.
unsafe fn row_at(hwnd: HWND, state: &RefCell<CommandBarState>, x: i32, y: i32) -> Option<usize> {
    let g = Geo::new(GetDpiForWindow(hwnd) as f32 / 96.0);
    let st = state.borrow();
    let n_vis = st.items.len().saturating_sub(st.scroll).min(MAX_ROWS);
    if x < g.m || x >= g.m + g.card_w {
        return None;
    }
    let rel = y - g.list_top();
    if rel < 0 {
        return None;
    }
    let row = (rel / g.stride) as usize;
    if row >= n_vis {
        return None;
    }
    Some(st.scroll + row)
}

unsafe fn execute_index(hwnd: HWND, state: &RefCell<CommandBarState>, idx: usize) {
    let (item, cb, mode, q) = {
        let s = state.borrow();
        (s.items.get(idx).cloned(), s.on_execute.clone(), s.mode, s.query.trim().to_string())
    };
    let _ = ShowWindow(hwnd, SW_HIDE);
    if let (Some(item), Some(cb)) = (item, cb.as_ref()) {
        cb(item.action);
    } else if let (CommandBarMode::RenameTab(tab_idx), Some(cb)) = (mode, cb) {
        cb(CommandAction::RenameTab(tab_idx, q));
    }
}

fn copy_to_clipboard(hwnd: HWND, text: &str) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = utf16.len() * std::mem::size_of::<u16>();

    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(Some(hwnd)).is_ok() || OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if opened {
            let _ = EmptyClipboard();
            if let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes) {
                let ptr = GlobalLock(handle);
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, ptr as *mut u8, bytes);
                    let _ = GlobalUnlock(handle);
                    let _ = SetClipboardData(13 /* CF_UNICODETEXT */, Some(windows::Win32::Foundation::HANDLE(handle.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }
}

fn read_clipboard_text(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(Some(hwnd)).is_ok() || OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !opened {
            return None;
        }

        let mut res = None;
        if let Ok(handle) = GetClipboardData(13 /* CF_UNICODETEXT */) {
            if !handle.0.is_null() {
                let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0));
                if !ptr.is_null() {
                    let u16_ptr = ptr as *const u16;
                    let mut len = 0;
                    while *u16_ptr.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(u16_ptr, len);
                    res = String::from_utf16(slice).ok();
                    let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
                }
            }
        }
        let _ = CloseClipboard();
        res
    }
}

// ───────────────────────────── Window proc ─────────────────────────────

unsafe extern "system" fn cmd_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<CommandBarState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<CommandBarState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_KILLFOCUS => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_CHAR => {
            let c = wparam.0 as u16;
            if c >= 32 {
                if let Some(ch) = char::from_u32(c as u32) {
                    {
                        let mut s = state.borrow_mut();
                        if s.is_all_selected {
                            s.query.clear();
                            s.is_all_selected = false;
                        }
                        s.query.push(ch);
                    }
                    refresh_items(state);
                    repaint(hwnd, state);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            let item_count = state.borrow().items.len();

            let ctrl = (windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32,
            ) as u16 & 0x8000) != 0
                || (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
                    windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32,
                ) as u16 & 0x8000) != 0;

            if ctrl {
                match vk {
                    0x41 => { // 'A' -> Select All
                        state.borrow_mut().is_all_selected = true;
                        repaint(hwnd, state);
                        return LRESULT(0);
                    }
                    0x43 => { // 'C' -> Copy
                        let q = state.borrow().query.clone();
                        if !q.is_empty() {
                            copy_to_clipboard(hwnd, &q);
                        }
                        return LRESULT(0);
                    }
                    0x58 => { // 'X' -> Cut
                        let q = state.borrow().query.clone();
                        if !q.is_empty() {
                            copy_to_clipboard(hwnd, &q);
                            let mut s = state.borrow_mut();
                            s.query.clear();
                            s.is_all_selected = false;
                            drop(s);
                            refresh_items(state);
                            repaint(hwnd, state);
                        }
                        return LRESULT(0);
                    }
                    0x56 => { // 'V' -> Paste
                        if let Some(text) = read_clipboard_text(hwnd) {
                            let clean: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                            if !clean.is_empty() {
                                let mut s = state.borrow_mut();
                                if s.is_all_selected {
                                    s.query = clean;
                                    s.is_all_selected = false;
                                } else {
                                    s.query.push_str(&clean);
                                }
                                drop(s);
                                refresh_items(state);
                                repaint(hwnd, state);
                            }
                        }
                        return LRESULT(0);
                    }
                    0x08 => { // Ctrl + Backspace -> delete word
                        let mut s = state.borrow_mut();
                        if s.is_all_selected {
                            s.query.clear();
                            s.is_all_selected = false;
                        } else if !s.query.is_empty() {
                            let trimmed = s.query.trim_end();
                            if let Some(idx) = trimmed.rfind(' ') {
                                s.query.truncate(idx);
                            } else {
                                s.query.clear();
                            }
                        }
                        drop(s);
                        refresh_items(state);
                        repaint(hwnd, state);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            match vk {
                x if x == VK_ESCAPE.0 as u32 => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    return LRESULT(0);
                }
                x if x == VK_TAB.0 as u32 => {
                    // Autocomplete the query with the selected item's URL / title
                    let auto = {
                        let mut s = state.borrow_mut();
                        s.is_all_selected = false;
                        s.items.get(s.selected_index).map(|item| {
                            if item.subtitle.starts_with("http") {
                                item.subtitle.clone()
                            } else if let Some(u) = item.subtitle.strip_prefix("Recent: ") {
                                u.to_string()
                            } else {
                                item.title.clone()
                            }
                        })
                    };
                    if let Some(text) = auto {
                        state.borrow_mut().query = text;
                        refresh_items(state);
                        repaint(hwnd, state);
                    }
                    return LRESULT(0);
                }
                x if x == VK_BACK.0 as u32 => {
                    let mut s = state.borrow_mut();
                    if s.is_all_selected {
                        s.query.clear();
                        s.is_all_selected = false;
                        drop(s);
                        refresh_items(state);
                        repaint(hwnd, state);
                    } else if !s.query.is_empty() {
                        s.query.pop();
                        drop(s);
                        refresh_items(state);
                        repaint(hwnd, state);
                    }
                    return LRESULT(0);
                }
                x if x == VK_UP.0 as u32 => {
                    {
                        let mut s = state.borrow_mut();
                        if s.selected_index > 0 {
                            s.selected_index -= 1;
                        } else if item_count > 0 {
                            s.selected_index = item_count - 1;
                        }
                        ensure_visible(&mut s);
                    }
                    repaint(hwnd, state);
                    return LRESULT(0);
                }
                x if x == VK_DOWN.0 as u32 => {
                    {
                        let mut s = state.borrow_mut();
                        if item_count > 0 {
                            s.selected_index = (s.selected_index + 1) % item_count;
                        }
                        ensure_visible(&mut s);
                    }
                    repaint(hwnd, state);
                    return LRESULT(0);
                }
                x if x == VK_RETURN.0 as u32 => {
                    let idx = state.borrow().selected_index;
                    execute_index(hwnd, state, idx);
                    return LRESULT(0);
                }
                _ => {}
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let hit = row_at(hwnd, state, x, y);
            if hit != state.borrow().hovered_index {
                state.borrow_mut().hovered_index = hit;
                repaint(hwnd, state);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as i32;
            {
                let mut s = state.borrow_mut();
                let max_scroll = s.items.len().saturating_sub(MAX_ROWS);
                if delta > 0 {
                    s.scroll = s.scroll.saturating_sub(1);
                } else if delta < 0 {
                    s.scroll = (s.scroll + 1).min(max_scroll);
                }
            }
            repaint(hwnd, state);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            if let Some(idx) = row_at(hwnd, state, x, y) {
                execute_index(hwnd, state, idx);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            // Layered window: contents are pushed via UpdateLayeredWindow,
            // so just validate the paint request.
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_url_query() {
        assert!(!is_url_query("hello"));
        assert!(!is_url_query("hello hii"));
        assert!(!is_url_query("what is rust?"));
        assert!(!is_url_query("google"));
        assert!(!is_url_query("arc browser"));

        assert!(is_url_query("google.com"));
        assert!(is_url_query("https://google.com"));
        assert!(is_url_query("http://localhost:3000"));
        assert!(is_url_query("localhost:8080"));
        assert!(is_url_query("127.0.0.1:8000"));
        assert!(is_url_query("github.com/rust-lang/rust"));
        assert!(is_url_query("en.wikipedia.org/wiki/Rust"));
    }

    #[test]
    fn test_action_items_query_priority() {
        let mut cache = std::collections::HashMap::new();

        // 1. Plain search query "hello" MUST have Google search as item #0
        let items = get_action_items("hello", CommandBarMode::NewTab, &None, &mut cache);
        assert!(!items.is_empty());
        assert_eq!(items[0].title, "Search Google for \"hello\"");
        assert_eq!(items[0].url, "https://www.google.com/search?q=hello");

        // 2. Direct URL "github.com" MUST have direct site navigation as item #0
        let items_url = get_action_items("github.com", CommandBarMode::NewTab, &None, &mut cache);
        assert!(!items_url.is_empty());
        assert!(items_url[0].title.to_lowercase().contains("github") || items_url[0].url.contains("github.com"));
    }
}
