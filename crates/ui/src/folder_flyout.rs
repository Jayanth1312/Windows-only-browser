use std::cell::RefCell;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, SelectObject, SetBkMode, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, GetWindowLongPtrW,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWLP_USERDATA,
    IDC_ARROW, LoadCursorW, SM_CXSCREEN, SM_CYSCREEN, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    ULW_ALPHA, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_CHAR,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
    WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::command_bar::finalize_alpha;
use crate::sidebar::{domain_of, make_font, rc, rgb, text, Canvas, FaviconData};
use crate::window::UiError;

#[derive(Debug, Clone)]
pub struct FlyoutItem {
    pub tab_id: String,
    pub title: String,
    pub url: String,
    pub favicon: Option<std::sync::Arc<FaviconData>>,
}

pub struct FolderFlyoutState {
    pub folder_id: String,
    pub folder_name: String,
    pub all_items: Vec<FlyoutItem>,
    pub search_query: String,
    pub hovered_idx: Option<usize>, // None, Some(0..filtered.len()) or Some(9999) for "+ New Tab"
    pub on_select_tab: Option<Rc<dyn Fn(String)>>,
    pub on_new_tab: Option<Rc<dyn Fn(String)>>,
    pub main_hwnd: HWND,
}

pub struct FolderFlyout {
    pub hwnd: HWND,
    pub state: Rc<RefCell<FolderFlyoutState>>,
}

impl FolderFlyout {
    pub fn new(owner: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserFolderFlyoutClass");
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(folder_flyout_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(FolderFlyoutState {
            folder_id: String::new(),
            folder_name: String::new(),
            all_items: Vec::new(),
            search_query: String::new(),
            hovered_idx: None,
            on_select_tab: None,
            on_new_tab: None,
            main_hwnd: owner,
        }));

        let raw_state = Rc::into_raw(state.clone()) as *mut _;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Folder Flyout"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                260,
                320,
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

    pub fn current_folder_id(&self) -> String {
        self.state.borrow().folder_id.clone()
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
        folder_id: &str,
        folder_name: &str,
        items: Vec<FlyoutItem>,
        on_select_tab: Box<dyn Fn(String)>,
        on_new_tab: Box<dyn Fn(String)>,
    ) {
        {
            let mut s = self.state.borrow_mut();
            s.folder_id = folder_id.to_string();
            s.folder_name = folder_name.to_string();
            s.all_items = items;
            s.search_query.clear();
            s.hovered_idx = None;
            s.on_select_tab = Some(Rc::new(on_select_tab));
            s.on_new_tab = Some(Rc::new(on_new_tab));
        }

        self.relayout_and_render(screen_x, screen_y);
    }

    pub fn relayout_and_render(&self, screen_x: i32, screen_y: i32) {
        let s = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

        let filtered_count = {
            let st = self.state.borrow();
            get_filtered_items(&st.all_items, &st.search_query).len()
        };

        let card_w = px(250.0);
        let header_h = px(44.0);
        let pad = px(8.0);
        let row_h = px(42.0);
        let new_tab_h = px(36.0);

        let total_rows = filtered_count.max(1) as i32;
        let card_h = (header_h + pad + total_rows * row_h + new_tab_h + pad).clamp(px(140.0), px(460.0));

        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

        let mut final_x = screen_x;
        let mut final_y = screen_y;

        if final_x + card_w > screen_w - px(10.0) {
            final_x = screen_w - card_w - px(10.0);
        }
        if final_x < px(10.0) {
            final_x = px(10.0);
        }
        if final_y + card_h > screen_h - px(10.0) {
            final_y = screen_h - card_h - px(10.0);
        }
        if final_y < px(10.0) {
            final_y = px(10.0);
        }

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                final_x,
                final_y,
                card_w,
                card_h,
                SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }

        render_flyout(self.hwnd, &self.state.borrow());
    }
}

fn get_filtered_items<'a>(items: &'a [FlyoutItem], query: &str) -> Vec<&'a FlyoutItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return items.iter().collect();
    }
    items
        .iter()
        .filter(|it| it.title.to_lowercase().contains(&q) || it.url.to_lowercase().contains(&q))
        .collect()
}

fn hit_test(x: i32, y: i32, w: i32, _h: i32, s: f32, filtered_count: usize) -> Option<usize> {
    let px = |v: f32| (v * s).round() as i32;
    let header_h = px(44.0);
    let pad = px(8.0);
    let row_h = px(42.0);
    let new_tab_h = px(36.0);

    let list_top = header_h + pad;
    let list_bottom = list_top + (filtered_count as i32) * row_h;

    if y >= list_top && y < list_bottom {
        let idx = ((y - list_top) / row_h) as usize;
        if idx < filtered_count {
            return Some(idx);
        }
    }

    let new_tab_top = list_bottom + px(4.0);
    if y >= new_tab_top && y < new_tab_top + new_tab_h && x >= pad && x < w - pad {
        return Some(9999);
    }

    None
}

fn render_flyout(hwnd: HWND, state: &FolderFlyoutState) {
    let mut rect = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
    }
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);

    let s = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;

    let hdc_screen = unsafe { windows::Win32::Graphics::Gdi::GetDC(None) };
    let mem_dc = unsafe { CreateCompatibleDC(Some(hdc_screen)) };

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

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let dib = unsafe {
        CreateDIBSection(
            Some(mem_dc),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
        .unwrap()
    };
    let old_bmp = unsafe { SelectObject(mem_dc, dib.into()) };

    let cv = Canvas {
        dc: mem_dc,
        bits: bits as *mut u32,
        w,
        h,
        clip: std::cell::Cell::new(rc(0, 0, w, h)),
    };

    // Dark sleek card with rounded corners
    let card_rect = rc(0, 0, w, h);
    let radius = px(8.0);
    let bg_color = rgb(28, 28, 28);
    cv.fill_round(&card_rect, radius, bg_color, 1.0);
    cv.stroke_round(&card_rect, radius, rgb(255, 255, 255), 0.08, 1.0);

    let pad = px(8.0);
    let header_h = px(44.0);
    let row_h = px(42.0);

    unsafe {
        SetBkMode(mem_dc, TRANSPARENT);
    }

    // Search bar at top
    let search_rect = rc(pad, pad, w - pad, header_h);
    cv.fill_round(&search_rect, px(6.0), rgb(255, 255, 255), 0.05);
    cv.stroke_round(&search_rect, px(6.0), rgb(255, 255, 255), 0.08, 1.0);

    let font_search = unsafe { make_font(w!("Segoe UI"), px(12.0), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY) };
    let old_font = unsafe { SelectObject(mem_dc, font_search.into()) };

    // Search icon
    let icon_rect = rc(search_rect.left + px(8.0), search_rect.top, search_rect.left + px(26.0), search_rect.bottom);
    unsafe {
        text(
            mem_dc,
            "🔍",
            icon_rect,
            DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
            rgb(145, 145, 145),
        );
    }

    let text_rect = rc(icon_rect.right + px(4.0), search_rect.top, search_rect.right - px(8.0), search_rect.bottom);
    unsafe {
        if state.search_query.is_empty() {
            let placeholder = format!("Search {}...", state.folder_name);
            text(
                mem_dc,
                &placeholder,
                text_rect,
                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0 | DT_END_ELLIPSIS.0,
                rgb(130, 130, 130),
            );
        } else {
            text(
                mem_dc,
                &state.search_query,
                text_rect,
                DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                rgb(240, 240, 240),
            );
        }
    }

    let filtered = get_filtered_items(&state.all_items, &state.search_query);

    let font_title = unsafe { make_font(w!("Segoe UI"), px(12.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY) };
    let font_sub = unsafe { make_font(w!("Segoe UI"), px(10.5), FW_NORMAL.0 as i32, CLEARTYPE_QUALITY) };

    let mut cur_y = header_h + pad;

    if filtered.is_empty() {
        let empty_r = rc(pad, cur_y, w - pad, cur_y + row_h);
        unsafe {
            let _ = SelectObject(mem_dc, font_sub.into());
            text(
                mem_dc,
                "No sites in this folder",
                empty_r,
                DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                rgb(130, 132, 145),
            );
        }
        cur_y += row_h;
    } else {
        for (i, item) in filtered.iter().enumerate() {
            if cur_y + row_h > h - px(44.0) {
                break;
            }
            let row_r = rc(pad, cur_y, w - pad, cur_y + row_h);
            let is_hover = state.hovered_idx == Some(i);

            if is_hover {
                cv.fill_round(&row_r, px(6.0), rgb(255, 255, 255), 0.08);
            }

            // Draw Favicon or badge
            let fav_size = px(20.0);
            let fav_x = row_r.left + px(8.0);
            let fav_y = row_r.top + (row_h - fav_size) / 2;
            let fav_r = rc(fav_x, fav_y, fav_x + fav_size, fav_y + fav_size);

            if let Some(ref fav) = item.favicon {
                crate::sidebar::draw_favicon_to_canvas(&cv, fav, &fav_r);
            } else {
                cv.fill_round(&fav_r, px(4.0), rgb(255, 255, 255), 0.10);
                unsafe {
                    let _ = SelectObject(mem_dc, font_sub.into());
                    text(
                        mem_dc,
                        "🌐",
                        fav_r,
                        DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
                        rgb(200, 200, 210),
                    );
                }
            }

            // Title & Domain
            let text_x = fav_r.right + px(8.0);
            let title_r = rc(text_x, row_r.top + px(4.0), row_r.right - px(8.0), row_r.top + px(22.0));
            let sub_r = rc(text_x, row_r.top + px(21.0), row_r.right - px(8.0), row_r.bottom - px(3.0));

            unsafe {
                let _ = SelectObject(mem_dc, font_title.into());
                text(
                    mem_dc,
                    &item.title,
                    title_r,
                    DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0 | DT_END_ELLIPSIS.0,
                    rgb(235, 235, 240),
                );
            }

            let sub_text = {
                let d = domain_of(&item.url);
                if d.is_empty() {
                    state.folder_name.clone()
                } else {
                    format!("{} • {}", d, state.folder_name)
                }
            };
            unsafe {
                let _ = SelectObject(mem_dc, font_sub.into());
                text(
                    mem_dc,
                    &sub_text,
                    sub_r,
                    DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0 | DT_END_ELLIPSIS.0,
                    rgb(130, 132, 145),
                );
            }

            cur_y += row_h;
        }
    }

    // "+ New Tab in <Folder>" row
    let new_tab_r = rc(pad, cur_y + px(2.0), w - pad, cur_y + px(34.0));
    let is_new_tab_hover = state.hovered_idx == Some(9999);
    if is_new_tab_hover {
        cv.fill_round(&new_tab_r, px(5.0), rgb(255, 255, 255), 0.08);
    }
    let new_tab_label = format!("+ New Tab in {}", state.folder_name);
    unsafe {
        let _ = SelectObject(mem_dc, font_sub.into());
        text(
            mem_dc,
            &new_tab_label,
            new_tab_r,
            DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
            if is_new_tab_hover { rgb(240, 240, 245) } else { rgb(150, 152, 165) },
        );
    }

    unsafe {
        let _ = SelectObject(mem_dc, old_font);
        let _ = DeleteObject(font_search.into());
        let _ = DeleteObject(font_title.into());
        let _ = DeleteObject(font_sub.into());
        let _ = GdiFlush();
    }

    unsafe {
        finalize_alpha(
            bits as *mut u32,
            w,
            h,
            &card_rect,
            radius as f32,
            0.0,
            0.0,
            0.0,
        );
    }

    let mut blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let pt_src = POINT { x: 0, y: 0 };
    let size = SIZE { cx: w, cy: h };

    unsafe {
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(hdc_screen),
            None,
            Some(&size),
            Some(mem_dc),
            Some(&pt_src),
            COLORREF(0),
            Some(&mut blend),
            ULW_ALPHA,
        );
        let _ = SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mem_dc);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
    }
}

unsafe extern "system" fn folder_flyout_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<FolderFlyoutState>;

    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        if !cs.lpCreateParams.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            render_flyout(hwnd, &state.borrow());
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let s = GetDpiForWindow(hwnd) as f32 / 96.0;

            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;

            let filtered_len = {
                let st = state.borrow();
                get_filtered_items(&st.all_items, &st.search_query).len()
            };

            let hit = hit_test(x, y, w, h, s, filtered_len);
            if hit != state.borrow().hovered_idx {
                state.borrow_mut().hovered_idx = hit;
                render_flyout(hwnd, &state.borrow());
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let s = GetDpiForWindow(hwnd) as f32 / 96.0;

            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;

            let (filtered_items, folder_id, select_cb, new_tab_cb) = {
                let st = state.borrow();
                let q = st.search_query.trim().to_lowercase();
                let filtered = st.all_items.iter().filter(|it| {
                    q.is_empty() || it.title.to_lowercase().contains(&q) || it.url.to_lowercase().contains(&q)
                }).cloned().collect::<Vec<_>>();
                (
                    filtered,
                    st.folder_id.clone(),
                    st.on_select_tab.clone(),
                    st.on_new_tab.clone(),
                )
            };

            let hit = hit_test(x, y, w, h, s, filtered_items.len());
            if let Some(h) = hit {
                let _ = ShowWindow(hwnd, SW_HIDE);
                if h == 9999 {
                    if let Some(cb) = new_tab_cb {
                        cb(folder_id);
                    }
                } else if h < filtered_items.len() {
                    if let Some(cb) = select_cb {
                        cb(filtered_items[h].tab_id.clone());
                    }
                }
            }
            LRESULT(0)
        }
        WM_CHAR => {
            let ch = wparam.0 as u16;
            if ch >= 32 {
                if let Some(c) = char::from_u32(ch as u32) {
                    state.borrow_mut().search_query.push(c);
                    state.borrow_mut().hovered_idx = None;
                    render_flyout(hwnd, &state.borrow());
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 as i32 {
                0x08 => {
                    // VK_BACK
                    let mut s = state.borrow_mut();
                    s.search_query.pop();
                    s.hovered_idx = None;
                    render_flyout(hwnd, &s);
                }
                0x1B => {
                    // VK_ESCAPE
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                0x0D => {
                    // VK_RETURN
                    let (first_tab_id, select_cb) = {
                        let st = state.borrow();
                        let q = st.search_query.trim().to_lowercase();
                        let tid = st.all_items.iter().find(|it| {
                            q.is_empty() || it.title.to_lowercase().contains(&q) || it.url.to_lowercase().contains(&q)
                        }).map(|it| it.tab_id.clone());
                        (tid, st.on_select_tab.clone())
                    };
                    if let Some(tid) = first_tab_id {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        if let Some(cb) = select_cb {
                            cb(tid);
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            SetCursor(Some(cursor));
            LRESULT(1)
        }
        WM_DESTROY => {
            let _ = Rc::from_raw(state_ptr);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
