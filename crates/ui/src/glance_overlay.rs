use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, GdiFlush, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    IsWindowVisible, LoadCursorW, RegisterClassW, SetCursor, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, UpdateLayeredWindow, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA,
    HWND_TOP, IDC_ARROW, IDC_HAND, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA,
    WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::sidebar::{draw_svg_icon, rc, rgb, Canvas};
use crate::window::UiError;

const WM_MOUSELEAVE: u32 = 0x02A3;

const SVG_CLOSE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>"##;
const SVG_EXPAND: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" y1="3" x2="14" y2="10"/><line x1="3" y1="21" x2="10" y2="14"/></svg>"##;
const SVG_SPLIT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="3"/><line x1="12" y1="4" x2="12" y2="20"/></svg>"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlanceAction {
    Close,
    OpenAsTab,
    OpenInSplit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlanceHit {
    Backdrop,
    Close,
    OpenAsTab,
    OpenInSplit,
}

pub struct GlanceOverlayState {
    pub parent_hwnd: HWND,
    pub web_hwnd: HWND,
    pub visible: bool,
    pub content_rect: RECT,
    pub card_rect: RECT,
    hovered: GlanceHit,
    pressed: Option<GlanceHit>,
    btn_close: RECT,
    btn_expand: RECT,
    btn_split: RECT,
    is_tracking: bool,
    pub on_action: Option<Rc<dyn Fn(GlanceAction)>>,
}

pub struct GlanceOverlay {
    pub hwnd: HWND,
    pub web_hwnd: HWND,
    pub state: Rc<RefCell<GlanceOverlayState>>,
}

impl GlanceOverlay {
    pub fn new(parent_hwnd: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserGlanceOverlayClass");
        let web_class_name = w!("BrowserGlanceWebHostClass");
        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(glance_wndproc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let web_wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(glance_web_wndproc),
                hInstance: hinstance,
                lpszClassName: web_class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&web_wc);

            let state = Rc::new(RefCell::new(GlanceOverlayState {
                parent_hwnd,
                web_hwnd: HWND::default(),
                visible: false,
                content_rect: RECT::default(),
                card_rect: RECT::default(),
                hovered: GlanceHit::Backdrop,
                pressed: None,
                btn_close: RECT::default(),
                btn_expand: RECT::default(),
                btn_split: RECT::default(),
                is_tracking: false,
                on_action: None,
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut c_void;

            // Owned by main_hwnd (NO WS_EX_TOPMOST so it never floats over other apps)
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                class_name,
                w!("Glance Overlay"),
                WS_POPUP,
                0,
                0,
                100,
                100,
                Some(parent_hwnd),
                None,
                Some(hinstance),
                Some(raw_state),
            )?;

            if hwnd.0.is_null() {
                return Err(UiError::WindowCreationFailed);
            }

            // Owned by `hwnd` (the backdrop), so it always sits cleanly above the backdrop
            // and never z-fights with main_hwnd's background WebView2 children.
            let web_hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                web_class_name,
                w!("Glance Web"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                100,
                100,
                Some(hwnd),
                None,
                Some(hinstance),
                None,
            )?;

            if web_hwnd.0.is_null() {
                let _ = DestroyWindow(hwnd);
                return Err(UiError::WindowCreationFailed);
            }

            state.borrow_mut().web_hwnd = web_hwnd;

            Ok(Self {
                hwnd,
                web_hwnd,
                state,
            })
        }
    }

    pub fn web_hwnd(&self) -> HWND {
        self.web_hwnd
    }

    pub fn set_on_action<F: Fn(GlanceAction) + 'static>(&self, cb: F) {
        self.state.borrow_mut().on_action = Some(Rc::new(cb));
    }

    /// Computes the modal card rectangle in `main_hwnd` client coordinates:
    /// touches the top and bottom of `content_rect` (full height) with dimmed margins
    /// on left/right, leaving room on the **right** side for the Glance window controls.
    pub fn compute_card_rect(parent_hwnd: HWND, content_rect: RECT) -> RECT {
        let dpi = unsafe { GetDpiForWindow(parent_hwnd) }.max(96);
        let s = dpi as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

        let cw = (content_rect.right - content_rect.left).max(100);
        let ch = (content_rect.bottom - content_rect.top).max(100);

        let right_clearance = px(62.0); // 14px gap + 36px button + 12px outer margin
        let left_min_margin = px(20.0);
        let max_card_w = (cw - right_clearance - left_min_margin).max(160);
        let card_w = ((cw as f32 * 0.76).round() as i32)
            .max(px(320.0).min(max_card_w))
            .min(max_card_w);

        // Center horizontally, clamped so the right-side controls always fit inside content_rect
        let centered_x = content_rect.left + (cw - card_w) / 2;
        let max_x = content_rect.right - right_clearance - card_w;
        let card_x = centered_x.min(max_x).max(content_rect.left + left_min_margin);

        // Touch the top and bottom edges of content_rect
        let card_y = content_rect.top;
        let card_h = ch;

        RECT {
            left: card_x,
            top: card_y,
            right: card_x + card_w,
            bottom: card_y + card_h,
        }
    }

    pub fn show(&self, content_rect: RECT, card_rect: RECT) {
        {
            let mut st = self.state.borrow_mut();
            st.visible = true;
            st.content_rect = content_rect;
            st.card_rect = card_rect;
            st.hovered = GlanceHit::Backdrop;
            st.pressed = None;
        }
        unsafe {
            self.render_and_update();
        }
    }

    pub fn update_layout(&self, content_rect: RECT, card_rect: RECT) {
        if !self.state.borrow().visible || !self.is_visible() {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            st.content_rect = content_rect;
            st.card_rect = card_rect;
        }
        unsafe {
            self.render_and_update();
        }
    }

    pub fn hide(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.visible = false;
            st.hovered = GlanceHit::Backdrop;
            st.pressed = None;
            st.is_tracking = false;
        }
        unsafe {
            let _ = ShowWindow(self.web_hwnd, SW_HIDE);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    unsafe fn render_and_update(&self) {
        render_glance_hwnd(self.hwnd, &self.state);
    }
}

unsafe fn render_glance_hwnd(hwnd: HWND, state_rc: &Rc<RefCell<GlanceOverlayState>>) {
    let (visible, content_rect, card_rect, parent, web_hwnd, hovered, pressed) = {
        let st = state_rc.borrow();
        (
            st.visible,
            st.content_rect,
            st.card_rect,
            st.parent_hwnd,
            st.web_hwnd,
            st.hovered,
            st.pressed,
        )
    };
    if !visible {
        return;
    }

    let mut pt = POINT {
        x: content_rect.left,
        y: content_rect.top,
    };
    let _ = ClientToScreen(parent, &mut pt);

    let mut card_screen_pt = POINT {
        x: card_rect.left,
        y: card_rect.top,
    };
    let _ = ClientToScreen(parent, &mut card_screen_pt);

    let w = (content_rect.right - content_rect.left).max(10);
    let h = (content_rect.bottom - content_rect.top).max(10);
    let card_w = (card_rect.right - card_rect.left).max(10);
    let card_h = (card_rect.bottom - card_rect.top).max(10);

    let dpi = GetDpiForWindow(hwnd).max(96);
    let s = dpi as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;

    // Local coordinates relative to content_rect origin (0, 0)
    let local_card = RECT {
        left: card_rect.left - content_rect.left,
        top: card_rect.top - content_rect.top,
        right: card_rect.right - content_rect.left,
        bottom: card_rect.bottom - content_rect.top,
    };

    // Place the 3 circular Glance window controls on the RIGHT side of the card
    let btn_size = px(36.0);
    let btn_gap = px(10.0);
    let btn_x = (local_card.right + px(14.0)).min(w - btn_size - px(8.0));
    let btn_y0 = (local_card.top + px(16.0)).max(px(8.0));

    let btn_close = rc(btn_x, btn_y0, btn_x + btn_size, btn_y0 + btn_size);
    let btn_expand = rc(
        btn_x,
        btn_y0 + btn_size + btn_gap,
        btn_x + btn_size,
        btn_y0 + 2 * btn_size + btn_gap,
    );
    let btn_split = rc(
        btn_x,
        btn_y0 + 2 * (btn_size + btn_gap),
        btn_x + btn_size,
        btn_y0 + 3 * btn_size + 2 * btn_gap,
    );

    {
        let mut st = state_rc.borrow_mut();
        st.btn_close = btn_close;
        st.btn_expand = btn_expand;
        st.btn_split = btn_split;
    }

    let mem_dc = match CreateCompatibleDC(None) {
        dc if !dc.0.is_null() => dc,
        _ => return,
    };

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut c_void = std::ptr::null_mut();
    let dib = match CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(bm) if !bits.is_null() => bm,
        _ => {
            let _ = DeleteDC(mem_dc);
            return;
        }
    };

    let old_bm = SelectObject(mem_dc, dib.into());
    let buf = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);

    // Draw dimmed backdrop + sharp card shadow + crisp 1px sharp hairline border (no roundness)
    let cx = (local_card.left + local_card.right) as f32 * 0.5;
    let cy = (local_card.top + local_card.bottom) as f32 * 0.5;
    let hx = (local_card.right - local_card.left) as f32 * 0.5;
    let hy = (local_card.bottom - local_card.top) as f32 * 0.5;
    let blur = 20.0 * s;
    let dim_a = 0.42_f32;

    for y in 0..h {
        let py_f = y as f32 + 0.5;
        for x in 0..w {
            let px_f = x as f32 + 0.5;

            // Signed distance to sharp rectangle (0px corner radius)
            let dx = (px_f - cx).abs() - hx;
            let dy = (py_f - cy).abs() - hy;
            let ox = dx.max(0.0);
            let oy = dy.max(0.0);
            let d = (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0);

            if d <= 0.0 {
                // Inside sharp card: transparent
                buf[(y * w + x) as usize] = 0;
            } else {
                // Soft shadow outside sharp edges
                let shadow = if d < blur {
                    let t = 1.0 - (d / blur);
                    0.35 * t * t
                } else {
                    0.0
                };

                let total_dark_a = (dim_a + shadow * (1.0 - dim_a)).clamp(0.0, 0.90);

                // Crisp 1px sharp border outside card edge
                let border_a = if d <= 1.1 { 0.24 } else { 0.0 };

                let final_a = (total_dark_a + border_a * (1.0 - total_dark_a)).clamp(0.0, 1.0);
                let white_val = (border_a * 255.0).round() as u32;
                let a_byte = (final_a * 255.0).round() as u32;
                buf[(y * w + x) as usize] =
                    (a_byte << 24) | (white_val << 16) | (white_val << 8) | white_val;
            }
        }
    }

    let cv = Canvas {
        dc: mem_dc,
        bits: bits as *mut u32,
        w,
        h,
        clip: std::cell::Cell::new(rc(0, 0, w, h)),
    };

    let draw_control_btn = |rect: RECT, hit: GlanceHit, svg: &str| {
        let is_pressed = pressed == Some(hit);
        let is_hovered = hovered == hit;
        let (bg, bg_a, border_a, icon_color) = if is_pressed {
            (rgb(64, 64, 74), 0.98, 0.42, "#FFFFFF")
        } else if is_hovered {
            (rgb(48, 48, 56), 0.96, 0.34, "#FFFFFF")
        } else {
            (rgb(26, 26, 32), 0.92, 0.20, "#E8E8EE")
        };
        let radius = (rect.right - rect.left) / 2;
        cv.fill_round(&rect, radius, bg, bg_a);
        cv.stroke_round(&rect, radius, rgb(255, 255, 255), border_a, 1.0);

        let icon_pad = px(9.0);
        let icon_rect = rc(
            rect.left + icon_pad,
            rect.top + icon_pad,
            rect.right - icon_pad,
            rect.bottom - icon_pad,
        );
        draw_svg_icon(&cv, &icon_rect, svg, icon_color);
    };

    draw_control_btn(btn_close, GlanceHit::Close, SVG_CLOSE);
    draw_control_btn(btn_expand, GlanceHit::OpenAsTab, SVG_EXPAND);
    draw_control_btn(btn_split, GlanceHit::OpenInSplit, SVG_SPLIT);

    let _ = GdiFlush();

    let src_pt = POINT { x: 0, y: 0 };
    let dst_pt = POINT { x: pt.x, y: pt.y };
    let size = SIZE { cx: w, cy: h };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    let _ = UpdateLayeredWindow(
        hwnd,
        None,
        Some(&dst_pt),
        Some(&size),
        Some(mem_dc),
        Some(&src_pt),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOP),
        pt.x,
        pt.y,
        w,
        h,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );

    // Position the owned opaque WebView host window directly above the backdrop
    if !web_hwnd.0.is_null() {
        let _ = SetWindowPos(
            web_hwnd,
            Some(HWND_TOP),
            card_screen_pt.x,
            card_screen_pt.y,
            card_w,
            card_h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }

    SelectObject(mem_dc, old_bm);
    let _ = DeleteObject(dib.into());
    let _ = DeleteDC(mem_dc);
}

fn pt_in_circle(x: i32, y: i32, r: RECT) -> bool {
    let cx = (r.left + r.right) as f32 * 0.5;
    let cy = (r.top + r.bottom) as f32 * 0.5;
    let rad = (r.right - r.left) as f32 * 0.5 + 2.0;
    let dx = x as f32 - cx;
    let dy = y as f32 - cy;
    dx * dx + dy * dy <= rad * rad
}

fn hit_test_glance(st: &GlanceOverlayState, x: i32, y: i32) -> GlanceHit {
    if pt_in_circle(x, y, st.btn_close) {
        GlanceHit::Close
    } else if pt_in_circle(x, y, st.btn_expand) {
        GlanceHit::OpenAsTab
    } else if pt_in_circle(x, y, st.btn_split) {
        GlanceHit::OpenInSplit
    } else {
        GlanceHit::Backdrop
    }
}

unsafe extern "system" fn glance_web_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = CreateSolidBrush(rgb(27, 27, 30));
            let _ = FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn glance_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return LRESULT(0);
    }

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<GlanceOverlayState>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state_rc = Rc::from_raw(ptr);
    let state_keep = state_rc.clone();
    std::mem::forget(state_rc);

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => LRESULT(0),
        WM_SETCURSOR => {
            let hovered = state_keep.borrow().hovered;
            let cursor_id = match hovered {
                GlanceHit::Close | GlanceHit::OpenAsTab | GlanceHit::OpenInSplit => IDC_HAND,
                GlanceHit::Backdrop => IDC_ARROW,
            };
            if let Ok(cur) = LoadCursorW(None, cursor_id) {
                SetCursor(Some(cur));
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut need_redraw = false;
            {
                let mut st = state_keep.borrow_mut();
                if !st.visible {
                    return LRESULT(0);
                }
                if !st.is_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    st.is_tracking = true;
                }
                let hit = hit_test_glance(&st, x, y);
                if st.hovered != hit {
                    st.hovered = hit;
                    need_redraw = true;
                }
            }
            if need_redraw {
                render_glance_hwnd(hwnd, &state_keep);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let mut need_redraw = false;
            {
                let mut st = state_keep.borrow_mut();
                st.is_tracking = false;
                if !st.visible {
                    return LRESULT(0);
                }
                if st.hovered != GlanceHit::Backdrop || st.pressed.is_some() {
                    st.hovered = GlanceHit::Backdrop;
                    st.pressed = None;
                    need_redraw = true;
                }
            }
            if need_redraw {
                render_glance_hwnd(hwnd, &state_keep);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            {
                let mut st = state_keep.borrow_mut();
                let hit = hit_test_glance(&st, x, y);
                st.pressed = Some(hit);
            }
            render_glance_hwnd(hwnd, &state_keep);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let (action, cb) = {
                let mut st = state_keep.borrow_mut();
                let hit = hit_test_glance(&st, x, y);
                st.pressed = None;
                let act = match hit {
                    GlanceHit::Close => Some(GlanceAction::Close),
                    GlanceHit::OpenAsTab => Some(GlanceAction::OpenAsTab),
                    GlanceHit::OpenInSplit => Some(GlanceAction::OpenInSplit),
                    GlanceHit::Backdrop => Some(GlanceAction::Close),
                };
                (act, st.on_action.clone())
            };
            render_glance_hwnd(hwnd, &state_keep);
            if let (Some(a), Some(f)) = (action, cb) {
                f(a);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                let cb = state_keep.borrow().on_action.clone();
                if let Some(f) = cb {
                    f(GlanceAction::Close);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<GlanceOverlayState>;
            if !raw.is_null() {
                let _ = Rc::from_raw(raw);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl Drop for GlanceOverlay {
    fn drop(&mut self) {
        unsafe {
            if !self.web_hwnd.0.is_null() {
                let _ = DestroyWindow(self.web_hwnd);
            }
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_card_rect_leaves_right_clearance_for_controls() {
        let content = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        let card = GlanceOverlay::compute_card_rect(HWND::default(), content);
        assert!(card.left >= 20);
        assert_eq!(card.top, content.top);
        assert_eq!(card.bottom, content.bottom);
        // Right clearance must leave at least 62px for the right-side circular buttons
        assert!(content.right - card.right >= 62);
        assert!(card.right > card.left + 300);
    }

    #[test]
    fn test_compute_card_rect_narrow_window_still_fits_right_controls() {
        let content = RECT {
            left: 0,
            top: 0,
            right: 450,
            bottom: 400,
        };
        let card = GlanceOverlay::compute_card_rect(HWND::default(), content);
        assert!(card.left >= 20);
        assert_eq!(card.top, content.top);
        assert_eq!(card.bottom, content.bottom);
        assert!(content.right - card.right >= 62);
        assert!(card.right > card.left);
    }
}
