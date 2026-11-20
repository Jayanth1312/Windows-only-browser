use windows::core::{w, BOOL, PCSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsZoomed, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
    HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS,
};

// DWM attribute constants for Windows 11
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
pub const DWMWA_WINDOW_CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
pub const DWMWA_BORDER_COLOR: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(34);
pub const DWMWCP_ROUND: u32 = 2;

pub fn enable_process_dark_mode() {
    unsafe {
        if let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) {
            // Ordinal 135: SetPreferredAppMode (2 = ForceDark)
            let set_preferred_app_mode: Option<unsafe extern "system" fn(i32) -> i32> =
                std::mem::transmute(GetProcAddress(uxtheme, PCSTR(135 as *const u8)));
            if let Some(set_mode) = set_preferred_app_mode {
                set_mode(2);
            }
            // Ordinal 136: FlushMenuThemes
            let flush_menu_themes: Option<unsafe extern "system" fn()> =
                std::mem::transmute(GetProcAddress(uxtheme, PCSTR(136 as *const u8)));
            if let Some(flush) = flush_menu_themes {
                flush();
            }
        }
    }
}

pub fn allow_dark_mode_for_window(hwnd: HWND) {
    unsafe {
        if let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) {
            // Ordinal 133: AllowDarkModeForWindow
            let allow_dark_mode: Option<unsafe extern "system" fn(HWND, BOOL) -> BOOL> =
                std::mem::transmute(GetProcAddress(uxtheme, PCSTR(133 as *const u8)));
            if let Some(allow) = allow_dark_mode {
                let _ = allow(hwnd, BOOL(1));
            }
        }
    }
}

pub fn enable_modern_frame(hwnd: HWND) {
    enable_process_dark_mode();
    allow_dark_mode_for_window(hwnd);
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
    is_interactive_widget: bool,
) -> LRESULT {
    if is_interactive_widget {
        return LRESULT(HTCLIENT as isize);
    }

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

    // Draggable caption area when sidebar is open
    if sidebar_visible {
        let sidebar_left = rect.right - sidebar_width;
        if x >= sidebar_left && y < rect.top + caption_height {
            return LRESULT(HTCAPTION as isize);
        }
    }

    // Thin top drag strip above content to allow moving / double-click maximizing the window
    let drag_h = border + (6.0 * (dpi as f32 / 96.0)).round() as i32;
    if y < rect.top + drag_h {
        return LRESULT(HTCAPTION as isize);
    }

    LRESULT(HTCLIENT as isize)
}
