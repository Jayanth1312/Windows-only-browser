use std::cell::RefCell;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, SelectObject, SetBkMode, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FlashWindow, GetWindowLongPtrW,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWLP_USERDATA,
    IDC_ARROW, IDC_HAND, LoadCursorW, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    ULW_ALPHA, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
    WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

#[link(name = "user32")]
extern "system" {
    fn MessageBeep(u_type: u32) -> i32;
}

use crate::command_bar::finalize_alpha;
use crate::sidebar::{draw_svg_icon, make_font, rc, rgb, text, Canvas};
use crate::window::UiError;

const SVG_COOKIE_SWIRL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2a10 10 0 1 0 10 10 4 4 0 0 1-5-5 4 4 0 0 1-5-5"/><path d="M8.5 8.5v.01"/><path d="M7.5 15.5v.01"/><path d="M12 12v.01"/><path d="M11 17v.01"/><path d="M16 15v.01"/></svg>"##;
const SVG_CLOSE_X: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogHit {
    None,
    CloseBtn,
    RemoveBtn,
    CancelBtn,
}

pub struct ClearDataDialogState {
    pub domain: String,
    pub hovered: DialogHit,
    pub on_choice: Option<Rc<dyn Fn(bool)>>, // true = Remove, false = Cancel
    pub parent_hwnd: HWND,
}

pub struct ClearDataDialog {
    pub hwnd: HWND,
    pub state: Rc<RefCell<ClearDataDialogState>>,
}

impl ClearDataDialog {
    pub fn new(owner: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserClearDataDialogClass");
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(dialog_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(ClearDataDialogState {
            domain: String::new(),
            hovered: DialogHit::None,
            on_choice: None,
            parent_hwnd: owner,
        }));

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Clear Data Confirmation"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0, 0, 10, 10,
                Some(owner),
                None,
                Some(hinstance.into()),
                Some(Rc::into_raw(state.clone()) as *const _),
            )?
        };

        Ok(Self { hwnd, state })
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn show(&self, domain: &str, on_choice: Box<dyn Fn(bool)>) {
        {
            let mut st = self.state.borrow_mut();
            st.domain = domain.to_string();
            st.on_choice = Some(Rc::new(on_choice));
            st.hovered = DialogHit::None;
        }

        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = dpi as f32 / 96.0;
        let dlg_w = (470.0 * scale).round() as i32;
        let dlg_h = (170.0 * scale).round() as i32;

        let st = self.state.borrow();
        let mut parent_wr = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(st.parent_hwnd, &mut parent_wr);
        }
        let pw = parent_wr.right - parent_wr.left;
        let ph = parent_wr.bottom - parent_wr.top;

        let final_x = parent_wr.left + (pw - dlg_w) / 2;
        let final_y = parent_wr.top + (ph - dlg_h) / 3;

        let parent = st.parent_hwnd;
        unsafe {
            let _ = EnableWindow(parent, false);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                final_x,
                final_y,
                dlg_w,
                dlg_h,
                SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        self.render();
    }

    pub fn hide(&self) {
        let parent = self.state.borrow().parent_hwnd;
        unsafe {
            let _ = EnableWindow(parent, true);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = SetForegroundWindow(parent);
            let _ = SetFocus(Some(parent));
        }
    }

    pub fn render(&self) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = dpi as f32 / 96.0;
        let dlg_w = (470.0 * scale).round() as i32;
        let dlg_h = (170.0 * scale).round() as i32;

        let st = self.state.borrow();
        let l = compute_dialog_layout(dlg_w, dlg_h, scale);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: dlg_w,
                biHeight: -dlg_h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        unsafe {
            let screen_dc = windows::Win32::Graphics::Gdi::GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(
                Some(screen_dc),
                &mut bmi,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            ).unwrap();
            let old_bmp = SelectObject(mem_dc, dib.into());
            SetBkMode(mem_dc, TRANSPARENT);

            let cv = Canvas {
                dc: mem_dc,
                bits: bits as *mut u32,
                w: dlg_w,
                h: dlg_h,
                clip: std::cell::Cell::new(rc(0, 0, dlg_w, dlg_h)),
            };

            draw_dialog_gdi(&cv, mem_dc, &l, &st, scale);

            let px = |val: f32| (val * scale).round() as i32;
            finalize_alpha(
                bits as *mut u32,
                dlg_w,
                dlg_h,
                &l.dialog_rect,
                px(12.0) as f32,
                4.0 * scale,
                14.0 * scale,
                0.40,
            );
            let _ = GdiFlush();

            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            let pt_src = POINT { x: 0, y: 0 };
            let size = SIZE { cx: dlg_w, cy: dlg_h };
            let mut wr = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut wr);
            let pt_dst = POINT { x: wr.left, y: wr.top };

            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen_dc),
                Some(&pt_dst),
                Some(&size),
                Some(mem_dc),
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );

            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem_dc);
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, screen_dc);
        }
    }
}

struct DialogLayout {
    dialog_rect: RECT,
    #[allow(dead_code)]
    header_rect: RECT,
    header_icon: RECT,
    header_text: RECT,
    close_btn: RECT,
    qmark_circle_center: POINT,
    qmark_circle_radius: i32,
    msg_rect: RECT,
    remove_btn: RECT,
    cancel_btn: RECT,
}

fn compute_dialog_layout(w: i32, h: i32, scale: f32) -> DialogLayout {
    let px = |val: f32| (val * scale).round() as i32;

    let dialog_rect = rc(0, 0, w, h);
    let header_h = px(36.0);
    let header_rect = rc(0, 0, w, header_h);

    let h_icon_sz = px(16.0);
    let header_icon = rc(px(14.0), (header_h - h_icon_sz) / 2, px(14.0) + h_icon_sz, (header_h - h_icon_sz) / 2 + h_icon_sz);

    let close_sz = px(28.0);
    let close_btn = rc(w - px(10.0) - close_sz, (header_h - close_sz) / 2, w - px(10.0), (header_h - close_sz) / 2 + close_sz);
    let header_text = rc(header_icon.right + px(8.0), 0, close_btn.left - px(6.0), header_h);

    let qmark_r = px(17.0);
    let qmark_circle_center = POINT { x: px(40.0), y: px(82.0) };

    let msg_rect = rc(px(72.0), px(54.0), w - px(20.0), px(112.0));

    let btn_h = px(32.0);
    let btn_y = h - px(14.0) - btn_h;

    let can_w = px(80.0);
    let cancel_btn = rc(w - px(16.0) - can_w, btn_y, w - px(16.0), btn_y + btn_h);

    let rem_w = px(84.0);
    let remove_btn = rc(cancel_btn.left - px(10.0) - rem_w, btn_y, cancel_btn.left - px(10.0), btn_y + btn_h);

    DialogLayout {
        dialog_rect,
        header_rect,
        header_icon,
        header_text,
        close_btn,
        qmark_circle_center,
        qmark_circle_radius: qmark_r,
        msg_rect,
        remove_btn,
        cancel_btn,
    }
}

unsafe fn draw_dialog_gdi(
    cv: &Canvas,
    mem_dc: windows::Win32::Graphics::Gdi::HDC,
    l: &DialogLayout,
    st: &ClearDataDialogState,
    scale: f32,
) {
    let px = |val: f32| (val * scale).round() as i32;

    // Background: Dark styled modal surface matching screenshot
    cv.fill_round(&l.dialog_rect, px(12.0), rgb(24, 25, 29), 0.98);
    cv.stroke_round(&l.dialog_rect, px(12.0), rgb(255, 255, 255), 0.14, 1.0);

    let f_title = make_font(w!("Segoe UI"), px(13.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);
    let f_body = make_font(w!("Segoe UI"), px(13.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let f_qmark = make_font(w!("Segoe UI"), px(18.0), FW_BOLD.0 as i32, CLEARTYPE_QUALITY);
    let f_btn = make_font(w!("Segoe UI"), px(12.5), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY);

    // ── 1. Header ──
    draw_svg_icon(cv, &l.header_icon, SVG_COOKIE_SWIRL, "#d1d5db");

    SelectObject(mem_dc, f_title.into());
    text(mem_dc, "Removing Cookies and Site Data", l.header_text,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(243, 244, 246));

    // Close button (X)
    let is_close_hov = st.hovered == DialogHit::CloseBtn;
    if is_close_hov {
        cv.fill_round(&l.close_btn, px(6.0), rgb(255, 255, 255), 0.12);
    }
    let x_icon_pad = px(7.0);
    let x_icon_rc = rc(l.close_btn.left + x_icon_pad, l.close_btn.top + x_icon_pad, l.close_btn.right - x_icon_pad, l.close_btn.bottom - x_icon_pad);
    draw_svg_icon(cv, &x_icon_rc, SVG_CLOSE_X, if is_close_hov { "#ffffff" } else { "#959bb0" });

    // ── 2. Question Mark Badge (?) ──
    let cx = l.qmark_circle_center.x;
    let cy = l.qmark_circle_center.y;
    let cr = l.qmark_circle_radius;
    let circle_rc = rc(cx - cr, cy - cr, cx + cr, cy + cr);

    cv.fill_round(&circle_rc, cr, rgb(38, 40, 48), 1.0);
    cv.stroke_round(&circle_rc, cr, rgb(160, 165, 175), 0.70, 2.0);

    SelectObject(mem_dc, f_qmark.into());
    text(mem_dc, "?", circle_rc,
         DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(243, 244, 246));

    // ── 3. Dialog Message ──
    SelectObject(mem_dc, f_body.into());
    let prompt_msg = format!(
        "Removing cookies and site data may log you out of websites. Are you sure you want to remove cookies and site data for {}?",
        st.domain
    );
    text(mem_dc, &prompt_msg, l.msg_rect,
         DT_LEFT.0 | DT_WORDBREAK.0 | DT_NOPREFIX.0,
         rgb(230, 235, 245));

    // ── 4. Action Buttons (Pale Red Remove & Transparent Gray-Bordered Cancel) ──
    // Remove Button (Pale red matching user prompt)
    let is_rem_hov = st.hovered == DialogHit::RemoveBtn;
    let pale_red = rgb(185, 45, 45);
    cv.fill_round(&l.remove_btn, px(6.0), pale_red, if is_rem_hov { 1.0 } else { 0.88 });
    cv.stroke_round(&l.remove_btn, px(6.0), rgb(248, 113, 113), 0.35, 1.0);

    SelectObject(mem_dc, f_btn.into());
    text(mem_dc, "Remove", l.remove_btn,
         DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(255, 255, 255));

    // Cancel Button (Transparent bg, gray border)
    let is_can_hov = st.hovered == DialogHit::CancelBtn;
    if is_can_hov {
        cv.fill_round(&l.cancel_btn, px(6.0), rgb(255, 255, 255), 0.10);
    }
    cv.stroke_round(&l.cancel_btn, px(6.0), rgb(160, 165, 175), if is_can_hov { 0.65 } else { 0.40 }, 1.0);

    text(mem_dc, "Cancel", l.cancel_btn,
         DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         if is_can_hov { rgb(255, 255, 255) } else { rgb(243, 244, 246) });
}

fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn hit_test_dialog(x: i32, y: i32, l: &DialogLayout) -> DialogHit {
    if contains(&l.close_btn, x, y) { return DialogHit::CloseBtn; }
    if contains(&l.remove_btn, x, y) { return DialogHit::RemoveBtn; }
    if contains(&l.cancel_btn, x, y) { return DialogHit::CancelBtn; }
    DialogHit::None
}

unsafe extern "system" fn dialog_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !cs.is_null() && !(*cs).lpCreateParams.is_null() {
                let state_ptr = (*cs).lpCreateParams as *const RefCell<ClearDataDialogState>;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ClearDataDialogState>;
            if !ptr.is_null() {
                let parent = (*ptr).borrow().parent_hwnd;
                unsafe {
                    let _ = EnableWindow(parent, true);
                }
            }
            LRESULT(0)
        },
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ClearDataDialogState>;
            if !ptr.is_null() {
                let st = (*ptr).borrow();
                if st.hovered != DialogHit::None {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    return LRESULT(1);
                }
            }
            SetCursor(LoadCursorW(None, IDC_ARROW).ok());
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ClearDataDialogState>;
            if !ptr.is_null() {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let mut wr = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut wr);
                let w = wr.right - wr.left;
                let h = wr.bottom - wr.top;

                let (l, old_hit) = {
                    let st = (*ptr).borrow();
                    (compute_dialog_layout(w, h, scale), st.hovered)
                };

                let hit = hit_test_dialog(x, y, &l);
                if hit != old_hit {
                    (*ptr).borrow_mut().hovered = hit;
                    let dlg = ClearDataDialog { hwnd, state: Rc::from_raw(ptr) };
                    dlg.render();
                    let _ = Rc::into_raw(dlg.state);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ClearDataDialogState>;
            if !ptr.is_null() {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let mut wr = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut wr);
                let w = wr.right - wr.left;
                let h = wr.bottom - wr.top;

                let st = (*ptr).borrow();
                let l = compute_dialog_layout(w, h, scale);
                let hit = hit_test_dialog(x, y, &l);
                let parent = st.parent_hwnd;
                let cb_opt = st.on_choice.clone();
                drop(st);

                match hit {
                    DialogHit::RemoveBtn => {
                        unsafe {
                            let _ = EnableWindow(parent, true);
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            let _ = SetForegroundWindow(parent);
                            let _ = SetFocus(Some(parent));
                        }
                        if let Some(cb) = cb_opt {
                            cb(true);
                        }
                    }
                    DialogHit::CancelBtn | DialogHit::CloseBtn => {
                        unsafe {
                            let _ = EnableWindow(parent, true);
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            let _ = SetForegroundWindow(parent);
                            let _ = SetFocus(Some(parent));
                        }
                        if let Some(cb) = cb_opt {
                            cb(false);
                        }
                    }
                    DialogHit::None => {}
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ClearDataDialogState>;
            if wparam.0 == 0x0D { // VK_RETURN -> Remove
                if !ptr.is_null() {
                    let (parent, cb_opt) = {
                        let st = (*ptr).borrow();
                        (st.parent_hwnd, st.on_choice.clone())
                    };
                    unsafe {
                        let _ = EnableWindow(parent, true);
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        let _ = SetForegroundWindow(parent);
                        let _ = SetFocus(Some(parent));
                    }
                    if let Some(cb) = cb_opt {
                        cb(true);
                    }
                }
                return LRESULT(0);
            }
            if wparam.0 == 0x1B { // VK_ESCAPE -> Cancel
                if !ptr.is_null() {
                    let (parent, cb_opt) = {
                        let st = (*ptr).borrow();
                        (st.parent_hwnd, st.on_choice.clone())
                    };
                    unsafe {
                        let _ = EnableWindow(parent, true);
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        let _ = SetForegroundWindow(parent);
                        let _ = SetFocus(Some(parent));
                    }
                    if let Some(cb) = cb_opt {
                        cb(false);
                    }
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KILLFOCUS => {
            if unsafe { IsWindowVisible(hwnd).as_bool() } {
                unsafe {
                    let _ = MessageBeep(0);
                    let _ = FlashWindow(hwnd, true);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(Some(hwnd));
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
