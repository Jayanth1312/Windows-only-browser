use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, SelectObject, SetBkMode, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
    FW_NORMAL, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, GetWindowLongPtrW,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWLP_USERDATA,
    IDC_ARROW, LoadCursorW, SM_CXSCREEN, SM_CYSCREEN, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    ULW_ALPHA, WM_ACTIVATE, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
    WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::command_bar::finalize_alpha;
use crate::sidebar::{make_font, rc, rgb, text, Canvas};

#[derive(Debug, Clone)]
pub struct ContextMenuItem {
    pub id: u32,
    pub text: String,
    pub shortcut: Option<String>,
    pub is_separator: bool,
    pub disabled: bool,
}

impl ContextMenuItem {
    pub fn item(id: u32, text: &str, shortcut: Option<&str>) -> Self {
        Self {
            id,
            text: text.to_string(),
            shortcut: shortcut.map(|s| s.to_string()),
            is_separator: false,
            disabled: false,
        }
    }

    pub fn item_disabled(id: u32, text: &str, shortcut: Option<&str>) -> Self {
        Self {
            id,
            text: text.to_string(),
            shortcut: shortcut.map(|s| s.to_string()),
            is_separator: false,
            disabled: true,
        }
    }

    pub fn separator() -> Self {
        Self {
            id: 0,
            text: String::new(),
            shortcut: None,
            is_separator: true,
            disabled: false,
        }
    }
}

pub struct ContextMenuState {
    pub items: Vec<ContextMenuItem>,
    pub hovered: Option<usize>,
    pub on_select: Option<Box<dyn Fn(u32)>>,
    pub main_hwnd: HWND,
}

pub struct ContextMenu {
    pub hwnd: HWND,
    pub state: Rc<RefCell<ContextMenuState>>,
}

impl ContextMenu {
    pub fn new(owner: HWND) -> Result<Self, windows::core::Error> {
        let class_name = w!("ZenContextMenuWindow");
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };

        let wnd_class = WNDCLASSW {
            style: windows::Win32::UI::WindowsAndMessaging::WNDCLASS_STYLES(0),
            lpfnWndProc: Some(menu_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: windows::Win32::UI::WindowsAndMessaging::HICON::default(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };

        unsafe {
            let _ = RegisterClassW(&wnd_class);
        }

        let state = Rc::new(RefCell::new(ContextMenuState {
            items: Vec::new(),
            hovered: None,
            on_select: None,
            main_hwnd: owner,
        }));

        let raw_state = Rc::into_raw(state.clone()) as *mut _;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Context Menu"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                300,
                200,
                Some(owner),
                None,
                Some(hinstance.into()),
                Some(raw_state),
            )?
        };

        Ok(Self { hwnd, state })
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn show(
        &self,
        screen_x: i32,
        screen_y: i32,
        items: Vec<ContextMenuItem>,
        on_select: Box<dyn Fn(u32)>,
    ) {
        {
            let mut s = self.state.borrow_mut();
            s.items = items;
            s.hovered = None;
            s.on_select = Some(on_select);
        }

        let s = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

        let card_w = px(240.0);
        let pad_y = px(6.0);
        let row_h = px(32.0);
        let sep_h = px(9.0);

        let mut card_h = pad_y * 2;
        for it in &self.state.borrow().items {
            if it.is_separator {
                card_h += sep_h;
            } else {
                card_h += row_h;
            }
        }

        let total_w = card_w;
        let total_h = card_h;

        // Ensure menu fits on screen
        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

        let mut pos_x = screen_x;
        let mut pos_y = screen_y;

        if pos_x + total_w > screen_w {
            pos_x = (screen_w - total_w).max(0);
        }
        if pos_y + total_h > screen_h {
            pos_y = (screen_h - total_h).max(0);
        }

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                pos_x,
                pos_y,
                total_w,
                total_h,
                SWP_SHOWWINDOW,
            );
            repaint(self.hwnd, &self.state);
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
    }
}

unsafe fn item_at(hwnd: HWND, state: &RefCell<ContextMenuState>, x: i32, y: i32) -> Option<usize> {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;

    let card_w = px(240.0);
    let pad_y = px(6.0);
    let row_h = px(32.0);
    let sep_h = px(9.0);

    let card = rc(0, 0, card_w, 2000);
    if x < card.left || x >= card.right {
        return None;
    }

    let mut cur_y = pad_y;
    let st = state.borrow();
    for (i, it) in st.items.iter().enumerate() {
        if it.is_separator {
            if y >= cur_y && y < cur_y + sep_h {
                return None;
            }
            cur_y += sep_h;
        } else if it.disabled {
            if y >= cur_y && y < cur_y + row_h {
                return None;
            }
            cur_y += row_h;
        } else {
            if y >= cur_y && y < cur_y + row_h {
                return Some(i);
            }
            cur_y += row_h;
        }
    }
    None
}

unsafe fn repaint(hwnd: HWND, state: &RefCell<ContextMenuState>) {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;

    let card_w = px(240.0);
    let pad_y = px(6.0);
    let row_h = px(32.0);
    let sep_h = px(9.0);

    let items = state.borrow().items.clone();
    let hovered = state.borrow().hovered;

    let mut card_h = pad_y * 2;
    for it in &items {
        if it.is_separator {
            card_h += sep_h;
        } else {
            card_h += row_h;
        }
    }

    let total_w = card_w;
    let total_h = card_h;
    let card = rc(0, 0, card_w, card_h);

    let hdc_screen = windows::Win32::Graphics::Gdi::GetDC(None);
    let mem_dc = CreateCompatibleDC(Some(hdc_screen));
    windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: total_w,
            biHeight: -total_h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let mem_bmp = CreateDIBSection(
        Some(mem_dc),
        &bmi,
        DIB_RGB_COLORS,
        &mut bits,
        None,
        0,
    ).unwrap();

    let old_bmp = SelectObject(mem_dc, mem_bmp.into());
    std::ptr::write_bytes(bits as *mut u8, 0, (total_w * total_h * 4) as usize);

    // CRITICAL: Set transparent background mode so DrawTextW doesn't fill opaque white blocks!
    let _ = SetBkMode(mem_dc, TRANSPARENT);

    let cv = Canvas {
        dc: mem_dc,
        bits: bits as *mut u32,
        w: total_w,
        h: total_h,
        clip: Cell::new(RECT { left: 0, top: 0, right: total_w, bottom: total_h }),
    };

    // 1. Pure grayish-black background
    cv.fill_round(&card, px(8.0), rgb(28, 28, 28), 1.0);

    // 2. Delicate 1px border
    cv.stroke_round(&card, px(8.0), rgb(255, 255, 255), 0.08, 1.0);

    let text_font = make_font(w!("Segoe UI"), px(13.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let sc_font = make_font(w!("Segoe UI"), px(12.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY);
    let old_font = SelectObject(mem_dc, text_font.into());

    let mut cur_y = card.top + pad_y;
    for (i, it) in items.iter().enumerate() {
        if it.is_separator {
            let mid_y = cur_y + sep_h / 2;
            cv.fill_rect(
                &rc(card.left + px(10.0), mid_y, card.right - px(10.0), mid_y + 1),
                rgb(255, 255, 255),
                0.08,
            );
            cur_y += sep_h;
        } else {
            let is_hov = hovered == Some(i) && !it.disabled;
            let row = rc(card.left + px(5.0), cur_y, card.right - px(5.0), cur_y + row_h);

            if is_hov {
                cv.fill_round(&row, px(6.0), rgb(255, 255, 255), 0.09);
            }

            let text_rect = rc(row.left + px(10.0), cur_y, row.right - px(10.0), cur_y + row_h);
            let text_color = if it.disabled {
                rgb(105, 105, 105)
            } else {
                rgb(240, 240, 240)
            };
            let sc_color = if it.disabled {
                rgb(85, 85, 85)
            } else {
                rgb(145, 145, 145)
            };

            text(
                mem_dc,
                &it.text,
                text_rect,
                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                text_color,
            );

            if let Some(ref sc) = it.shortcut {
                SelectObject(mem_dc, sc_font.into());
                text(
                    mem_dc,
                    sc,
                    text_rect,
                    DT_RIGHT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                    sc_color,
                );
                SelectObject(mem_dc, text_font.into());
            }

            cur_y += row_h;
        }
    }

    let _ = GdiFlush();
    finalize_alpha(
        bits as *mut u32,
        total_w,
        total_h,
        &card,
        px(8.0) as f32,
        0.0,
        0.0,
        0.0,
    );

    let size = SIZE { cx: total_w, cy: total_h };
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
    let _ = DeleteObject(text_font.into());
    let _ = DeleteObject(sc_font.into());
    let _ = SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(mem_bmp.into());
    let _ = DeleteDC(mem_dc);
}

unsafe extern "system" fn menu_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<ContextMenuState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<ContextMenuState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xffff) as u16;
            if active == 0 {
                // Deactivated when user clicks outside
                let _ = ShowWindow(hwnd, SW_HIDE);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KILLFOCUS => LRESULT(0),
        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            if vk == 27 {
                // ESC
                let _ = ShowWindow(hwnd, SW_HIDE);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let hit = item_at(hwnd, state, x, y);
            if hit != state.borrow().hovered {
                state.borrow_mut().hovered = hit;
                repaint(hwnd, state);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let hit = item_at(hwnd, state, x, y);
            let _ = ShowWindow(hwnd, SW_HIDE);

            if let Some(idx) = hit {
                let item = state.borrow().items.get(idx).cloned();
                let cb = state.borrow().on_select.as_ref().map(|f| f.as_ref() as *const dyn Fn(u32));
                if let (Some(item), Some(cb_ptr)) = (item, cb) {
                    (*cb_ptr)(item.id);
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if let Ok(c) = LoadCursorW(None, IDC_ARROW) {
                let _ = SetCursor(Some(c));
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
