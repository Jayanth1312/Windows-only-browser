use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, IsZoomed, PostMessageW, PostQuitMessage, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, MSG, SWP_FRAMECHANGED,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
    SW_SHOW, WM_APP, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST,
    WM_PAINT, WM_SIZE, WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_OVERLAPPEDWINDOW,
};

use crate::frame::{enable_modern_frame, handle_nccalcsize, handle_nchittest};
use crate::sidebar::{CaptionButton, SidebarClickTarget, SidebarRenderer, TabItemView};

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
    SelectTab(usize),
    CloseTab(usize),
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
            title: "Arc Browser".to_string(),
            width: 1280,
            height: 800,
            sidebar_width: 260,
            caption_height: 48,
        }
    }
}

pub type ResizeCallback = Rc<dyn Fn(RECT, RECT)>;
pub type ActionCallback = Rc<dyn Fn(SidebarAction)>;
pub type CustomMsgHandler = Rc<dyn Fn(u32, WPARAM, LPARAM)>;

struct WindowState {
    config: WindowConfig,
    sidebar: SidebarRenderer,
    sidebar_visible: bool,
    tabs: Vec<TabItemView>,
    on_resize: Option<ResizeCallback>,
    on_action: Option<ActionCallback>,
    on_custom_msg: Option<CustomMsgHandler>,
    hovered_target: SidebarClickTarget,
    pressed_target: SidebarClickTarget,
    is_tracking_mouse: bool,
}

pub struct BrowserWindow {
    hwnd: HWND,
    state: Rc<RefCell<WindowState>>,
}

impl BrowserWindow {
    pub fn new(config: WindowConfig) -> Result<Self, UiError> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        let class_name = w!("ArcBrowserWindowClass");

        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };

            let _ = RegisterClassW(&wc);

            let state = Rc::new(RefCell::new(WindowState {
                config,
                sidebar: SidebarRenderer::new(),
                sidebar_visible: true,
                tabs: Vec::new(),
                on_resize: None,
                on_action: None,
                on_custom_msg: None,
                hovered_target: SidebarClickTarget::None,
                pressed_target: SidebarClickTarget::None,
                is_tracking_mouse: false,
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut _;

            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("Arc Browser"),
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

    pub fn update_tabs(&self, tabs: Vec<TabItemView>) {
        self.state.borrow_mut().tabs = tabs;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
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

    pub fn get_layout_rects(&self) -> (RECT, RECT) {
        let mut client_rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client_rect);
        }

        let width = client_rect.right - client_rect.left;
        let height = client_rect.bottom - client_rect.top;
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = dpi as f32 / 96.0;

        let sidebar_visible = self.state.borrow().sidebar_visible;

        if sidebar_visible {
            let sidebar_width = (self.state.borrow().config.sidebar_width as f32 * scale).round() as i32;
            let content_width = (width - sidebar_width).max(0);

            let content_rect = RECT {
                left: 0,
                top: 0,
                right: content_width,
                bottom: height,
            };

            let sidebar_rect = RECT {
                left: content_width,
                top: 0,
                right: width,
                bottom: height,
            };

            (content_rect, sidebar_rect)
        } else {
            let content_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };

            let btn_width = (46.0 * scale).round() as i32;
            let btn_height = (32.0 * scale).round() as i32;
            let sidebar_rect = RECT {
                left: width - (btn_width * 4),
                top: 0,
                right: width,
                bottom: btn_height,
            };

            (content_rect, sidebar_rect)
        }
    }

    pub fn trigger_resize(&self) {
        let (content_rect, sidebar_rect) = self.get_layout_rects();
        let cb = self.state.borrow().on_resize.clone();
        if let Some(cb) = cb {
            cb(content_rect, sidebar_rect);
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
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
            let (sidebar_w, caption_h, sidebar_vis) = {
                let b = state.borrow();
                let dpi = GetDpiForWindow(hwnd);
                let sw = (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;
                let ch = (b.config.caption_height as f32 * (dpi as f32 / 96.0)).round() as i32;
                (sw, ch, b.sidebar_visible)
            };
            return handle_nchittest(hwnd, lparam, sidebar_w, caption_h, sidebar_vis);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;

            let (sidebar_vis, tabs, hovered, pressed) = {
                let b = state.borrow();
                (
                    b.sidebar_visible,
                    b.tabs.clone(),
                    b.hovered_target.clone(),
                    b.pressed_target.clone(),
                )
            };

            let sidebar_rect = if sidebar_vis {
                let sidebar_w = (state.borrow().config.sidebar_width as f32 * scale).round() as i32;
                RECT {
                    left: (width - sidebar_w).max(0),
                    top: 0,
                    right: width,
                    bottom: height,
                }
            } else {
                let btn_w = (46.0 * scale).round() as i32;
                let btn_h = (32.0 * scale).round() as i32;
                RECT {
                    left: width - (btn_w * 4),
                    top: 0,
                    right: width,
                    bottom: btn_h,
                }
            };

            state.borrow().sidebar.render(
                hwnd,
                hdc,
                sidebar_rect,
                sidebar_vis,
                &tabs,
                &hovered,
                &pressed,
            );

            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let client_width = client_rect.right;

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

            let (sidebar_vis, tabs_len) = {
                let b = state.borrow();
                (b.sidebar_visible, b.tabs.len())
            };

            let target = state
                .borrow()
                .sidebar
                .hit_test(hwnd, x, y, client_width, sidebar_vis, tabs_len);

            let old_target = state.borrow().hovered_target.clone();
            if target != old_target {
                state.borrow_mut().hovered_target = target;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            state.borrow_mut().is_tracking_mouse = false;
            let old_hov = state.borrow().hovered_target.clone();
            let old_pre = state.borrow().pressed_target.clone();
            if old_hov != SidebarClickTarget::None || old_pre != SidebarClickTarget::None {
                state.borrow_mut().hovered_target = SidebarClickTarget::None;
                state.borrow_mut().pressed_target = SidebarClickTarget::None;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let client_width = client_rect.right;

            let (sidebar_vis, tabs_len) = {
                let b = state.borrow();
                (b.sidebar_visible, b.tabs.len())
            };

            let target = state
                .borrow()
                .sidebar
                .hit_test(hwnd, x, y, client_width, sidebar_vis, tabs_len);

            if target != SidebarClickTarget::None {
                state.borrow_mut().pressed_target = target;
                let _ = SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let client_width = client_rect.right;

            let pressed = state.borrow().pressed_target.clone();
            if pressed != SidebarClickTarget::None {
                let _ = ReleaseCapture();
                state.borrow_mut().pressed_target = SidebarClickTarget::None;

                let (sidebar_vis, tabs_len) = {
                    let b = state.borrow();
                    (b.sidebar_visible, b.tabs.len())
                };

                let released = state
                    .borrow()
                    .sidebar
                    .hit_test(hwnd, x, y, client_width, sidebar_vis, tabs_len);

                let _ = InvalidateRect(Some(hwnd), None, false);

                if pressed == released {
                    let on_action = state.borrow().on_action.clone();
                    match pressed {
                        SidebarClickTarget::Caption(CaptionButton::Minimize) => {
                            let _ = ShowWindow(hwnd, SW_MINIMIZE);
                        }
                        SidebarClickTarget::Caption(CaptionButton::Maximize) => {
                            if IsZoomed(hwnd).as_bool() {
                                let _ = ShowWindow(hwnd, SW_RESTORE);
                            } else {
                                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                            }
                        }
                        SidebarClickTarget::Caption(CaptionButton::Close) => {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                        SidebarClickTarget::ToggleSidebar => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleSidebar);
                            }
                        }
                        SidebarClickTarget::NavBack => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavBack);
                            }
                        }
                        SidebarClickTarget::NavForward => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavForward);
                            }
                        }
                        SidebarClickTarget::NavReload => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavReload);
                            }
                        }
                        SidebarClickTarget::NavNewTab => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavNewTab);
                            }
                        }
                        SidebarClickTarget::SelectTab(idx) => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::SelectTab(idx));
                            }
                        }
                        SidebarClickTarget::CloseTab(idx) => {
                            if let Some(cb) = on_action {
                                cb(SidebarAction::CloseTab(idx));
                            }
                        }
                        _ => {}
                    }
                }
                return LRESULT(0);
            }
        }
        WM_SIZE => {
            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;

            let sidebar_vis = state.borrow().sidebar_visible;

            let (content_rect, sidebar_rect) = if sidebar_vis {
                let sidebar_w = (state.borrow().config.sidebar_width as f32 * scale).round() as i32;
                let content_width = (width - sidebar_w).max(0);
                (
                    RECT { left: 0, top: 0, right: content_width, bottom: height },
                    RECT { left: content_width, top: 0, right: width, bottom: height },
                )
            } else {
                let btn_w = (46.0 * scale).round() as i32;
                let btn_h = (32.0 * scale).round() as i32;
                (
                    RECT { left: 0, top: 0, right: width, bottom: height },
                    RECT { left: width - (btn_w * 4), top: 0, right: width, bottom: btn_h },
                )
            };

            let on_resize = state.borrow().on_resize.clone();
            if let Some(cb) = on_resize {
                cb(content_rect, sidebar_rect);
            }

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
            let ctrl = (windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32,
            ) as u16
                & 0x8000)
                != 0;
            if ctrl {
                let on_action = state.borrow().on_action.clone();
                match wparam.0 as u32 {
                    0x54 => { // 'T' -> New Tab / Command Bar
                        if let Some(cb) = on_action {
                            cb(SidebarAction::NavNewTab);
                        }
                        return LRESULT(0);
                    }
                    0x53 => { // 'S' -> Toggle Sidebar
                        if let Some(cb) = on_action {
                            cb(SidebarAction::ToggleSidebar);
                        }
                        return LRESULT(0);
                    }
                    0x52 => { // 'R' -> Reload
                        if let Some(cb) = on_action {
                            cb(SidebarAction::NavReload);
                        }
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
        }
        msg if msg >= WM_APP => {
            let handler = state.borrow().on_custom_msg.clone();
            if let Some(handler) = handler {
                handler(msg, wparam, lparam);
            }
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