use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, FillRect, LineTo, MoveToEx,
    SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DRAW_TEXT_FORMAT, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    FW_NORMAL, HDC, OUT_DEFAULT_PRECIS, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::IsZoomed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionButton {
    None,
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarClickTarget {
    None,
    Caption(CaptionButton),
    NavBack,
    NavForward,
    NavReload,
    NavNewTab,
    ToggleSidebar,
    SelectTab(usize),
    CloseTab(usize),
}

#[derive(Debug, Clone)]
pub struct TabItemView {
    pub id: String,
    pub title: String,
    pub is_active: bool,
}

pub struct SidebarRenderer {
    bg_color: COLORREF,
    border_color: COLORREF,
    #[allow(dead_code)]
    text_color: COLORREF,
}

impl Default for SidebarRenderer {
    fn default() -> Self {
        Self {
            bg_color: COLORREF(0x001a1616),     // Arc dark slate RGB(22, 22, 26)
            border_color: COLORREF(0x00302a27), // RGB(39, 42, 48)
            text_color: COLORREF(0x00908880),   // RGB(128, 136, 144)
        }
    }
}

impl SidebarRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hit_test(
        &self,
        hwnd: HWND,
        x: i32,
        y: i32,
        client_width: i32,
        sidebar_visible: bool,
        tabs_count: usize,
    ) -> SidebarClickTarget {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = dpi as f32 / 96.0;
        let btn_width = (46.0 * scale).round() as i32;
        let btn_height = (32.0 * scale).round() as i32;

        if !sidebar_visible {
            // When collapsed, check floating top-right buttons
            if y >= 0 && y < btn_height {
                if x >= client_width - btn_width && x < client_width {
                    return SidebarClickTarget::Caption(CaptionButton::Close);
                } else if x >= client_width - (btn_width * 2) && x < client_width - btn_width {
                    return SidebarClickTarget::Caption(CaptionButton::Maximize);
                } else if x >= client_width - (btn_width * 3) && x < client_width - (btn_width * 2) {
                    return SidebarClickTarget::Caption(CaptionButton::Minimize);
                } else if x >= client_width - (btn_width * 4) && x < client_width - (btn_width * 3) {
                    return SidebarClickTarget::ToggleSidebar;
                }
            }
            return SidebarClickTarget::None;
        }

        // Sidebar is visible on the right:
        // 1. Caption buttons at top-right
        if y >= 0 && y < btn_height {
            if x >= client_width - btn_width && x < client_width {
                return SidebarClickTarget::Caption(CaptionButton::Close);
            } else if x >= client_width - (btn_width * 2) && x < client_width - btn_width {
                return SidebarClickTarget::Caption(CaptionButton::Maximize);
            } else if x >= client_width - (btn_width * 3) && x < client_width - (btn_width * 2) {
                return SidebarClickTarget::Caption(CaptionButton::Minimize);
            }
        }

        let nav_y = btn_height + (8.0 * scale).round() as i32;
        let nav_h = (30.0 * scale).round() as i32;
        let tool_w = (30.0 * scale).round() as i32;
        let pad_x = (14.0 * scale).round() as i32;
        let sidebar_w = (260.0 * scale).round() as i32;
        let sidebar_left = client_width - sidebar_w;

        // 2. Navigation toolbar buttons
        if y >= nav_y && y < nav_y + nav_h && x >= sidebar_left {
            let rel_x = x - sidebar_left - pad_x;
            if rel_x >= 0 {
                let btn_idx = rel_x / (tool_w + 4);
                match btn_idx {
                    0 => return SidebarClickTarget::NavBack,
                    1 => return SidebarClickTarget::NavForward,
                    2 => return SidebarClickTarget::NavReload,
                    3 => return SidebarClickTarget::NavNewTab,
                    4 => return SidebarClickTarget::ToggleSidebar,
                    _ => {}
                }
            }
        }

        // 3. Tab list items
        let list_top = nav_y + nav_h + (14.0 * scale).round() as i32;
        let tab_h = (36.0 * scale).round() as i32;
        let tab_pad = (4.0 * scale).round() as i32;

        if y >= list_top && x >= sidebar_left {
            let rel_y = y - list_top;
            let tab_total = tab_h + tab_pad;
            let idx = (rel_y / tab_total) as usize;
            if idx < tabs_count {
                let tab_y_start = list_top + (idx as i32 * tab_total);
                if y >= tab_y_start && y < tab_y_start + tab_h {
                    // Check if clicked the close button on the tab
                    let close_w = (28.0 * scale).round() as i32;
                    let close_right = client_width - (14.0 * scale).round() as i32;
                    let close_left = close_right - close_w;
                    if x >= close_left && x < close_right {
                        return SidebarClickTarget::CloseTab(idx);
                    }
                    return SidebarClickTarget::SelectTab(idx);
                }
            }
        }

        SidebarClickTarget::None
    }

    pub fn get_caption_buttons_rect(&self, hwnd: HWND, client_width: i32, sidebar_visible: bool) -> RECT {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = dpi as f32 / 96.0;
        let btn_width = (46.0 * scale).round() as i32;
        let btn_height = (32.0 * scale).round() as i32;
        let btn_count = if sidebar_visible { 3 } else { 4 };

        RECT {
            left: client_width - (btn_width * btn_count),
            top: 0,
            right: client_width,
            bottom: btn_height,
        }
    }

    pub fn render(
        &self,
        hwnd: HWND,
        hdc: HDC,
        sidebar_rect: RECT,
        sidebar_visible: bool,
        tabs: &[TabItemView],
        hovered_target: &SidebarClickTarget,
        pressed_target: &SidebarClickTarget,
    ) {
        let width = sidebar_rect.right - sidebar_rect.left;
        let height = sidebar_rect.bottom - sidebar_rect.top;
        if width <= 0 || height <= 0 {
            return;
        }

        unsafe {
            // Double buffer memory DC
            let mem_dc = CreateCompatibleDC(Some(hdc));
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());

            // Fill sidebar background
            let bg_brush = CreateSolidBrush(self.bg_color);
            let local_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            FillRect(mem_dc, &local_rect, bg_brush);
            let _ = DeleteObject(bg_brush.into());

            // Draw left separator line (between content and sidebar)
            if sidebar_visible {
                let border_pen = CreatePen(PS_SOLID, 1, self.border_color);
                let old_pen = SelectObject(mem_dc, border_pen.into());
                let _ = MoveToEx(mem_dc, 0, 0, None);
                let _ = LineTo(mem_dc, 0, height);
                let _ = SelectObject(mem_dc, old_pen);
                let _ = DeleteObject(border_pen.into());
            }

            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;
            let btn_width = (46.0 * scale).round() as i32;
            let btn_height = (32.0 * scale).round() as i32;
            let is_maximized = IsZoomed(hwnd).as_bool();

            let hover_color = COLORREF(0x00342d2d); // RGB(45, 45, 52)
            let press_color = COLORREF(0x00463c3c); // RGB(60, 60, 70)
            let close_hover_color = COLORREF(0x001c2bc4); // RGB(196, 43, 28) - Fluent Red
            let close_press_color = COLORREF(0x00121eaf);

            let _ = SetBkMode(mem_dc, TRANSPARENT);

            // Icon font for caption & navigation
            let icon_font = CreateFontW(
                -(10.0 * scale).round() as i32,
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
            let text_format = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0);

            // 1. Draw Caption Controls
            let mut close_rect = RECT {
                left: width - btn_width,
                top: 0,
                right: width,
                bottom: btn_height,
            };
            let mut max_rect = RECT {
                left: width - (btn_width * 2),
                top: 0,
                right: width - btn_width,
                bottom: btn_height,
            };
            let mut min_rect = RECT {
                left: width - (btn_width * 3),
                top: 0,
                right: width - (btn_width * 2),
                bottom: btn_height,
            };

            // Minimize
            let is_min_hov = *hovered_target == SidebarClickTarget::Caption(CaptionButton::Minimize);
            let is_min_pre = *pressed_target == SidebarClickTarget::Caption(CaptionButton::Minimize);
            if is_min_pre {
                let b = CreateSolidBrush(press_color);
                FillRect(mem_dc, &min_rect, b);
                let _ = DeleteObject(b.into());
            } else if is_min_hov {
                let b = CreateSolidBrush(hover_color);
                FillRect(mem_dc, &min_rect, b);
                let _ = DeleteObject(b.into());
            }
            let _ = SetTextColor(mem_dc, COLORREF(0x00d0d0d0));
            let mut min_glyph = vec![0xE921, 0];
            DrawTextW(mem_dc, &mut min_glyph, &mut min_rect, text_format);

            // Maximize / Restore
            let is_max_hov = *hovered_target == SidebarClickTarget::Caption(CaptionButton::Maximize);
            let is_max_pre = *pressed_target == SidebarClickTarget::Caption(CaptionButton::Maximize);
            if is_max_pre {
                let b = CreateSolidBrush(press_color);
                FillRect(mem_dc, &max_rect, b);
                let _ = DeleteObject(b.into());
            } else if is_max_hov {
                let b = CreateSolidBrush(hover_color);
                FillRect(mem_dc, &max_rect, b);
                let _ = DeleteObject(b.into());
            }
            let mut max_glyph = if is_maximized {
                vec![0xE923, 0]
            } else {
                vec![0xE922, 0]
            };
            DrawTextW(mem_dc, &mut max_glyph, &mut max_rect, text_format);

            // Close
            let is_close_hov = *hovered_target == SidebarClickTarget::Caption(CaptionButton::Close);
            let is_close_pre = *pressed_target == SidebarClickTarget::Caption(CaptionButton::Close);
            if is_close_pre {
                let b = CreateSolidBrush(close_press_color);
                FillRect(mem_dc, &close_rect, b);
                let _ = DeleteObject(b.into());
            } else if is_close_hov {
                let b = CreateSolidBrush(close_hover_color);
                FillRect(mem_dc, &close_rect, b);
                let _ = DeleteObject(b.into());
            }
            if is_close_hov || is_close_pre {
                let _ = SetTextColor(mem_dc, COLORREF(0x00ffffff));
            } else {
                let _ = SetTextColor(mem_dc, COLORREF(0x00d0d0d0));
            }
            let mut close_glyph = vec![0xE8BB, 0];
            DrawTextW(mem_dc, &mut close_glyph, &mut close_rect, text_format);

            // If sidebar is collapsed, draw floating Toggle Sidebar button beside minimize
            if !sidebar_visible {
                let mut tog_rect = RECT {
                    left: width - (btn_width * 4),
                    top: 0,
                    right: width - (btn_width * 3),
                    bottom: btn_height,
                };
                let is_tog_hov = *hovered_target == SidebarClickTarget::ToggleSidebar;
                if is_tog_hov {
                    let b = CreateSolidBrush(hover_color);
                    FillRect(mem_dc, &tog_rect, b);
                    let _ = DeleteObject(b.into());
                }
                let _ = SetTextColor(mem_dc, COLORREF(0x00d0d0d0));
                let mut tog_glyph = vec![0xE8A0, 0]; // Sidebar icon
                DrawTextW(mem_dc, &mut tog_glyph, &mut tog_rect, text_format);
            }

            // 2. If sidebar is visible, render navigation toolbar and tab list
            if sidebar_visible {
                let nav_y = btn_height + (6.0 * scale).round() as i32;
                let nav_h = (28.0 * scale).round() as i32;
                let tool_w = (28.0 * scale).round() as i32;
                let pad_x = (14.0 * scale).round() as i32;

                let nav_buttons = [
                    (0xE72B, SidebarClickTarget::NavBack),       // Back
                    (0xE72A, SidebarClickTarget::NavForward),    // Forward
                    (0xE72C, SidebarClickTarget::NavReload),     // Reload
                    (0xE710, SidebarClickTarget::NavNewTab),     // New Tab (+)
                    (0xE8A0, SidebarClickTarget::ToggleSidebar), // Toggle Sidebar
                ];

                for (idx, (glyph_code, target)) in nav_buttons.iter().enumerate() {
                    let mut b_rect = RECT {
                        left: pad_x + (idx as i32 * (tool_w + 4)),
                        top: nav_y,
                        right: pad_x + (idx as i32 * (tool_w + 4)) + tool_w,
                        bottom: nav_y + nav_h,
                    };

                    let is_hov = *hovered_target == *target;
                    let is_pre = *pressed_target == *target;

                    if is_pre {
                        let b = CreateSolidBrush(press_color);
                        FillRect(mem_dc, &b_rect, b);
                        let _ = DeleteObject(b.into());
                    } else if is_hov {
                        let b = CreateSolidBrush(hover_color);
                        FillRect(mem_dc, &b_rect, b);
                        let _ = DeleteObject(b.into());
                    }

                    let _ = SetTextColor(mem_dc, if is_hov { COLORREF(0x00ffffff) } else { COLORREF(0x00b0b0b8) });
                    let mut g = vec![*glyph_code, 0];
                    DrawTextW(mem_dc, &mut g, &mut b_rect, text_format);
                }

                // 3. Render Tab List
                let list_top = nav_y + nav_h + (14.0 * scale).round() as i32;
                let tab_h = (36.0 * scale).round() as i32;
                let tab_pad = (4.0 * scale).round() as i32;
                let tab_total = tab_h + tab_pad;

                // Segoe UI font for tab titles
                let tab_font = CreateFontW(
                    -(12.0 * scale).round() as i32,
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

                for (i, tab) in tabs.iter().enumerate() {
                    let tab_top = list_top + (i as i32 * tab_total);
                    if tab_top + tab_h > height {
                        break;
                    }

                    let tab_rect = RECT {
                        left: (10.0 * scale).round() as i32,
                        top: tab_top,
                        right: width - (10.0 * scale).round() as i32,
                        bottom: tab_top + tab_h,
                    };

                    let is_active = tab.is_active;
                    let is_tab_hov = *hovered_target == SidebarClickTarget::SelectTab(i)
                        || *hovered_target == SidebarClickTarget::CloseTab(i);

                    // Tab background
                    if is_active {
                        let active_bg = CreateSolidBrush(COLORREF(0x00322828)); // Arc active tab `#282832`
                        FillRect(mem_dc, &tab_rect, active_bg);
                        let _ = DeleteObject(active_bg.into());
                    } else if is_tab_hov {
                        let hov_bg = CreateSolidBrush(COLORREF(0x00262020)); // Hover `#202026`
                        FillRect(mem_dc, &tab_rect, hov_bg);
                        let _ = DeleteObject(hov_bg.into());
                    }

                    // Tab title text
                    let _ = SelectObject(mem_dc, tab_font.into());
                    let title_color = if is_active {
                        COLORREF(0x00ffffff) // White
                    } else if is_tab_hov {
                        COLORREF(0x00d0d0d5)
                    } else {
                        COLORREF(0x009898a0) // Muted gray
                    };
                    let _ = SetTextColor(mem_dc, title_color);

                    let mut tab_title_u16: Vec<u16> = tab.title.encode_utf16().collect();
                    tab_title_u16.push(0);
                    let mut title_rect = RECT {
                        left: tab_rect.left + (12.0 * scale).round() as i32,
                        top: tab_rect.top,
                        right: tab_rect.right - (28.0 * scale).round() as i32,
                        bottom: tab_rect.bottom,
                    };
                    DrawTextW(
                        mem_dc,
                        &mut tab_title_u16,
                        &mut title_rect,
                        DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0),
                    );

                    // Tab close button (×)
                    if is_active || is_tab_hov {
                        let _ = SelectObject(mem_dc, icon_font.into());
                        let mut close_tab_rect = RECT {
                            left: tab_rect.right - (26.0 * scale).round() as i32,
                            top: tab_rect.top + (4.0 * scale).round() as i32,
                            right: tab_rect.right - (4.0 * scale).round() as i32,
                            bottom: tab_rect.bottom - (4.0 * scale).round() as i32,
                        };

                        let is_close_btn_hov = *hovered_target == SidebarClickTarget::CloseTab(i);
                        if is_close_btn_hov {
                            let b = CreateSolidBrush(close_hover_color);
                            FillRect(mem_dc, &close_tab_rect, b);
                            let _ = DeleteObject(b.into());
                            let _ = SetTextColor(mem_dc, COLORREF(0x00ffffff));
                        } else {
                            let _ = SetTextColor(mem_dc, COLORREF(0x00a0a0a8));
                        }

                        let mut close_tab_glyph = vec![0xE8BB, 0];
                        DrawTextW(mem_dc, &mut close_tab_glyph, &mut close_tab_rect, text_format);
                    }
                }

                let _ = DeleteObject(tab_font.into());
            }

            // Restore and clean up
            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(icon_font.into());

            // Blit to screen
            let _ = BitBlt(
                hdc,
                sidebar_rect.left,
                sidebar_rect.top,
                width,
                height,
                Some(mem_dc),
                0,
                0,
                SRCCOPY,
            );

            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);
        }
    }
}