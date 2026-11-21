use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, GetTextExtentPoint32W, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    FW_NORMAL, FW_SEMIBOLD,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, KillTimer, RegisterClassW,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow,
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::command_bar::finalize_alpha;
use crate::sidebar::{make_font, rc, rgb, text, Canvas};
use crate::window::UiError;

const TOAST_TIMER_DISMISS: usize = 0x5101;

pub struct ToastState {
    pub parent_hwnd: HWND,
    pub message: String,
    pub glyph: u16,
}

pub struct ToastOverlay {
    pub hwnd: HWND,
    pub state: Rc<RefCell<ToastState>>,
}

impl ToastOverlay {
    pub fn new(parent_hwnd: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserToastClass");
        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(toast_wndproc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let state = Rc::new(RefCell::new(ToastState {
                parent_hwnd,
                message: String::new(),
                glyph: 0xE72D,
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut _;

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                class_name,
                w!("Browser Toast"),
                WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
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

            Ok(Self { hwnd, state })
        }
    }

    pub fn show(&self, message: &str) {
        let glyph = if message.contains("Zoom") || message.contains("zoom") {
            0xE71E // Zoom in Segoe Fluent Icons
        } else if message.contains("Copied") || message.contains("copied") || message.contains("URL") {
            0xE72D // Share / Link
        } else if message.contains("Reopened") {
            0xE7A7 // Rotate / History
        } else if message.contains("Favourite") || message.contains("Favorite") {
            0xE735 // Star / Favourite in Segoe Fluent Icons
        } else {
            0xE946 // Info
        };
        self.show_with_glyph(message, glyph);
    }

    pub fn show_with_glyph(&self, message: &str, glyph: u16) {
        {
            let mut st = self.state.borrow_mut();
            st.message = message.to_string();
            st.glyph = glyph;
        }

        unsafe {
            let _ = KillTimer(Some(self.hwnd), TOAST_TIMER_DISMISS);
            self.render_and_update();
            let _ = SetTimer(Some(self.hwnd), TOAST_TIMER_DISMISS, 2200, None);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TOAST_TIMER_DISMISS);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    unsafe fn render_and_update(&self) {
        let dpi = GetDpiForWindow(self.hwnd);
        let s = dpi as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

        let st = self.state.borrow();
        let parent = st.parent_hwnd;

        // Measure text
        let mem_dc = match CreateCompatibleDC(None) {
            dc if !dc.0.is_null() => dc,
            _ => return,
        };

        let font_title = make_font(w!("Segoe UI"), px(14.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
        let font_icon = make_font(w!("Segoe Fluent Icons"), px(16.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);

        let old_font = SelectObject(mem_dc, font_title.into());
        let text_w = {
            let buf: Vec<u16> = st.message.encode_utf16().collect();
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(mem_dc, &buf, &mut sz);
            sz.cx
        };

        let shadow_m = px(16.0);
        let icon_box_w = px(24.0);
        let gap = px(10.0);
        let pad_h = px(18.0);
        let card_h = px(44.0);
        let card_w = (pad_h + icon_box_w + gap + text_w + pad_h).max(px(160.0));

        let win_w = card_w + shadow_m * 2;
        let win_h = card_h + shadow_m * 2;

        // Create 32-bit DIB Section
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: win_w,
                biHeight: -win_h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = match CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = SelectObject(mem_dc, old_font);
                let _ = DeleteObject(font_title.into());
                let _ = DeleteObject(font_icon.into());
                let _ = DeleteDC(mem_dc);
                return;
            }
        };

        let old_bmp = SelectObject(mem_dc, dib.into());
        std::ptr::write_bytes(bits as *mut u8, 0, (win_w * win_h * 4) as usize);

        let cv = Canvas {
            dc: mem_dc,
            bits: bits as *mut u32,
            w: win_w,
            h: win_h,
            clip: std::cell::Cell::new(rc(0, 0, win_w, win_h)),
        };

        let card_l = shadow_m;
        let card_t = shadow_m;
        let card_r = card_l + card_w;
        let card_b = card_t + card_h;
        let card_rc = rc(card_l, card_t, card_r, card_b);
        let radius = px(12.0);

        // Card background: pure grayish black
        cv.fill_round(&card_rc, radius, rgb(28, 28, 28), 0.98);

        // Subtle modern border
        cv.stroke_round(&card_rc, radius, rgb(255, 255, 255), 0.10, 1.0);

        // Draw Icon
        let icon_r = rc(card_l + pad_h - px(2.0), card_t, card_l + pad_h + icon_box_w, card_b);
        SelectObject(mem_dc, font_icon.into());
        let icon_str = String::from_utf16_lossy(&[st.glyph]);
        // Subtle crisp cyan-blue accent for glyph
        let icon_color = rgb(110, 195, 255);
        text(mem_dc, &icon_str, icon_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, icon_color);

        // Draw Text
        let text_l = icon_r.right + gap;
        let text_r = card_r - pad_h;
        let text_rc = rc(text_l, card_t, text_r, card_b);
        SelectObject(mem_dc, font_title.into());
        text(mem_dc, &st.message, text_rc, DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(245, 246, 250));

        let _ = GdiFlush();

        // Anti-aliased alpha mask and smooth drop shadow
        finalize_alpha(
            bits as *mut u32,
            win_w,
            win_h,
            &card_rc,
            radius as f32,
            px(3.0) as f32,
            px(10.0) as f32,
            0.45,
        );

        // Position on top-left of the browser client window
        let mut pt = POINT { x: px(24.0), y: px(24.0) };
        let _ = ClientToScreen(parent, &mut pt);

        let window_pt = POINT { x: pt.x - shadow_m, y: pt.y - shadow_m };
        let win_size = SIZE { cx: win_w, cy: win_h };
        let src_pt = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let _ = UpdateLayeredWindow(
            self.hwnd,
            None,
            Some(&window_pt),
            Some(&win_size),
            Some(mem_dc),
            Some(&src_pt),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        let _ = SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            window_pt.x,
            window_pt.y,
            win_w,
            win_h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );

        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);

        // Cleanup
        let _ = SelectObject(mem_dc, old_bmp);
        let _ = SelectObject(mem_dc, old_font);
        let _ = DeleteObject(dib.into());
        let _ = DeleteObject(font_title.into());
        let _ = DeleteObject(font_icon.into());
        let _ = DeleteDC(mem_dc);
    }
}

impl Drop for ToastOverlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn toast_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<ToastState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<ToastState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_TIMER if wparam.0 == TOAST_TIMER_DISMISS => {
            let _ = KillTimer(Some(hwnd), TOAST_TIMER_DISMISS);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TOAST_TIMER_DISMISS);
            let _ = Rc::from_raw(state_ptr);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
