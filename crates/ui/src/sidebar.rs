use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush, DeleteDC,
    DeleteObject, FillRect, LineTo, MoveToEx, SelectObject, SetBkMode, SetTextColor,
    TextOutW, HDC, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::IsZoomed;

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

    pub fn render(&self, hwnd: HWND, hdc: HDC, sidebar_rect: RECT, _caption_height: i32) {
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

            // Draw window controls in caption area
            let dpi = GetDpiForWindow(hwnd);
            let btn_width = (46.0 * (dpi as f32 / 96.0)).round() as i32;
            let btn_height = (32.0 * (dpi as f32 / 96.0)).round() as i32;
            let is_maximized = IsZoomed(hwnd).as_bool();

            let control_pen = CreatePen(PS_SOLID, 1, self.text_color);
            let _ = SelectObject(mem_dc, control_pen.into());

            // Close button (X)
            let close_center_x = width - btn_width / 2;
            let btn_center_y = btn_height / 2;
            let x_radius = 5;
            let _ = MoveToEx(mem_dc, close_center_x - x_radius, btn_center_y - x_radius, None);
            let _ = LineTo(mem_dc, close_center_x + x_radius + 1, btn_center_y + x_radius + 1);
            let _ = MoveToEx(mem_dc, close_center_x + x_radius, btn_center_y - x_radius, None);
            let _ = LineTo(mem_dc, close_center_x - x_radius - 1, btn_center_y + x_radius + 1);

            // Maximize / Restore button
            let max_center_x = width - btn_width - btn_width / 2;
            let rect_radius = 5;
            if is_maximized {
                // Restore icon (two overlapping boxes)
                let _ = MoveToEx(mem_dc, max_center_x - rect_radius + 2, btn_center_y - rect_radius, None);
                let _ = LineTo(mem_dc, max_center_x + rect_radius + 2, btn_center_y - rect_radius);
                let _ = LineTo(mem_dc, max_center_x + rect_radius + 2, btn_center_y + rect_radius - 2);

                let _ = MoveToEx(mem_dc, max_center_x - rect_radius, btn_center_y - rect_radius + 2, None);
                let _ = LineTo(mem_dc, max_center_x + rect_radius, btn_center_y - rect_radius + 2);
                let _ = LineTo(mem_dc, max_center_x + rect_radius, btn_center_y + rect_radius + 2);
                let _ = LineTo(mem_dc, max_center_x - rect_radius, btn_center_y + rect_radius + 2);
                let _ = LineTo(mem_dc, max_center_x - rect_radius, btn_center_y - rect_radius + 2);
            } else {
                // Maximize icon (single rectangle)
                let _ = MoveToEx(mem_dc, max_center_x - rect_radius, btn_center_y - rect_radius, None);
                let _ = LineTo(mem_dc, max_center_x + rect_radius, btn_center_y - rect_radius);
                let _ = LineTo(mem_dc, max_center_x + rect_radius, btn_center_y + rect_radius);
                let _ = LineTo(mem_dc, max_center_x - rect_radius, btn_center_y + rect_radius);
                let _ = LineTo(mem_dc, max_center_x - rect_radius, btn_center_y - rect_radius);
            }

            // Minimize button (-)
            let min_center_x = width - (btn_width * 2) - btn_width / 2;
            let _ = MoveToEx(mem_dc, min_center_x - 5, btn_center_y, None);
            let _ = LineTo(mem_dc, min_center_x + 6, btn_center_y);

            // Draw clean subtle header label
            let _ = SetBkMode(mem_dc, TRANSPARENT);
            let _ = SetTextColor(mem_dc, self.text_color);
            let header_text: Vec<u16> = "Arc Browser".encode_utf16().collect();
            let pad_x = 20;
            let pad_y = btn_height + 15;
            let _ = TextOutW(mem_dc, pad_x, pad_y, &header_text);

            // Clean up pens
            let _ = SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(border_pen.into());
            let _ = DeleteObject(control_pen.into());

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
