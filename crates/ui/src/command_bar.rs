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
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, IsWindowVisible,
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, SWP_NOACTIVATE, SW_HIDE, SW_SHOW, ULW_ALPHA,
    WM_CHAR, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WNDCLASSW, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::sidebar::{accent_for_url, make_font, Canvas};
use storage::Storage;

// Logical (96-dpi) metrics
const MARGIN: f32 = 48.0;      // transparent space around the card for soft spread shadow
const CARD_W: f32 = 560.0;     // width matching Search Metrics.fieldWidth (sleek and compact)
const INPUT_H: f32 = 46.0;     // sleek input box height
const CARD_GAP: f32 = 8.0;     // floating gap between input card and suggestions card (Search style)
const ROW_H: f32 = 36.0;       // height of each suggestion row
const ROW_GAP: f32 = 2.0;      // spacing between rows
const LIST_PAD: f32 = 6.0;     // padding inside suggestions card
const CARD_RADIUS: f32 = 14.0; // corner radius matching Search 14px continuous
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
    ToggleZapper,
    RestoreAllHidden,
    CopyCleanLink,
    ToggleShield,
    ToggleSiteShield,
    ToggleSiteCard,
    ToggleReader,
    CycleReaderLook,
    CopyPageAsMarkdown,
    CopyMarkdownLink,
    OpenInSplitView,
    SwapSplit,
    EvenSplit,
    SeparateSplit,
    CloseSplit,
}

#[derive(Debug, Clone)]
pub struct CommandItem {
    pub primary: String,
    pub secondary: String,
    pub is_chip: bool,
    pub is_bookmark: bool,
    pub url: String,
    pub favicon: Option<std::sync::Arc<crate::sidebar::FaviconData>>,
    pub icon_glyph: u16,
    pub icon_accent: COLORREF,
    pub action: CommandAction,
}

impl CommandItem {
    pub fn title(&self) -> &str {
        &self.primary
    }
    pub fn subtitle(&self) -> &str {
        &self.secondary
    }
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
    text: COLORREF,
    text_dim: COLORREF,
    placeholder: COLORREF,
    caret: COLORREF,
    hover: COLORREF,
    hover_a: f32,
    selected: COLORREF,
    selected_a: f32,
    selected_text: COLORREF,
    selected_sub: COLORREF,
    chip_bg: COLORREF,
    chip_bg_a: f32,
    chip_border_a: f32,
    chip_fg: COLORREF,
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
            text: rgb(30, 32, 40),
            text_dim: rgb(122, 128, 142),
            placeholder: rgb(165, 170, 182),
            caret: rgb(88, 172, 204),
            hover: rgb(0, 0, 0),
            hover_a: 0.045,
            selected: rgb(0, 0, 0),
            selected_a: 0.07,
            selected_text: rgb(20, 20, 20),
            selected_sub: rgb(100, 105, 115),
            chip_bg: rgb(0, 0, 0),
            chip_bg_a: 0.07,
            chip_border_a: 0.12,
            chip_fg: rgb(80, 80, 80),
            shadow_a: 0.25,
        }
    }

    fn dark() -> Self {
        Self {
            // Pure grayish-black synchronized with the rest of the darkmode UI (#1c1c1c)
            card: rgb(28, 28, 28),
            border: rgb(255, 255, 255),
            border_a: 0.08,
            text: rgb(240, 240, 240),
            text_dim: rgb(145, 145, 145),
            placeholder: rgb(130, 130, 130),
            caret: rgb(235, 235, 235),
            hover: rgb(255, 255, 255),
            hover_a: 0.05,
            selected: rgb(255, 255, 255),
            selected_a: 0.09,
            selected_text: rgb(255, 255, 255),
            selected_sub: rgb(180, 180, 180),
            chip_bg: rgb(255, 255, 255),
            chip_bg_a: 0.08,
            chip_border_a: 0.14,
            chip_fg: rgb(190, 190, 190),
            shadow_a: 0.40,
        }
    }
}

// ───────────────────────────── Geometry ─────────────────────────────

struct Geo {
    m: i32,
    card_w: i32,
    input_h: i32,
    card_gap: i32,
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
            card_gap: px(CARD_GAP),
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

    fn drop_h(&self, visible_rows: usize) -> i32 {
        if visible_rows == 0 {
            0
        } else {
            self.pad * 2 + visible_rows as i32 * self.row_h + (visible_rows as i32 - 1).max(0) * self.gap
        }
    }

    fn total_h(&self, visible_rows: usize) -> i32 {
        if visible_rows == 0 {
            self.input_h + 2 * self.m
        } else {
            self.input_h + self.card_gap + self.drop_h(visible_rows) + 2 * self.m
        }
    }

    fn input_card(&self) -> RECT {
        rc(self.m, self.m, self.m + self.card_w, self.m + self.input_h)
    }

    fn drop_card(&self, visible_rows: usize) -> Option<RECT> {
        if visible_rows == 0 {
            None
        } else {
            let top = self.m + self.input_h + self.card_gap;
            Some(rc(self.m, top, self.m + self.card_w, top + self.drop_h(visible_rows)))
        }
    }

    /// Client-space y of the first row inside the dropdown card.
    fn list_top(&self) -> i32 {
        self.m + self.input_h + self.card_gap + self.pad
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
    content_rect: RECT,
    is_all_selected: bool,
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
        let class_name = w!("BrowserCommandBarWindowClass");
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
            content_rect: RECT::default(),
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
            s.content_rect = content_rect;
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
        self.state.borrow_mut().content_rect = content_rect;
        let mut pt_top_left = POINT { x: content_rect.left, y: content_rect.top };
        let mut pt_bottom_right = POINT { x: content_rect.right, y: content_rect.bottom };

        let owner = self.state.borrow().main_hwnd;
        let s = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let g = Geo::new(s);

        unsafe {
            let _ = ClientToScreen(owner, &mut pt_top_left);
            let _ = ClientToScreen(owner, &mut pt_bottom_right);
        }

        let content_w = (pt_bottom_right.x - pt_top_left.x).max(g.card_w);
        let content_h = pt_bottom_right.y - pt_top_left.y;

        // Horizontally centered inside stage; vertically placed slightly above center (~42%), matching Search
        let card_x = pt_top_left.x + (content_w - g.card_w) / 2;
        let card_y = (pt_top_left.y + ((content_h - g.input_h) as f32 * 0.42).round() as i32)
            .max(pt_top_left.y + (16.0 * s).round() as i32);

        let n_vis = {
            let st = self.state.borrow();
            st.items.len().saturating_sub(st.scroll).min(MAX_ROWS)
        };
        let win_h = g.total_h(n_vis);

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                card_x - g.m,
                card_y - g.m,
                g.win_w(),
                win_h,
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
            primary: format!("Rename tab to: \"{}\"", display_title),
            secondary: "Press Enter to save changes (Esc to cancel)".to_string(),
            is_chip: false,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE70F,
            icon_accent: rgb(200, 200, 200),
            action: CommandAction::RenameTab(tab_idx, q.to_string()),
        });
        return items;
    }

    // Query is empty: initially show single clean input bar without suggestions card (Image 1)
    if q.is_empty() {
        return items;
    }

    let popular_sites: [(&str, &str, &str, &[&str], u16, COLORREF); 14] = [
        ("YouTube", "https://www.youtube.com", "https://www.youtube.com/results?search_query=", &["youtube", "you", "yt"], 0xE714, rgb(255, 0, 0)),
        ("Google", "https://www.google.com", "https://www.google.com/search?q=", &["google", "goo"], 0xE721, rgb(66, 133, 244)),
        ("GitHub", "https://github.com", "https://github.com/search?q=", &["github", "gh", "git"], 0xEB51, rgb(160, 160, 160)),
        ("Reddit", "https://reddit.com", "https://reddit.com/search/?q=", &["reddit", "red"], 0xE8BD, rgb(255, 69, 0)),
        ("Wikipedia", "https://wikipedia.org", "https://en.wikipedia.org/wiki/Special:Search?search=", &["wikipedia", "wiki"], 0xE774, rgb(160, 160, 160)),
        ("X", "https://x.com", "https://x.com/search?q=", &["x", "twitter", "tw"], 0xE774, rgb(29, 155, 240)),
        ("Amazon", "https://www.amazon.com", "https://www.amazon.com/s?k=", &["amazon", "amz"], 0xE774, rgb(255, 153, 0)),
        ("ChatGPT", "https://chatgpt.com", "https://chatgpt.com/?q=", &["chatgpt", "gpt", "openai"], 0xE774, rgb(16, 163, 127)),
        ("Claude", "https://claude.ai", "https://claude.ai/new?q=", &["claude"], 0xE774, rgb(217, 119, 87)),
        ("Perplexity", "https://perplexity.ai", "https://www.perplexity.ai/search?q=", &["perplexity"], 0xE774, rgb(32, 178, 170)),
        ("Spotify", "https://open.spotify.com", "https://open.spotify.com/search/", &["spotify"], 0xE774, rgb(30, 215, 96)),
        ("Netflix", "https://www.netflix.com", "https://www.netflix.com/search?q=", &["netflix"], 0xE774, rgb(229, 9, 20)),
        ("Twitch", "https://www.twitch.tv", "https://www.twitch.tv/search?term=", &["twitch"], 0xE774, rgb(145, 71, 255)),
        ("Stack Overflow", "https://stackoverflow.com", "https://stackoverflow.com/search?q=", &["stackoverflow", "so"], 0xE774, rgb(244, 128, 36)),
    ];

    let mut resolve_fav = |url: &str| -> Option<std::sync::Arc<crate::sidebar::FaviconData>> {
        let domain = if url.contains("://") {
            crate::sidebar::domain_of(url)
        } else {
            url.trim().to_string()
        };
        if domain.is_empty() {
            return None;
        }
        let clean = domain.trim_start_matches("www.");
        if let Some(fav) = favicon_cache.get(&domain).or_else(|| favicon_cache.get(clean)) {
            return Some(fav.clone());
        }
        if let Some(ref st_cell) = storage {
            if let Ok(st) = st_cell.try_borrow() {
                if let Ok(Some(bytes)) = st.get_favicon(&domain).or_else(|_| st.get_favicon(clean)) {
                    if let Some(fav) = crate::sidebar::decode_favicon(&bytes) {
                        favicon_cache.insert(domain, fav.clone());
                        return Some(fav);
                    }
                }
            }
        }
        None
    };

    let mut added_keys = std::collections::HashSet::new();

    // ── Check if query has an active site prefix (e.g. "youtube funny cats" or "yt cats") ──
    for (name, site_url, search_prefix, aliases, glyph, col) in &popular_sites {
        for &alias in *aliases {
            let prefix_space = format!("{} ", alias);
            if q_lower.starts_with(&prefix_space) {
                let term = q[prefix_space.len()..].trim();
                if !term.is_empty() {
                    let search_url = format!("{}{}", search_prefix, urlencoding(term));
                    let fav = resolve_fav(site_url);
                    items.push(CommandItem {
                        primary: term.to_string(),
                        secondary: format!("Search {}", name),
                        is_chip: true,
                        is_bookmark: false,
                        url: search_url.clone(),
                        favicon: fav,
                        icon_glyph: *glyph,
                        icon_accent: *col,
                        action: wrap_action(search_url, mode),
                    });
                    added_keys.insert(term.to_lowercase());
                }
                break;
            }
        }
    }

    let is_url = is_url_query(q);

    if is_url {
        // Direct URL is Priority #1
        let clean_q = q_lower
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        let target_url = if q.starts_with("http://") || q.starts_with("https://") || q.starts_with("file://") || q.starts_with("about:") {
            q.to_string()
        } else {
            format!("https://{}", clean_q)
        };

        let fav = resolve_fav(&target_url);
        items.push(CommandItem {
            primary: clean_q.to_string(),
            secondary: "Open URL".to_string(),
            is_chip: false,
            is_bookmark: false,
            url: target_url.clone(),
            favicon: fav,
            icon_glyph: 0xE774,
            icon_accent: accent_for_url(&target_url),
            action: wrap_action(target_url, mode),
        });
        added_keys.insert(clean_q.to_string());
        added_keys.insert(q_lower.clone());
    } else {
        // ── 1. Site Search Shortcut (Tab to search, matching Image 2) ──
        // e.g. typing "you" -> "Search YouTube" [Tab]
        for (name, site_url, _, aliases, glyph, col) in &popular_sites {
            let matched_alias = aliases.iter().find(|&&a| a.starts_with(&q_lower) || q_lower.starts_with(a))
                .or_else(|| if name.to_lowercase().starts_with(&q_lower) { Some(&name) } else { None });
            if let Some(_) = matched_alias {
                let fav = resolve_fav(site_url);
                items.push(CommandItem {
                    primary: format!("Search {}", name),
                    secondary: "Tab".to_string(),
                    is_chip: true,
                    is_bookmark: false,
                    url: site_url.to_string(),
                    favicon: fav,
                    icon_glyph: *glyph,
                    icon_accent: *col,
                    action: wrap_action(site_url.to_string(), mode),
                });
                break;
            }
        }
    }

    // ── 2. History & Bookmarks Matches (Image 2 Rows 2 & 4) ──
    if let Some(ref st_cell) = storage {
        if let Ok(st) = st_cell.try_borrow() {
            if let Ok(history) = st.query_history(q, 10) {
                for h in history {
                    let h_url_clean = h.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
                    let key = h_url_clean.to_lowercase();
                    if added_keys.contains(&key) {
                        continue;
                    }
                    added_keys.insert(key);

                    let fav = resolve_fav(&h.url);
                    let is_deep = h_url_clean.contains('/') || h_url_clean.contains('?');

                    if is_deep {
                        // Deep URL / Bookmark / subpage: Left is clean URL, Right is bookmark / page title
                        let display_title = if h.title.is_empty() {
                            crate::sidebar::domain_of(&h.url)
                        } else {
                            h.title.clone()
                        };
                        items.push(CommandItem {
                            primary: h_url_clean.to_string(),
                            secondary: display_title,
                            is_chip: false,
                            is_bookmark: true,
                            url: h.url.clone(),
                            favicon: fav,
                            icon_glyph: 0xE774,
                            icon_accent: accent_for_url(&h.url),
                            action: wrap_action(h.url.clone(), mode),
                        });
                    } else {
                        // Root or visited query: Left is title, Right is domain
                        let domain = crate::sidebar::domain_of(&h.url);
                        let primary = if h.title.is_empty() { domain.clone() } else { h.title.clone() };
                        let glyph = if fav.is_none() { 0xE81C } else { 0xE774 };
                        items.push(CommandItem {
                            primary,
                            secondary: domain,
                            is_chip: false,
                            is_bookmark: false,
                            url: h.url.clone(),
                            favicon: fav,
                            icon_glyph: glyph,
                            icon_accent: accent_for_url(&h.url),
                            action: wrap_action(h.url.clone(), mode),
                        });
                    }
                    if items.len() >= 4 {
                        break;
                    }
                }
            }
        }
    }

    // ── 3. Exact Domain / Popular Site Match (Image 2 Row 3) ──
    for (name, site_url, _, aliases, glyph, col) in &popular_sites {
        let pop_domain = crate::sidebar::domain_of(site_url);
        let clean_domain = pop_domain.trim_start_matches("www.");
        let matched = clean_domain.contains(&q_lower)
            || name.to_lowercase().contains(&q_lower)
            || aliases.iter().any(|&a| a == q_lower);

        if matched && !added_keys.contains(&clean_domain.to_lowercase()) {
            added_keys.insert(clean_domain.to_lowercase());
            let fav = resolve_fav(site_url);
            items.push(CommandItem {
                primary: clean_domain.to_string(),
                secondary: name.to_string(),
                is_chip: false,
                is_bookmark: false,
                url: site_url.to_string(),
                favicon: fav,
                icon_glyph: *glyph,
                icon_accent: *col,
                action: wrap_action(site_url.to_string(), mode),
            });
        }
    }

    // ── 4. Web Search query (Google, Image 2 Row 5) ──
    let search_url = format!("https://www.google.com/search?q={}", urlencoding(q));
    if !added_keys.contains(&search_url.to_lowercase()) {
        added_keys.insert(search_url.to_lowercase());
        let fav = resolve_fav("https://www.google.com");
        items.push(CommandItem {
            primary: q.to_string(),
            secondary: "Google".to_string(),
            is_chip: false,
            is_bookmark: false,
            url: search_url.clone(),
            favicon: fav,
            icon_glyph: 0xE721,
            icon_accent: rgb(66, 133, 244),
            action: wrap_action(search_url, mode),
        });
    }

    // ── 5. Quick Commands Autocomplete ──
    if "new tab".contains(&q_lower) || "create tab".contains(&q_lower) {
        items.push(CommandItem {
            primary: "New Tab".to_string(),
            secondary: "Ctrl+T".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE710,
            icon_accent: rgb(72, 187, 120),
            action: CommandAction::NewTab,
        });
    }
    if "toggle sidebar".contains(&q_lower) || "sidebar".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Toggle Sidebar".to_string(),
            secondary: "Ctrl+S".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE700,
            icon_accent: rgb(235, 110, 80),
            action: CommandAction::ToggleSidebar,
        });
    }
    if "element zapper".contains(&q_lower) || "zap".contains(&q_lower) || "hide element".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Element Zapper".to_string(),
            secondary: "Ctrl+Shift+H".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE7B5,
            icon_accent: rgb(239, 68, 68),
            action: CommandAction::ToggleZapper,
        });
    }
    if "restore hidden elements".contains(&q_lower) || "unhide".contains(&q_lower) || "show hidden".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Restore All Hidden Elements".to_string(),
            secondary: "Reset site".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE777,
            icon_accent: rgb(59, 130, 246),
            action: CommandAction::RestoreAllHidden,
        });
    }
    if "reader mode".contains(&q_lower) || "reading mode".contains(&q_lower) || "reader".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Toggle Reader Mode".to_string(),
            secondary: "Ctrl+Shift+R".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE736,
            icon_accent: rgb(56, 189, 248),
            action: CommandAction::ToggleReader,
        });
    }
    if "reader look".contains(&q_lower) || "reader theme".contains(&q_lower) || "reader appearance".contains(&q_lower) || "reader light".contains(&q_lower) || "reader dark".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Reader Look: Cycle Light / Dark / System".to_string(),
            secondary: "Reader appearance".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE790,
            icon_accent: rgb(167, 139, 250),
            action: CommandAction::CycleReaderLook,
        });
    }
    if "copy page as markdown".contains(&q_lower) || "export markdown".contains(&q_lower) || "page markdown".contains(&q_lower) || q_lower == "markdown" {
        items.push(CommandItem {
            primary: "Copy Page as Markdown".to_string(),
            secondary: "Export article".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE8C8,
            icon_accent: rgb(52, 211, 153),
            action: CommandAction::CopyPageAsMarkdown,
        });
    }
    if "copy as markdown link".contains(&q_lower) || "copy markdown link".contains(&q_lower) || "markdown link".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Copy as Markdown Link".to_string(),
            secondary: "[title](url)".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE71B,
            icon_accent: rgb(96, 165, 250),
            action: CommandAction::CopyMarkdownLink,
        });
    }
    if "copy clean link".contains(&q_lower) || "clean link".contains(&q_lower) || "clean url".contains(&q_lower) || "strip tracking".contains(&q_lower) || "clean".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Copy Clean Link".to_string(),
            secondary: "Ctrl+Shift+C".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE8C8,
            icon_accent: rgb(52, 211, 153),
            action: CommandAction::CopyCleanLink,
        });
    }
    if "shield".contains(&q_lower) || "adblock".contains(&q_lower) || "ads".contains(&q_lower) || "blocker".contains(&q_lower) || "trackers".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Shield: Toggle Ad & Tracker Protection".to_string(),
            secondary: "Global protection".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xEA18,
            icon_accent: rgb(56, 189, 248),
            action: CommandAction::ToggleShield,
        });
        items.push(CommandItem {
            primary: "Shield: Toggle for Current Site".to_string(),
            secondary: "Whitelist host".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xEA18,
            icon_accent: rgb(245, 158, 11),
            action: CommandAction::ToggleSiteShield,
        });
        items.push(CommandItem {
            primary: "Site Information & Permissions".to_string(),
            secondary: "View security, permissions & shields".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE713,
            icon_accent: rgb(96, 165, 250),
            action: CommandAction::ToggleSiteCard,
        });
    }

    if "split view".contains(&q_lower) || "split".contains(&q_lower) || "side by side".contains(&q_lower) || "dual pane".contains(&q_lower) {
        items.push(CommandItem {
            primary: "Split View: Open Page Side-by-Side".to_string(),
            secondary: "Ctrl+Alt+N".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE740,
            icon_accent: rgb(96, 165, 250),
            action: CommandAction::OpenInSplitView,
        });
        items.push(CommandItem {
            primary: "Split View: Swap Left and Right Panes".to_string(),
            secondary: "Ctrl+Alt+S".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE746,
            icon_accent: rgb(167, 139, 250),
            action: CommandAction::SwapSplit,
        });
        items.push(CommandItem {
            primary: "Split View: Reset Divider to 50/50".to_string(),
            secondary: "Ctrl+Alt+E".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE71D,
            icon_accent: rgb(52, 211, 153),
            action: CommandAction::EvenSplit,
        });
        items.push(CommandItem {
            primary: "Split View: Separate into Independent Tabs".to_string(),
            secondary: "Ctrl+Alt+U".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE899,
            icon_accent: rgb(245, 158, 11),
            action: CommandAction::SeparateSplit,
        });
        items.push(CommandItem {
            primary: "Split View: Close Both Panes".to_string(),
            secondary: "Ctrl+Alt+W".to_string(),
            is_chip: true,
            is_bookmark: false,
            url: String::new(),
            favicon: None,
            icon_glyph: 0xE711,
            icon_accent: rgb(239, 68, 68),
            action: CommandAction::CloseSplit,
        });
    } else {
        if "swap".contains(&q_lower) {
            items.push(CommandItem {
                primary: "Split View: Swap Left and Right Panes".to_string(),
                secondary: "Ctrl+Alt+S".to_string(),
                is_chip: true,
                is_bookmark: false,
                url: String::new(),
                favicon: None,
                icon_glyph: 0xE746,
                icon_accent: rgb(167, 139, 250),
                action: CommandAction::SwapSplit,
            });
        }
        if "even".contains(&q_lower) || "50/50".contains(&q_lower) {
            items.push(CommandItem {
                primary: "Split View: Reset Divider to 50/50".to_string(),
                secondary: "Ctrl+Alt+E".to_string(),
                is_chip: true,
                is_bookmark: false,
                url: String::new(),
                favicon: None,
                icon_glyph: 0xE71D,
                icon_accent: rgb(52, 211, 153),
                action: CommandAction::EvenSplit,
            });
        }
        if "unsplit".contains(&q_lower) || "separate".contains(&q_lower) {
            items.push(CommandItem {
                primary: "Split View: Separate into Independent Tabs".to_string(),
                secondary: "Ctrl+Alt+U".to_string(),
                is_chip: true,
                is_bookmark: false,
                url: String::new(),
                favicon: None,
                icon_glyph: 0xE899,
                icon_accent: rgb(245, 158, 11),
                action: CommandAction::SeparateSplit,
            });
        }
    }

    items
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

/// Renders soft drop shadows and rounded corners for input card and optional floating dropdown card.
pub(crate) unsafe fn finalize_alpha_cards(
    bits: *mut u32,
    w: i32,
    h: i32,
    card_in: &RECT,
    card_drop: Option<&RECT>,
    radius: f32,
    shadow_dy: f32,
    blur: f32,
    shadow_a: f32,
) {
    let hx_in = (card_in.right - card_in.left) as f32 / 2.0;
    let hy_in = (card_in.bottom - card_in.top) as f32 / 2.0;
    let cx_in = card_in.left as f32 + hx_in;
    let cy_in = card_in.top as f32 + hy_in;
    let rad_in = radius.min(hx_in).min(hy_in);

    let drop_info = card_drop.map(|cd| {
        let hx = (cd.right - cd.left) as f32 / 2.0;
        let hy = (cd.bottom - cd.top) as f32 / 2.0;
        let cx = cd.left as f32 + hx;
        let cy = cd.top as f32 + hy;
        let rad = radius.min(hx).min(hy);
        (cx, cy, hx, hy, rad)
    });

    for y in 0..h {
        for x in 0..w {
            let p = bits.add((y * w + x) as usize);
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;

            let d_in = rr_dist(fx, fy, cx_in, cy_in, hx_in, hy_in, rad_in);
            let ds_in = rr_dist(fx, fy - shadow_dy, cx_in, cy_in, hx_in, hy_in, rad_in);

            let (d, ds) = if let Some((cx_d, cy_d, hx_d, hy_d, rad_d)) = drop_info {
                let d_drop = rr_dist(fx, fy, cx_d, cy_d, hx_d, hy_d, rad_d);
                let ds_drop = rr_dist(fx, fy - shadow_dy, cx_d, cy_d, hx_d, hy_d, rad_d);
                (d_in.min(d_drop), ds_in.min(ds_drop))
            } else {
                (d_in, ds_in)
            };

            if d <= -1.0 {
                *p = (*p & 0x00FF_FFFF) | 0xFF00_0000;
                continue;
            }

            let cov = (0.5 - d).clamp(0.0, 1.0);
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
    finalize_alpha_cards(bits, w, h, card, None, radius, shadow_dy, blur, shadow_a);
}

/// Renders the floating command bar into a layered window bitmap with soft shadows.
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
    let width = g.win_w();
    let height = g.total_h(n_vis);
    let card_in = g.input_card();
    let card_drop = g.drop_card(n_vis);

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

    let icon = make_font(w!("Segoe Fluent Icons"), px(12.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);
    let input_font = make_font(w!("Segoe UI"), px(15.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let title_font = make_font(w!("Segoe UI"), px(13.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let title_bold = make_font(w!("Segoe UI"), px(13.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
    let sub_font = make_font(w!("Segoe UI"), px(11.5), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let chip_font = make_font(w!("Segoe UI"), px(10.5), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
    let old_font = SelectObject(mem_dc, title_font.into());

    // ── 1. Input Box Card ──
    cv.fill_rect(&card_in, th.card, 1.0);
    cv.stroke_round(&card_in, g.radius, th.border, th.border_a, 1.0);

    let in_top = card_in.top;
    let in_mid = in_top + g.input_h / 2;

    SelectObject(mem_dc, input_font.into());
    // Clean indentation from left edge (~18px), matching Image 1 & 2
    let text_r = rc(card_in.left + px(18.0), in_top, card_in.right - px(18.0), in_top + g.input_h);
    let caret_h = px(20.0);
    let mut caret_x = text_r.left;
    let is_all_selected = state.borrow().is_all_selected;

    if query.is_empty() {
        let placeholder = match mode {
            CommandBarMode::RenameTab(_) => "Enter new tab title...",
            _ => "Enter a web address",
        };
        crate::sidebar::text(mem_dc, placeholder, text_r,
            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0, th.placeholder);
    } else {
        if is_all_selected {
            let tw = text_w(mem_dc, &query);
            let sel_r = rc(text_r.left, in_mid - caret_h / 2 - px(2.0), (text_r.left + tw + px(4.0)).min(text_r.right), in_mid + caret_h / 2 + px(2.0));
            cv.fill_round(&sel_r, px(4.0), th.selected, th.selected_a * 2.0);
        }
        crate::sidebar::text(mem_dc, &query, text_r,
            DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0, th.text);
        caret_x = (text_r.left + text_w(mem_dc, &query) + px(1.0)).min(text_r.right - px(2.0));
    }
    cv.fill_rect(&rc(caret_x, in_mid - caret_h / 2, caret_x + px(1.5).max(1), in_mid + caret_h / 2), th.caret, 1.0);

    // ── 2. Suggestions Dropdown Card (Floating below input with gap) ──
    if let Some(drop_card) = card_drop {
        cv.fill_rect(&drop_card, th.card, 1.0);
        cv.stroke_round(&drop_card, g.radius, th.border, th.border_a, 1.0);

        for row_i in 0..n_vis {
            let idx = scroll + row_i;
            let item = &items[idx];
            let top = g.list_top() + row_i as i32 * g.stride;
            let row = rc(drop_card.left + g.pad, top, drop_card.right - g.pad, top + g.row_h);
            let mid = top + g.row_h / 2;
            let is_sel = idx == selected_idx;
            let is_hov = hovered_idx == Some(idx);

            if is_sel {
                cv.fill_round(&row, px(8.0), th.selected, th.selected_a);
            } else if is_hov {
                cv.fill_round(&row, px(8.0), th.hover, th.hover_a);
            }

            // Left icon
            let bs = px(18.0);
            let bl = row.left + px(10.0);
            let badge = rc(bl, mid - bs / 2, bl + bs, mid - bs / 2 + bs);

            if let Some(ref fav) = item.favicon {
                cv.draw_image_scaled(&badge, &fav.rgba, fav.width, fav.height);
            } else if !item.url.is_empty() && (item.icon_glyph == 0 || item.icon_glyph == 0xE774) {
                let tab_item = crate::sidebar::TabItem {
                    id: String::new(),
                    title: item.primary.clone(),
                    url: item.url.clone(),
                    accent: item.icon_accent,
                    favicon: None,
                    is_loaded: true,
                    is_muted: false,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                    custom_letter: None,
                    is_split: false,
                };
                crate::sidebar::draw_webapp_icon(&cv, &badge, &tab_item, px(4.0), px);
            } else {
                SelectObject(mem_dc, icon.into());
                let glyph = char::from_u32(item.icon_glyph as u32).unwrap_or('\u{E774}').to_string();
                crate::sidebar::text(mem_dc, &glyph, badge, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, item.icon_accent);
            }

            // Right secondary meta / chip
            let mut title_right = row.right - px(10.0);
            if !item.secondary.is_empty() {
                if item.is_chip {
                    SelectObject(mem_dc, chip_font.into());
                    let cw = text_w(mem_dc, &item.secondary) + px(14.0);
                    let ch = px(18.0);
                    let chip = rc(row.right - px(8.0) - cw, mid - ch / 2, row.right - px(8.0), mid - ch / 2 + ch);
                    cv.fill_round(&chip, px(4.0), th.chip_bg, th.chip_bg_a);
                    cv.stroke_round(&chip, px(4.0), th.border, th.chip_border_a, 1.0);
                    crate::sidebar::text(mem_dc, &item.secondary, chip,
                        DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, th.chip_fg);
                    title_right = chip.left - px(10.0);
                } else {
                    SelectObject(mem_dc, sub_font.into());
                    let max_w = (row.right - row.left) * 45 / 100;
                    let text_w_sec = text_w(mem_dc, &item.secondary);
                    let bookmark_pad = if item.is_bookmark { px(16.0) } else { 0 };
                    let total_sec_w = (text_w_sec + bookmark_pad).min(max_w);
                    let sec_rect = rc(row.right - px(8.0) - total_sec_w + bookmark_pad, row.top, row.right - px(8.0), row.bottom);

                    if item.is_bookmark {
                        let bm_rect = rc(sec_rect.left - bookmark_pad, row.top, sec_rect.left, row.bottom);
                        SelectObject(mem_dc, icon.into());
                        crate::sidebar::text(mem_dc, "\u{E718}", bm_rect,
                            DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, th.text_dim);
                        SelectObject(mem_dc, sub_font.into());
                    }

                    crate::sidebar::text(mem_dc, &item.secondary, sec_rect,
                        DT_RIGHT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                        if is_sel { th.selected_sub } else { th.text_dim });
                    title_right = (row.right - px(8.0) - total_sec_w) - px(10.0);
                }
            }

            // Left primary text
            SelectObject(mem_dc, if is_sel { title_bold.into() } else { title_font.into() });
            let tr = rc(badge.right + px(10.0), row.top, title_right, row.bottom);
            crate::sidebar::text(mem_dc, &item.primary, tr,
                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0,
                if is_sel { th.selected_text } else { th.text });
        }
    }

    // ── 3. Alpha Mask + Shadows ──
    let _ = GdiFlush();
    finalize_alpha_cards(
        bits as *mut u32,
        width,
        height,
        &card_in,
        card_drop.as_ref(),
        g.radius as f32,
        px(8.0) as f32,
        px(24.0) as f32,
        th.shadow_a,
    );

    // Sync layered window position & bounds atomically
    let (card_x, card_y) = {
        let st = state.borrow();
        let owner = st.main_hwnd;
        let cr = st.content_rect;
        let mut pt_tl = POINT { x: cr.left, y: cr.top };
        let mut pt_br = POINT { x: cr.right, y: cr.bottom };
        if cr.right <= cr.left || cr.bottom <= cr.top {
            let mut rc_client = RECT::default();
            let _ = GetClientRect(owner, &mut rc_client);
            pt_tl = POINT { x: rc_client.left, y: rc_client.top };
            pt_br = POINT { x: rc_client.right, y: rc_client.bottom };
        }
        let _ = ClientToScreen(owner, &mut pt_tl);
        let _ = ClientToScreen(owner, &mut pt_br);
        let content_w = (pt_br.x - pt_tl.x).max(g.card_w);
        let content_h = pt_br.y - pt_tl.y;
        let cx = pt_tl.x + (content_w - g.card_w) / 2;
        let cy = (pt_tl.y + ((content_h - g.input_h) as f32 * 0.42).round() as i32)
            .max(pt_tl.y + (16.0 * s).round() as i32);
        (cx, cy)
    };

    let _ = SetWindowPos(
        hwnd,
        None,
        card_x - g.m,
        card_y - g.m,
        width,
        height,
        SWP_NOACTIVATE,
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
    for f in [icon, input_font, title_font, title_bold, sub_font, chip_font] {
        let _ = DeleteObject(f.into());
    }
    let _ = SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(mem_bmp.into());
    let _ = DeleteDC(mem_dc);
}

// ───────────────────────────── Input helpers ─────────────────────────────

/// Item index under a client-space point, if any.
unsafe fn row_at(hwnd: HWND, state: &RefCell<CommandBarState>, x: i32, y: i32) -> Option<usize> {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let g = Geo::new(s);
    let st = state.borrow();
    let n_vis = st.items.len().saturating_sub(st.scroll).min(MAX_ROWS);
    if n_vis == 0 {
        return None;
    }
    if x < g.m || x >= g.m + g.card_w {
        return None;
    }
    let list_top = g.list_top();
    let rel = y - list_top;
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
                            let mut clean: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                            // Sanitize URL tracking parameters on paste (Clean Link feature)
                            let trimmed = clean.trim();
                            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                                clean = engine::clean_link::clean_url(trimmed);
                            }
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
                    // Autocomplete the query with the selected item's site prefix or clean URL
                    let auto = {
                        let mut s = state.borrow_mut();
                        s.is_all_selected = false;
                        if let Some(item) = s.items.get(s.selected_index) {
                            if item.is_chip && item.primary.starts_with("Search ") {
                                let site = item.primary.strip_prefix("Search ").unwrap_or("");
                                Some(format!("{} ", site.to_lowercase()))
                            } else if !item.url.is_empty() {
                                let clean = item.url.trim_start_matches("https://").trim_start_matches("http://");
                                Some(clean.to_string())
                            } else {
                                Some(item.primary.clone())
                            }
                        } else {
                            None
                        }
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

        // 1. Plain search query "hello" has Google search as item #0
        let items = get_action_items("hello", CommandBarMode::NewTab, &None, &mut cache);
        assert!(!items.is_empty());
        assert_eq!(items[0].primary, "hello");
        assert_eq!(items[0].secondary, "Google");
        assert_eq!(items[0].url, "https://www.google.com/search?q=hello");

        // 2. Direct URL "github.com" has direct site navigation as item #0
        let items_url = get_action_items("github.com", CommandBarMode::NewTab, &None, &mut cache);
        assert!(!items_url.is_empty());
        assert!(items_url[0].primary.to_lowercase().contains("github") || items_url[0].url.contains("github.com"));

        // 3. Query "you" matches YouTube site search with Tab chip as item #0
        let items_you = get_action_items("you", CommandBarMode::NewTab, &None, &mut cache);
        assert!(!items_you.is_empty());
        assert_eq!(items_you[0].primary, "Search YouTube");
        assert_eq!(items_you[0].secondary, "Tab");
        assert!(items_you[0].is_chip);
    }
}
