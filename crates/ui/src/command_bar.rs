use std::cell::RefCell;
use std::rc::Rc;
use windows::core::w;
use windows::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW,
    EndPaint, FillRect, InvalidateRect, LineTo, MoveToEx, SelectObject, SetBkMode,
    SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_NORMAL,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_BACK, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, IsWindowVisible,
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, SWP_NOACTIVATE, SW_HIDE, SW_SHOW,
    WM_CHAR, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::enable_modern_frame;

#[derive(Debug, Clone)]
pub enum CommandAction {
    Navigate(String),
    NewTab,
    CloseTab,
    ToggleSidebar,
}

#[derive(Debug, Clone)]
pub struct CommandItem {
    pub title: String,
    pub subtitle: String,
    pub icon_glyph: u16,
    pub action: CommandAction,
}

pub type CommandCallback = Rc<dyn Fn(CommandAction)>;

struct CommandBarState {
    query: String,
    selected_index: usize,
    hovered_index: Option<usize>,
    on_execute: Option<CommandCallback>,
    main_hwnd: HWND,
    width: i32,
    height: i32,
}

pub struct CommandBar {
    hwnd: HWND,
    state: Rc<RefCell<CommandBarState>>,
}

impl CommandBar {
    pub fn new<F>(owner: HWND, on_execute: F) -> Result<Self, windows::core::Error>
    where
        F: Fn(CommandAction) + 'static,
    {
        let class_name = w!("ArcCommandBarWindowClass");
        let hinstance = unsafe { GetModuleHandleW(None)?.into() };

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(cmd_wndproc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(CommandBarState {
            query: String::new(),
            selected_index: 0,
            hovered_index: None,
            on_execute: Some(Rc::new(on_execute)),
            main_hwnd: owner,
            width: 580,
            height: 330,
        }));

        let raw_state = Rc::into_raw(state.clone()) as *mut _;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_name,
                w!("Command Bar"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                580,
                330,
                Some(owner),
                None,
                Some(hinstance),
                Some(raw_state),
            )?
        };

        enable_modern_frame(hwnd);

        Ok(Self { hwnd, state })
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn toggle(&self, content_rect: RECT) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show(content_rect);
        }
    }

    pub fn show(&self, content_rect: RECT) {
        {
            let mut s = self.state.borrow_mut();
            s.query.clear();
            s.selected_index = 0;
            s.hovered_index = None;
        }
        self.recenter(content_rect);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetFocus(Some(self.hwnd));
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn recenter(&self, content_rect: RECT) {
        let mut pt_top_left = POINT {
            x: content_rect.left,
            y: content_rect.top,
        };
        let mut pt_bottom_right = POINT {
            x: content_rect.right,
            y: content_rect.bottom,
        };

        let (owner, w, h) = {
            let s = self.state.borrow();
            let dpi = unsafe { GetDpiForWindow(self.hwnd) };
            let scale = dpi as f32 / 96.0;
            (
                s.main_hwnd,
                (s.width as f32 * scale).round() as i32,
                (s.height as f32 * scale).round() as i32,
            )
        };

        unsafe {
            let _ = ClientToScreen(owner, &mut pt_top_left);
            let _ = ClientToScreen(owner, &mut pt_bottom_right);
        }

        let content_w = pt_bottom_right.x - pt_top_left.x;
        let content_h = pt_bottom_right.y - pt_top_left.y;

        let center_x = pt_top_left.x + (content_w - w) / 2;
        let center_y = pt_top_left.y + (content_h - h) / 2;

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                center_x,
                center_y,
                w,
                h,
                SWP_NOACTIVATE,
            );
        }
    }
}

fn get_action_items(query: &str) -> Vec<CommandItem> {
    let mut items = Vec::new();
    let q = query.trim();

    if !q.is_empty() {
        if q.starts_with("http://") || q.starts_with("https://") || q.contains('.') && !q.contains(' ') {
            let target_url = if q.starts_with("http://") || q.starts_with("https://") {
                q.to_string()
            } else {
                format!("https://{}", q)
            };
            items.push(CommandItem {
                title: format!("Open URL: {}", q),
                subtitle: "Navigate directly".to_string(),
                icon_glyph: 0xE774, // Globe
                action: CommandAction::Navigate(target_url),
            });
        } else {
            items.push(CommandItem {
                title: format!("Search Google for \"{}\"", q),
                subtitle: "Search web".to_string(),
                icon_glyph: 0xE721, // Search
                action: CommandAction::Navigate(format!("https://www.google.com/search?q={}", urlencoding(q))),
            });
        }
    } else {
        items.push(CommandItem {
            title: "Command Bar (Ctrl+T) Actions".to_string(),
            subtitle: "Search or open".to_string(),
            icon_glyph: 0xE756, // Pin/Command
            action: CommandAction::Navigate("https://www.google.com".to_string()),
        });
    }

    items.push(CommandItem {
        title: "Google Search".to_string(),
        subtitle: "https://www.google.com".to_string(),
        icon_glyph: 0xE774,
        action: CommandAction::Navigate("https://www.google.com".to_string()),
    });
    items.push(CommandItem {
        title: "GitHub".to_string(),
        subtitle: "https://github.com".to_string(),
        icon_glyph: 0xEB51,
        action: CommandAction::Navigate("https://github.com".to_string()),
    });
    items.push(CommandItem {
        title: "YouTube".to_string(),
        subtitle: "https://youtube.com".to_string(),
        icon_glyph: 0xE714,
        action: CommandAction::Navigate("https://youtube.com".to_string()),
    });
    items.push(CommandItem {
        title: "New Tab".to_string(),
        subtitle: "Ctrl+T".to_string(),
        icon_glyph: 0xE710,
        action: CommandAction::NewTab,
    });
    items.push(CommandItem {
        title: "Toggle Sidebar".to_string(),
        subtitle: "Ctrl+S".to_string(),
        icon_glyph: 0xE8A0,
        action: CommandAction::ToggleSidebar,
    });

    items
}

fn urlencoding(input: &str) -> String {
    let mut encoded = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            encoded.push(b as char);
        } else if b == b' ' {
            encoded.push('+');
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}

unsafe extern "system" fn cmd_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<CommandBarState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<CommandBarState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_KILLFOCUS => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_CHAR => {
            let c = wparam.0 as u16;
            if c >= 32 {
                if let Some(ch) = char::from_u32(c as u32) {
                    state.borrow_mut().query.push(ch);
                    state.borrow_mut().selected_index = 0;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            let query_len = state.borrow().query.len();
            let items = get_action_items(&state.borrow().query);
            let item_count = items.len();

            match vk {
                x if x == VK_ESCAPE.0 as u32 => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    return LRESULT(0);
                }
                x if x == VK_BACK.0 as u32 => {
                    if query_len > 0 {
                        state.borrow_mut().query.pop();
                        state.borrow_mut().selected_index = 0;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    return LRESULT(0);
                }
                x if x == VK_UP.0 as u32 => {
                    let mut s = state.borrow_mut();
                    if s.selected_index > 0 {
                        s.selected_index -= 1;
                    } else if item_count > 0 {
                        s.selected_index = item_count - 1;
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                x if x == VK_DOWN.0 as u32 => {
                    let mut s = state.borrow_mut();
                    if item_count > 0 {
                        s.selected_index = (s.selected_index + 1) % item_count;
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                x if x == VK_RETURN.0 as u32 => {
                    let idx = state.borrow().selected_index;
                    let chosen = items.get(idx).cloned();
                    let cb = state.borrow().on_execute.clone();
                    let _ = ShowWindow(hwnd, SW_HIDE);

                    if let Some(item) = chosen {
                        if let Some(cb) = cb {
                            cb(item.action);
                        }
                    }
                    return LRESULT(0);
                }
                _ => {}
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;
            let input_h = (54.0 * scale).round() as i32;
            let item_h = (44.0 * scale).round() as i32;

            if y >= input_h {
                let idx = ((y - input_h) / item_h) as usize;
                let items = get_action_items(&state.borrow().query);
                if idx < items.len() {
                    let old = state.borrow().hovered_index;
                    if old != Some(idx) {
                        state.borrow_mut().hovered_index = Some(idx);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            } else if state.borrow().hovered_index.is_some() {
                state.borrow_mut().hovered_index = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;
            let input_h = (54.0 * scale).round() as i32;
            let item_h = (44.0 * scale).round() as i32;

            if y >= input_h {
                let idx = ((y - input_h) / item_h) as usize;
                let items = get_action_items(&state.borrow().query);
                if let Some(item) = items.get(idx).cloned() {
                    let cb = state.borrow().on_execute.clone();
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    if let Some(cb) = cb {
                        cb(item.action);
                    }
                    return LRESULT(0);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;

            let (query, selected_idx, hovered_idx) = {
                let s = state.borrow();
                (s.query.clone(), s.selected_index, s.hovered_index)
            };
            let items = get_action_items(&query);

            // Double buffering
            let mem_dc = CreateCompatibleDC(Some(hdc));
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());

            // 1. Draw card background (Arc sleek dark theme `#1c1c22`)
            let card_bg = COLORREF(0x00221c1c);
            let bg_brush = CreateSolidBrush(card_bg);
            FillRect(mem_dc, &client_rect, bg_brush);
            let _ = DeleteObject(bg_brush.into());

            // Border (subtle dark slate `#32323e`)
            let border_pen = CreatePen(PS_SOLID, 1, COLORREF(0x003e3232));
            let old_pen = SelectObject(mem_dc, border_pen.into());
            let _ = MoveToEx(mem_dc, 0, 0, None);
            let _ = LineTo(mem_dc, width - 1, 0);
            let _ = LineTo(mem_dc, width - 1, height - 1);
            let _ = LineTo(mem_dc, 0, height - 1);
            let _ = LineTo(mem_dc, 0, 0);

            let input_h = (54.0 * scale).round() as i32;

            // 2. Search icon 🔍
            let icon_font = CreateFontW(
                -(14.0 * scale).round() as i32,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe Fluent Icons"),
            );
            let old_font = SelectObject(mem_dc, icon_font.into());
            let _ = SetBkMode(mem_dc, TRANSPARENT);
            let _ = SetTextColor(mem_dc, COLORREF(0x00a0a0a0));
            let mut search_glyph = vec![0xE721, 0];
            let mut search_icon_rect = RECT {
                left: (16.0 * scale).round() as i32,
                top: 0,
                right: (44.0 * scale).round() as i32,
                bottom: input_h,
            };
            DrawTextW(
                mem_dc,
                &mut search_glyph,
                &mut search_icon_rect,
                DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0),
            );

            // 3. Search query / placeholder
            let text_font = CreateFontW(
                -(14.0 * scale).round() as i32,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let _ = SelectObject(mem_dc, text_font.into());

            let mut input_text_rect = RECT {
                left: (46.0 * scale).round() as i32,
                top: 0,
                right: width - 20,
                bottom: input_h,
            };

            if query.is_empty() {
                let _ = SetTextColor(mem_dc, COLORREF(0x00787880)); // Muted placeholder
                let mut placeholder: Vec<u16> = "Search or Enter URL...".encode_utf16().collect();
                placeholder.push(0);
                DrawTextW(
                    mem_dc,
                    &mut placeholder,
                    &mut input_text_rect,
                    DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0),
                );
            } else {
                let _ = SetTextColor(mem_dc, COLORREF(0x00ffffff)); // Crisp white text
                let mut display_query: Vec<u16> = query.encode_utf16().collect();
                display_query.push(0);
                DrawTextW(
                    mem_dc,
                    &mut display_query,
                    &mut input_text_rect,
                    DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0),
                );
            }

            // Separator line between search box and list
            let _ = MoveToEx(mem_dc, 12, input_h, None);
            let _ = LineTo(mem_dc, width - 12, input_h);

            // 4. Action items list
            let item_h = (44.0 * scale).round() as i32;
            let pad_x = (10.0 * scale).round() as i32;

            for (i, item) in items.iter().enumerate() {
                let item_top = input_h + (i as i32 * item_h) + (4.0 * scale).round() as i32;
                if item_top + item_h > height {
                    break;
                }

                let item_rect = RECT {
                    left: pad_x,
                    top: item_top,
                    right: width - pad_x,
                    bottom: item_top + item_h,
                };

                let is_selected = i == selected_idx;
                let is_hovered = hovered_idx == Some(i);

                if is_selected {
                    // Arc teal accent badge `#008299`
                    let sel_brush = CreateSolidBrush(COLORREF(0x00998200));
                    FillRect(mem_dc, &item_rect, sel_brush);
                    let _ = DeleteObject(sel_brush.into());
                } else if is_hovered {
                    let hov_brush = CreateSolidBrush(COLORREF(0x002c2626));
                    FillRect(mem_dc, &item_rect, hov_brush);
                    let _ = DeleteObject(hov_brush.into());
                }

                // Item icon
                let _ = SelectObject(mem_dc, icon_font.into());
                let icon_color = if is_selected {
                    COLORREF(0x00ffffff)
                } else {
                    COLORREF(0x0038bdf8) // Sky blue icon
                };
                let _ = SetTextColor(mem_dc, icon_color);

                let mut glyph = vec![item.icon_glyph, 0];
                let mut icon_rect = RECT {
                    left: item_rect.left + (10.0 * scale).round() as i32,
                    top: item_rect.top,
                    right: item_rect.left + (34.0 * scale).round() as i32,
                    bottom: item_rect.bottom,
                };
                DrawTextW(
                    mem_dc,
                    &mut glyph,
                    &mut icon_rect,
                    DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0),
                );

                // Item title
                let _ = SelectObject(mem_dc, text_font.into());
                let title_color = if is_selected {
                    COLORREF(0x00ffffff)
                } else {
                    COLORREF(0x00e0e0e0)
                };
                let _ = SetTextColor(mem_dc, title_color);

                let mut title_u16: Vec<u16> = item.title.encode_utf16().collect();
                title_u16.push(0);
                let mut title_rect = RECT {
                    left: item_rect.left + (40.0 * scale).round() as i32,
                    top: item_rect.top,
                    right: item_rect.right - (80.0 * scale).round() as i32,
                    bottom: item_rect.bottom,
                };
                DrawTextW(
                    mem_dc,
                    &mut title_u16,
                    &mut title_rect,
                    DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0),
                );

                // Right arrow → for active or subtitle
                if is_selected {
                    let _ = SelectObject(mem_dc, icon_font.into());
                    let _ = SetTextColor(mem_dc, COLORREF(0x00ffffff));
                    let mut arrow_glyph = vec![0xE72A, 0]; // Right arrow
                    let mut arrow_rect = RECT {
                        left: item_rect.right - (36.0 * scale).round() as i32,
                        top: item_rect.top,
                        right: item_rect.right - (12.0 * scale).round() as i32,
                        bottom: item_rect.bottom,
                    };
                    DrawTextW(
                        mem_dc,
                        &mut arrow_glyph,
                        &mut arrow_rect,
                        DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0),
                    );
                }
            }

            // Clean up fonts and pens
            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(icon_font.into());
            let _ = DeleteObject(text_font.into());

            let _ = SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(border_pen.into());

            // Blit to screen
            let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);

            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
