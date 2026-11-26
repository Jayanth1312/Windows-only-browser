use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush,
    SelectObject, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL,
    FW_SEMIBOLD,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, CS_DBLCLKS, CS_HREDRAW,
    CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
    ULW_ALPHA, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const AC_SRC_OVER: u32 = 0x00;
const AC_SRC_ALPHA: u32 = 0x01;

use crate::command_bar::finalize_alpha;
use crate::sidebar::{make_font, rc, rgb, text, Canvas};
use crate::window::UiError;

pub struct SplitDropOverlayState {
    pub parent_hwnd: HWND,
    pub target_rect: RECT,
    pub title: String,
    pub accent: COLORREF,
}

pub struct SplitDropOverlay {
    pub hwnd: HWND,
    pub state: Rc<RefCell<SplitDropOverlayState>>,
}

impl SplitDropOverlay {
    pub fn new(parent_hwnd: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserSplitDropOverlayClass");
        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(overlay_wndproc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let state = Rc::new(RefCell::new(SplitDropOverlayState {
                parent_hwnd,
                target_rect: RECT::default(),
                title: "Drop to Split View".to_string(),
                accent: rgb(80, 140, 240),
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut std::ffi::c_void;

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
                class_name,
                w!("Split Drop Overlay"),
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

            Ok(Self { hwnd, state })
        }
    }

    pub fn show(&self, target_rect: RECT, title: &str, accent: COLORREF) {
        {
            let mut st = self.state.borrow_mut();
            st.target_rect = target_rect;
            st.title = title.to_string();
            st.accent = accent;
        }

        unsafe {
            self.render_and_update();
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    unsafe fn render_and_update(&self) {
        let (rect, title, _accent, parent) = {
            let st = self.state.borrow();
            (st.target_rect, st.title.clone(), st.accent, st.parent_hwnd)
        };

        let mut pt = POINT { x: rect.left, y: rect.top };
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(parent, &mut pt);

        let w = (rect.right - rect.left).max(10);
        let h = (rect.bottom - rect.top).max(10);

        let dpi = GetDpiForWindow(self.hwnd);
        let s = dpi as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

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
        std::ptr::write_bytes(bits as *mut u8, 0, (w * h * 4) as usize);

        let cv = Canvas {
            dc: mem_dc,
            bits: bits as *mut u32,
            w,
            h,
            clip: std::cell::Cell::new(rc(0, 0, w, h)),
        };

        // Draw translucent neutral backdrop & border
        let margin = px(6.0);
        let r = rc(margin, margin, w - margin, h - margin);
        let corner_radius = px(12.0);

        // Subtle fill with dark glass/surface
        cv.fill_round(&r, corner_radius, rgb(255, 255, 255), 0.06);
        // Clean highlight border
        cv.stroke_round(&r, corner_radius, rgb(255, 255, 255), 0.35, 1.5);

        // Inner badge in the center
        let badge_w = px(160.0).min(w - margin * 4);
        let badge_h = px(40.0);
        let badge_x = (w - badge_w) / 2;
        let badge_y = (h - badge_h) / 2;
        let badge_rect = rc(badge_x, badge_y, badge_x + badge_w, badge_y + badge_h);

        cv.fill_round(&badge_rect, px(8.0), rgb(24, 24, 28), 0.95);
        cv.stroke_round(&badge_rect, px(8.0), rgb(255, 255, 255), 0.22, 1.0);

        // Draw icon & title
        let font_title = make_font(w!("Segoe UI"), px(13.0), FW_SEMIBOLD.0 as i32, ANTIALIASED_QUALITY);
        let font_icon = make_font(w!("Segoe Fluent Icons"), px(15.0), FW_NORMAL.0 as i32, ANTIALIASED_QUALITY);

        let icon_r = rc(badge_x + px(10.0), badge_y, badge_x + px(34.0), badge_y + badge_h);
        SelectObject(mem_dc, font_icon.into());
        let icon_str = String::from_utf16_lossy(&[0xE740]); // Split View icon
        text(mem_dc, &icon_str, icon_r, DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0, rgb(220, 220, 225));

        let text_r = rc(badge_x + px(36.0), badge_y, badge_x + badge_w - px(10.0), badge_y + badge_h);
        SelectObject(mem_dc, font_title.into());
        text(mem_dc, &title, text_r, DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0, rgb(255, 255, 255));

        let _ = GdiFlush();

        finalize_alpha(
            bits as *mut u32,
            w,
            h,
            &r,
            corner_radius as f32,
            0.0,
            0.0,
            0.0,
        );

        let pt_src = POINT { x: 0, y: 0 };
        let pt_dst = POINT { x: pt.x, y: pt.y };
        let sz = SIZE { cx: w, cy: h };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let _ = UpdateLayeredWindow(
            self.hwnd,
            None,
            Some(&pt_dst),
            Some(&sz),
            Some(mem_dc),
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        let _ = SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            pt_dst.x,
            pt_dst.y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );

        let _ = SelectObject(mem_dc, old_bm);
        let _ = DeleteObject(font_title.into());
        let _ = DeleteObject(font_icon.into());
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mem_dc);
    }
}

impl Drop for SplitDropOverlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SplitDropOverlayState;

    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw = cs.lpCreateParams as isize;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = Rc::from_raw(state_ptr);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
