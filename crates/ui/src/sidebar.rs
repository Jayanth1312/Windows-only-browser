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

pub struct SidebarRenderer {
    bg_color: COLORREF,
    border_color: COLORREF,
    text_color: COLORREF,
}

impl Default for SidebarRenderer {
    fn default() -> Self {
        Self {
            // Arc-style dark slate
            bg_color: COLORREF(0x001a1616),     // RGB(22, 22, 26) in 0x00bbggrr
            border_color: COLORREF(0x00302a27), // RGB(39, 42, 48)
            text_color: COLORREF(0x00908880),   // RGB(128, 136, 144)
        }
    }
}

impl SidebarRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hit_test_button(&self, hwnd: HWND, x: i32, y: i32, client_width: i32) -> CaptionButton {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let btn_width = (46.0 * (dpi as f32 / 96.0)).round() as i32;
        let btn_height = (32.0 * (dpi as f32 / 96.0)).round() as i32;

        if y < 0 || y >= btn_height {
            return CaptionButton::None;
        }

        if x >= client_width - btn_width && x < client_width {
            CaptionButton::Close
        } else if x >= client_width - (btn_width * 2) && x < client_width - btn_width {
            CaptionButton::Maximize
        } else if x >= client_width - (btn_width * 3) && x < client_width - (btn_width * 2) {
            CaptionButton::Minimize
        } else {
            CaptionButton::None
        }
    }

    pub fn get_caption_buttons_rect(&self, hwnd: HWND, client_width: i32) -> RECT {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let btn_width = (46.0 * (dpi as f32 / 96.0)).round() as i32;
        let btn_height = (32.0 * (dpi as f32 / 96.0)).round() as i32;
        RECT {
            left: client_width - (btn_width * 3),
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
        _caption_height: i32,
        hovered_button: CaptionButton,
        pressed_button: CaptionButton,
    ) {
        let width = sidebar_rect.right - sidebar_rect.left;
        let height = sidebar_rect.bottom - sidebar_rect.top;
        if width <= 0 || height <= 0 {
            return;
        }

        unsafe {
            // Create double-buffer memory DC to avoid flicker
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
            let border_pen = CreatePen(PS_SOLID, 1, self.border_color);
            let old_pen = SelectObject(mem_dc, border_pen.into());
            let _ = MoveToEx(mem_dc, 0, 0, None);
            let _ = LineTo(mem_dc, 0, height);

            // Button geometry
            let dpi = GetDpiForWindow(hwnd);
            let dpi_scale = dpi as f32 / 96.0;
            let btn_width = (46.0 * dpi_scale).round() as i32;
            let btn_height = (32.0 * dpi_scale).round() as i32;
            let is_maximized = IsZoomed(hwnd).as_bool();

            // WinUI-3 Fluent button rectangles
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

            // 1. Draw button background highlights
            let hover_brush_color = COLORREF(0x00342d2d); // RGB(45, 45, 52)
            let press_brush_color = COLORREF(0x00463c3c); // RGB(60, 60, 70)
            let close_hover_color = COLORREF(0x001c2bc4); // RGB(196, 43, 28) - Fluent Red
            let close_press_color = COLORREF(0x00121eaf); // RGB(175, 30, 18) - Darker Red

            // Minimize background
            if pressed_button == CaptionButton::Minimize {
                let brush = CreateSolidBrush(press_brush_color);
                FillRect(mem_dc, &min_rect, brush);
                let _ = DeleteObject(brush.into());
            } else if hovered_button == CaptionButton::Minimize {
                let brush = CreateSolidBrush(hover_brush_color);
                FillRect(mem_dc, &min_rect, brush);
                let _ = DeleteObject(brush.into());
            }

            // Maximize background
            if pressed_button == CaptionButton::Maximize {
                let brush = CreateSolidBrush(press_brush_color);
                FillRect(mem_dc, &max_rect, brush);
                let _ = DeleteObject(brush.into());
            } else if hovered_button == CaptionButton::Maximize {
                let brush = CreateSolidBrush(hover_brush_color);
                FillRect(mem_dc, &max_rect, brush);
                let _ = DeleteObject(brush.into());
            }

            // Close background
            if pressed_button == CaptionButton::Close {
                let brush = CreateSolidBrush(close_press_color);
                FillRect(mem_dc, &close_rect, brush);
                let _ = DeleteObject(brush.into());
            } else if hovered_button == CaptionButton::Close {
                let brush = CreateSolidBrush(close_hover_color);
                FillRect(mem_dc, &close_rect, brush);
                let _ = DeleteObject(brush.into());
            }

            // 2. Draw WinUI-3 glyphs using Segoe Fluent Icons font
            let _ = SetBkMode(mem_dc, TRANSPARENT);

            let font_height = -(10.0 * dpi_scale).round() as i32;
            let icon_font = CreateFontW(
                font_height,
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

            // Minimize glyph: \u{E921}
            let _ = SetTextColor(mem_dc, COLORREF(0x00d0d0d0));
            let mut min_glyph: Vec<u16> = vec![0xE921, 0];
            DrawTextW(mem_dc, &mut min_glyph, &mut min_rect, text_format);

            // Maximize / Restore glyph: \u{E923} (Restore) or \u{E922} (Maximize)
            let mut max_glyph: Vec<u16> = if is_maximized {
                vec![0xE923, 0]
            } else {
                vec![0xE922, 0]
            };
            DrawTextW(mem_dc, &mut max_glyph, &mut max_rect, text_format);

            // Close glyph: \u{E8BB}
            if hovered_button == CaptionButton::Close || pressed_button == CaptionButton::Close {
                let _ = SetTextColor(mem_dc, COLORREF(0x00ffffff)); // Crisp white when hovered
            } else {
                let _ = SetTextColor(mem_dc, COLORREF(0x00d0d0d0));
            }
            let mut close_glyph: Vec<u16> = vec![0xE8BB, 0];
            DrawTextW(mem_dc, &mut close_glyph, &mut close_rect, text_format);

            // Restore font
            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(icon_font.into());

            // 3. Draw clean Arc Browser header title with Segoe UI font
            let title_font = CreateFontW(
                -(13.0 * dpi_scale).round() as i32,
                0,
                0,
                0,
                600, // Semi-bold
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
            let old_title_font = SelectObject(mem_dc, title_font.into());
            let _ = SetTextColor(mem_dc, self.text_color);
            let mut header_text: Vec<u16> = "Arc Browser".encode_utf16().collect();
            header_text.push(0);
            let mut header_rect = RECT {
                left: (20.0 * dpi_scale).round() as i32,
                top: btn_height + (16.0 * dpi_scale).round() as i32,
                right: width - 20,
                bottom: btn_height + (48.0 * dpi_scale).round() as i32,
            };
            DrawTextW(
                mem_dc,
                &mut header_text,
                &mut header_rect,
                DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0),
            );

            let _ = SelectObject(mem_dc, old_title_font);
            let _ = DeleteObject(title_font.into());

            // Clean up separator pen
            let _ = SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(border_pen.into());

            // Blit to screen HDC
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

            // Clean up double buffer
            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);
        }
    }
}
