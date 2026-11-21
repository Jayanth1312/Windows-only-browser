//! Filled icon set used by folder covers and the icon picker.
//!
//! All glyphs are solid (filled) 24x24 shapes. `__COLOR__` is substituted at
//! render time. Legacy outline icons (see `icon_picker::LEGACY_ICON_DEFS`)
//! still resolve so previously-saved folder icons keep working.

use crate::icon_picker::{IconDef, LEGACY_ICON_DEFS};

macro_rules! fill {
    ($d:literal) => {
        concat!(r##"<path fill="__COLOR__" d=""##, $d, r##""/>"##)
    };
}

macro_rules! fill_eo {
    ($d:literal) => {
        concat!(
            r##"<path fill="__COLOR__" fill-rule="evenodd" d=""##,
            $d,
            r##""/>"##
        )
    };
}

/// Order shown in the icon picker (7 columns x 8 rows).
pub const FOLDER_ICONS: &[&str] = &[
    "home", "star", "heart", "bookmark", "code", "terminal", "zap",
    "flame", "lock", "key", "shield", "mail", "message-square", "phone",
    "bell", "cloud", "database", "server", "layers", "grid", "image",
    "camera", "video", "music", "palette", "flag", "map-pin", "globe",
    "compass", "eye", "person", "briefcase", "shopping-bag", "gift", "coffee",
    "utensils", "plane", "book", "graduation", "wrench", "puzzle", "bug",
    "flask", "lightbulb", "dollar-sign", "moon", "rss", "megaphone", "link",
    "download", "gamepad", "pencil", "check", "grid-dots", "circle", "inbox",
];

pub const FILLED_ICONS: &[IconDef] = &[
    IconDef { id: "home", aliases: &["🏠"], body: fill!("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z") },
    IconDef { id: "star", aliases: &["⭐", "★"], body: fill!("M12 17.27 18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z") },
    IconDef { id: "heart", aliases: &["♥"], body: fill!("M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z") },
    IconDef { id: "bookmark", aliases: &["🔖"], body: fill!("M17 3H7c-1.1 0-1.99.9-1.99 2L5 21l7-3 7 3V5c0-1.1-.9-2-2-2z") },
    IconDef { id: "code", aliases: &["<>"], body: fill!("M9.4 16.6 4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0 4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z") },
    IconDef { id: "terminal", aliases: &[">_"], body: fill_eo!("M20 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2zM7.5 15.9 6.1 14.5 8.6 12 6.1 9.5 7.5 8.1 11.4 12zM17.5 15h-6v-2h6z") },
    IconDef { id: "zap", aliases: &["⚡"], body: fill!("M7 2v11h3v9l7-12h-4l4-8z") },

    IconDef { id: "flame", aliases: &["🔥", "fire"], body: fill!("M13.5.67s.74 2.65.74 4.8c0 2.06-1.35 3.73-3.41 3.73-2.07 0-3.63-1.67-3.63-3.73l.03-.36C5.21 7.51 4 10.62 4 14c0 4.42 3.58 8 8 8s8-3.58 8-8C20 8.61 17.41 3.8 13.5.67zM11.71 19c-1.78 0-3.22-1.4-3.22-3.14 0-1.62 1.05-2.76 2.81-3.12 1.77-.36 3.6-1.21 4.62-2.58.39 1.29.59 2.65.59 4.04 0 2.65-2.15 4.8-4.8 4.8z") },
    IconDef { id: "lock", aliases: &["🔒"], body: fill!("M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z") },
    IconDef { id: "key", aliases: &["🔑"], body: fill!("M12.65 10C11.83 7.67 9.61 6 7 6c-3.31 0-6 2.69-6 6s2.69 6 6 6c2.61 0 4.83-1.67 5.65-4H17v4h4v-4h2v-4H12.65zM7 14c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2z") },
    IconDef { id: "shield", aliases: &["🛡"], body: fill!("M12 1 3 5v6c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V5l-9-4z") },
    IconDef { id: "mail", aliases: &["✉"], body: fill!("M20 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4-8 5-8-5V6l8 5 8-5v2z") },
    IconDef { id: "message-square", aliases: &["💬", "chat"], body: fill!("M20 2H4c-1.1 0-1.99.9-1.99 2L2 22l4-4h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2z") },
    IconDef { id: "phone", aliases: &["📞"], body: fill!("M6.62 10.79c1.44 2.83 3.76 5.14 6.59 6.59l2.2-2.2c.27-.27.67-.36 1.02-.24 1.12.37 2.33.57 3.57.57.55 0 1 .45 1 1V20c0 .55-.45 1-1 1-9.39 0-17-7.61-17-17 0-.55.45-1 1-1h3.5c.55 0 1 .45 1 1 0 1.25.2 2.45.57 3.57.11.35.03.74-.25 1.02l-2.2 2.2z") },

    IconDef { id: "bell", aliases: &["🔔"], body: fill!("M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z") },
    IconDef { id: "cloud", aliases: &["☁"], body: fill!("M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z") },
    IconDef { id: "database", aliases: &["🗄"], body: fill!("M2 20h20v-4H2v4zm2-3h2v2H4v-2zM2 4v4h20V4H2zm4 3H4V5h2v2zm-4 7h20v-4H2v4zm2-3h2v2H4v-2z") },
    IconDef { id: "server", aliases: &["🖥"], body: fill!("M20 13H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1v-6c0-.55-.45-1-1-1zM7 19c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zM20 3H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1V4c0-.55-.45-1-1-1zM7 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2z") },
    IconDef { id: "layers", aliases: &["📚"], body: fill!("M11.99 18.54l-7.37-5.73L3 14.07l9 7 9-7-1.63-1.27-7.38 5.74zM12 16l7.36-5.73L21 9l-9-7-9 7 1.63 1.27L12 16z") },
    IconDef {
        id: "grid",
        aliases: &["⊞", "🪟", "window"],
        body: r##"<rect x="3" y="3" width="8" height="8" rx="2" fill="__COLOR__"/><rect x="13" y="3" width="8" height="8" rx="2" fill="__COLOR__"/><rect x="3" y="13" width="8" height="8" rx="2" fill="__COLOR__"/><rect x="13" y="13" width="8" height="8" rx="2" fill="__COLOR__"/>"##,
    },
    IconDef { id: "image", aliases: &["🖼"], body: fill!("M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z") },

    IconDef {
        id: "camera",
        aliases: &["📷"],
        body: r##"<circle cx="12" cy="12" r="3.2" fill="__COLOR__"/><path fill="__COLOR__" d="M9 2 7.17 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2h-3.17L15 2H9zm3 15c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5z"/>"##,
    },
    IconDef { id: "video", aliases: &["🎬", "movie"], body: fill!("M18 4l2 4h-3l-2-4h-2l2 4h-3l-2-4H8l2 4H7L5 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V4h-4z") },
    IconDef { id: "music", aliases: &["🎵"], body: fill!("M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z") },
    IconDef { id: "palette", aliases: &["🎨"], body: fill!("M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 9 6.5 9 8 9.67 8 10.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z") },
    IconDef { id: "flag", aliases: &["🚩"], body: fill!("M14.4 6 14 4H5v17h2v-7h5.6l.4 2h7V6z") },
    IconDef { id: "map-pin", aliases: &["📍"], body: fill!("M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7zm0 9.5c-1.38 0-2.5-1.12-2.5-2.5s1.12-2.5 2.5-2.5 2.5 1.12 2.5 2.5-1.12 2.5-2.5 2.5z") },
    IconDef { id: "globe", aliases: &["🌐", "🌍"], body: fill!("M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z") },

    IconDef { id: "compass", aliases: &["🧭"], body: fill!("M12 10.9c-.61 0-1.1.49-1.1 1.1s.49 1.1 1.1 1.1c.61 0 1.1-.49 1.1-1.1s-.49-1.1-1.1-1.1zM12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm2.19 12.19L6 18l3.81-8.19L18 6l-3.81 8.19z") },
    IconDef { id: "eye", aliases: &["👁"], body: fill!("M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zM12 17c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z") },
    IconDef { id: "person", aliases: &["👤"], body: fill!("M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z") },
    IconDef { id: "briefcase", aliases: &["💼"], body: fill!("M20 6h-4V4c0-1.11-.89-2-2-2h-4c-1.11 0-2 .89-2 2v2H4c-1.11 0-1.99.89-1.99 2L2 19c0 1.11.89 2 2 2h16c1.11 0 2-.89 2-2V8c0-1.11-.89-2-2-2zm-6 0h-4V4h4v2z") },
    IconDef { id: "shopping-bag", aliases: &["🛍", "bag"], body: fill!("M18 6h-2c0-2.21-1.79-4-4-4S8 3.79 8 6H6c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm-6-2c1.1 0 2 .9 2 2h-4c0-1.1.9-2 2-2zm0 10c-2.21 0-4-1.79-4-4h2c0 1.1.9 2 2 2s2-.9 2-2h2c0 2.21-1.79 4-4 4z") },
    IconDef {
        id: "gift",
        aliases: &["📦"],
        body: r##"<path fill="__COLOR__" d="M3 7h8v4H3zM13 7h8v4h-8zM4 12h7v9H4zM13 12h7v9h-7z"/><path fill="__COLOR__" d="M12 6C12 3.8 10 2.5 8.6 3.3 7.2 4.1 7.5 6 12 6zM12 6c0-2.2 2-3.5 3.4-2.7C16.8 4.1 16.5 6 12 6z"/>"##,
    },
    IconDef { id: "coffee", aliases: &["☕"], body: fill!("M18.5 3H6c-1.1 0-2 .9-2 2v5.71c0 3.83 2.95 7.18 6.78 7.29 3.96.12 7.22-3.06 7.22-7v-1h.5c1.93 0 3.5-1.57 3.5-3.5S20.43 3 18.5 3zM16 5v3H6V5h10zm2.5 3H18V5h.5c.83 0 1.5.67 1.5 1.5S19.33 8 18.5 8zM4 19h16v2H4z") },

    IconDef { id: "utensils", aliases: &["🍴"], body: fill!("M11 9H9V2H7v7H5V2H3v7c0 2.12 1.66 3.84 3.75 3.97V22h2.5v-9.03C11.34 12.84 13 11.12 13 9V2h-2v7zm5-3v8h2.5v8H21V2c-2.76 0-5 2.24-5 4z") },
    IconDef { id: "plane", aliases: &["✈", "airplane"], body: fill!("M21 16v-2l-8-5V3.5c0-.83-.67-1.5-1.5-1.5S10 2.67 10 3.5V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5l8 2.5z") },
    IconDef { id: "book", aliases: &["📖"], body: fill!("M18 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 4h5v8l-2.5-1.5L6 12V4z") },
    IconDef { id: "graduation", aliases: &["🎓"], body: fill!("M5 13.18v4L12 21l7-3.82v-4L12 17l-7-3.82zM12 3 1 9l11 6 9-4.91V17h2V9L12 3z") },
    IconDef { id: "wrench", aliases: &["🔧"], body: fill!("M22.7 19l-9.1-9.1c.9-2.3.4-5-1.5-6.9-2-2-5-2.4-7.4-1.3L9 6 6 9 1.6 4.7C.4 7.1.9 10.1 2.9 12.1c1.9 1.9 4.6 2.4 6.9 1.5l9.1 9.1c.4.4 1 .4 1.4 0l2.3-2.3c.5-.4.5-1.1.1-1.4z") },
    IconDef { id: "puzzle", aliases: &["🧩"], body: fill!("M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V20c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V22H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z") },
    IconDef {
        id: "bug",
        aliases: &["🐛"],
        body: r##"<ellipse cx="12" cy="14" rx="5" ry="6.5" fill="__COLOR__"/><circle cx="12" cy="6.2" r="2.6" fill="__COLOR__"/><path d="M3.5 12.5h4.5M16 12.5h4.5M4.5 7.5l4 2.5M19.5 7.5l-4 2.5M4.5 19l3.8-2.2M19.5 19l-3.8-2.2M9.6 3.8 8.2 2M14.4 3.8 15.8 2" stroke="__COLOR__" stroke-width="1.8" stroke-linecap="round" fill="none"/>"##,
    },

    IconDef { id: "flask", aliases: &["🧪"], body: fill!("M9 2h6v2h-1v4.6L19.4 18A2 2 0 0 1 17.7 21H6.3A2 2 0 0 1 4.6 18L10 8.6V4H9z") },
    IconDef { id: "lightbulb", aliases: &["💡"], body: fill!("M9 21c0 .55.45 1 1 1h4c.55 0 1-.45 1-1v-1H9v1zm3-19C8.14 2 5 5.14 5 9c0 2.38 1.19 4.47 3 5.74V17c0 .55.45 1 1 1h6c.55 0 1-.45 1-1v-2.26c1.81-1.27 3-3.36 3-5.74 0-3.86-3.14-7-7-7z") },
    IconDef { id: "dollar-sign", aliases: &["$"], body: fill!("M11.8 10.9c-2.27-.59-3-1.2-3-2.15 0-1.09 1.01-1.85 2.7-1.85 1.78 0 2.44.85 2.5 2.1h2.21c-.07-1.72-1.12-3.3-3.21-3.81V3h-3v2.16c-1.94.42-3.5 1.68-3.5 3.61 0 2.31 1.91 3.46 4.7 4.13 2.5.6 3 1.48 3 2.41 0 .69-.49 1.79-2.7 1.79-2.06 0-2.87-.92-2.98-2.1h-2.2c.12 2.19 1.76 3.42 3.68 3.83V21h3v-2.15c1.95-.37 3.5-1.5 3.5-3.55 0-2.84-2.43-3.81-4.7-4.4z") },
    IconDef { id: "moon", aliases: &["🌙"], body: fill!("M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9 9-4.03 9-9c0-.46-.04-.92-.1-1.36-.98 1.37-2.58 2.26-4.4 2.26-2.98 0-5.4-2.42-5.4-5.4 0-1.81.89-3.42 2.26-4.4-.44-.06-.9-.1-1.36-.1z") },
    IconDef { id: "rss", aliases: &["📡"], body: fill!("M6.18 15.64a2.18 2.18 0 0 1 2.18 2.18C8.36 19 7.38 20 6.18 20 5 20 4 19 4 17.82a2.18 2.18 0 0 1 2.18-2.18M4 4.44A15.56 15.56 0 0 1 19.56 20h-2.83A12.73 12.73 0 0 0 4 7.27V4.44m0 5.66a9.9 9.9 0 0 1 9.9 9.9h-2.83A7.07 7.07 0 0 0 4 12.93V10.1z") },
    IconDef { id: "megaphone", aliases: &["📢"], body: fill!("M18 11v2h4v-2h-4zm-2 6.61c.96.71 2.21 1.65 3.2 2.39.4-.53.8-1.07 1.2-1.6-.99-.74-2.24-1.68-3.2-2.4-.4.54-.8 1.08-1.2 1.61zM20.4 5.6c-.4-.53-.8-1.07-1.2-1.6-.99.74-2.24 1.68-3.2 2.4.4.53.8 1.07 1.2 1.6.96-.72 2.21-1.65 3.2-2.4zM4 9c-1.1 0-2 .9-2 2v2c0 1.1.9 2 2 2h1v4h2v-4h1l5 3V6L8 9H4zm11.5 3c0-1.33-.58-2.53-1.5-3.35v6.69c.92-.81 1.5-2.01 1.5-3.34z") },
    IconDef { id: "link", aliases: &["🔗"], body: fill!("M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z") },

    IconDef { id: "download", aliases: &["⬇"], body: fill!("M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z") },
    IconDef { id: "gamepad", aliases: &["🎮"], body: fill!("M21 6H3c-1.1 0-2 .9-2 2v8c0 1.1.9 2 2 2h18c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm-10 7H8v3H6v-3H3v-2h3V8h2v3h3v2zm4.5 2c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm4-3c-.83 0-1.5-.67-1.5-1.5S18.67 9 19.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z") },
    IconDef { id: "pencil", aliases: &["✏", "pen"], body: fill!("M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z") },
    IconDef { id: "check", aliases: &["✔"], body: fill!("M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z") },
    IconDef {
        id: "grid-dots",
        aliases: &["⠿"],
        body: r##"<circle cx="5" cy="5" r="2" fill="__COLOR__"/><circle cx="12" cy="5" r="2" fill="__COLOR__"/><circle cx="19" cy="5" r="2" fill="__COLOR__"/><circle cx="5" cy="12" r="2" fill="__COLOR__"/><circle cx="12" cy="12" r="2" fill="__COLOR__"/><circle cx="19" cy="12" r="2" fill="__COLOR__"/><circle cx="5" cy="19" r="2" fill="__COLOR__"/><circle cx="12" cy="19" r="2" fill="__COLOR__"/><circle cx="19" cy="19" r="2" fill="__COLOR__"/>"##,
    },
    IconDef { id: "circle", aliases: &["⬤"], body: r##"<circle cx="12" cy="12" r="8" fill="__COLOR__"/>"## },
    IconDef { id: "inbox", aliases: &["📥"], body: fill!("M19 3H4.99c-1.11 0-1.98.89-1.98 2L3 19c0 1.1.88 2 1.99 2H19c1.1 0 2-.9 2-2V5c0-1.11-.9-2-2-2zm0 12h-4c0 1.66-1.35 3-3 3s-3-1.34-3-3H4.99V5H19v10z") },

    // Not shown in the picker grid; used by the picker's "clear" button.
    IconDef { id: "trash", aliases: &["🗑"], body: fill!("M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z") },
];

/// Resolve an icon id or alias: filled set first, then legacy outline icons.
pub fn lookup(key: &str) -> Option<&'static IconDef> {
    let k = key.trim();
    FILLED_ICONS
        .iter()
        .find(|d| d.id == k || d.aliases.contains(&k))
        .or_else(|| LEGACY_ICON_DEFS.iter().find(|d| d.id == k || d.aliases.contains(&k)))
}

/// tiny-skia pixmaps are premultiplied; the canvas expects straight alpha.
pub fn straight_rgba(pm: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.pixels().len() * 4);
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}
