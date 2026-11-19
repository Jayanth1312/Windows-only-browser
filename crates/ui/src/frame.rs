use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsZoomed, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
    HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS,
};

// DWM attribute constants for Windows 11
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
pub const DWMWA_WINDOW_CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
pub const DWMWCP_ROUND: u32 = 2;

pub fn enable_modern_frame(hwnd: HWND) {
    unsafe {
        // 1. Enable immersive dark mode
        let dark_mode = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const _ as _,
            std::mem::size_of::<BOOL>() as u32,
        );

        // 2. Enable Windows 11 rounded corners
        let corner_preference = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner_preference as *const _ as _,
            std::mem::size_of::<u32>() as u32,
        );

        // 3. Extend frame into client area by 1 pixel to activate DWM shadow
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

pub fn handle_nccalcsize(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if wparam.0 == 0 {
        return None;
    }

    unsafe {
        if IsZoomed(hwnd).as_bool() {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut mi).as_bool() {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                params.rgrc[0] = mi.rcWork;
            }
        }
    }

    Some(LRESULT(0))
}

pub fn handle_nchittest(
    hwnd: HWND,
    lparam: LPARAM,
    sidebar_width: i32,
    caption_height: i32,
    sidebar_visible: bool,
) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

    let mut rect = RECT::default();
    unsafe {
        if !GetWindowRect(hwnd, &mut rect).is_ok() {
            return LRESULT(HTCLIENT as isize);
        }
    }

    let is_maximized = unsafe { IsZoomed(hwnd).as_bool() };

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let border = if is_maximized {
        0
    } else {
        (8.0 * (dpi as f32 / 96.0)).round() as i32
    };

    // Check corners and borders for resizing (only when not maximized)
    if !is_maximized {
        if x < rect.left + border && y < rect.top + border {
            return LRESULT(HTTOPLEFT as isize);
        }
        if x >= rect.right - border && y < rect.top + border {
            return LRESULT(HTTOPRIGHT as isize);
        }
        if x < rect.left + border && y >= rect.bottom - border {
            return LRESULT(HTBOTTOMLEFT as isize);
        }
        if x >= rect.right - border && y >= rect.bottom - border {
            return LRESULT(HTBOTTOMRIGHT as isize);
        }

        if x < rect.left + border {
            return LRESULT(HTLEFT as isize);
        }
        if x >= rect.right - border {
            return LRESULT(HTRIGHT as isize);
        }
        if y < rect.top + border {
            return LRESULT(HTTOP as isize);
        }
        if y >= rect.bottom - border {
            return LRESULT(HTBOTTOM as isize);
        }
    }

    // Window controls at top-right
    let btn_width = (46.0 * (dpi as f32 / 96.0)).round() as i32;
    let btn_height = (32.0 * (dpi as f32 / 96.0)).round() as i32;
    let btn_count = if sidebar_visible { 3 } else { 4 };
    let buttons_left = rect.right - (btn_width * btn_count);

    if x >= buttons_left && x < rect.right && y >= rect.top && y < rect.top + btn_height {
        return LRESULT(HTCLIENT as isize);
    }

    // Draggable caption area within sidebar top
    if sidebar_visible {
        let sidebar_left = rect.right - sidebar_width;
        if x >= sidebar_left && y < rect.top + caption_height {
            return LRESULT(HTCAPTION as isize);
        }
    }

    // Thin top drag strip above content
    if y < rect.top + 6 {
        return LRESULT(HTCAPTION as isize);
    }

    LRESULT(HTCLIENT as isize)
}
