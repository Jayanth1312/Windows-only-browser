use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, SelectObject, SetBkMode, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    FW_SEMIBOLD, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, GetWindowLongPtrW,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWLP_USERDATA,
    IDC_ARROW, LoadCursorW, SM_CXSCREEN, SM_CYSCREEN, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    ULW_ALPHA, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
    WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::command_bar::finalize_alpha;
use crate::sidebar::{make_font, rc, rgb, text, Canvas};
use crate::window::UiError;

pub struct IconDef {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub body: &'static str,
}

// Picker order + filled glyphs live in `icons.rs`. Older outline icons below
// stay resolvable (via `icons::lookup`) so saved folder icons keep rendering.
pub use crate::icons::FOLDER_ICONS;

pub const LEGACY_ICON_DEFS: &[IconDef] = &[
    IconDef { id: "plane", aliases: &["✈", "airplane"], body: r##"<path d="M17.8 19.2 16 11l3.5-3.5C21 6 21.5 4 21 3c-1-.5-3 0-4.5 1.5L13 8 4.8 6.2c-.5-.1-.9.1-1.1.5l-.3.5c-.2.5-.1 1 .3 1.3L9 12l-2 3H4l-1 1 3 2 2 3 1-1v-3l3-2 3.5 5.3c.3.4.8.5 1.3.3l.5-.2c.4-.3.6-.7.5-1.2z" fill="__COLOR__" stroke="__COLOR__" stroke-width="0.5"/>"## },
    IconDef { id: "football", aliases: &["🏈"], body: r##"<path d="M11.66 2.04a16.51 16.51 0 0 0-9.62 9.62c-.25.86.35 1.7 1.23 1.73A16.54 16.54 0 0 0 12.89 22c.88.03 1.48-.57 1.73-1.43a16.51 16.51 0 0 0 7.34-7.34c.25-.86-.35-1.7-1.23-1.73A16.54 16.54 0 0 0 11.66 2.04z" stroke="__COLOR__" stroke-width="2" fill="none"/><line x1="8" y1="8" x2="16" y2="16" stroke="__COLOR__" stroke-width="2"/><line x1="12" y1="9" x2="15" y2="12" stroke="__COLOR__" stroke-width="2"/><line x1="9" y1="12" x2="12" y2="15" stroke="__COLOR__" stroke-width="2"/>"## },
    IconDef { id: "baseball", aliases: &["⚾"], body: r##"<circle cx="12" cy="12" r="10" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M5.5 3.5a11 11 0 0 1 0 17M18.5 3.5a11 11 0 0 0 0 17" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "shopping-bag", aliases: &["🛍", "bag"], body: r##"<path d="M6 2 3 6v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6l-3-4z" stroke="__COLOR__" stroke-width="2" fill="none"/><line x1="3" y1="6" x2="21" y2="6" stroke="__COLOR__" stroke-width="2"/><path d="M16 10a4 4 0 0 1-8 0" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "bed", aliases: &["🛏"], body: r##"<path d="M2 4v16M2 8h18a2 2 0 0 1 2 2v10M2 17h20M6 8v9" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "bell", aliases: &["🔔"], body: r##"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "bookmark", aliases: &["🔖"], body: r##"<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "book", aliases: &["📖"], body: r##"<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1-2.5-2.5z" stroke="__COLOR__" stroke-width="2" fill="none"/><polyline points="4 19.5 6.5 22 20 22" stroke="__COLOR__" stroke-width="2" fill="none"/><line x1="6" y1="6" x2="16" y2="6" stroke="__COLOR__" stroke-width="2"/><line x1="6" y1="10" x2="16" y2="10" stroke="__COLOR__" stroke-width="2"/>"## },
    IconDef { id: "briefcase", aliases: &["💼"], body: r##"<rect width="20" height="14" x="2" y="7" rx="2" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "pencil", aliases: &["✏", "pen"], body: r##"<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "bug", aliases: &["🐛"], body: r##"<rect width="8" height="14" x="8" y="6" rx="4" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="m19 7-3 2M5 7l3 2M19 19l-3-2M5 19l3 2M20 13h-4M4 13h4M10 4l1 2M14 4l-1 2" stroke="__COLOR__" stroke-width="2"/>"## },
    IconDef { id: "wrench", aliases: &["🔧"], body: r##"<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "coffee", aliases: &["☕"], body: r##"<path d="M17 8h1a4 4 0 1 1 0 8h-1M3 8h14v9a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4ZM6 2v2M10 2v2M14 2v2" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "phone", aliases: &["📞"], body: r##"<path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "credit-card", aliases: &["💳"], body: r##"<rect width="20" height="14" x="2" y="5" rx="2" stroke="__COLOR__" stroke-width="2" fill="none"/><line x1="2" x2="22" y1="10" y2="10" stroke="__COLOR__" stroke-width="2"/>"## },
    IconDef { id: "message-square", aliases: &["💬", "chat"], body: r##"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "check", aliases: &["✔"], body: r##"<polyline points="20 6 9 17 4 12" stroke="__COLOR__" stroke-width="2.5" fill="none"/>"## },
    IconDef { id: "circle", aliases: &["⬤"], body: r##"<circle cx="12" cy="12" r="7" fill="__COLOR__"/>"## },
    IconDef { id: "cloud", aliases: &["☁"], body: r##"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "code", aliases: &["<>"], body: r##"<polyline points="16 18 22 12 16 6" stroke="__COLOR__" stroke-width="2.2" fill="none"/><polyline points="8 6 2 12 8 18" stroke="__COLOR__" stroke-width="2.2" fill="none"/>"## },
    IconDef { id: "database", aliases: &["🗄"], body: r##"<ellipse cx="12" cy="5" rx="9" ry="3" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5M3 12c0 1.66 4 3 9 3s9-1.34 9-3" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "tools", aliases: &["🛠"], body: r##"<path d="m15 12-8.5 8.5c-.83.83-2.17.83-3 0a2.12 2.12 0 0 1 0-3L12 9M17.64 15 22 10.64M20.91 3.26a5 5 0 0 0-7.07 7.07M14 6l4 4" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "utensils", aliases: &["🍴"], body: r##"<path d="M18 2v20M21 15V2a5 5 0 0 0-5 5v8h5ZM2 2v20M5 2v6a3 3 0 0 0 6 0V2" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "egg", aliases: &["🥚"], body: r##"<path d="M12 22c4.97 0 9-4.03 9-9 0-4.97-4.03-11-9-11S3 8.03 3 13c0 4.97 4.03 9 9 9Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "puzzle", aliases: &["🧩"], body: r##"<path d="M19.4 7.8c0-1.5-1.3-2.8-2.8-2.8h-1.7V3.3c0-1.6-1.3-2.9-2.9-2.9S9.1 1.7 9.1 3.3V5H7.5c-1.6 0-2.9 1.3-2.9 2.8v1.7H3.3c-1.6 0-2.9 1.3-2.9 2.9s1.3 2.8 2.9 2.8H4.6v1.7c0 1.6 1.3 2.9 2.9 2.9h1.7v1.7c0 1.6 1.3 2.9 2.8 2.9s2.8-1.3 2.8-2.9V21.6h1.7c1.6 0 2.9-1.3 2.9-2.8v-1.7h1.7c1.6 0 2.9-1.3 2.9-2.8s-1.3-2.9-2.9-2.9h-1.7V7.8z" stroke="__COLOR__" stroke-width="1.8" fill="none"/>"## },
    IconDef { id: "eye", aliases: &["👁"], body: r##"<path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" stroke="__COLOR__" stroke-width="2" fill="none"/><circle cx="12" cy="12" r="3" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "burger", aliases: &["🍔"], body: r##"<path d="M4 11h16a1 1 0 0 0 1-1A7 7 0 0 0 3 10a1 1 0 0 0 1 1zm-1 4h18M5 19h14a2 2 0 0 0 2-2H3a2 2 0 0 0 2 2z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "fish", aliases: &["🐟"], body: r##"<path d="M6.5 12c.9-3.5 4.9-6 8.5-6 3.6 0 6.1 2.5 7 6-.9 3.5-3.4 6-7 6s-7.6-2.5-8.5-6Z" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M18 12v.5M2 12l3.5-3.5L2 5v14l3.5-3.5L2 12Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "flag", aliases: &["🚩"], body: r##"<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "flame", aliases: &["🔥", "fire"], body: r##"<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.4-.5-2-1-3-1.1-2.1-.2-4.1 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.2.4-2.3 1-3a2.5 2.5 0 0 0 2.5 2.5z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "flask", aliases: &["🧪"], body: r##"<path d="M10 2v7.5a2 2 0 0 1-.2.9L4.7 20.5a1 1 0 0 0 .9 1.5h12.8a1 1 0 0 0 .9-1.5l-5.1-10.1a2 2 0 0 1-.2-.9V2M8.5 2h7M7 16h10" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "folder", aliases: &["📁"], body: r##"<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9L9.6 3.9A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "gamepad", aliases: &["🎮"], body: r##"<line x1="6" x2="10" y1="12" y2="12" stroke="__COLOR__" stroke-width="2"/><line x1="8" x2="8" y1="10" y2="14" stroke="__COLOR__" stroke-width="2"/><line x1="15" x2="15.01" y1="13" y2="13" stroke="__COLOR__" stroke-width="2.5"/><line x1="18" x2="18.01" y1="11" y2="11" stroke="__COLOR__" stroke-width="2.5"/><rect width="20" height="12" x="2" y="6" rx="6" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "lightbulb", aliases: &["💡"], body: r##"<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5M9 18h6M10 22h4" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "gift", aliases: &["📦"], body: r##"<rect x="3" y="8" width="18" height="4" rx="1" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M12 8v13M19 12v7a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-7M7.5 8a2.5 2.5 0 0 1 0-5A4.8 4.8 0 0 1 12 8a4.8 4.8 0 0 1 4.5-5 2.5 2.5 0 0 1 0 5" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "grid", aliases: &["⊞", "🪟", "window"], body: r##"<rect width="7" height="7" x="3" y="3" rx="1" stroke="__COLOR__" stroke-width="2" fill="none"/><rect width="7" height="7" x="14" y="3" rx="1" stroke="__COLOR__" stroke-width="2" fill="none"/><rect width="7" height="7" x="14" y="14" rx="1" stroke="__COLOR__" stroke-width="2" fill="none"/><rect width="7" height="7" x="3" y="14" rx="1" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "grid-dots", aliases: &["⠿"], body: r##"<circle cx="5" cy="5" r="2" fill="__COLOR__"/><circle cx="12" cy="5" r="2" fill="__COLOR__"/><circle cx="19" cy="5" r="2" fill="__COLOR__"/><circle cx="5" cy="12" r="2" fill="__COLOR__"/><circle cx="12" cy="12" r="2" fill="__COLOR__"/><circle cx="19" cy="12" r="2" fill="__COLOR__"/><circle cx="5" cy="19" r="2" fill="__COLOR__"/><circle cx="12" cy="19" r="2" fill="__COLOR__"/><circle cx="19" cy="19" r="2" fill="__COLOR__"/>"## },
    IconDef { id: "heart", aliases: &["♥"], body: r##"<path d="M19 14c1.5-1.5 3-3.2 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.8 0-3 .5-4.5 2-1.5-1.5-2.7-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4 3 5.5l7 7Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "ice-cream", aliases: &["🍦"], body: r##"<path d="m7 11 5 11 5-11M12 2a5 5 0 0 0-5 5v4h10V7a5 5 0 0 0-5-5z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "image", aliases: &["🖼"], body: r##"<rect width="18" height="18" x="3" y="3" rx="2" stroke="__COLOR__" stroke-width="2" fill="none"/><circle cx="9" cy="9" r="2" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "inbox", aliases: &["📥"], body: r##"<polyline points="22 12 16 12 14 15 10 15 8 12 2 12" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M5.4 5.1 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.4-6.9A2 2 0 0 0 16.8 4H7.2a2 2 0 0 0-1.8 1.1z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "key", aliases: &["🔑"], body: r##"<circle cx="7.5" cy="15.5" r="5.5" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="m21 2-9.6 9.6M15.5 7.5l3 3M18.5 4.5l3 3" stroke="__COLOR__" stroke-width="2"/>"## },

    IconDef { id: "layers", aliases: &["📚"], body: r##"<path d="m12.8 2.2a2 2 0 0 0-1.6 0L2.6 6.1a1 1 0 0 0 0 1.8l8.6 3.9a2 2 0 0 0 1.6 0l8.6-3.9a1 1 0 0 0 0-1.8Z" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="m22 17.6-9.2 4.2a2 2 0 0 1-1.6 0L2 17.6M22 12.6l-9.2 4.2a2 2 0 0 1-1.6 0L2 12.6" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "leaf", aliases: &["🌿"], body: r##"<path d="M11 20A7 7 0 0 1 9.8 6.1C15.5 5 17 4.5 19 2c1 2 2 4.2 2 8 0 5.5-4.8 10-10 10Z" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M2 21c0-3 1.9-5.4 5.1-6 2.4-.5 4.9-2 5.9-3" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "zap", aliases: &["⚡"], body: r##"<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "map-pin", aliases: &["📍"], body: r##"<path d="M20 10c0 5-5.5 10.2-7.4 11.8a1 1 0 0 1-1.2 0C9.5 20.2 4 15 4 10a8 8 0 0 1 16 0" stroke="__COLOR__" stroke-width="2" fill="none"/><circle cx="12" cy="10" r="3" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "lock", aliases: &["🔒"], body: r##"<rect width="18" height="11" x="3" y="11" rx="2" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M7 11V7a5 5 0 0 1 10 0v4" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "rss", aliases: &["📡"], body: r##"<path d="M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16" stroke="__COLOR__" stroke-width="2" fill="none"/><circle cx="5" cy="19" r="1" fill="__COLOR__"/>"## },
    IconDef { id: "dollar-sign", aliases: &["$"], body: r##"<line x1="12" x2="12" y1="2" y2="22" stroke="__COLOR__" stroke-width="2"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },

    IconDef { id: "mail", aliases: &["✉"], body: r##"<rect width="20" height="16" x="2" y="4" rx="2" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="m22 7-9 5.7a1.9 1.9 0 0 1-2 0L2 7" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "map", aliases: &["🗺"], body: r##"<polygon points="3 6 9 3 15 6 21 3 21 18 15 21 9 18 3 21" stroke="__COLOR__" stroke-width="2" fill="none"/><line x1="9" y1="3" x2="9" y2="18" stroke="__COLOR__" stroke-width="2"/><line x1="15" y1="6" x2="15" y2="21" stroke="__COLOR__" stroke-width="2"/>"## },
    IconDef { id: "megaphone", aliases: &["📢"], body: r##"<path d="m3 11 18-5v12L3 13v-2z" stroke="__COLOR__" stroke-width="2" fill="none"/><path d="M11.6 16.8a3 3 0 1 1-5.8-1.6" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "moon", aliases: &["🌙"], body: r##"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "music", aliases: &["🎵"], body: r##"<path d="M9 18V5l12-2v13M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm12-2a3 3 0 1 0 0-6 3 3 0 0 0 0 6z" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
    IconDef { id: "compass", aliases: &["🧭"], body: r##"<circle cx="12" cy="12" r="10" stroke="__COLOR__" stroke-width="2" fill="none"/><polygon points="16.2 7.8 14.1 14.1 7.8 16.2 9.9 9.9 16.2 7.8" stroke="__COLOR__" stroke-width="1.8" fill="none"/>"## },
    IconDef { id: "radiation", aliases: &["☢"], body: r##"<circle cx="12" cy="12" r="3" fill="__COLOR__"/><path d="M12 2a10 10 0 0 0-5 1.3l2.5 4.3a5 5 0 0 1 5 0L17 3.3A10 10 0 0 0 12 2zm-8.7 15a10 10 0 0 0 5 8.7l2.5-4.3a5 5 0 0 1-2.5-4.3H3.3zm17.4 0h-5a5 5 0 0 1-2.5 4.3l2.5 4.3a10 10 0 0 0 5-8.6z" stroke="__COLOR__" stroke-width="1.8" fill="none"/>"## },

    IconDef { id: "asterisk", aliases: &["*", "Fonts"], body: r##"<path d="M12 2v20M2 12h20M4.93 4.93l14.14 14.14M4.93 19.07l14.14-14.14" stroke="__COLOR__" stroke-width="2.5" stroke-linecap="round"/>"## },
    IconDef { id: "trash", aliases: &["🗑"], body: r##"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M10 11v6M14 11v6" stroke="__COLOR__" stroke-width="2" fill="none"/>"## },
];

/// SVG body for an icon id/alias (filled set first, then legacy). `None` for the default folder.
pub fn icon_body(key: &str) -> Option<&'static str> {
    let t = key.trim();
    if t.is_empty() || t == "📁" || t == "folder" {
        return None;
    }
    crate::icons::lookup(t).map(|d| d.body)
}

static ICON_CACHE: OnceLock<RwLock<HashMap<(String, String, u32), Vec<u8>>>> = OnceLock::new();

pub fn render_icon_vector(key: &str, color_hex: &str, size: u32) -> Option<Vec<u8>> {
    let trimmed = key.trim();
    if trimmed.is_empty() || trimmed == "📁" || trimmed == "folder" {
        return None;
    }
    let def = crate::icons::lookup(trimmed)?;
    let body = def.body.replace("__COLOR__", color_hex);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 24 24">{body}</svg>"#
    );
    let opt = resvg::usvg::Options::default();
    if let Ok(rtree) = resvg::usvg::Tree::from_str(&svg, &opt) {
        if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(size, size) {
            let sx = size as f32 / rtree.size().width();
            let sy = size as f32 / rtree.size().height();
            let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);
            resvg::render(&rtree, transform, &mut pixmap.as_mut());
            return Some(crate::icons::straight_rgba(&pixmap));
        }
    }
    None
}

pub fn get_rendered_icon(key: &str, color_hex: &str, size: u32) -> Option<Vec<u8>> {
    let cache = ICON_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let cache_key = (key.to_string(), color_hex.to_string(), size);
    if let Ok(c) = cache.read() {
        if let Some(data) = c.get(&cache_key) {
            return Some(data.clone());
        }
    }
    if let Some(rendered) = render_icon_vector(key, color_hex, size) {
        if let Ok(mut c) = cache.write() {
            c.insert(cache_key, rendered.clone());
        }
        return Some(rendered);
    }
    None
}

pub struct IconPickerState {
    pub folder_id: String,
    pub hovered: Option<usize>, // None, Some(0..FOLDER_ICONS.len()) or Some(9999) for trash
    pub on_select: Option<Rc<dyn Fn(String, String)>>,
    pub main_hwnd: HWND,
}

pub struct IconPicker {
    pub hwnd: HWND,
    pub state: Rc<RefCell<IconPickerState>>,
}

impl IconPicker {
    pub fn new(owner: HWND) -> Result<Self, UiError> {
        let class_name = w!("BrowserIconPickerClass");
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(icon_picker_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let state = Rc::new(RefCell::new(IconPickerState {
            folder_id: String::new(),
            hovered: None,
            on_select: None,
            main_hwnd: owner,
        }));

        let raw_state = Rc::into_raw(state.clone()) as *mut _;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Icon Picker"),
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                260,
                280,
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
        folder_id: &str,
        on_select: Box<dyn Fn(String, String)>,
    ) {
        {
            let mut s = self.state.borrow_mut();
            s.folder_id = folder_id.to_string();
            s.hovered = None;
            s.on_select = Some(Rc::new(on_select));
        }

        let s = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;

        let card_w = px(252.0);
        let header_h = px(40.0);
        let pad = px(10.0);
        let cell_size = px(32.0);
        let cols = 7;
        let rows = ((FOLDER_ICONS.len() + cols - 1) / cols) as i32;
        let card_h = header_h + pad + rows * cell_size + pad;

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

        render_picker(self.hwnd, &self.state.borrow());
    }
}

fn hit_test(x: i32, y: i32, s: f32) -> Option<usize> {
    let px = |v: f32| (v * s).round() as i32;
    let card_w = px(252.0);
    let header_h = px(40.0);
    let pad = px(10.0);
    let cell_size = px(32.0);
    let cols = 7;

    // Check trash can hit in top right
    let trash_size = px(28.0);
    let trash_x = card_w - pad - trash_size;
    let trash_y = (header_h - trash_size) / 2;
    if x >= trash_x && x < trash_x + trash_size && y >= trash_y && y < trash_y + trash_size {
        return Some(9999);
    }

    let grid_top = header_h + pad;
    if y < grid_top || x < pad || x >= pad + cols * cell_size {
        return None;
    }

    let col = (x - pad) / cell_size;
    let row = (y - grid_top) / cell_size;
    if col >= 0 && col < cols && row >= 0 {
        let idx = (row * cols + col) as usize;
        if idx < FOLDER_ICONS.len() {
            return Some(idx);
        }
    }
    None
}

fn render_picker(hwnd: HWND, state: &IconPickerState) {
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

    // Dark sleek card
    let card_rect = rc(0, 0, w, h);
    let radius = px(8.0);
    let bg_color = rgb(28, 28, 32);
    cv.fill_round(&card_rect, radius, bg_color, 1.0);
    cv.stroke_round(&card_rect, radius, rgb(255, 255, 255), 0.16, 1.0);

    let pad = px(10.0);
    let header_h = px(40.0);
    let cols = 7;
    let cell_size = px(32.0);

    unsafe {
        SetBkMode(mem_dc, TRANSPARENT);
    }

    // Title "Icons"
    let font_title = unsafe { make_font(w!("Segoe UI"), px(13.0), FW_SEMIBOLD.0 as i32, CLEARTYPE_QUALITY) };
    let old_font = unsafe { SelectObject(mem_dc, font_title.into()) };

    let title_rect = rc(pad, 0, w - pad, header_h);
    unsafe {
        text(
            mem_dc,
            "Icons",
            title_rect,
            DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0,
            rgb(240, 240, 245),
        );
    }

    // Trash / Clear button
    let trash_size = px(28.0);
    let trash_x = w - pad - trash_size;
    let trash_y = (header_h - trash_size) / 2;
    let trash_rect = rc(trash_x, trash_y, trash_x + trash_size, trash_y + trash_size);

    let is_trash_hover = state.hovered == Some(9999);
    if is_trash_hover {
        cv.fill_round(&trash_rect, px(5.0), rgb(235, 87, 87), 0.25);
    }
    let trash_icon_size = px(16.0);
    let t_ix = trash_x + (trash_size - trash_icon_size) / 2;
    let t_iy = trash_y + (trash_size - trash_icon_size) / 2;
    let t_ir = rc(t_ix, t_iy, t_ix + trash_icon_size, t_iy + trash_icon_size);
    if let Some(rgba) = get_rendered_icon("trash", if is_trash_hover { "#ff6464" } else { "#a0a2af" }, trash_icon_size as u32) {
        cv.draw_image_scaled(&t_ir, &rgba, trash_icon_size as u32, trash_icon_size as u32);
    }

    // Icon grid
    let grid_top = header_h + pad;
    let icon_render_size = px(20.0);
    for (i, icon_id) in FOLDER_ICONS.iter().enumerate() {
        let c = (i % cols) as i32;
        let r = (i / cols) as i32;
        let x = pad + c * cell_size;
        let y = grid_top + r * cell_size;
        let cell_r = rc(x, y, x + cell_size, y + cell_size);

        let is_hover = state.hovered == Some(i);
        if is_hover {
            cv.fill_round(&cell_r, px(5.0), rgb(255, 255, 255), 0.15);
            cv.stroke_round(&cell_r, px(5.0), rgb(112, 134, 255), 0.7, 1.0);
        }

        let ix = x + (cell_size - icon_render_size) / 2;
        let iy = y + (cell_size - icon_render_size) / 2;
        let ir = rc(ix, iy, ix + icon_render_size, iy + icon_render_size);

        let color = if is_hover { "#ffffff" } else { "#d0d2de" };
        if let Some(rgba) = get_rendered_icon(icon_id, color, icon_render_size as u32) {
            cv.draw_image_scaled(&ir, &rgba, icon_render_size as u32, icon_render_size as u32);
        }
    }

    unsafe {
        let _ = SelectObject(mem_dc, old_font);
        let _ = DeleteObject(font_title.into());
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

unsafe extern "system" fn icon_picker_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<IconPickerState>;

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
            render_picker(hwnd, &state.borrow());
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let s = GetDpiForWindow(hwnd) as f32 / 96.0;

            let hit = hit_test(x, y, s);
            if hit != state.borrow().hovered {
                state.borrow_mut().hovered = hit;
                render_picker(hwnd, &state.borrow());
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let s = GetDpiForWindow(hwnd) as f32 / 96.0;

            if let Some(hit) = hit_test(x, y, s) {
                let (folder_id, cb) = {
                    let st = state.borrow();
                    (st.folder_id.clone(), st.on_select.as_ref().map(|f| f.clone()))
                };
                let _ = ShowWindow(hwnd, SW_HIDE);

                if let Some(cb) = cb {
                    if hit == 9999 {
                        // Reset to default folder icon
                        cb(folder_id, "📁".to_string());
                    } else if hit < FOLDER_ICONS.len() {
                        cb(folder_id, FOLDER_ICONS[hit].to_string());
                    }
                }
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as usize {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
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
