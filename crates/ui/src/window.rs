use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint,
    FillRect, FillRgn, InvalidateRect, ScreenToClient, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW, IsZoomed, KillTimer, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, CS_DBLCLKS, CW_USEDEFAULT, GWLP_USERDATA,
    HTCLIENT, IDC_ARROW, IDC_HAND, IDC_SIZEWE, MSG, SWP_FRAMECHANGED, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    WM_APP, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_INITMENUPOPUP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW,
};

use crate::frame::{enable_modern_frame, handle_nccalcsize, handle_nchittest};
use crate::sidebar::{
    ActiveTab, CaptionButton, SidebarHit, SidebarRenderer, SidebarState, TabItem,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Error, Debug)]
pub enum UiError {
    #[error("Windows error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Window creation failed")]
    WindowCreationFailed,
}

#[derive(Debug, Clone)]
pub enum SidebarAction {
    NavBack,
    NavForward,
    NavReload,
    NavNewTab,
    ToggleSidebar,
    UrlBarClicked,
    SelectTab(usize),
    CloseTab(usize),
    SelectPinned(usize),
    ClosePinnedTab(usize),
    PinTab(usize),
    UnpinTab(usize),
    DuplicateTab(usize),
    PromptRenameTab(usize),
    RenameTab(usize, String),
    CopyUrl,
    TogglePinCurrentTab,
    OpenSettings,
    OpenDownloads,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextTab,
    PrevTab,
    HardReload,
    ReopenClosedTab,
    ToggleMuteTab(usize),
    ToggleMutePinnedTab(usize),
    UnloadTab(usize),
    ShareTab(usize),
    SharePinnedTab(usize),
    CloseOtherTabs(usize),
    NewTabBelow(usize),
    ReloadSpecificTab(usize),
    ReloadSpecificPinnedTab(usize),
}

pub struct WindowConfig {
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
    pub caption_height: i32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Browser".to_string(),
            width: 1280,
            height: 800,
            sidebar_width: 280,
            caption_height: 32,
        }
    }
}

pub type ResizeCallback = Rc<dyn Fn(RECT, RECT)>;
pub type ActionCallback = Rc<dyn Fn(SidebarAction)>;
pub type CustomMsgHandler = Rc<dyn Fn(u32, WPARAM, LPARAM)>;

struct WindowState {
    config: WindowConfig,
    sidebar: SidebarRenderer,
    sidebar_state: SidebarState,
    sidebar_visible: bool,
    is_resizing_sidebar: bool,
    drag_start_x: i32,
    drag_start_width: i32,
    is_loading: bool,
    loading_progress: f32,
    pressed_hit: SidebarHit,
    on_resize: Option<ResizeCallback>,
    on_action: Option<ActionCallback>,
    on_custom_msg: Option<CustomMsgHandler>,
    is_tracking_mouse: bool,
    context_menu: Option<crate::context_menu::ContextMenu>,
}

#[inline]
unsafe fn ensure_dark_menus() {
    crate::frame::enable_process_dark_mode();
}

pub struct BrowserWindow {
    hwnd: HWND,
    state: Rc<RefCell<WindowState>>,
}

impl BrowserWindow {
    pub fn new(config: WindowConfig) -> Result<Self, UiError> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            ensure_dark_menus();
        }

        let class_name = w!("BrowserWindowClass");

        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let arrow_cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                hCursor: arrow_cursor,
                lpszClassName: class_name,
                ..Default::default()
            };

            let _ = RegisterClassW(&wc);

            let initial_state = SidebarState::default();

            let state = Rc::new(RefCell::new(WindowState {
                config,
                sidebar: SidebarRenderer::new(),
                sidebar_state: initial_state,
                sidebar_visible: true,
                is_resizing_sidebar: false,
                drag_start_x: 0,
                drag_start_width: 280,
                is_loading: false,
                loading_progress: 0.0,
                pressed_hit: SidebarHit::None,
                on_resize: None,
                on_action: None,
                on_custom_msg: None,
                is_tracking_mouse: false,
                context_menu: None,
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut _;

            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("Browser"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1280,
                800,
                None,
                None,
                Some(hinstance),
                Some(raw_state),
            )?;

            if hwnd.0 == std::ptr::null_mut() {
                return Err(UiError::WindowCreationFailed);
            }

            let menu = crate::context_menu::ContextMenu::new(hwnd).ok();
            state.borrow_mut().context_menu = menu;

            Ok(Self { hwnd, state })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_resize_callback<F>(&self, callback: F)
    where
        F: Fn(RECT, RECT) + 'static,
    {
        self.state.borrow_mut().on_resize = Some(Rc::new(callback));
        self.trigger_resize();
    }

    pub fn set_action_callback<F>(&self, callback: F)
    where
        F: Fn(SidebarAction) + 'static,
    {
        self.state.borrow_mut().on_action = Some(Rc::new(callback));
    }

    pub fn set_custom_msg_handler<F>(&self, handler: F)
    where
        F: Fn(u32, WPARAM, LPARAM) + 'static,
    {
        self.state.borrow_mut().on_custom_msg = Some(Rc::new(handler));
    }

    pub fn update_tabs(&self, tabs: Vec<TabItem>, active_idx: usize) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.tabs = tabs;
            if active_idx < b.sidebar_state.tabs.len() {
                b.sidebar_state.active = Some(ActiveTab::Tab(active_idx));
            } else if b.sidebar_state.tabs.is_empty() {
                b.sidebar_state.active = None;
            }
        }
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn update_tabs_and_pinned(
        &self,
        tabs: Vec<TabItem>,
        pinned: Vec<TabItem>,
        active: Option<ActiveTab>,
        can_go_back: bool,
        can_go_forward: bool,
        has_closed_tabs: bool,
    ) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.tabs = tabs;
            b.sidebar_state.pinned = pinned;
            b.sidebar_state.active = active;
            b.sidebar_state.can_go_back = can_go_back;
            b.sidebar_state.can_go_forward = can_go_forward;
            b.sidebar_state.has_closed_tabs = has_closed_tabs;
        }
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn is_sidebar_visible(&self) -> bool {
        self.state.borrow().sidebar_visible
    }

    pub fn set_sidebar_visible(&self, visible: bool) {
        self.state.borrow_mut().sidebar_visible = visible;
        self.trigger_resize();
    }

    pub fn toggle_sidebar(&self) {
        let new_vis = !self.state.borrow().sidebar_visible;
        self.set_sidebar_visible(new_vis);
    }

    pub fn update_pinned_favicon(&self, url: &str, favicon: std::sync::Arc<crate::sidebar::FaviconData>) {
        let domain = crate::sidebar::domain_of(url);
        if domain.is_empty() {
            return;
        }
        let changed = {
            let mut b = self.state.borrow_mut();
            let mut changed = false;
            for p in b.sidebar_state.pinned.iter_mut() {
                let p_domain = crate::sidebar::domain_of(&p.url);
                if !p_domain.is_empty() && p_domain == domain {
                    p.favicon = Some(favicon.clone());
                    changed = true;
                }
            }
            changed
        };
        if changed {
            unsafe {
                invalidate_sidebar(self.hwnd, &self.state);
            }
        }
    }

    pub fn get_layout_rects(&self) -> (RECT, RECT) {
        compute_layout_rects(self.hwnd, &self.state)
    }

    pub fn trigger_resize(&self) {
        let (content_rect, sidebar_rect) = self.get_layout_rects();
        let cb = self.state.borrow().on_resize.clone();
        if let Some(cb) = cb {
            cb(content_rect, sidebar_rect);
        }
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn set_loading(&self, is_loading: bool, progress: f32) {
        {
            let mut b = self.state.borrow_mut();
            b.is_loading = is_loading;
            b.loading_progress = progress.clamp(0.0, 1.0);
        }
        unsafe {
            let (content_rect, _) = self.get_layout_rects();
            let top_strip = RECT {
                left: content_rect.left,
                top: 0,
                right: content_rect.right,
                bottom: content_rect.top + 8,
            };
            let _ = InvalidateRect(Some(self.hwnd), Some(&top_strip), false);
        }
    }

    pub fn set_zoom(&self, zoom: f64) {
        self.state.borrow_mut().sidebar_state.zoom = zoom;
        unsafe {
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn show_toast(&self, message: &str) {
        let hwnd = self.hwnd();
        self.state.borrow_mut().sidebar_state.toast = Some(message.to_string());
        unsafe {
            let _ = SetTimer(Some(hwnd), 0x4201, 2500, None);
            invalidate_sidebar(hwnd, &self.state);
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
    }

    pub fn run_message_loop(&self) {
        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

// ───────────────────────────── Helpers ─────────────────────────────

fn caption_of(hit: SidebarHit) -> CaptionButton {
    match hit {
        SidebarHit::Caption(c) => c,
        _ => CaptionButton::None,
    }
}

fn compute_layout_rects(hwnd: HWND, state: &RefCell<WindowState>) -> (RECT, RECT) {
    let mut client_rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client_rect);
    }

    let width = client_rect.right - client_rect.left;
    let height = client_rect.bottom - client_rect.top;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = dpi as f32 / 96.0;

    let sidebar_visible = state.borrow().sidebar_visible;
    let pad = (8.0 * scale).round() as i32;

    if sidebar_visible {
        let sidebar_width = (state.borrow().config.sidebar_width as f32 * scale).round() as i32;
        let available_content_w = (width - sidebar_width).max(0);

        let content_rect = RECT {
            left: pad,
            top: pad,
            right: (available_content_w - pad).max(pad),
            bottom: (height - pad).max(pad),
        };

        let sidebar_rect = RECT {
            left: available_content_w,
            top: 0,
            right: width,
            bottom: height,
        };

        (content_rect, sidebar_rect)
    } else {
        let content_rect = RECT {
            left: pad,
            top: pad,
            right: (width - pad).max(pad),
            bottom: (height - pad).max(pad),
        };

        let sidebar_rect = RECT {
            left: width,
            top: 0,
            right: width,
            bottom: height,
        };

        (content_rect, sidebar_rect)
    }
}

/// Sidebar rect in client coordinates.
unsafe fn sidebar_rect_of(hwnd: HWND, state: &RefCell<WindowState>) -> RECT {
    let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
    sidebar_rect
}

/// Hit-test a point given in client coordinates.
unsafe fn hit_at(hwnd: HWND, state: &RefCell<WindowState>, x: i32, y: i32) -> SidebarHit {
    let b = state.borrow();
    if !b.sidebar_visible {
        return SidebarHit::None;
    }

    let r = sidebar_rect_of(hwnd, state);
    if x < r.left || x >= r.right || y < r.top || y >= r.bottom {
        return SidebarHit::None;
    }

    b.sidebar.hit_test(
        hwnd,
        x - r.left,
        y - r.top,
        r.right - r.left,
        r.bottom - r.top,
        &b.sidebar_state,
    )
}

unsafe fn invalidate_sidebar(hwnd: HWND, state: &RefCell<WindowState>) {
    let r = sidebar_rect_of(hwnd, state);
    let _ = InvalidateRect(Some(hwnd), Some(&r), false);
}

fn is_on_sidebar_edge(hwnd: HWND, state: &RefCell<WindowState>, x: i32, _y: i32) -> bool {
    let b = state.borrow();
    if !b.sidebar_visible {
        return false;
    }
    drop(b);
    let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
    let border_x = sidebar_rect.left;
    (x - border_x).abs() <= 5
}

unsafe fn clamp_scroll(hwnd: HWND, state: &RefCell<WindowState>) {
    let r = sidebar_rect_of(hwnd, state);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let mut b = state.borrow_mut();
    if b.sidebar_visible {
        let max = b.sidebar.max_scroll(hwnd, h, w, &b.sidebar_state);
        let cur = b.sidebar_state.scroll;
        b.sidebar_state.scroll = cur.clamp(0, max);
    }
}

// ───────────────────────────── Window proc ─────────────────────────────

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<WindowState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<WindowState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        enable_modern_frame(hwnd);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_NCCALCSIZE => {
            if let Some(res) = handle_nccalcsize(hwnd, wparam, lparam) {
                return res;
            }
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let hit = hit_at(hwnd, state, pt.x, pt.y);
            let is_edge = is_on_sidebar_edge(hwnd, state, pt.x, pt.y);

            let (sidebar_w, caption_h, sidebar_vis) = {
                let b = state.borrow();
                let dpi = GetDpiForWindow(hwnd);
                let sw = (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;
                let ch = (32.0 * (dpi as f32 / 96.0)).round() as i32;
                (sw, ch, b.sidebar_visible)
            };

            // Sidebar client area must always be HTCLIENT to prevent non-client hover/flicker loops
            let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
            if sidebar_vis && pt.x >= sidebar_rect.left && pt.y >= caption_h {
                return LRESULT(HTCLIENT as isize);
            }

            let is_interactive = is_edge || !matches!(hit, SidebarHit::None);
            return handle_nchittest(hwnd, lparam, sidebar_w, caption_h, sidebar_vis, is_interactive);
        }
        WM_SETCURSOR => {
            let hit_test = (lparam.0 & 0xffff) as i16 as u32;
            if hit_test == HTCLIENT {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                let is_resizing = state.borrow().is_resizing_sidebar;
                if is_resizing || is_on_sidebar_edge(hwnd, state, pt.x, pt.y) {
                    if let Ok(c) = LoadCursorW(None, IDC_SIZEWE) {
                        let _ = SetCursor(Some(c));
                        return LRESULT(1);
                    }
                } else {
                    let hit = hit_at(hwnd, state, pt.x, pt.y);
                    let is_interactive = matches!(
                        hit,
                        SidebarHit::Pinned(_)
                            | SidebarHit::Tab(_)
                            | SidebarHit::TabClose(_)
                            | SidebarHit::NewTab
                            | SidebarHit::Back
                            | SidebarHit::Forward
                            | SidebarHit::Reload
                            | SidebarHit::ToggleSidebar
                            | SidebarHit::UrlBar
                            | SidebarHit::ZoomReset
                            | SidebarHit::Downloads
                            | SidebarHit::Settings
                    );
                    let cursor_id = if is_interactive { IDC_HAND } else { IDC_ARROW };
                    if let Ok(c) = LoadCursorW(None, cursor_id) {
                        let _ = SetCursor(Some(c));
                        return LRESULT(1);
                    }
                }
            }
        }
        WM_ERASEBKGND => {
            return LRESULT(1);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let b = state.borrow();

            let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);

            // Fill window background only when update region actually touches webview area (never repaint over webview unnecessarily)
            if ps.rcPaint.left < sidebar_rect.left {
                let brush = CreateSolidBrush(b.sidebar.theme.bg);
                let frame_rect = if b.sidebar_visible {
                    RECT {
                        left: 0,
                        top: 0,
                        right: sidebar_rect.left,
                        bottom: client.bottom,
                    }
                } else {
                    client
                };
                let _ = FillRect(hdc, &frame_rect, brush);
                let _ = DeleteObject(brush.into());
            }

            // If loading, draw sleek capsule pill loading progress bar with rounded corners in #14dcff
            if b.is_loading && b.loading_progress > 0.0 {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let bar_h = (3.5 * scale).round().max(3.0) as i32;
                let bar_y = (content_rect.top - bar_h - 2).max(1);
                let max_w = (content_rect.right - content_rect.left).max(0);
                let current_w = (max_w as f32 * b.loading_progress).round().max(bar_h as f32 * 2.0) as i32;
                let bar_left = content_rect.left;
                let bar_right = (content_rect.left + current_w).min(content_rect.right);
                if bar_right > bar_left {
                    let rgn = CreateRoundRectRgn(bar_left, bar_y, bar_right, bar_y + bar_h, bar_h, bar_h);
                    let load_brush = CreateSolidBrush(COLORREF(0x00ffdc14)); // #14dcff (RGB: 20, 220, 255)
                    let _ = FillRgn(hdc, rgn, load_brush);
                    let _ = DeleteObject(load_brush.into());
                    let _ = DeleteObject(rgn.into());
                }
            }

            // Render sidebar smoothly from offscreen memory DC
            if b.sidebar_visible {
                let hovered = caption_of(b.sidebar_state.hovered);
                let pressed = caption_of(b.pressed_hit);
                b.sidebar.render(hwnd, hdc, sidebar_rect, &b.sidebar_state, hovered, pressed);
            }

            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if state.borrow().is_resizing_sidebar {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;

                let (start_x, start_w) = {
                    let b = state.borrow();
                    (b.drag_start_x, b.drag_start_width)
                };

                let delta_x = start_x - x;
                let delta_logical = ((delta_x as f32) / scale).round() as i32;
                let clamped_w = (start_w + delta_logical).clamp(200, 520);

                let changed = {
                    let mut b = state.borrow_mut();
                    if b.config.sidebar_width != clamped_w {
                        b.config.sidebar_width = clamped_w;
                        true
                    } else {
                        false
                    }
                };

                if changed {
                    let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);
                    let cb = state.borrow().on_resize.clone();
                    if let Some(cb) = cb {
                        cb(content_rect, sidebar_rect);
                    }
                    clamp_scroll(hwnd, state);
                    let update_r = RECT {
                        left: sidebar_rect.left - 10,
                        top: 0,
                        right: sidebar_rect.right,
                        bottom: sidebar_rect.bottom,
                    };
                    let _ = InvalidateRect(Some(hwnd), Some(&update_r), false);
                    let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                }
                return LRESULT(0);
            }

            if !state.borrow().is_tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                state.borrow_mut().is_tracking_mouse = true;
            }

            let hit = hit_at(hwnd, state, x, y);
            if hit != state.borrow().sidebar_state.hovered {
                state.borrow_mut().sidebar_state.hovered = hit;
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            state.borrow_mut().is_tracking_mouse = false;
            let changed = {
                let b = state.borrow();
                b.sidebar_state.hovered != SidebarHit::None || b.pressed_hit != SidebarHit::None
            };
            if changed {
                {
                    let mut b = state.borrow_mut();
                    b.sidebar_state.hovered = SidebarHit::None;
                    b.pressed_hit = SidebarHit::None;
                }
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            if let Some(ref menu) = state.borrow().context_menu {
                if menu.is_visible() {
                    menu.hide();
                }
            }
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if is_on_sidebar_edge(hwnd, state, x, y) {
                let mut b = state.borrow_mut();
                b.is_resizing_sidebar = true;
                b.drag_start_x = x;
                b.drag_start_width = b.config.sidebar_width;
                drop(b);
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

            let hit = hit_at(hwnd, state, x, y);
            if hit != SidebarHit::None {
                state.borrow_mut().pressed_hit = hit;
                let _ = SetCapture(hwnd);
                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if state.borrow().is_resizing_sidebar {
                state.borrow_mut().is_resizing_sidebar = false;
                let _ = ReleaseCapture();
                return LRESULT(0);
            }

            let pressed = state.borrow().pressed_hit;
            if pressed != SidebarHit::None {
                let _ = ReleaseCapture();
                state.borrow_mut().pressed_hit = SidebarHit::None;
                let released = hit_at(hwnd, state, x, y);
                invalidate_sidebar(hwnd, state);

                if pressed == released {
                    match pressed {
                        SidebarHit::Caption(CaptionButton::Minimize) => {
                            let _ = ShowWindow(hwnd, SW_MINIMIZE);
                        }
                        SidebarHit::Caption(CaptionButton::Maximize) => {
                            if IsZoomed(hwnd).as_bool() {
                                let _ = ShowWindow(hwnd, SW_RESTORE);
                            } else {
                                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                            }
                        }
                        SidebarHit::Caption(CaptionButton::Close) => {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                        SidebarHit::ZoomReset => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ZoomReset);
                            }
                        }
                        SidebarHit::ToggleSidebar => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleSidebar);
                            }
                        }
                        SidebarHit::Back => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavBack);
                            }
                        }
                        SidebarHit::Forward => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavForward);
                            }
                        }
                        SidebarHit::Reload => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavReload);
                            }
                        }
                        SidebarHit::UrlBar => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::UrlBarClicked);
                            }
                        }
                        SidebarHit::NewTab => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavNewTab);
                            }
                        }
                        SidebarHit::Tab(i) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::SelectTab(i));
                            }
                        }
                        SidebarHit::Downloads => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::OpenDownloads);
                            }
                        }
                        SidebarHit::Settings => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::OpenSettings);
                            }
                        }
                        SidebarHit::TabClose(i) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::CloseTab(i));
                            }
                        }
                        SidebarHit::Pinned(i) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::SelectPinned(i));
                            }
                        }
                        _ => {}
                    }
                }
                return LRESULT(0);
            }
        }
        WM_RBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let hit = hit_at(hwnd, state, x, y);
            let has_closed = state.borrow().sidebar_state.has_closed_tabs;

            match hit {
                SidebarHit::Tab(idx) => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let tab_opt = state.borrow().sidebar_state.tabs.get(idx).cloned();
                    if let Some(tab) = tab_opt {
                        let mute_label = if tab.is_muted { "Unmute Tab" } else { "Mute Tab" };
                        let unload_item = if tab.is_loaded {
                            crate::context_menu::ContextMenuItem::item(5, "Unload Tab", None)
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(5, "Unloaded (Sleeping)", None)
                        };
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };

                        let items = vec![
                            crate::context_menu::ContextMenuItem::item(11, "New Tab Below", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(1, "Reload Tab", Some("Ctrl+R")),
                            crate::context_menu::ContextMenuItem::item(2, mute_label, None),
                            crate::context_menu::ContextMenuItem::item(3, "Pin Tab", Some("Ctrl+Shift+X")),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(4, "Change Label...", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            unload_item,
                            crate::context_menu::ContextMenuItem::item(6, "Duplicate Tab", None),
                            crate::context_menu::ContextMenuItem::item(7, "Share Tab (Copy Link)", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(8, "Close Tab", Some("Ctrl+W")),
                            crate::context_menu::ContextMenuItem::item(9, "Close Other Tabs", None),
                            reopen_item,
                        ];

                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::ReloadSpecificTab(idx)),
                                        2 => cb(SidebarAction::ToggleMuteTab(idx)),
                                        3 => cb(SidebarAction::PinTab(idx)),
                                        4 => cb(SidebarAction::PromptRenameTab(idx)),
                                        5 => cb(SidebarAction::UnloadTab(idx)),
                                        6 => cb(SidebarAction::DuplicateTab(idx)),
                                        7 => cb(SidebarAction::ShareTab(idx)),
                                        8 => cb(SidebarAction::CloseTab(idx)),
                                        9 => cb(SidebarAction::CloseOtherTabs(idx)),
                                        10 => cb(SidebarAction::ReopenClosedTab),
                                        11 => cb(SidebarAction::NewTabBelow(idx)),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                    }
                    return LRESULT(0);
                }
                SidebarHit::Pinned(idx) => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let pin_opt = state.borrow().sidebar_state.pinned.get(idx).cloned();
                    if let Some(pin) = pin_opt {
                        let mute_label = if pin.is_muted { "Unmute Tab" } else { "Mute Tab" };
                        let unload_item = if pin.is_loaded {
                            crate::context_menu::ContextMenuItem::item(5, "Unload Tab", None)
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(5, "Unloaded (Sleeping)", None)
                        };
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };

                        let items = vec![
                            crate::context_menu::ContextMenuItem::item(1, "Reload Tab", Some("Ctrl+R")),
                            crate::context_menu::ContextMenuItem::item(2, mute_label, None),
                            crate::context_menu::ContextMenuItem::item(3, "Unpin Tab", Some("Ctrl+Shift+X")),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(4, "Change Label...", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            unload_item,
                            crate::context_menu::ContextMenuItem::item(6, "Duplicate Tab", None),
                            crate::context_menu::ContextMenuItem::item(7, "Share Tab (Copy Link)", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(8, "Close Tab", Some("Ctrl+W")),
                            reopen_item,
                        ];

                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::ReloadSpecificPinnedTab(idx)),
                                        2 => cb(SidebarAction::ToggleMutePinnedTab(idx)),
                                        3 => cb(SidebarAction::UnpinTab(idx)),
                                        4 => cb(SidebarAction::PromptRenameTab(idx)),
                                        5 => cb(SidebarAction::UnloadTab(idx)),
                                        6 => cb(SidebarAction::DuplicateTab(idx)),
                                        7 => cb(SidebarAction::SharePinnedTab(idx)),
                                        8 => cb(SidebarAction::ClosePinnedTab(idx)),
                                        10 => cb(SidebarAction::ReopenClosedTab),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                    }
                    return LRESULT(0);
                }
                _ => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let r = sidebar_rect_of(hwnd, state);
                    if pt.x >= r.left && state.borrow().sidebar_visible {
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(2, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(2, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };
                        let items = vec![
                            crate::context_menu::ContextMenuItem::item(1, "New Tab", Some("Ctrl+T")),
                            reopen_item,
                        ];
                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::NavNewTab),
                                        2 => cb(SidebarAction::ReopenClosedTab),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                        return LRESULT(0);
                    }
                }
            }
            return LRESULT(0);
        }
        WM_INITMENUPOPUP => {
            unsafe {
                if let Ok(menu_hwnd) = FindWindowW(w!("#32768"), windows::core::PCWSTR::null()) {
                    if menu_hwnd.0 != std::ptr::null_mut() {
                        crate::frame::allow_dark_mode_for_window(menu_hwnd);
                    }
                }
            }
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as i32;
            let mut pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let r = sidebar_rect_of(hwnd, state);
            if pt.x >= r.left && state.borrow().sidebar_visible {
                let dpi = GetDpiForWindow(hwnd);
                let step = (48.0 * dpi as f32 / 96.0) as i32;
                {
                    let mut b = state.borrow_mut();
                    let cur = b.sidebar_state.scroll;
                    b.sidebar_state.scroll = cur - delta * step / 120;
                }
                clamp_scroll(hwnd, state);
                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }
        }
        WM_SIZE => {
            let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);

            let on_resize = state.borrow().on_resize.clone();
            if let Some(cb) = on_resize {
                cb(content_rect, sidebar_rect);
            }

            clamp_scroll(hwnd, state);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let suggested_rect = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested_rect.left,
                suggested_rect.top,
                suggested_rect.right - suggested_rect.left,
                suggested_rect.bottom - suggested_rect.top,
                SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetAsyncKeyState, VK_CONTROL, VK_SHIFT, VK_MENU};
            let ctrl = (unsafe { GetKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0;
            let shift = (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0;
            let alt = (unsafe { GetKeyState(VK_MENU.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as u16 & 0x8000) != 0;

            let on_action = state.borrow().on_action.clone();
            let vk = wparam.0 as u32;

            if alt {
                if vk == 0x25 { // Left arrow -> Back
                    if let Some(ref cb) = on_action { cb(SidebarAction::NavBack); }
                    return LRESULT(0);
                } else if vk == 0x27 { // Right arrow -> Forward
                    if let Some(ref cb) = on_action { cb(SidebarAction::NavForward); }
                    return LRESULT(0);
                }
            }

            if ctrl {
                if shift {
                    match vk {
                        0x43 => { // 'C' -> Copy URL
                            if let Some(ref cb) = on_action { cb(SidebarAction::CopyUrl); }
                            return LRESULT(0);
                        }
                        0x58 => { // 'X' -> Pin Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::TogglePinCurrentTab); }
                            return LRESULT(0);
                        }
                        0x52 => { // 'R' -> Hard Reload
                            if let Some(ref cb) = on_action { cb(SidebarAction::HardReload); }
                            return LRESULT(0);
                        }
                        0x09 => { // Tab -> Prev Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::PrevTab); }
                            return LRESULT(0);
                        }
                        0x54 => { // 'T' -> Reopen Closed Tab!
                            if let Some(ref cb) = on_action { cb(SidebarAction::ReopenClosedTab); }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                } else {
                    match vk {
                        0x54 => { // 'T' -> New Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::NavNewTab); }
                            return LRESULT(0);
                        }
                        0x57 => { // 'W' -> Close Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::CloseTab(usize::MAX)); }
                            return LRESULT(0);
                        }
                        0x53 | 0x42 => { // 'S' or 'B' -> Toggle Sidebar
                            if let Some(ref cb) = on_action { cb(SidebarAction::ToggleSidebar); }
                            return LRESULT(0);
                        }
                        0x52 => { // 'R' -> Reload
                            if let Some(ref cb) = on_action { cb(SidebarAction::NavReload); }
                            return LRESULT(0);
                        }
                        0x4C => { // 'L' -> Focus URL Bar
                            if let Some(ref cb) = on_action { cb(SidebarAction::UrlBarClicked); }
                            return LRESULT(0);
                        }
                        0xBC => { // ',' -> Settings
                            if let Some(ref cb) = on_action { cb(SidebarAction::OpenSettings); }
                            return LRESULT(0);
                        }
                        0xBB | 0x6B => { // '+' or '=' -> Zoom In
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomIn); }
                            return LRESULT(0);
                        }
                        0xBD | 0x6D => { // '-' -> Zoom Out
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomOut); }
                            return LRESULT(0);
                        }
                        0x30 | 0x60 => { // '0' -> Reset Zoom
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomReset); }
                            return LRESULT(0);
                        }
                        0x44 => { // 'D' -> Pin Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::TogglePinCurrentTab); }
                            return LRESULT(0);
                        }
                        0x09 => { // Tab -> Next Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::NextTab); }
                            return LRESULT(0);
                        }
                        0x31..=0x39 => { // '1'..'9' -> Select Tab
                            let idx = (vk - 0x31) as usize;
                            if let Some(ref cb) = on_action { cb(SidebarAction::SelectTab(idx)); }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
            } else if vk == 0x74 { // F5 -> Reload
                if let Some(ref cb) = on_action { cb(SidebarAction::NavReload); }
                return LRESULT(0);
            }
        }
        msg if msg >= WM_APP => {
            let handler = state.borrow().on_custom_msg.clone();
            if let Some(handler) = handler {
                handler(msg, wparam, lparam);
            }
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == 0x4201 => {
            unsafe {
                let _ = KillTimer(Some(hwnd), 0x4201);
                state.borrow_mut().sidebar_state.toast = None;
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_CLOSE => {
            let handler = state.borrow().on_custom_msg.clone();
            if let Some(handler) = handler {
                handler(WM_CLOSE, wparam, lparam);
            }
            let _ = DestroyWindow(hwnd);
            return LRESULT(0);
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}