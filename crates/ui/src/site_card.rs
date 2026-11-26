use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, GetTextExtentPoint32W, SelectObject, SetBkMode, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWLP_USERDATA,
    IDC_ARROW, IDC_HAND, LoadCursorW, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    ULW_ALPHA, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
    WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::command_bar::finalize_alpha;
use crate::sidebar::{draw_svg_icon, make_font, rc, rgb, text, Canvas};
use crate::window::UiError;

const SVG_SHARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8"/><polyline points="16 6 12 2 8 6"/><line x1="12" y1="2" x2="12" y2="15"/></svg>"##;
const SVG_BOOKMARK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/></svg>"##;
const SVG_BOOKMARK_FILLED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="__COLOR__" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/></svg>"##;
const SVG_ZAP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>"##;
const SVG_SCREENSHOT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z"/><circle cx="12" cy="13" r="3"/></svg>"##;
const SVG_LOCK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"##;
const SVG_WARN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>"##;
const SVG_TRASH: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="__COLOR__" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>"##;

#[derive(Debug, Clone)]
pub struct SiteCardData {
    pub url: String,
    pub host: String,
    pub is_secure: bool,
    pub is_bookmarked: bool,
    pub shield_enabled: bool,
    pub shield_paused_for_site: bool,
    pub trackers_blocked_count: u32,
    pub hidden_elements_count: usize,
    pub is_reader: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteCardHit {
    None,
    Share,
    Bookmark,
    Zap,
    Screenshot,
    RestoreHidden,
    ReaderMode,
    ToggleShield,
    SecurityBadge,
    ClearCookies,
}

#[derive(Debug, Clone)]
pub enum SiteCardAction {
    Share,
    Bookmark,
    LaunchElementZapper,
    Screenshot,
    RestoreHiddenElements,
    ToggleReader,
    ToggleShield,
    ClearSiteData,
    Close,
}

pub struct SiteCardState {
    pub data: SiteCardData,
    pub hovered: SiteCardHit,
    pub on_action: Option<Rc<dyn Fn(SiteCardAction)>>,
    pub main_hwnd: HWND,
    pub last_closed: Cell<Option<Instant>>,
}

pub struct SiteCard {
    pub hwnd: HWND,
    pub state: Rc<RefCell<SiteCardState>>,
}

impl SiteCard {
    pub fn new(owner: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserSiteCardClass");
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(site_card_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(SiteCardState {
            data: SiteCardData {
                url: String::new(),
                host: String::new(),
                is_secure: true,
                is_bookmarked: false,
                shield_enabled: true,
                shield_paused_for_site: false,
                trackers_blocked_count: 0,
                hidden_elements_count: 0,
                is_reader: false,
            },
            hovered: SiteCardHit::None,
            on_action: None,
            main_hwnd: owner,
            last_closed: Cell::new(None),
        }));

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Site Card"),
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

    pub fn is_visible_or_recently_closed(&self) -> bool {
        if self.is_visible() {
            return true;
        }
        if let Some(t) = self.state.borrow().last_closed.get() {
            if t.elapsed().as_millis() < 350 {
                return true;
            }
        }
        false
    }

    pub fn show(
        &self,
        anchor_right_x: i32,
        anchor_y: i32,
        data: SiteCardData,
        on_action: Box<dyn Fn(SiteCardAction)>,
    ) {
        {
            let mut st = self.state.borrow_mut();
            st.data = data;
            st.on_action = Some(Rc::new(on_action));
            st.hovered = SiteCardHit::None;
        }

        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = dpi as f32 / 96.0;
        let card_w = (228.0 * scale).round() as i32;

        let st = self.state.borrow();
        let l = compute_layout(card_w, scale, &st.data);
        drop(st);

        let final_h = l.fitted_h;
        let final_x = anchor_right_x - card_w;
        let final_y = anchor_y;

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                final_x,
                final_y,
                card_w,
                final_h,
                SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        self.render();
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn update_data(&self, data: SiteCardData) {
        self.state.borrow_mut().data = data;
        if self.is_visible() {
            self.render();
        }
    }

    pub fn render(&self) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = dpi as f32 / 96.0;
        let card_w = (228.0 * scale).round() as i32;

        let st = self.state.borrow();
        let l = compute_layout(card_w, scale, &st.data);
        let card_h = l.fitted_h;

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: card_w,
                biHeight: -card_h,
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

            std::ptr::write_bytes(bits as *mut u8, 0, (card_w * card_h * 4) as usize);

            let cv = Canvas {
                dc: mem_dc,
                bits: bits as *mut u32,
                w: card_w,
                h: card_h,
                clip: std::cell::Cell::new(rc(0, 0, card_w, card_h)),
            };

            draw_site_card_gdi(&cv, mem_dc, &l, &st, scale);

            let px = |val: f32| (val * scale).round() as i32;
            finalize_alpha(
                bits as *mut u32,
                card_w,
                card_h,
                &l.card_rect,
                px(12.0) as f32,
                0.0,
                0.0,
                0.0,
            );
            let _ = GdiFlush();

            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            let pt_src = POINT { x: 0, y: 0 };
            let size = SIZE { cx: card_w, cy: card_h };
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

struct SiteCardLayout {
    card_rect: RECT,
    utilities_header: RECT,
    share_btn: RECT,
    bookmark_btn: RECT,
    zap_btn: RECT,
    screenshot_btn: RECT,
    restore_btn: Option<RECT>,
    settings_header: RECT,
    reader_row: RECT,
    reader_toggle: RECT,
    shield_row: RECT,
    shield_toggle: RECT,
    secure_btn: RECT,
    clear_data_btn: RECT,
    fitted_h: i32,
}

fn compute_layout(w: i32, scale: f32, data: &SiteCardData) -> SiteCardLayout {
    let px = |val: f32| (val * scale).round() as i32;
    let pad_x = px(12.0);
    let mut cur_y = px(12.0);

    // ── Utilities Header (Clean, non-bold distinction) ──
    let utilities_header = rc(pad_x, cur_y, w - pad_x, cur_y + px(16.0));
    cur_y += px(16.0) + px(8.0);

    // ── 4 Main Utility Buttons (Share, Bookmark, Zap, Screenshot) ──
    let usable_w = w - pad_x * 2;
    let btn_sz = px(40.0);
    let gap = (usable_w - 4 * btn_sz) / 3;

    let b0_x = pad_x;
    let share_btn = rc(b0_x, cur_y, b0_x + btn_sz, cur_y + btn_sz);
    let b1_x = b0_x + btn_sz + gap;
    let bookmark_btn = rc(b1_x, cur_y, b1_x + btn_sz, cur_y + btn_sz);
    let b2_x = b1_x + btn_sz + gap;
    let zap_btn = rc(b2_x, cur_y, b2_x + btn_sz, cur_y + btn_sz);
    let b3_x = b2_x + btn_sz + gap;
    let screenshot_btn = rc(b3_x, cur_y, b3_x + btn_sz, cur_y + btn_sz);
    cur_y += btn_sz;

    // Optional compact restore button if elements are hidden
    let restore_btn = if data.hidden_elements_count > 0 {
        cur_y += px(6.0);
        let r = rc(pad_x, cur_y, w - pad_x, cur_y + px(22.0));
        cur_y += px(22.0);
        Some(r)
    } else {
        None
    };

    cur_y += px(12.0);

    // ── Settings Header (Clean, non-bold distinction) ──
    let settings_header = rc(pad_x, cur_y, w - pad_x, cur_y + px(16.0));
    cur_y += px(16.0) + px(8.0);

    let tog_w = px(38.0);
    let tog_h = px(22.0);

    // ── Reader Mode Toggle Row ──
    let reader_row_h = px(28.0);
    let reader_row = rc(pad_x, cur_y, w - pad_x, cur_y + reader_row_h);
    let r_tog_y = cur_y + (reader_row_h - tog_h) / 2;
    let reader_toggle = rc(w - pad_x - tog_w, r_tog_y, w - pad_x, r_tog_y + tog_h);
    cur_y += reader_row_h + px(6.0);

    // ── Block Ads & Trackers Toggle Row ──
    let shield_row_h = px(28.0);
    let shield_row = rc(pad_x, cur_y, w - pad_x, cur_y + shield_row_h);
    let tog_y = cur_y + (shield_row_h - tog_h) / 2;
    let shield_toggle = rc(w - pad_x - tog_w, tog_y, w - pad_x, tog_y + tog_h);
    cur_y += shield_row_h + px(12.0);

    // ── Bottom Row: Secure (Left) + Clear Cookies/Data Button (Right) ──
    let bot_h = px(30.0);
    let sec_w = if data.is_secure { px(76.0) } else { px(92.0) };
    let secure_btn = rc(pad_x, cur_y, pad_x + sec_w, cur_y + bot_h);
    let clear_w = px(66.0);
    let clear_data_btn = rc(w - pad_x - clear_w, cur_y, w - pad_x, cur_y + bot_h);
    cur_y += bot_h + px(12.0);

    let fitted_h = cur_y;
    let card_rect = rc(0, 0, w, fitted_h);

    SiteCardLayout {
        card_rect,
        utilities_header,
        share_btn,
        bookmark_btn,
        zap_btn,
        screenshot_btn,
        restore_btn,
        settings_header,
        reader_row,
        reader_toggle,
        shield_row,
        shield_toggle,
        secure_btn,
        clear_data_btn,
        fitted_h,
    }
}

unsafe fn draw_site_card_gdi(
    cv: &Canvas,
    mem_dc: windows::Win32::Graphics::Gdi::HDC,
    l: &SiteCardLayout,
    st: &SiteCardState,
    scale: f32,
) {
    let px = |val: f32| (val * scale).round() as i32;

    // Card background: sleek deep dark surface with subtle border
    cv.fill_round(&l.card_rect, px(12.0), rgb(24, 25, 29), 0.98);
    cv.stroke_round(&l.card_rect, px(12.0), rgb(255, 255, 255), 0.12, 1.0);

    let f_header = unsafe { make_font(w!("Segoe UI"), px(11.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY) };
    let f_btn_label = unsafe { make_font(w!("Segoe UI"), px(10.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY) };
    let f_text = unsafe { make_font(w!("Segoe UI"), px(13.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY) };
    let f_badge = unsafe { make_font(w!("Segoe UI"), px(11.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY) };

    unsafe {
        SetBkMode(mem_dc, TRANSPARENT);
    }

    // ── 1. Utilities Header ──
    unsafe {
        SelectObject(mem_dc, f_header.into());
    }
    text(mem_dc, "Utilities", l.utilities_header,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(148, 155, 175));

    // ── 2. Utility Buttons (Share, Book, Zap, Sc - Squarish, Icons only) ──
    let render_util_btn = |btn_rc: &RECT, hit_id: SiteCardHit, icon_svg: &str, active_color: Option<&str>| {
        let is_hov = st.hovered == hit_id;
        cv.fill_round(btn_rc, px(8.0), rgb(255, 255, 255), if is_hov { 0.12 } else { 0.05 });
        cv.stroke_round(btn_rc, px(8.0), rgb(255, 255, 255), if is_hov { 0.20 } else { 0.08 }, 1.0);

        let icon_sz = px(20.0);
        let icon_x = btn_rc.left + (btn_rc.right - btn_rc.left - icon_sz) / 2;
        let icon_y = btn_rc.top + (btn_rc.bottom - btn_rc.top - icon_sz) / 2;
        let icon_r = rc(icon_x, icon_y, icon_x + icon_sz, icon_y + icon_sz);

        let color_hex = if let Some(c) = active_color {
            c
        } else if is_hov {
            "#ffffff"
        } else {
            "#959bb0"
        };
        draw_svg_icon(cv, &icon_r, icon_svg, color_hex);
    };

    render_util_btn(&l.share_btn, SiteCardHit::Share, SVG_SHARE, None);
    let book_svg = if st.data.is_bookmarked { SVG_BOOKMARK_FILLED } else { SVG_BOOKMARK };
    let book_col = if st.data.is_bookmarked { Some("#38bdf8") } else { None };
    render_util_btn(&l.bookmark_btn, SiteCardHit::Bookmark, book_svg, book_col);
    render_util_btn(&l.zap_btn, SiteCardHit::Zap, SVG_ZAP, None);
    render_util_btn(&l.screenshot_btn, SiteCardHit::Screenshot, SVG_SCREENSHOT, None);

    // Optional restore button
    if let Some(ref r_btn) = l.restore_btn {
        let is_hov = st.hovered == SiteCardHit::RestoreHidden;
        cv.fill_round(r_btn, px(5.0), rgb(251, 191, 36), if is_hov { 0.24 } else { 0.14 });
        cv.stroke_round(r_btn, px(5.0), rgb(251, 191, 36), 0.35, 1.0);
        unsafe {
            SelectObject(mem_dc, f_btn_label.into());
        }
        let msg = format!("Restore {} hidden element(s)", st.data.hidden_elements_count);
        text(mem_dc, &msg, *r_btn,
             DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
             rgb(253, 230, 138));
    }

    // ── 3. Settings Header ──
    unsafe {
        SelectObject(mem_dc, f_header.into());
    }
    text(mem_dc, "Settings", l.settings_header,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(148, 155, 175));

    // ── 4. Reader Mode & Block Ads & Trackers Toggle Rows ──
    unsafe {
        SelectObject(mem_dc, f_text.into());
    }
    let reader_text_r = rc(l.reader_row.left, l.reader_row.top, l.reader_toggle.left - px(8.0), l.reader_row.bottom);
    text(mem_dc, "Reader Mode", reader_text_r,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(243, 244, 246));
    let r_on = st.data.is_reader;
    let r_tog = &l.reader_toggle;
    if r_on {
        cv.fill_round(r_tog, px(11.0), rgb(56, 189, 248), 0.95);
    } else {
        cv.fill_round(r_tog, px(11.0), rgb(255, 255, 255), 0.15);
    }
    let knob_sz = px(16.0);
    let r_knob_pad = (r_tog.bottom - r_tog.top - knob_sz) / 2;
    let r_knob_x = if r_on {
        r_tog.right - r_knob_pad - knob_sz
    } else {
        r_tog.left + r_knob_pad
    };
    let r_knob_rc = rc(r_knob_x, r_tog.top + r_knob_pad, r_knob_x + knob_sz, r_tog.top + r_knob_pad + knob_sz);
    cv.fill_round(&r_knob_rc, px(8.0), rgb(255, 255, 255), 1.0);

    let shield_text_r = rc(l.shield_row.left, l.shield_row.top, l.shield_toggle.left - px(8.0), l.shield_row.bottom);
    text(mem_dc, "Block Ads & Trackers", shield_text_r,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         rgb(243, 244, 246));

    // Toggle switch
    let shield_on = st.data.shield_enabled && !st.data.shield_paused_for_site;
    let tog_r = &l.shield_toggle;
    if shield_on {
        cv.fill_round(tog_r, px(11.0), rgb(56, 189, 248), 0.95);
    } else {
        cv.fill_round(tog_r, px(11.0), rgb(255, 255, 255), 0.15);
    }
    let knob_pad = (tog_r.bottom - tog_r.top - knob_sz) / 2;
    let knob_x = if shield_on {
        tog_r.right - knob_pad - knob_sz
    } else {
        tog_r.left + knob_pad
    };
    let knob_rc = rc(knob_x, tog_r.top + knob_pad, knob_x + knob_sz, tog_r.top + knob_pad + knob_sz);
    cv.fill_round(&knob_rc, px(8.0), rgb(255, 255, 255), 1.0);

    // ── 5. Bottom Row: Secure Button & Clear Cookies Button ──
    // Secure button (Left, tight padding-x, simple lock without green)
    let is_sec_hov = st.hovered == SiteCardHit::SecurityBadge;
    cv.fill_round(&l.secure_btn, px(8.0), rgb(255, 255, 255), if is_sec_hov { 0.12 } else { 0.06 });
    cv.stroke_round(&l.secure_btn, px(8.0), rgb(255, 255, 255), if is_sec_hov { 0.22 } else { 0.10 }, 1.0);

    unsafe {
        SelectObject(mem_dc, f_badge.into());
    }
    let (sec_svg, sec_col, sec_label) = if st.data.is_secure {
        (SVG_LOCK, if is_sec_hov { "#ffffff" } else { "#959bb0" }, "Secure")
    } else {
        (SVG_WARN, "#ef4444", "Not Secure")
    };
    let mut sec_t_sz = SIZE::default();
    let sec_u16: Vec<u16> = sec_label.encode_utf16().collect();
    unsafe {
        let _ = GetTextExtentPoint32W(mem_dc, &sec_u16, &mut sec_t_sz);
    }
    let sec_icon_sz = px(13.0);
    let sec_gap = px(5.0);
    let sec_content_w = sec_icon_sz + sec_gap + sec_t_sz.cx;
    let sec_btn_w = l.secure_btn.right - l.secure_btn.left;
    let sec_pad_x = ((sec_btn_w - sec_content_w) / 2).max(px(4.0));

    let sec_icon_left = l.secure_btn.left + sec_pad_x;
    let sec_icon_top = l.secure_btn.top + (l.secure_btn.bottom - l.secure_btn.top - sec_icon_sz) / 2;
    let sec_icon_r = rc(sec_icon_left, sec_icon_top, sec_icon_left + sec_icon_sz, sec_icon_top + sec_icon_sz);
    draw_svg_icon(cv, &sec_icon_r, sec_svg, sec_col);

    let sec_text_r = rc(sec_icon_r.right + sec_gap, l.secure_btn.top, l.secure_btn.right - sec_pad_x, l.secure_btn.bottom);
    text(mem_dc, sec_label, sec_text_r,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         if st.data.is_secure { rgb(226, 232, 240) } else { rgb(248, 113, 113) });

    // Clear cookies/data button (Right, neutral hover, with symmetrically centered [ 🗑️ Clear ])
    let is_trash_hov = st.hovered == SiteCardHit::ClearCookies;
    cv.fill_round(&l.clear_data_btn, px(8.0), rgb(255, 255, 255), if is_trash_hov { 0.12 } else { 0.06 });
    cv.stroke_round(&l.clear_data_btn, px(8.0), rgb(255, 255, 255), if is_trash_hov { 0.22 } else { 0.10 }, 1.0);

    let trash_label = "Clear";
    let mut trash_t_sz = SIZE::default();
    let trash_u16: Vec<u16> = trash_label.encode_utf16().collect();
    unsafe {
        let _ = GetTextExtentPoint32W(mem_dc, &trash_u16, &mut trash_t_sz);
    }
    let trash_sz = px(13.0);
    let trash_gap = px(5.0);
    let trash_content_w = trash_sz + trash_gap + trash_t_sz.cx;
    let trash_btn_w = l.clear_data_btn.right - l.clear_data_btn.left;
    let trash_pad_x = ((trash_btn_w - trash_content_w) / 2).max(px(4.0));

    let trash_left = l.clear_data_btn.left + trash_pad_x;
    let trash_top = l.clear_data_btn.top + (l.clear_data_btn.bottom - l.clear_data_btn.top - trash_sz) / 2;
    let trash_r = rc(trash_left, trash_top, trash_left + trash_sz, trash_top + trash_sz);
    let trash_col = if is_trash_hov { "#ffffff" } else { "#959bb0" };
    draw_svg_icon(cv, &trash_r, SVG_TRASH, trash_col);

    let trash_text_r = rc(trash_r.right + trash_gap, l.clear_data_btn.top, l.clear_data_btn.right - trash_pad_x, l.clear_data_btn.bottom);
    text(mem_dc, trash_label, trash_text_r,
         DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
         if is_trash_hov { rgb(255, 255, 255) } else { rgb(226, 232, 240) });
}

fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn hit_test_site_card(x: i32, y: i32, l: &SiteCardLayout) -> SiteCardHit {
    if contains(&l.share_btn, x, y) { return SiteCardHit::Share; }
    if contains(&l.bookmark_btn, x, y) { return SiteCardHit::Bookmark; }
    if contains(&l.zap_btn, x, y) { return SiteCardHit::Zap; }
    if contains(&l.screenshot_btn, x, y) { return SiteCardHit::Screenshot; }
    if let Some(ref r_btn) = l.restore_btn {
        if contains(r_btn, x, y) { return SiteCardHit::RestoreHidden; }
    }
    if contains(&l.reader_toggle, x, y) || contains(&l.reader_row, x, y) { return SiteCardHit::ReaderMode; }
    if contains(&l.shield_toggle, x, y) || contains(&l.shield_row, x, y) { return SiteCardHit::ToggleShield; }
    if contains(&l.secure_btn, x, y) { return SiteCardHit::SecurityBadge; }
    if contains(&l.clear_data_btn, x, y) { return SiteCardHit::ClearCookies; }
    SiteCardHit::None
}

unsafe extern "system" fn site_card_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !cs.is_null() && !(*cs).lpCreateParams.is_null() {
                let state_ptr = (*cs).lpCreateParams as *const RefCell<SiteCardState>;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<SiteCardState>;
            if !ptr.is_null() {
                let st = (*ptr).borrow();
                if st.hovered != SiteCardHit::None {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    return LRESULT(1);
                }
            }
            SetCursor(LoadCursorW(None, IDC_ARROW).ok());
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<SiteCardState>;
            if !ptr.is_null() {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let mut wr = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut wr);
                let card_w = wr.right - wr.left;

                let (l, old_hit) = {
                    let st = (*ptr).borrow();
                    (compute_layout(card_w, scale, &st.data), st.hovered)
                };

                let hit = hit_test_site_card(x, y, &l);
                if hit != old_hit {
                    (*ptr).borrow_mut().hovered = hit;
                    let sc = SiteCard { hwnd, state: Rc::from_raw(ptr) };
                    sc.render();
                    let _ = Rc::into_raw(sc.state);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<SiteCardState>;
            if !ptr.is_null() {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let mut wr = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut wr);
                let card_w = wr.right - wr.left;

                let st = (*ptr).borrow();
                let l = compute_layout(card_w, scale, &st.data);
                let hit = hit_test_site_card(x, y, &l);
                let cb_opt = st.on_action.clone();
                drop(st);

                if let Some(cb) = cb_opt {
                    match hit {
                        SiteCardHit::Share => {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            cb(SiteCardAction::Share);
                        }
                        SiteCardHit::Bookmark => {
                            {
                                let mut st_mut = (*ptr).borrow_mut();
                                st_mut.data.is_bookmarked = !st_mut.data.is_bookmarked;
                            }
                            let sc = SiteCard { hwnd, state: Rc::from_raw(ptr) };
                            sc.render();
                            let _ = Rc::into_raw(sc.state);

                            cb(SiteCardAction::Bookmark);
                        }
                        SiteCardHit::Zap => {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            cb(SiteCardAction::LaunchElementZapper);
                        }
                        SiteCardHit::Screenshot => {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            cb(SiteCardAction::Screenshot);
                        }
                        SiteCardHit::RestoreHidden => {
                            cb(SiteCardAction::RestoreHiddenElements);
                        }
                        SiteCardHit::ReaderMode => {
                            {
                                let mut st_mut = (*ptr).borrow_mut();
                                st_mut.data.is_reader = !st_mut.data.is_reader;
                            }
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            cb(SiteCardAction::ToggleReader);
                        }
                        SiteCardHit::ToggleShield => {
                            {
                                let mut st_mut = (*ptr).borrow_mut();
                                st_mut.data.shield_paused_for_site = !st_mut.data.shield_paused_for_site;
                            }
                            let sc = SiteCard { hwnd, state: Rc::from_raw(ptr) };
                            sc.render();
                            let _ = Rc::into_raw(sc.state);

                            cb(SiteCardAction::ToggleShield);
                        }
                        SiteCardHit::SecurityBadge => {
                            // Can show connection status
                        }
                        SiteCardHit::ClearCookies => {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            cb(SiteCardAction::ClearSiteData);
                        }
                        SiteCardHit::None => {}
                    }
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == 0x1B { // VK_ESCAPE
                let _ = ShowWindow(hwnd, SW_HIDE);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KILLFOCUS => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<SiteCardState>;
            if !ptr.is_null() {
                unsafe {
                    (*ptr).borrow().last_closed.set(Some(Instant::now()));
                }
            }
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
