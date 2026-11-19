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
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    GWLP_USERDATA, MSG, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOW, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_PAINT, WM_SIZE, WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_OVERLAPPEDWINDOW,
};

use crate::frame::{enable_modern_frame, handle_nccalcsize, handle_nchittest};
use crate::sidebar::SidebarRenderer;

#[derive(Error, Debug)]
pub enum UiError {
    #[error("Windows error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Window creation failed")]
    WindowCreationFailed,
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

pub type ResizeCallback = Box<dyn Fn(RECT, RECT)>;

struct WindowState {
    config: WindowConfig,
    sidebar: SidebarRenderer,
    on_resize: Option<ResizeCallback>,
}

pub struct BrowserWindow {
    hwnd: HWND,
    state: Rc<RefCell<WindowState>>,
}

impl BrowserWindow {
    pub fn new(config: WindowConfig) -> Result<Self, UiError> {
        // Set per-monitor V2 DPI awareness
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
                on_resize: None,
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
        self.state.borrow_mut().on_resize = Some(Box::new(callback));
        // Trigger initial resize calculation
        self.trigger_resize();
    }

    pub fn get_layout_rects(&self) -> (RECT, RECT) {
        let mut client_rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client_rect);
        }

        let width = client_rect.right - client_rect.left;
        let height = client_rect.bottom - client_rect.top;

        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let sidebar_width = (self.state.borrow().config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;

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
    }

    pub fn trigger_resize(&self) {
        let (content_rect, sidebar_rect) = self.get_layout_rects();
        if let Some(ref cb) = self.state.borrow().on_resize {
            cb(content_rect, sidebar_rect);
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), Some(&sidebar_rect), false);
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
            let (sidebar_w, caption_h) = {
                let b = state.borrow();
                let dpi = GetDpiForWindow(hwnd);
                let sw = (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;
                let ch = (b.config.caption_height as f32 * (dpi as f32 / 96.0)).round() as i32;
                (sw, ch)
            };
            return handle_nchittest(hwnd, lparam, sidebar_w, caption_h);
        }
        WM_ERASEBKGND => {
            // Return 1 to eliminate background erasing flicker
            return LRESULT(1);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let dpi = GetDpiForWindow(hwnd);
            let (sidebar_w, caption_h) = {
                let b = state.borrow();
                let sw = (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;
                let ch = (b.config.caption_height as f32 * (dpi as f32 / 96.0)).round() as i32;
                (sw, ch)
            };

            let sidebar_rect = RECT {
                left: (width - sidebar_w).max(0),
                top: 0,
                right: width,
                bottom: height,
            };

            state.borrow().sidebar.render(hwnd, hdc, sidebar_rect, caption_h);

            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        WM_SIZE => {
            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let dpi = GetDpiForWindow(hwnd);
            let sidebar_w = {
                let b = state.borrow();
                (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32
            };

            let content_width = (width - sidebar_w).max(0);
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

            if let Some(ref cb) = state.borrow().on_resize {
                cb(content_rect, sidebar_rect);
            }

            let _ = InvalidateRect(Some(hwnd), Some(&sidebar_rect), false);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            // Suggested new window rect is in lparam
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
        WM_DESTROY => {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}
