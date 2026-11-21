use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use engine::{init_com, BrowserShortcut, WebViewController, WebViewEngine, ZapperMessage};
use storage::Storage;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ui::{
    accent_for_url, friendly_title_for_url, BrowserWindow, CommandAction, CommandBar,
    CommandBarMode, SidebarAction, TabItem, WindowConfig,
};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

fn get_app_data_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let new_path = PathBuf::from(&local_app_data).join("Browser");
        let old_path = PathBuf::from(&local_app_data).join("ArcBrowser");
        if !new_path.exists() && old_path.exists() {
            let _ = std::fs::rename(&old_path, &new_path);
        }
        if new_path.exists() {
            new_path
        } else if old_path.exists() {
            old_path
        } else {
            new_path
        }
    } else {
        PathBuf::from(".browser")
    }
}

fn log_timing(stage: &str, duration_ms: f64, total_ms: f64) {
    let log_line = format!(
        "[STARTUP TIMING] {:<32} | Stage: {:>7.2} ms | Total: {:>7.2} ms\n",
        stage, duration_ms, total_ms
    );
    print!("{}", log_line);
    let _ = std::io::stdout().flush();

    let app_dir = get_app_data_dir();
    let _ = std::fs::create_dir_all(&app_dir);
    let log_file = app_dir.join("cold_start.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_file) {
        let _ = file.write_all(log_line.as_bytes());
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub is_open: bool,
    pub open_progress: f32,
}

#[derive(Clone, Debug)]
struct ClosedTab {
    url: String,
    title: String,
    is_pinned: bool,
}

struct Tab {
    id: String,
    title: String,
    url: String,
    accent: COLORREF,
    favicon: Option<std::sync::Arc<ui::FaviconData>>,
    is_pinned: bool,
    folder_id: Option<String>,
    custom_title: bool,
    is_muted: bool,
    controller: Option<WebViewController>,
    last_active: Instant,
    anim_progress: f32,
    is_closing: bool,
}

enum AppEvent {
    CreateTab(String),
    CreateTabAt(String, usize),
    SwitchTab(usize),
    CloseTab(usize),
    CloseOtherTabs(usize),
    ReopenClosedTab,
    ToggleMuteTab(usize),
    ToggleMutePinnedTab(usize),
    UnloadTab(usize),
    ShareTab(usize),
    SharePinnedTab(usize),
    ReloadSpecificTab(usize),
    ReloadSpecificPinnedTab(usize),
    PromptRenameTab(usize),
    RenameTab(usize, String),
    Navigate(String),
    NavBack,
    NavForward,
    NavReload,
    HardReload,
    ToggleSidebar,
    OpenCommandBar(CommandBarMode),
    TitleChanged { tab_id: String, title: String },
    UrlChanged { tab_id: String, url: String },
    FaviconChanged { tab_id: String, bytes: Vec<u8> },
    NavigationStarted { tab_id: String },
    NavigationCompleted { tab_id: String, _success: bool },
    SelectPinned(usize),
    ClosePinnedTab(usize),
    PinTab(usize),
    UnpinTab(usize),
    DuplicateTab(usize),
    CopyUrl,
    CopyCleanLink,
    CopyCleanTab(usize),
    CopyCleanPinnedTab(usize),
    TogglePinCurrentTab,
    OpenSettings,
    OpenDownloads,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextTab,
    PrevTab,
    FullScreenChanged(bool),
    TabAnimTick,
    CreateFolder,
    ToggleFolder(usize),
    RenameFolder(String, String),
    ChangeFolderIcon(String, String),
    DeleteFolder(String),
    MoveTabToFolder(usize, Option<String>),
    NewTabInFolder(String),
    SelectTabId(String),
    ToggleZapper,
    ElementZapped {
        tab_id: String,
        selector: String,
        label: String,
        note: String,
    },
    ZapperUndo {
        tab_id: String,
    },
    ZapperStatus {
        tab_id: String,
        active: bool,
    },
    ZapperRequestCss {
        tab_id: String,
    },
    RestoreAllHiddenElements,
}

fn copy_to_clipboard(hwnd: HWND, text: &str) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = utf16.len() * std::mem::size_of::<u16>();

    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() || OpenClipboard(Some(hwnd)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if opened {
            let _ = EmptyClipboard();
            if let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes) {
                let ptr = GlobalLock(handle);
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, ptr as *mut u8, bytes);
                    let _ = GlobalUnlock(handle);
                    let _ = SetClipboardData(13 /* CF_UNICODETEXT */, Some(windows::Win32::Foundation::HANDLE(handle.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }
}

struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab_idx: usize,
    pinned_shortcuts: Vec<TabItem>,
    folders: Vec<Folder>,
    next_tab_id: usize,
    window: Rc<BrowserWindow>,
    command_bar: Rc<RefCell<Option<CommandBar>>>,
    engine: Rc<WebViewEngine>,
    storage: Rc<RefCell<Storage>>,
    event_tx: mpsc::Sender<AppEvent>,
    favicon_cache: std::collections::HashMap<String, std::sync::Arc<ui::FaviconData>>,
    extension_dir: Option<PathBuf>,
    extension_loaded: bool,
    persist_dirty: bool,
    last_persist: Instant,
    closed_tabs: Vec<ClosedTab>,
    sidebar_before_fullscreen: Option<bool>,
    last_anim_tick: Instant,
}

const UBOL_VERSION: &str = "2026.1116.1650";

/// Downloads and unpacks the official uBlock Origin Lite (Chromium build) once.
fn ensure_ubol(ext_dir: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    if ext_dir.join("manifest.json").exists() {
        return true;
    }
    let parent = match ext_dir.parent() {
        Some(p) => p,
        None => return false,
    };
    let _ = std::fs::create_dir_all(ext_dir);
    let zip_path = parent.join("ubol.zip");
    let url = format!(
        "https://github.com/uBlockOrigin/uBOL-home/releases/download/{v}/uBOLite_{v}.chromium.zip",
        v = UBOL_VERSION
    );
    let dl = std::process::Command::new("curl.exe")
        .args(["-s", "-L", "--fail", "--max-time", "180", "-o"])
        .arg(&zip_path)
        .arg(&url)
        .creation_flags(0x08000000)
        .status();
    if !matches!(dl, Ok(s) if s.success()) {
        tracing::warn!("uBOL download failed");
        let _ = std::fs::remove_file(&zip_path);
        return false;
    }
    let ex = std::process::Command::new("tar.exe")
        .arg("-xf")
        .arg(&zip_path)
        .arg("-C")
        .arg(ext_dir)
        .creation_flags(0x08000000)
        .status();
    let _ = std::fs::remove_file(&zip_path);
    if !matches!(ex, Ok(s) if s.success()) {
        tracing::warn!("uBOL extract failed");
    }
    ext_dir.join("manifest.json").exists()
}

fn get_base_domain(domain: &str) -> Option<String> {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() > 2 {
        Some(parts[parts.len() - 2..].join("."))
    } else {
        None
    }
}

fn fetch_highres_favicon(domain: String, tab_id: String, event_tx: mpsc::Sender<AppEvent>, hwnd_val: usize) {
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
        let v2_url = format!("https://t1.gstatic.com/faviconV2?client=SOCIAL&type=FAVICON&fallback_opts=TYPE,SIZE,URL&url=https://{}&size=128", domain);
        if let Ok(out) = std::process::Command::new("curl.exe")
            .args(["-s", "-L", "-A", ua, "--max-time", "2", &v2_url])
            .creation_flags(0x08000000)
            .output()
        {
            if out.status.success() && out.stdout.len() > 150 {
                let _ = event_tx.send(AppEvent::FaviconChanged {
                    tab_id,
                    bytes: out.stdout,
                });
                unsafe {
                    let hwnd = windows::Win32::Foundation::HWND(hwnd_val as *mut _);
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        windows::Win32::UI::WindowsAndMessaging::WM_APP,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
            }
        }
    });
}

impl BrowserApp {
    fn new(
        window: Rc<BrowserWindow>,
        command_bar: Rc<RefCell<Option<CommandBar>>>,
        engine: Rc<WebViewEngine>,
        storage: Rc<RefCell<Storage>>,
        event_tx: mpsc::Sender<AppEvent>,
        extension_dir: Option<PathBuf>,
    ) -> Self {
        let mut favicon_cache = std::collections::HashMap::new();

        let (saved_pinned, saved_regular, saved_active_idx) = if let Ok(st) = storage.try_borrow() {
            let all_saved = st.get_saved_tabs().unwrap_or_default();
            let mut pinned = Vec::new();
            let mut regular = Vec::new();
            for s in all_saved {
                if s.is_pinned {
                    pinned.push(s);
                } else {
                    regular.push(s);
                }
            }
            let active_idx = st.get_saved_active_tab_idx().ok().flatten().unwrap_or(0);
            (pinned, regular, active_idx)
        } else {
            (Vec::new(), Vec::new(), 0)
        };

        let saved_folders = if let Ok(st) = storage.try_borrow() {
            st.get_saved_folders().unwrap_or_default()
        } else {
            Vec::new()
        };

        let folders: Vec<Folder> = if !saved_folders.is_empty() {
            saved_folders.into_iter().map(|f| Folder {
                id: f.id,
                name: f.name,
                icon: f.icon,
                is_open: f.is_open,
                open_progress: if f.is_open { 1.0 } else { 0.0 },
            }).collect()
        } else {
            vec![
                Folder { id: "f-web2".into(), name: "Web2".into(), icon: "<>".into(), is_open: true, open_progress: 1.0 },
                Folder { id: "f-designs".into(), name: "Designs".into(), icon: "♥".into(), is_open: false, open_progress: 0.0 },
                Folder { id: "f-mcp".into(), name: "MCP".into(), icon: "🔧".into(), is_open: false, open_progress: 0.0 },
                Folder { id: "f-ai".into(), name: "AI".into(), icon: "🪟".into(), is_open: false, open_progress: 0.0 },
            ]
        };

        let pinned_shortcuts: Vec<TabItem> = if !saved_pinned.is_empty() {
            saved_pinned
                .into_iter()
                .map(|s| {
                    let domain = ui::domain_of(&s.url);
                    let fav = if !domain.is_empty() {
                        storage.try_borrow().ok().and_then(|st| {
                            st.get_favicon(&domain).ok().flatten().and_then(|b| ui::decode_favicon(&b))
                        })
                    } else {
                        None
                    };
                    if let Some(ref f) = fav {
                        favicon_cache.insert(domain.clone(), f.clone());
                        if let Some(base) = get_base_domain(&domain) {
                            favicon_cache.entry(base).or_insert_with(|| f.clone());
                        }
                    } else if !domain.is_empty() {
                        fetch_highres_favicon(
                            domain,
                            s.id.clone(),
                            event_tx.clone(),
                            window.hwnd().0 as usize,
                        );
                    }
                    TabItem {
                        id: s.id,
                        title: s.title,
                        url: s.url.clone(),
                        accent: accent_for_url(&s.url),
                        favicon: fav,
                        is_loaded: false,
                        is_muted: false,
                        anim_progress: 1.0,
                        is_closing: false,
                        folder_id: None,
                    }
                })
                .collect()
        } else {
            let defaults = vec![
                TabItem {
                    id: "pin-gmail".into(),
                    title: "Gmail".into(),
                    url: "https://mail.google.com".into(),
                    accent: accent_for_url("https://mail.google.com"),
                    favicon: storage.try_borrow().ok().and_then(|st| {
                        st.get_favicon("mail.google.com").ok().flatten().and_then(|b| ui::decode_favicon(&b))
                    }),
                    is_loaded: false,
                    is_muted: false,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                },
                TabItem {
                    id: "pin-github".into(),
                    title: "GitHub".into(),
                    url: "https://github.com".into(),
                    accent: accent_for_url("https://github.com"),
                    favicon: storage.try_borrow().ok().and_then(|st| {
                        st.get_favicon("github.com").ok().flatten().and_then(|b| ui::decode_favicon(&b))
                    }),
                    is_loaded: false,
                    is_muted: false,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                },
                TabItem {
                    id: "pin-youtube".into(),
                    title: "YouTube".into(),
                    url: "https://www.youtube.com".into(),
                    accent: accent_for_url("https://www.youtube.com"),
                    favicon: storage.try_borrow().ok().and_then(|st| {
                        st.get_favicon("www.youtube.com")
                            .or_else(|_| st.get_favicon("youtube.com"))
                            .ok()
                            .flatten()
                            .and_then(|b| ui::decode_favicon(&b))
                    }),
                    is_loaded: false,
                    is_muted: false,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                },
            ];
            for sc in &defaults {
                let domain = ui::domain_of(&sc.url);
                if let Some(ref fav) = sc.favicon {
                    favicon_cache.insert(domain.clone(), fav.clone());
                    if let Some(base) = get_base_domain(&domain) {
                        favicon_cache.entry(base).or_insert_with(|| fav.clone());
                    }
                } else if !domain.is_empty() {
                    fetch_highres_favicon(
                        domain,
                        sc.id.clone(),
                        event_tx.clone(),
                        window.hwnd().0 as usize,
                    );
                }
            }
            defaults
        };

        let mut max_id_num = 0usize;
        let mut restored_tabs = Vec::new();

        for s in saved_regular {
            if let Some(num_str) = s.id.strip_prefix("tab-") {
                if let Ok(n) = num_str.parse::<usize>() {
                    max_id_num = max_id_num.max(n);
                }
            }
            let domain = ui::domain_of(&s.url);
            let fav = if !domain.is_empty() {
                storage.try_borrow().ok().and_then(|st| {
                    st.get_favicon(&domain).ok().flatten().and_then(|b| ui::decode_favicon(&b))
                })
            } else {
                None
            };
            if let Some(ref f) = fav {
                favicon_cache.insert(domain.clone(), f.clone());
                if let Some(base) = get_base_domain(&domain) {
                    favicon_cache.entry(base).or_insert_with(|| f.clone());
                }
            }
            restored_tabs.push(Tab {
                id: s.id,
                title: s.title,
                url: s.url.clone(),
                accent: accent_for_url(&s.url),
                favicon: fav,
                is_pinned: false,
                folder_id: s.folder_id,
                custom_title: s.custom_title,
                is_muted: false,
                controller: None, // sleeping, will be activated if active tab
                last_active: Instant::now(),
                anim_progress: 1.0,
                is_closing: false,
            });
        }

        let active_tab_idx = if !restored_tabs.is_empty() {
            saved_active_idx.min(restored_tabs.len() - 1)
        } else {
            0
        };

        let next_tab_id = max_id_num.max(restored_tabs.len()) + 1;

        let mut app = Self {
            tabs: restored_tabs,
            active_tab_idx,
            pinned_shortcuts,
            folders,
            next_tab_id,
            window,
            command_bar,
            engine,
            storage,
            event_tx,
            favicon_cache,
            extension_dir,
            extension_loaded: false,
            persist_dirty: false,
            last_persist: Instant::now(),
            closed_tabs: Vec::new(),
            sidebar_before_fullscreen: None,
            last_anim_tick: Instant::now(),
        };

        app.persist_state();
        app
    }

    fn persist_state(&mut self) {
        if let Ok(st) = self.storage.try_borrow() {
            let saved_folders: Vec<storage::SavedFolder> = self.folders.iter().enumerate().map(|(pos, f)| {
                storage::SavedFolder {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    icon: f.icon.clone(),
                    is_open: f.is_open,
                    position: pos as i32,
                }
            }).collect();
            let _ = st.save_folders_state(&saved_folders);

            let mut saved = Vec::new();
            let mut pos = 0;
            for sc in &self.pinned_shortcuts {
                saved.push(storage::SavedTab {
                    id: sc.id.clone(),
                    title: sc.title.clone(),
                    url: sc.url.clone(),
                    position: pos,
                    is_pinned: true,
                    custom_title: false,
                    folder_id: None,
                });
                pos += 1;
            }
            for t in &self.tabs {
                if !t.is_pinned {
                    saved.push(storage::SavedTab {
                        id: t.id.clone(),
                        title: t.title.clone(),
                        url: t.url.clone(),
                        position: pos,
                        is_pinned: false,
                        custom_title: t.custom_title,
                        folder_id: t.folder_id.clone(),
                    });
                    pos += 1;
                }
            }
            let _ = st.save_tabs_state(&saved, self.active_tab_idx);
        }
        self.persist_dirty = false;
        self.last_persist = Instant::now();
    }

    /// Mark state as needing persistence without writing immediately.
    /// Call `flush_if_dirty` periodically (e.g. in the event loop) to batch writes.
    fn mark_dirty(&mut self) {
        self.persist_dirty = true;
    }

    /// Flush pending state to disk if dirty and enough time has elapsed (250ms debounce).
    fn flush_if_dirty(&mut self) {
        if self.persist_dirty && self.last_persist.elapsed().as_millis() >= 250 {
            self.persist_state();
        }
        self.check_tab_hibernation();
    }

    /// Check inactive tabs and suspend background tabs (> 3 mins) to reclaim memory and freeze JS timers.
    fn check_tab_hibernation(&mut self) {
        let now = Instant::now();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i != self.active_tab_idx && !tab.is_muted {
                if let Some(ref ctrl) = tab.controller {
                    if now.duration_since(tab.last_active).as_secs() >= 180 && !ctrl.is_suspended() {
                        let _ = ctrl.try_suspend();
                    }
                }
            }
        }
    }

    fn get_cached_favicon(&mut self, domain: &str) -> Option<std::sync::Arc<ui::FaviconData>> {
        if domain.is_empty() {
            return None;
        }
        if let Some(fav) = self.favicon_cache.get(domain) {
            return Some(fav.clone());
        }
        if let Some(base) = get_base_domain(domain) {
            if let Some(fav) = self.favicon_cache.get(&base) {
                return Some(fav.clone());
            }
        }
        if let Ok(st) = self.storage.try_borrow() {
            if let Ok(Some(bytes)) = st.get_favicon(domain) {
                if let Some(fav) = ui::decode_favicon(&bytes) {
                    self.favicon_cache.insert(domain.to_string(), fav.clone());
                    return Some(fav);
                }
            }
            if let Some(base) = get_base_domain(domain) {
                if let Ok(Some(bytes)) = st.get_favicon(&base) {
                    if let Some(fav) = ui::decode_favicon(&bytes) {
                        self.favicon_cache.insert(domain.to_string(), fav.clone());
                        return Some(fav);
                    }
                }
            }
        }
        None
    }

    fn create_tab(&mut self, url: &str, is_pinned: bool) {
        self.window.set_loading(true, 0.25);
        let tab_id = format!("tab-{}", self.next_tab_id);
        self.next_tab_id += 1;

        let (content_rect, _) = self.window.get_layout_rects();

        let tab_id_clone1 = tab_id.clone();
        let tx1 = self.event_tx.clone();
        let hwnd_val = self.window.hwnd().0 as usize;
        let on_title = Box::new(move |title: String| {
            let _ = tx1.send(AppEvent::TitleChanged {
                tab_id: tab_id_clone1.clone(),
                title,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone2 = tab_id.clone();
        let tx2 = self.event_tx.clone();
        let on_url = Box::new(move |new_url: String| {
            let _ = tx2.send(AppEvent::UrlChanged {
                tab_id: tab_id_clone2.clone(),
                url: new_url,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_fav = tab_id.clone();
        let tx_fav = self.event_tx.clone();
        let on_favicon = Box::new(move |bytes: Vec<u8>| {
            let _ = tx_fav.send(AppEvent::FaviconChanged {
                tab_id: tab_id_clone_fav.clone(),
                bytes,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx3 = self.event_tx.clone();
        let on_new_window = Box::new(move |target_url: String| {
            let _ = tx3.send(AppEvent::CreateTab(target_url));
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx4 = self.event_tx.clone();
        let on_shortcut = Box::new(move |sc: BrowserShortcut| {
            let ev = match sc {
                BrowserShortcut::NewTab => AppEvent::OpenCommandBar(CommandBarMode::NewTab),
                BrowserShortcut::CloseTab => AppEvent::CloseTab(usize::MAX),
                BrowserShortcut::ToggleSidebar => AppEvent::ToggleSidebar,
                BrowserShortcut::Reload => AppEvent::NavReload,
                BrowserShortcut::HardReload => AppEvent::HardReload,
                BrowserShortcut::NavBack => AppEvent::NavBack,
                BrowserShortcut::NavForward => AppEvent::NavForward,
                BrowserShortcut::CopyUrl => AppEvent::CopyUrl,
                BrowserShortcut::CopyCleanLink => AppEvent::CopyCleanLink,
                BrowserShortcut::FocusUrlBar => AppEvent::OpenCommandBar(CommandBarMode::NavigateCurrent),
                BrowserShortcut::OpenSettings => AppEvent::OpenSettings,
                BrowserShortcut::ZoomIn => AppEvent::ZoomIn,
                BrowserShortcut::ZoomOut => AppEvent::ZoomOut,
                BrowserShortcut::ZoomReset => AppEvent::ZoomReset,
                BrowserShortcut::PinTab => AppEvent::TogglePinCurrentTab,
                BrowserShortcut::NextTab => AppEvent::NextTab,
                BrowserShortcut::PrevTab => AppEvent::PrevTab,
                BrowserShortcut::SelectTab(idx) => AppEvent::SwitchTab(idx),
                BrowserShortcut::ReopenClosedTab => AppEvent::ReopenClosedTab,
                BrowserShortcut::ToggleZapper => AppEvent::ToggleZapper,
            };
            let _ = tx4.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_start = tab_id.clone();
        let tx_start = self.event_tx.clone();
        let on_nav_start = Box::new(move || {
            let _ = tx_start.send(AppEvent::NavigationStarted {
                tab_id: tab_id_clone_start.clone(),
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_complete = tab_id.clone();
        let tx_complete = self.event_tx.clone();
        let on_nav_complete = Box::new(move |success: bool| {
            let _ = tx_complete.send(AppEvent::NavigationCompleted {
                tab_id: tab_id_clone_complete.clone(),
                _success: success,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx_fs = self.event_tx.clone();
        let on_fullscreen = Box::new(move |is_full: bool| {
            let _ = tx_fs.send(AppEvent::FullScreenChanged(is_full));
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_zapper = tab_id.clone();
        let tx_zapper = self.event_tx.clone();
        let on_zapper = Box::new(move |msg: ZapperMessage| {
            let ev = match msg {
                ZapperMessage::Zap { selector, label, note } => AppEvent::ElementZapped {
                    tab_id: tab_id_clone_zapper.clone(),
                    selector,
                    label,
                    note,
                },
                ZapperMessage::Undo => AppEvent::ZapperUndo {
                    tab_id: tab_id_clone_zapper.clone(),
                },
                ZapperMessage::Status { active } => AppEvent::ZapperStatus {
                    tab_id: tab_id_clone_zapper.clone(),
                    active,
                },
                ZapperMessage::RequestCss => AppEvent::ZapperRequestCss {
                    tab_id: tab_id_clone_zapper.clone(),
                },
            };
            let _ = tx_zapper.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let controller = match self.engine.create_controller(
            self.window.hwnd(),
            content_rect,
            url,
            Some(on_nav_start),
            Some(on_nav_complete),
            Some(on_title),
            Some(on_url),
            Some(on_favicon),
            Some(on_new_window),
            Some(on_shortcut),
            Some(on_fullscreen),
            Some(on_zapper),
        ) {
            Ok(c) => {
                if !self.extension_loaded {
                    if let Some(ref ext_dir) = self.extension_dir {
                        let _ = c.load_extension(ext_dir);
                        self.extension_loaded = true;
                    }
                }
                c
            }
            Err(e) => {
                info!("Failed to create tab controller: {}", e);
                return;
            }
        };

        // Hide currently active tab if any
        if let Some(current) = self.tabs.get(self.active_tab_idx) {
            if let Some(ref ctrl) = current.controller {
                let _ = ctrl.set_visible(false);
            }
        }

        let new_idx = self.tabs.len();
        let initial_title = friendly_title_for_url(url);
        let initial_accent = accent_for_url(url);

        let domain = ui::domain_of(url);
        let preloaded_favicon = self.get_cached_favicon(&domain);

        self.tabs.push(Tab {
            id: tab_id,
            title: initial_title.clone(),
            url: url.to_string(),
            accent: initial_accent,
            favicon: preloaded_favicon,
            is_pinned,
            folder_id: None,
            custom_title: false,
            is_muted: false,
            controller: Some(controller),
            last_active: Instant::now(),
            anim_progress: if is_pinned { 1.0 } else { 0.05 },
            is_closing: false,
        });

        if !is_pinned {
            self.window.start_tab_anim_timer();
        }

        if let Ok(st) = self.storage.try_borrow() {
            let _ = st.add_history(url, &initial_title);
        }

        self.switch_tab(new_idx);
        self.persist_state();
    }

    fn create_tab_in_folder(&mut self, url: &str, folder_id: Option<String>) {
        self.create_tab(url, false);
        if let Some(tab) = self.tabs.last_mut() {
            tab.folder_id = folder_id;
        }
        self.sync_tabs_ui();
        self.mark_dirty();
    }

    fn ensure_tab_controller(&mut self, idx: usize) {
        if idx >= self.tabs.len() || self.tabs[idx].controller.is_some() {
            return;
        }
        self.window.set_loading(true, 0.25);

        let tab_id = self.tabs[idx].id.clone();
        let url = self.tabs[idx].url.clone();
        let (content_rect, _) = self.window.get_layout_rects();

        let tab_id_clone1 = tab_id.clone();
        let tx1 = self.event_tx.clone();
        let hwnd_val = self.window.hwnd().0 as usize;
        let on_title = Box::new(move |title: String| {
            let _ = tx1.send(AppEvent::TitleChanged {
                tab_id: tab_id_clone1.clone(),
                title,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone2 = tab_id.clone();
        let tx2 = self.event_tx.clone();
        let on_url = Box::new(move |new_url: String| {
            let _ = tx2.send(AppEvent::UrlChanged {
                tab_id: tab_id_clone2.clone(),
                url: new_url,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_fav = tab_id.clone();
        let tx_fav = self.event_tx.clone();
        let on_favicon = Box::new(move |bytes: Vec<u8>| {
            let _ = tx_fav.send(AppEvent::FaviconChanged {
                tab_id: tab_id_clone_fav.clone(),
                bytes,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx3 = self.event_tx.clone();
        let on_new_window = Box::new(move |target_url: String| {
            let _ = tx3.send(AppEvent::CreateTab(target_url));
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx4 = self.event_tx.clone();
        let on_shortcut = Box::new(move |sc: BrowserShortcut| {
            let ev = match sc {
                BrowserShortcut::NewTab => AppEvent::OpenCommandBar(CommandBarMode::NewTab),
                BrowserShortcut::CloseTab => AppEvent::CloseTab(usize::MAX),
                BrowserShortcut::ToggleSidebar => AppEvent::ToggleSidebar,
                BrowserShortcut::Reload => AppEvent::NavReload,
                BrowserShortcut::HardReload => AppEvent::HardReload,
                BrowserShortcut::NavBack => AppEvent::NavBack,
                BrowserShortcut::NavForward => AppEvent::NavForward,
                BrowserShortcut::CopyUrl => AppEvent::CopyUrl,
                BrowserShortcut::CopyCleanLink => AppEvent::CopyCleanLink,
                BrowserShortcut::FocusUrlBar => AppEvent::OpenCommandBar(CommandBarMode::NavigateCurrent),
                BrowserShortcut::OpenSettings => AppEvent::OpenSettings,
                BrowserShortcut::ZoomIn => AppEvent::ZoomIn,
                BrowserShortcut::ZoomOut => AppEvent::ZoomOut,
                BrowserShortcut::ZoomReset => AppEvent::ZoomReset,
                BrowserShortcut::PinTab => AppEvent::TogglePinCurrentTab,
                BrowserShortcut::NextTab => AppEvent::NextTab,
                BrowserShortcut::PrevTab => AppEvent::PrevTab,
                BrowserShortcut::SelectTab(idx) => AppEvent::SwitchTab(idx),
                BrowserShortcut::ReopenClosedTab => AppEvent::ReopenClosedTab,
                BrowserShortcut::ToggleZapper => AppEvent::ToggleZapper,
            };
            let _ = tx4.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_start = tab_id.clone();
        let tx_start = self.event_tx.clone();
        let on_nav_start = Box::new(move || {
            let _ = tx_start.send(AppEvent::NavigationStarted {
                tab_id: tab_id_clone_start.clone(),
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_complete = tab_id.clone();
        let tx_complete = self.event_tx.clone();
        let on_nav_complete = Box::new(move |success: bool| {
            let _ = tx_complete.send(AppEvent::NavigationCompleted {
                tab_id: tab_id_clone_complete.clone(),
                _success: success,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx_fs = self.event_tx.clone();
        let on_fullscreen = Box::new(move |is_full: bool| {
            let _ = tx_fs.send(AppEvent::FullScreenChanged(is_full));
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_zapper = tab_id.clone();
        let tx_zapper = self.event_tx.clone();
        let on_zapper = Box::new(move |msg: ZapperMessage| {
            let ev = match msg {
                ZapperMessage::Zap { selector, label, note } => AppEvent::ElementZapped {
                    tab_id: tab_id_clone_zapper.clone(),
                    selector,
                    label,
                    note,
                },
                ZapperMessage::Undo => AppEvent::ZapperUndo {
                    tab_id: tab_id_clone_zapper.clone(),
                },
                ZapperMessage::Status { active } => AppEvent::ZapperStatus {
                    tab_id: tab_id_clone_zapper.clone(),
                    active,
                },
                ZapperMessage::RequestCss => AppEvent::ZapperRequestCss {
                    tab_id: tab_id_clone_zapper.clone(),
                },
            };
            let _ = tx_zapper.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        match self.engine.create_controller(
            self.window.hwnd(),
            content_rect,
            &url,
            Some(on_nav_start),
            Some(on_nav_complete),
            Some(on_title),
            Some(on_url),
            Some(on_favicon),
            Some(on_new_window),
            Some(on_shortcut),
            Some(on_fullscreen),
            Some(on_zapper),
        ) {
            Ok(c) => {
                if !self.extension_loaded {
                    if let Some(ref ext_dir) = self.extension_dir {
                        let _ = c.load_extension(ext_dir);
                        self.extension_loaded = true;
                    }
                }
                if self.tabs[idx].is_muted {
                    let _ = c.set_muted(true);
                }
                self.tabs[idx].controller = Some(c);
            }
            Err(e) => {
                info!("Failed to create tab controller on wake: {}", e);
            }
        }
    }

    fn switch_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }

        if self.active_tab_idx < self.tabs.len() && self.active_tab_idx != idx {
            if let Some(ref current) = self.tabs[self.active_tab_idx].controller {
                let _ = current.set_visible(false);
            }
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.last_active = Instant::now();
            }
        }

        self.ensure_tab_controller(idx);

        self.active_tab_idx = idx;
        let (content_rect, _) = self.window.get_layout_rects();
        if let Some(ref active) = self.tabs[idx].controller {
            let _ = active.resume(); // Restore page state and JS timers from hibernation
            let _ = active.set_bounds(content_rect);
            let _ = active.set_visible(true);
            let _ = active.focus();
            let zoom = active.get_zoom_factor();
            self.window.set_zoom(zoom);
        }
        if let Some(tab) = self.tabs.get_mut(idx) {
            tab.last_active = Instant::now();
        }

        self.sync_tabs_ui();
        self.mark_dirty();
    }

    fn close_tab(&mut self, idx: usize) {
        let target_idx = if idx == usize::MAX {
            self.active_tab_idx
        } else {
            idx
        };

        if target_idx >= self.tabs.len() {
            return;
        }

        // Save to closed tabs stack for Ctrl+Shift+T reopening
        self.closed_tabs.push(ClosedTab {
            url: self.tabs[target_idx].url.clone(),
            title: self.tabs[target_idx].title.clone(),
            is_pinned: self.tabs[target_idx].is_pinned,
        });
        if self.closed_tabs.len() > 30 {
            self.closed_tabs.remove(0);
        }

        if self.tabs[target_idx].is_pinned {
            if let Some(ref ctrl) = self.tabs[target_idx].controller {
                let _ = ctrl.set_visible(false);
            }
            self.tabs.remove(target_idx);

            if self.tabs.is_empty() {
                self.active_tab_idx = 0;
                self.sync_tabs_ui();
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.window.hwnd()), None, true);
                }
                self.mark_dirty();
                return;
            }

            if self.active_tab_idx >= self.tabs.len() {
                self.active_tab_idx = self.tabs.len().saturating_sub(1);
            } else if self.active_tab_idx > target_idx {
                self.active_tab_idx -= 1;
            }

            self.ensure_tab_controller(self.active_tab_idx);

            let (content_rect, _) = self.window.get_layout_rects();
            if let Some(ref active) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                let _ = active.set_bounds(content_rect);
                let _ = active.set_visible(true);
            }

            self.sync_tabs_ui();
            self.persist_state();
            return;
        }

        if self.tabs[target_idx].is_closing {
            return;
        }

        self.tabs[target_idx].is_closing = true;

        // If closing active tab, switch active to another non-closing tab
        if self.active_tab_idx == target_idx {
            let next_tab = self.tabs.iter().enumerate()
                .filter(|(i, t)| *i != target_idx && !t.is_closing)
                .map(|(i, _)| i)
                .next();
            if let Some(new_idx) = next_tab {
                self.switch_tab(new_idx);
            } else {
                if let Some(ref ctrl) = self.tabs[target_idx].controller {
                    let _ = ctrl.set_visible(false);
                }
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.window.hwnd()), None, false);
                }
            }
        }

        self.window.start_tab_anim_timer();
        self.sync_tabs_ui();
    }

    fn on_tab_anim_tick(&mut self) {
        let speed = 0.12;
        // Folder open/close is time-based (frame-rate independent); eased at render time.
        let now = Instant::now();
        let mut dt = now.duration_since(self.last_anim_tick).as_secs_f32();
        self.last_anim_tick = now;
        if dt > 0.05 {
            dt = 0.016;
        }
        let f_step = dt / 0.28;
        let mut any_animating = false;

        for tab in self.tabs.iter_mut() {
            if tab.is_closing {
                tab.anim_progress = (tab.anim_progress - speed).max(0.0);
                if tab.anim_progress > 0.0 {
                    any_animating = true;
                }
            } else if tab.anim_progress < 1.0 {
                tab.anim_progress = (tab.anim_progress + speed).min(1.0);
                if tab.anim_progress < 1.0 {
                    any_animating = true;
                }
            }
        }

        for folder in self.folders.iter_mut() {
            let target = if folder.is_open { 1.0 } else { 0.0 };
            if (folder.open_progress - target).abs() > 0.01 {
                if folder.is_open {
                    folder.open_progress = (folder.open_progress + f_step).min(1.0);
                } else {
                    folder.open_progress = (folder.open_progress - f_step).max(0.0);
                }
                any_animating = true;
            } else {
                folder.open_progress = target;
            }
        }

        let mut i = 0;
        let mut removed = false;
        while i < self.tabs.len() {
            if self.tabs[i].is_closing && self.tabs[i].anim_progress <= 0.001 {
                if let Some(ref ctrl) = self.tabs[i].controller {
                    let _ = ctrl.set_visible(false);
                }
                self.tabs.remove(i);
                removed = true;
                if self.active_tab_idx >= self.tabs.len() {
                    self.active_tab_idx = self.tabs.len().saturating_sub(1);
                }
            } else {
                i += 1;
            }
        }

        if removed {
            if !self.tabs.is_empty() {
                self.ensure_tab_controller(self.active_tab_idx);
                let (content_rect, _) = self.window.get_layout_rects();
                if let Some(ref active) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                    let _ = active.set_bounds(content_rect);
                    let _ = active.set_visible(true);
                }
            } else {
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.window.hwnd()), None, false);
                }
            }
            self.persist_state();
        }

        self.sync_tabs_ui();

        if !any_animating {
            self.window.stop_tab_anim_timer();
        }
    }

    fn sync_tabs_ui(&self) {
        let mut pinned_items = Vec::new();
        let mut regular_items = Vec::new();
        let mut active_pinned = None;
        let mut active_regular = None;

        for (pi, sc) in self.pinned_shortcuts.iter().enumerate() {
            let sc_domain = ui::domain_of(&sc.url);
            let matching_open = self.tabs.iter().enumerate().find(|(_, t)| {
                t.is_pinned && (t.url == sc.url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
            });

            if let Some((tab_idx, t)) = matching_open {
                if tab_idx == self.active_tab_idx {
                    active_pinned = Some(pi);
                }
                pinned_items.push(TabItem {
                    id: sc.id.clone(),
                    title: t.title.clone(),
                    url: t.url.clone(),
                    accent: t.accent,
                    favicon: t.favicon.clone().or_else(|| sc.favicon.clone()),
                    is_loaded: true,
                    is_muted: t.is_muted,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                });
            } else {
                pinned_items.push(TabItem {
                    id: sc.id.clone(),
                    title: sc.title.clone(),
                    url: sc.url.clone(),
                    accent: sc.accent,
                    favicon: sc.favicon.clone(),
                    is_loaded: false,
                    is_muted: false,
                    anim_progress: 1.0,
                    is_closing: false,
                    folder_id: None,
                });
            }
        }

        for (i, t) in self.tabs.iter().enumerate() {
            if !t.is_pinned {
                if i == self.active_tab_idx {
                    active_regular = Some(regular_items.len());
                }
                regular_items.push(TabItem {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    url: t.url.clone(),
                    accent: t.accent,
                    favicon: t.favicon.clone(),
                    is_loaded: t.controller.is_some(),
                    is_muted: t.is_muted,
                    anim_progress: t.anim_progress,
                    is_closing: t.is_closing,
                    folder_id: t.folder_id.clone(),
                });
            }
        }

        let folder_items: Vec<ui::FolderItem> = self.folders.iter().map(|f| ui::FolderItem {
            id: f.id.clone(),
            name: f.name.clone(),
            icon: f.icon.clone(),
            is_open: f.is_open,
            open_progress: f.open_progress,
        }).collect();

        let active = if let Some(p) = active_pinned {
            Some(ui::ActiveTab::Pinned(p))
        } else if let Some(r) = active_regular {
            Some(ui::ActiveTab::Tab(r))
        } else {
            None
        };

        let (can_back, can_forward) = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            tab.controller.as_ref().map_or((false, false), |c| (c.can_go_back(), c.can_go_forward()))
        } else {
            (false, false)
        };

        let has_closed = !self.closed_tabs.is_empty();
        self.window.update_sidebar_data(regular_items, pinned_items, folder_items, active, can_back, can_forward, has_closed);
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::CreateTab(url) => {
                let clean = engine::clean_url(&url);
                self.create_tab(&clean, false);
            }
            AppEvent::SwitchTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    self.switch_tab(pos);
                }
            }
            AppEvent::SelectPinned(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    self.window.set_loading(true, 0.25);
                    let sc_url = sc.url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    let open_tab_pos = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    });
                    if let Some(pos) = open_tab_pos {
                        self.switch_tab(pos);
                    } else {
                        self.create_tab(&sc_url, true);
                    }
                }
            }
            AppEvent::ClosePinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    let sc_url = sc.url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    let open_tab_pos = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    });
                    if let Some(pos) = open_tab_pos {
                        if let Some(ref fav) = self.tabs[pos].favicon {
                            if let Some(sc_mut) = self.pinned_shortcuts.get_mut(idx) {
                                sc_mut.favicon = Some(fav.clone());
                            }
                        }
                        self.closed_tabs.push(ClosedTab {
                            url: self.tabs[pos].url.clone(),
                            title: self.tabs[pos].title.clone(),
                            is_pinned: true,
                        });
                        if self.closed_tabs.len() > 30 {
                            self.closed_tabs.remove(0);
                        }

                        if let Some(ref ctrl) = self.tabs[pos].controller {
                            let _ = ctrl.set_visible(false);
                        }
                        self.tabs.remove(pos);

                        if self.tabs.is_empty() {
                            unsafe {
                                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    Some(self.window.hwnd()),
                                    windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                                    windows::Win32::Foundation::WPARAM(0),
                                    windows::Win32::Foundation::LPARAM(0),
                                );
                            }
                            return;
                        } else {
                            if self.active_tab_idx >= self.tabs.len() {
                                self.active_tab_idx = self.tabs.len() - 1;
                            } else if self.active_tab_idx > pos {
                                self.active_tab_idx -= 1;
                            }
                            self.ensure_tab_controller(self.active_tab_idx);
                            let (content_rect, _) = self.window.get_layout_rects();
                            if let Some(ref active) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                                let _ = active.set_bounds(content_rect);
                                let _ = active.set_visible(true);
                            }
                        }
                        self.sync_tabs_ui();
                        self.persist_state();
                    }
                }
            }
            AppEvent::PinTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    self.tabs[pos].is_pinned = true;
                    let sc_url = self.tabs[pos].url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    let exists = self.pinned_shortcuts.iter().any(|s| {
                        s.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&s.url) == sc_domain)
                    });
                    if !exists {
                        self.pinned_shortcuts.push(TabItem {
                            id: format!("pin-{}", self.tabs[pos].id),
                            title: self.tabs[pos].title.clone(),
                            url: sc_url,
                            accent: self.tabs[pos].accent,
                            favicon: self.tabs[pos].favicon.clone(),
                            is_loaded: true,
                            is_muted: self.tabs[pos].is_muted,
                            anim_progress: 1.0,
                            is_closing: false,
                            folder_id: None,
                        });
                    }
                    self.window.show_toast("Added to Favourite tabs");
                    self.sync_tabs_ui();
                    self.mark_dirty();
                }
            }
            AppEvent::UnpinTab(idx) => {
                if idx < self.pinned_shortcuts.len() {
                    let removed = self.pinned_shortcuts.remove(idx);
                    let rem_domain = ui::domain_of(&removed.url);
                    let open_tab_pos = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == removed.url || (!rem_domain.is_empty() && ui::domain_of(&t.url) == rem_domain))
                    });
                    if let Some(pos) = open_tab_pos {
                        self.tabs[pos].is_pinned = false;
                    } else {
                        // Unpinning an inactive pinned tab turns it into a normal tab in sleeping stage!
                        self.tabs.push(Tab {
                            id: format!("tab-{}", self.next_tab_id),
                            title: removed.title,
                            url: removed.url,
                            accent: removed.accent,
                            favicon: removed.favicon,
                            is_pinned: false,
                            folder_id: None,
                            custom_title: false,
                            is_muted: false,
                            controller: None, // Sleeping!
                            last_active: Instant::now(),
                            anim_progress: 1.0,
                            is_closing: false,
                        });
                        self.next_tab_id += 1;
                    }
                    self.window.show_toast("Removed from Favourite tabs");
                    self.sync_tabs_ui();
                    self.mark_dirty();
                }
            }
            AppEvent::DuplicateTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    let url = self.tabs[pos].url.clone();
                    self.create_tab(&url, false);
                }
            }
            AppEvent::PromptRenameTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx);
                if let Some((_, tab)) = target {
                    let current_title = tab.title.clone();
                    self.window.start_inline_rename_tab(idx, &current_title);
                }
            }
            AppEvent::RenameTab(idx, new_title) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                let real_idx = target.unwrap_or(idx);
                if real_idx < self.tabs.len() {
                    let trimmed = new_title.trim();
                    if !trimmed.is_empty() {
                        self.tabs[real_idx].title = trimmed.to_string();
                        self.tabs[real_idx].custom_title = true;
                    }
                    self.sync_tabs_ui();
                    self.mark_dirty();
                }
            }
            AppEvent::CloseTab(idx) => {
                let target = if idx == usize::MAX {
                    Some(self.active_tab_idx)
                } else {
                    self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i)
                };
                if let Some(pos) = target {
                    if self.tabs[pos].is_pinned {
                        let tab_url = self.tabs[pos].url.clone();
                        let tab_domain = ui::domain_of(&tab_url);
                        if let Some(ref fav) = self.tabs[pos].favicon {
                            for sc in &mut self.pinned_shortcuts {
                                if sc.url == tab_url || (!tab_domain.is_empty() && ui::domain_of(&sc.url) == tab_domain) {
                                    sc.favicon = Some(fav.clone());
                                }
                            }
                        }
                        self.closed_tabs.push(ClosedTab {
                            url: self.tabs[pos].url.clone(),
                            title: self.tabs[pos].title.clone(),
                            is_pinned: true,
                        });
                        if self.closed_tabs.len() > 30 {
                            self.closed_tabs.remove(0);
                        }

                        if let Some(ref ctrl) = self.tabs[pos].controller {
                            let _ = ctrl.set_visible(false);
                        }
                        self.tabs.remove(pos);

                        if self.tabs.is_empty() {
                            unsafe {
                                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    Some(self.window.hwnd()),
                                    windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                                    windows::Win32::Foundation::WPARAM(0),
                                    windows::Win32::Foundation::LPARAM(0),
                                );
                            }
                            return;
                        } else {
                            if self.active_tab_idx >= self.tabs.len() {
                                self.active_tab_idx = self.tabs.len() - 1;
                            } else if self.active_tab_idx > pos {
                                self.active_tab_idx -= 1;
                            }
                            self.ensure_tab_controller(self.active_tab_idx);
                            let (content_rect, _) = self.window.get_layout_rects();
                            if let Some(ref active) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                                let _ = active.set_bounds(content_rect);
                                let _ = active.set_visible(true);
                            }
                        }
                        self.sync_tabs_ui();
                        self.persist_state();
                    } else {
                        self.close_tab(pos);
                    }
                }
            }
            AppEvent::CreateTabAt(url, at_idx) => {
                let clean = engine::clean_url(&url);
                self.create_tab(&clean, false);
                // Move the newly created tab from the end to at_idx if valid
                if self.tabs.len() > 1 && at_idx < self.tabs.len() {
                    let last_idx = self.tabs.len() - 1;
                    let new_tab = self.tabs.remove(last_idx);
                    let insert_pos = at_idx.min(self.tabs.len());
                    self.tabs.insert(insert_pos, new_tab);
                    self.switch_tab(insert_pos);
                }
            }
            AppEvent::ReopenClosedTab => {
                if let Some(closed) = self.closed_tabs.pop() {
                    self.create_tab(&closed.url, closed.is_pinned);
                    self.window.show_toast(&format!("Reopened: {}", closed.title));
                } else {
                    self.window.show_toast("No closed tabs to reopen");
                }
            }
            AppEvent::ToggleMuteTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    self.tabs[pos].is_muted = !self.tabs[pos].is_muted;
                    let muted = self.tabs[pos].is_muted;
                    if let Some(ref ctrl) = self.tabs[pos].controller {
                        let _ = ctrl.set_muted(muted);
                    }
                    if muted {
                        self.window.show_toast("Tab muted");
                    } else {
                        self.window.show_toast("Tab unmuted");
                    }
                    self.sync_tabs_ui();
                }
            }
            AppEvent::ToggleMutePinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    let sc_url = sc.url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    let open_tab_pos = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    });
                    if let Some(pos) = open_tab_pos {
                        self.tabs[pos].is_muted = !self.tabs[pos].is_muted;
                        let muted = self.tabs[pos].is_muted;
                        if let Some(ref ctrl) = self.tabs[pos].controller {
                            let _ = ctrl.set_muted(muted);
                        }
                        if muted {
                            self.window.show_toast("Pinned tab muted");
                        } else {
                            self.window.show_toast("Pinned tab unmuted");
                        }
                        self.sync_tabs_ui();
                    } else {
                        self.create_tab(&sc_url, true);
                        if let Some(last) = self.tabs.last_mut() {
                            last.is_muted = true;
                            if let Some(ref ctrl) = last.controller {
                                let _ = ctrl.set_muted(true);
                            }
                        }
                        self.window.show_toast("Pinned tab muted");
                        self.sync_tabs_ui();
                    }
                }
            }
            AppEvent::UnloadTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    if self.tabs[pos].controller.is_some() {
                        if pos == self.active_tab_idx && self.tabs.len() > 1 {
                            let alt_idx = if pos == 0 { 1 } else { pos - 1 };
                            self.switch_tab(alt_idx);
                        }
                        if let Some(tab) = self.tabs.get_mut(pos) {
                            if let Some(ctrl) = tab.controller.take() {
                                let _ = ctrl.set_visible(false);
                                drop(ctrl);
                            }
                        }
                        self.window.show_toast("Tab unloaded (sleeping)");
                        self.sync_tabs_ui();
                        self.mark_dirty();
                    }
                }
            }
            AppEvent::ShareTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    let tab = &self.tabs[pos];
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let to_copy = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    copy_to_clipboard(self.window.hwnd(), &to_copy);
                    self.window.show_toast("Link copied to clipboard!");
                }
            }
            AppEvent::SharePinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    copy_to_clipboard(self.window.hwnd(), &sc.url);
                    self.window.show_toast("Link copied to clipboard!");
                }
            }
            AppEvent::CopyCleanTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    let tab = &self.tabs[pos];
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let raw_url = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    let clean = engine::clean_url(&raw_url);
                    copy_to_clipboard(self.window.hwnd(), &clean);
                    if clean != raw_url {
                        self.window.show_toast("Clean link copied (tracking parameters removed)!");
                    } else {
                        self.window.show_toast("Clean link copied to clipboard!");
                    }
                }
            }
            AppEvent::CopyCleanPinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    let clean = engine::clean_url(&sc.url);
                    copy_to_clipboard(self.window.hwnd(), &clean);
                    if clean != sc.url {
                        self.window.show_toast("Clean link copied (tracking parameters removed)!");
                    } else {
                        self.window.show_toast("Clean link copied to clipboard!");
                    }
                }
            }
            AppEvent::CloseOtherTabs(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(keep_pos) = target {
                    let keep_id = self.tabs[keep_pos].id.clone();
                    let mut i = 0;
                    while i < self.tabs.len() {
                        if !self.tabs[i].is_pinned && self.tabs[i].id != keep_id {
                            self.closed_tabs.push(ClosedTab {
                                url: self.tabs[i].url.clone(),
                                title: self.tabs[i].title.clone(),
                                is_pinned: false,
                            });
                            if let Some(ref ctrl) = self.tabs[i].controller {
                                let _ = ctrl.set_visible(false);
                            }
                            self.tabs.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    if let Some(new_active) = self.tabs.iter().position(|t| t.id == keep_id) {
                        self.switch_tab(new_active);
                    }
                    self.sync_tabs_ui();
                    self.persist_state();
                    self.window.show_toast("Closed other tabs");
                }
            }
            AppEvent::ReloadSpecificTab(idx) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    if let Some(ref ctrl) = self.tabs[pos].controller {
                        let _ = ctrl.reload();
                    } else {
                        self.ensure_tab_controller(pos);
                    }
                }
            }
            AppEvent::ReloadSpecificPinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    let sc_url = sc.url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    let open_tab_pos = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    });
                    if let Some(pos) = open_tab_pos {
                        if let Some(ref ctrl) = self.tabs[pos].controller {
                            let _ = ctrl.reload();
                        } else {
                            self.ensure_tab_controller(pos);
                        }
                    } else {
                        self.create_tab(&sc_url, true);
                    }
                }
            }
            AppEvent::Navigate(url) => {
                let url = engine::clean_url(&url);
                self.window.set_loading(true, 0.25);
                let domain = ui::domain_of(&url);
                let cached_fav = self.get_cached_favicon(&domain);

                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.url = url.clone();
                    tab.accent = accent_for_url(&url);
                    tab.title = friendly_title_for_url(&url);
                    tab.favicon = cached_fav;

                    // Navigate webview immediately and focus
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.navigate(&url);
                        let _ = ctrl.focus();
                    }
                }
                if let Ok(st) = self.storage.try_borrow() {
                    let _ = st.add_history(&url, &friendly_title_for_url(&url));
                }
                self.sync_tabs_ui();
                self.mark_dirty();
            }
            AppEvent::NavBack => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.go_back();
                        self.sync_tabs_ui();
                    }
                }
            }
            AppEvent::NavForward => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.go_forward();
                        self.sync_tabs_ui();
                    }
                }
            }
            AppEvent::NavReload => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.reload();
                    }
                }
            }
            AppEvent::ToggleSidebar => {
                self.window.toggle_sidebar();
                let (content_rect, _) = self.window.get_layout_rects();
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.set_bounds(content_rect);
                    }
                }
            }
            AppEvent::OpenCommandBar(mode) => {
                let (content_rect, _) = self.window.get_layout_rects();
                if let Some(ref cb) = *self.command_bar.borrow() {
                    let initial = if mode == CommandBarMode::NavigateCurrent {
                        self.tabs.get(self.active_tab_idx).map(|t| t.url.as_str())
                    } else {
                        None
                    };
                    cb.toggle(content_rect, mode, initial);
                }
            }
            AppEvent::TitleChanged { tab_id, title } => {
                let mut changed = false;
                let mut url_to_save = String::new();
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if !tab.custom_title && !title.is_empty() && tab.title != title {
                        tab.title = title.clone();
                        url_to_save = tab.url.clone();
                        changed = true;
                    }
                }
                if !url_to_save.is_empty() {
                    if let Ok(st) = self.storage.try_borrow() {
                        let _ = st.add_history(&url_to_save, &title);
                    }
                }
                if changed {
                    self.sync_tabs_ui();
                    self.mark_dirty();
                }
            }
            AppEvent::UrlChanged { tab_id, url } => {
                let mut changed = false;
                let mut title_to_save = String::new();
                let mut domain_change = None;

                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if tab.url != url {
                        let old_domain = ui::domain_of(&tab.url);
                        let new_domain = ui::domain_of(&url);
                        tab.url = url.clone();
                        tab.accent = accent_for_url(&url);

                        if old_domain != new_domain && !new_domain.is_empty() {
                            tab.custom_title = false;
                            domain_change = Some(new_domain);
                        }

                        if !tab.custom_title {
                            if let Some(ref ctrl) = tab.controller {
                                if let Ok(ctrl_title) = ctrl.get_title() {
                                    if !ctrl_title.is_empty() && ctrl_title != "about:blank" {
                                        tab.title = ctrl_title;
                                    } else if tab.title == "New Tab" || tab.title.is_empty() {
                                        tab.title = friendly_title_for_url(&url);
                                    }
                                }
                            } else if tab.title == "New Tab" || tab.title.is_empty() {
                                tab.title = friendly_title_for_url(&url);
                            }
                        }
                        title_to_save = tab.title.clone();
                        changed = true;
                    }
                }

                if let Some(new_dom) = domain_change {
                    let fav = self.get_cached_favicon(&new_dom);
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.favicon = fav;
                    }
                }

                if !url.is_empty() && !title_to_save.is_empty() {
                    if let Ok(st) = self.storage.try_borrow() {
                        let _ = st.add_history(&url, &title_to_save);
                    }
                }
                if changed {
                    self.sync_tabs_ui();
                    self.mark_dirty();
                }
            }
            AppEvent::FaviconChanged { tab_id, bytes } => {
                if let Some(fav_data) = ui::decode_favicon(&bytes) {
                    let mut changed = false;
                    let mut url_to_match = String::new();
                    let new_res = fav_data.width * fav_data.height;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        url_to_match = tab.url.clone();
                        let existing_res = tab.favicon.as_ref().map_or(0, |f| f.width * f.height);
                        if new_res >= existing_res || tab.favicon.is_none() {
                            tab.favicon = Some(fav_data.clone());
                            changed = true;
                        }
                    }
                    for sc in &mut self.pinned_shortcuts {
                        let sc_domain = ui::domain_of(&sc.url);
                        let is_match = sc.id == tab_id
                            || (!url_to_match.is_empty() && (sc.url == url_to_match || (!sc_domain.is_empty() && sc_domain == ui::domain_of(&url_to_match))));
                        if is_match {
                            let existing_res = sc.favicon.as_ref().map_or(0, |f| f.width * f.height);
                            if new_res >= existing_res || sc.favicon.is_none() {
                                sc.favicon = Some(fav_data.clone());
                                changed = true;
                            }
                            if url_to_match.is_empty() {
                                url_to_match = sc.url.clone();
                            }
                        }
                    }
                    if !url_to_match.is_empty() {
                        let domain = ui::domain_of(&url_to_match);
                        self.window.update_pinned_favicon(&url_to_match, fav_data.clone());
                        if !domain.is_empty() {
                            self.favicon_cache.insert(domain.clone(), fav_data.clone());
                            if let Some(base) = get_base_domain(&domain) {
                                self.favicon_cache.entry(base).or_insert_with(|| fav_data.clone());
                            }
                            if let Some(ref cb) = *self.command_bar.borrow() {
                                cb.update_favicon(&domain, fav_data.clone());
                            }
                            // Save to disk cache if resolution is valid (>= 16px)
                            if fav_data.width >= 16 {
                                if let Ok(st) = self.storage.try_borrow() {
                                    let _ = st.save_favicon(&domain, &bytes);
                                }
                            }
                        }
                    }
                    if changed {
                        self.sync_tabs_ui();
                    }
                }
            }
            AppEvent::NavigationStarted { tab_id } => {
                let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                if is_active {
                    self.window.set_loading(true, 0.45);
                }
            }
            AppEvent::NavigationCompleted { tab_id, .. } => {
                let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                if is_active {
                    self.window.set_loading(false, 1.0);
                }
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        if let Ok(st) = self.storage.try_borrow() {
                            if let Ok(css) = st.get_hidden_elements_css(&domain, None) {
                                if !css.is_empty() {
                                    if let Some(ref ctrl) = tab.controller {
                                        let _ = ctrl.apply_zapper_rules(&css);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AppEvent::HardReload => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.reload();
                    }
                }
            }
            AppEvent::CopyUrl => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let to_copy = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    copy_to_clipboard(self.window.hwnd(), &to_copy);
                    self.window.show_toast("Copied current URL!");
                }
            }
            AppEvent::CopyCleanLink => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let raw_url = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    let clean = engine::clean_url(&raw_url);
                    copy_to_clipboard(self.window.hwnd(), &clean);
                    if clean != raw_url {
                        self.window.show_toast("Clean link copied (tracking parameters removed)!");
                    } else {
                        self.window.show_toast("Clean link copied to clipboard!");
                    }
                }
            }
            AppEvent::TogglePinCurrentTab => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.is_pinned = !tab.is_pinned;
                    if tab.is_pinned {
                        self.window.show_toast("Added to Favourite tabs");
                    } else {
                        self.window.show_toast("Removed from Favourite tabs");
                    }
                }
                self.sync_tabs_ui();
                self.mark_dirty();
            }
            AppEvent::OpenSettings => {
                self.create_tab("https://www.google.com/preferences", false);
            }
            AppEvent::OpenDownloads => {
                if let Ok(user_profile) = std::env::var("USERPROFILE") {
                    let dl = std::path::PathBuf::from(user_profile).join("Downloads");
                    let _ = std::process::Command::new("explorer").arg(dl).spawn();
                }
            }
            AppEvent::ZoomIn => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let curr = ctrl.get_zoom_factor();
                        let new_z = ((curr + 0.1) * 10.0).round() / 10.0;
                        let new_z = new_z.min(3.0);
                        let _ = ctrl.set_zoom_factor(new_z);
                        self.window.set_zoom(new_z);
                        self.window.show_toast(&format!("Zoom: {}%", (new_z * 100.0).round() as i32));
                    }
                }
            }
            AppEvent::ZoomOut => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let curr = ctrl.get_zoom_factor();
                        let new_z = ((curr - 0.1) * 10.0).round() / 10.0;
                        let new_z = new_z.max(0.3);
                        let _ = ctrl.set_zoom_factor(new_z);
                        self.window.set_zoom(new_z);
                        self.window.show_toast(&format!("Zoom: {}%", (new_z * 100.0).round() as i32));
                    }
                }
            }
            AppEvent::ZoomReset => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.set_zoom_factor(1.0);
                        self.window.set_zoom(1.0);
                        self.window.show_toast("Zoom: 100% (Reset)");
                    }
                }
            }
            AppEvent::NextTab => {
                if !self.tabs.is_empty() {
                    let next = (self.active_tab_idx + 1) % self.tabs.len();
                    self.switch_tab(next);
                }
            }
            AppEvent::PrevTab => {
                if !self.tabs.is_empty() {
                    let prev = (self.active_tab_idx + self.tabs.len() - 1) % self.tabs.len();
                    self.switch_tab(prev);
                }
            }
            AppEvent::FullScreenChanged(is_full) => {
                let hwnd = self.window.hwnd();
                let mut client_rect = RECT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client_rect);
                }

                if is_full {
                    if self.sidebar_before_fullscreen.is_none() {
                        self.sidebar_before_fullscreen = Some(self.window.is_sidebar_visible());
                    }
                    self.window.set_sidebar_visible(false);
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.set_bounds(client_rect);
                            ctrl.set_fullscreen_mode(true);
                        }
                    }
                } else {
                    let prev_vis = self.sidebar_before_fullscreen.take().unwrap_or(true);
                    self.window.set_sidebar_visible(prev_vis);
                    let (content_rect, _) = self.window.get_layout_rects();
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.set_bounds(content_rect);
                            ctrl.set_fullscreen_mode(false);
                        }
                    }
                }
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
                }
            }
            AppEvent::TabAnimTick => {
                self.on_tab_anim_tick();
            }
            AppEvent::CreateFolder => {
                let id = format!("f-{}", Instant::now().elapsed().as_nanos());
                let new_name = "New Folder".to_string();
                self.folders.push(Folder {
                    id: id.clone(),
                    name: new_name.clone(),
                    icon: "📁".to_string(),
                    is_open: true,
                    open_progress: 1.0,
                });
                self.sync_tabs_ui();
                self.persist_state();
                self.window.start_inline_rename_folder(&id, &new_name);
            }
            AppEvent::ToggleFolder(idx) => {
                if let Some(folder) = self.folders.get_mut(idx) {
                    folder.is_open = !folder.is_open;
                    self.window.start_tab_anim_timer();
                    self.sync_tabs_ui();
                    self.persist_state();
                }
            }
            AppEvent::RenameFolder(id, new_name) => {
                if let Some(folder) = self.folders.iter_mut().find(|f| f.id == id) {
                    let trimmed = new_name.trim();
                    if !trimmed.is_empty() {
                        folder.name = trimmed.to_string();
                    }
                }
                self.sync_tabs_ui();
                self.persist_state();
            }
            AppEvent::ChangeFolderIcon(id, new_icon) => {
                if let Some(folder) = self.folders.iter_mut().find(|f| f.id == id) {
                    folder.icon = new_icon;
                }
                self.sync_tabs_ui();
                self.persist_state();
            }
            AppEvent::DeleteFolder(id) => {
                self.folders.retain(|f| f.id != id);
                for tab in &mut self.tabs {
                    if tab.folder_id.as_deref() == Some(&id) {
                        tab.folder_id = None;
                    }
                }
                self.sync_tabs_ui();
                self.persist_state();
                self.window.show_toast("Folder deleted");
            }
            AppEvent::MoveTabToFolder(tab_idx, folder_id) => {
                let target = self.tabs.iter().enumerate().filter(|(_, t)| !t.is_pinned).nth(tab_idx).map(|(i, _)| i);
                if let Some(pos) = target {
                    self.tabs[pos].folder_id = folder_id;
                    self.sync_tabs_ui();
                    self.persist_state();
                }
            }
            AppEvent::NewTabInFolder(folder_id) => {
                self.create_tab_in_folder("https://www.google.com", Some(folder_id));
            }
            AppEvent::SelectTabId(tab_id) => {
                if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                    self.switch_tab(pos);
                }
            }
            AppEvent::ToggleZapper => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.toggle_zapper();
                    }
                }
            }
            AppEvent::ZapperStatus { tab_id, active } => {
                let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                if is_active {
                    if active {
                        self.window.show_toast("Element Zapper: Point & click to hide (Esc to exit, Ctrl+Z to undo)");
                    } else {
                        self.window.show_toast("Element Zapper closed");
                    }
                }
            }
            AppEvent::ZapperRequestCss { tab_id } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        if let Ok(st) = self.storage.try_borrow() {
                            if let Ok(css) = st.get_hidden_elements_css(&domain, None) {
                                if !css.is_empty() {
                                    if let Some(ref ctrl) = tab.controller {
                                        let _ = ctrl.apply_zapper_rules(&css);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AppEvent::ElementZapped { tab_id, selector, label, note } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        let note_opt = if note.is_empty() { None } else { Some(note.as_str()) };
                        let mut ok = false;
                        if let Ok(st) = self.storage.try_borrow() {
                            if st.add_hidden_element(&domain, &selector, &label, note_opt).is_ok() {
                                ok = true;
                                if let Ok(css) = st.get_hidden_elements_css(&domain, None) {
                                    if let Some(ref ctrl) = tab.controller {
                                        let _ = ctrl.apply_zapper_rules(&css);
                                    }
                                }
                            }
                        }
                        if ok {
                            let msg = if label.is_empty() {
                                "Hidden element on this site (Ctrl+Z to undo)".to_string()
                            } else {
                                format!("Hidden \"{}\" (Ctrl+Z to undo)", label)
                            };
                            self.window.show_toast(&msg);
                        }
                    }
                }
            }
            AppEvent::ZapperUndo { tab_id } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        let mut undone = None;
                        if let Ok(st) = self.storage.try_borrow() {
                            if let Ok(elem_opt) = st.pop_last_hidden_element(&domain) {
                                undone = elem_opt;
                                if let Ok(remaining_css) = st.get_hidden_elements_css(&domain, None) {
                                    if let Some(ref ctrl) = tab.controller {
                                        let _ = ctrl.apply_zapper_rules(&remaining_css);
                                    }
                                }
                            }
                        }
                        if let Some(elem) = undone {
                            let msg = if elem.label.is_empty() {
                                "Restored hidden element".to_string()
                            } else {
                                format!("Restored \"{}\"", elem.label)
                            };
                            self.window.show_toast(&msg);
                        } else {
                            self.window.show_toast("Nothing to undo on this site");
                        }
                    }
                }
            }
            AppEvent::RestoreAllHiddenElements => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        let mut count = 0;
                        if let Ok(st) = self.storage.try_borrow() {
                            if let Ok(deleted) = st.clear_hidden_elements(&domain) {
                                count = deleted;
                            }
                        }
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.apply_zapper_rules("");
                        }
                        if count > 0 {
                            self.window.show_toast(&format!(
                                "Restored {} hidden element{} on {}",
                                count,
                                if count == 1 { "" } else { "s" },
                                domain
                            ));
                        } else {
                            self.window.show_toast(&format!("No hidden elements on {}", domain));
                        }
                    }
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ui::enable_process_dark_mode();
    let t0 = Instant::now();

    // Initialize tracing
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();

    info!("Starting Browser for Windows...");

    let app_data_dir = get_app_data_dir();
    let webview_data_dir = app_data_dir.join("WebView2Data");
    let storage_db_path = app_data_dir.join("browser.db");

    // Stage 1: COM Init
    let t_before_com = Instant::now();
    init_com()?;
    let t_after_com = Instant::now();
    log_timing(
        "1. COM Init",
        t_after_com.duration_since(t_before_com).as_secs_f64() * 1000.0,
        t_after_com.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 2: Storage Init
    let t_before_storage = Instant::now();
    let storage = Rc::new(RefCell::new(Storage::open(&storage_db_path)?));
    let t_after_storage = Instant::now();
    log_timing(
        "2. Storage (SQLite) Init",
        t_after_storage.duration_since(t_before_storage).as_secs_f64() * 1000.0,
        t_after_storage.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 3: Window Creation
    let t_before_window = Instant::now();
    let config = WindowConfig {
        title: "Browser".to_string(),
        width: 1280,
        height: 800,
        sidebar_width: 280,
        caption_height: 32,
    };
    let window = Rc::new(BrowserWindow::new(config)?);
    let t_after_window = Instant::now();
    log_timing(
        "3. Window & Custom Frame Creation",
        t_after_window.duration_since(t_before_window).as_secs_f64() * 1000.0,
        t_after_window.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 4: WebView2 Engine
    let t_before_env = Instant::now();
    let engine = Rc::new(WebViewEngine::new(Some(&webview_data_dir))?);
    let t_after_env = Instant::now();
    log_timing(
        "4. WebView2 Environment Created",
        t_after_env.duration_since(t_before_env).as_secs_f64() * 1000.0,
        t_after_env.duration_since(t0).as_secs_f64() * 1000.0,
    );

    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    let command_bar_cell = Rc::new(RefCell::new(None));

    let ext_dir = app_data_dir.join("extensions").join("ubol");
    // Remove the old hand-written blocker so only real uBOL loads
    let _ = std::fs::remove_dir_all(app_data_dir.join("extensions").join("ublock_origin_lite"));
    let ubol_ready = ensure_ubol(&ext_dir);

    // Create App state
    let app = Rc::new(RefCell::new(BrowserApp::new(
        window.clone(),
        command_bar_cell.clone(),
        engine.clone(),
        storage.clone(),
        event_tx.clone(),
        // Set NO_ADBLOCK=1 or BROWSER_NO_ADBLOCK=1 to start without the extension (for isolating issues)
        if std::env::var_os("NO_ADBLOCK").is_some() || std::env::var_os("BROWSER_NO_ADBLOCK").is_some() || std::env::var_os("ARC_NO_ADBLOCK").is_some() || !ubol_ready { None } else { Some(ext_dir) },
    )));

    // Create Command Bar
    let hwnd = window.hwnd();
    let tx_cb = event_tx.clone();
    let command_bar = CommandBar::new(
        hwnd,
        Some(storage.clone()),
        Box::new(move |action| {
            let ev = match action {
                CommandAction::OpenInNewTab(url) => AppEvent::CreateTab(engine::clean_url(&url)),
                CommandAction::Navigate(url) => AppEvent::Navigate(engine::clean_url(&url)),
                CommandAction::RenameTab(idx, title) => AppEvent::RenameTab(idx, title),
                CommandAction::NewTab => AppEvent::CreateTab("https://www.google.com".to_string()),
                CommandAction::CloseTab => AppEvent::CloseTab(usize::MAX),
                CommandAction::ToggleSidebar => AppEvent::ToggleSidebar,
                CommandAction::ToggleZapper => AppEvent::ToggleZapper,
                CommandAction::RestoreAllHidden => AppEvent::RestoreAllHiddenElements,
                CommandAction::CopyCleanLink => AppEvent::CopyCleanLink,
            };
            let _ = tx_cb.send(ev);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        }),
    )?;
    *command_bar_cell.borrow_mut() = Some(command_bar);

    // Hook Window sidebar actions
    let tx_action = event_tx.clone();
    window.set_action_callback(move |action| {
        let ev = match action {
            SidebarAction::NavBack => AppEvent::NavBack,
            SidebarAction::NavForward => AppEvent::NavForward,
            SidebarAction::NavReload => AppEvent::NavReload,
            SidebarAction::NavNewTab => AppEvent::OpenCommandBar(CommandBarMode::NewTab),
            SidebarAction::ToggleSidebar => AppEvent::ToggleSidebar,
            SidebarAction::UrlBarClicked => AppEvent::OpenCommandBar(CommandBarMode::NavigateCurrent),
            SidebarAction::SelectTab(idx) => AppEvent::SwitchTab(idx),
            SidebarAction::CloseTab(idx) => AppEvent::CloseTab(idx),
            SidebarAction::ClosePinnedTab(idx) => AppEvent::ClosePinnedTab(idx),
            SidebarAction::SelectPinned(idx) => AppEvent::SelectPinned(idx),
            SidebarAction::PinTab(idx) => AppEvent::PinTab(idx),
            SidebarAction::UnpinTab(idx) => AppEvent::UnpinTab(idx),
            SidebarAction::DuplicateTab(idx) => AppEvent::DuplicateTab(idx),
            SidebarAction::PromptRenameTab(idx) => AppEvent::PromptRenameTab(idx),
            SidebarAction::RenameTab(idx, title) => AppEvent::RenameTab(idx, title),
            SidebarAction::CopyUrl => AppEvent::CopyUrl,
            SidebarAction::CopyCleanLink(idx) => AppEvent::CopyCleanTab(idx),
            SidebarAction::CopyCleanPinnedTab(idx) => AppEvent::CopyCleanPinnedTab(idx),
            SidebarAction::CopyCleanLinkActive => AppEvent::CopyCleanLink,
            SidebarAction::TogglePinCurrentTab => AppEvent::TogglePinCurrentTab,
            SidebarAction::OpenSettings => AppEvent::OpenSettings,
            SidebarAction::OpenDownloads => AppEvent::OpenDownloads,
            SidebarAction::ZoomIn => AppEvent::ZoomIn,
            SidebarAction::ZoomOut => AppEvent::ZoomOut,
            SidebarAction::ZoomReset => AppEvent::ZoomReset,
            SidebarAction::NextTab => AppEvent::NextTab,
            SidebarAction::PrevTab => AppEvent::PrevTab,
            SidebarAction::HardReload => AppEvent::HardReload,
            SidebarAction::ReopenClosedTab => AppEvent::ReopenClosedTab,
            SidebarAction::ToggleMuteTab(idx) => AppEvent::ToggleMuteTab(idx),
            SidebarAction::ToggleMutePinnedTab(idx) => AppEvent::ToggleMutePinnedTab(idx),
            SidebarAction::UnloadTab(idx) => AppEvent::UnloadTab(idx),
            SidebarAction::ShareTab(idx) => AppEvent::ShareTab(idx),
            SidebarAction::SharePinnedTab(idx) => AppEvent::SharePinnedTab(idx),
            SidebarAction::CloseOtherTabs(idx) => AppEvent::CloseOtherTabs(idx),
            SidebarAction::NewTabBelow(idx) => AppEvent::CreateTabAt("https://www.google.com".to_string(), idx + 1),
            SidebarAction::ReloadSpecificTab(idx) => AppEvent::ReloadSpecificTab(idx),
            SidebarAction::ReloadSpecificPinnedTab(idx) => AppEvent::ReloadSpecificPinnedTab(idx),
            SidebarAction::TabAnimTick => AppEvent::TabAnimTick,
            SidebarAction::CreateFolder => AppEvent::CreateFolder,
            SidebarAction::ToggleFolder(idx) => AppEvent::ToggleFolder(idx),
            SidebarAction::RenameFolder(id, name) => AppEvent::RenameFolder(id, name),
            SidebarAction::ChangeFolderIcon(id, icon) => AppEvent::ChangeFolderIcon(id, icon),
            SidebarAction::DeleteFolder(id) => AppEvent::DeleteFolder(id),
            SidebarAction::MoveTabToFolder(idx, fid) => AppEvent::MoveTabToFolder(idx, fid),
            SidebarAction::NewTabInFolder(fid) => AppEvent::NewTabInFolder(fid),
            SidebarAction::SelectTabId(tid) => AppEvent::SelectTabId(tid),
            SidebarAction::ToggleZapper => AppEvent::ToggleZapper,
            SidebarAction::RestoreAllHidden => AppEvent::RestoreAllHiddenElements,
        };
        let _ = tx_action.send(ev);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
        }
    });

    // Hook Window resize to sync active tab & command bar
    let app_for_resize = app.clone();
    let cb_for_resize = command_bar_cell.clone();
    window.set_resize_callback(move |content_rect, _sidebar_rect| {
        if let Ok(app) = app_for_resize.try_borrow() {
            if let Some(tab) = app.tabs.get(app.active_tab_idx) {
                if let Some(ref ctrl) = tab.controller {
                    let _ = ctrl.set_bounds(content_rect);
                }
            }
        }
        if let Ok(cb_opt) = cb_for_resize.try_borrow() {
            if let Some(ref cb) = *cb_opt {
                if cb.is_visible() {
                    cb.recenter(content_rect);
                }
            }
        }
    });

    // Handle custom WM_APP messages to dispatch events
    let app_for_events = app.clone();
    let rx_cell = Rc::new(RefCell::new(event_rx));
    window.set_custom_msg_handler(move |msg, _wparam, _lparam| {
        if msg >= WM_APP {
            if let Ok(rx) = rx_cell.try_borrow() {
                while let Ok(event) = rx.try_recv() {
                    if let Ok(mut app) = app_for_events.try_borrow_mut() {
                        app.handle_event(event);
                    }
                }
            }
            // Flush debounced persist if enough time has passed
            if let Ok(mut app) = app_for_events.try_borrow_mut() {
                app.flush_if_dirty();
            }
        } else if msg == windows::Win32::UI::WindowsAndMessaging::WM_CLOSE {
            if let Ok(mut app) = app_for_events.try_borrow_mut() {
                app.persist_state(); // Force-flush on exit
            }
        }
    });

    // Initial tab
    let has_saved_tabs = !app.borrow().tabs.is_empty();
    let t_before_tab = Instant::now();
    if !has_saved_tabs {
        let target_url = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "https://www.google.com".to_string());
        app.borrow_mut().create_tab(&target_url, false);
    } else {
        let active_idx = app.borrow().active_tab_idx;
        app.borrow_mut().ensure_tab_controller(active_idx);
        let (content_rect, _) = window.get_layout_rects();
        if let Some(ref active) = app.borrow().tabs.get(active_idx).and_then(|t| t.controller.as_ref()) {
            let _ = active.set_bounds(content_rect);
            let _ = active.set_visible(true);
            let _ = active.focus();
        }
        app.borrow().sync_tabs_ui();
    }
    let t_after_tab = Instant::now();
    log_timing(
        "5. Initial Tab Created",
        t_after_tab.duration_since(t_before_tab).as_secs_f64() * 1000.0,
        t_after_tab.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Show window
    window.show();
    let t_shown = Instant::now();
    log_timing(
        "*. Window Visible (Interactive)",
        t_shown.duration_since(t_after_tab).as_secs_f64() * 1000.0,
        t_shown.duration_since(t0).as_secs_f64() * 1000.0,
    );

    info!("Entering message loop...");
    window.run_message_loop();

    info!("Browser exiting.");
    Ok(())
}
