use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use engine::{init_com, BrowserShortcut, ReaderLook, ReaderMessage, WebViewController, WebViewEngine, ZapperMessage};
use storage::Storage;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ui::{
    accent_for_url, friendly_title_for_url, BrowserWindow, CommandAction, CommandBar,
    CommandBarMode, SidebarAction, SiteCardAction, SiteCardData, TabItem, WindowConfig,
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
    custom_letter: Option<String>,
    is_muted: bool,
    controller: Option<WebViewController>,
    last_active: Instant,
    anim_progress: f32,
    is_closing: bool,
    pending_permissions: Vec<ui::SitePermissionKind>,
    is_reader: bool,
    seeking_reader_twin: bool,
    reader_gen: u64,
    reader_markdown: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TabSplit {
    pub id: String,
    pub tabs: Vec<String>,     // 2 to 4 tab IDs in order (left to right)
    pub sizes: Vec<f32>,       // Fractions adding up to 1.0
    pub focused_tab_id: String,
}

impl TabSplit {
    pub const MOST: usize = 4;

    pub fn new(id: String, tabs: Vec<String>, focused_tab_id: String) -> Self {
        let count = tabs.len();
        let sizes = Self::even(count);
        Self {
            id,
            tabs,
            sizes,
            focused_tab_id,
        }
    }

    pub fn with_sizes(id: String, tabs: Vec<String>, sizes: Vec<f32>, focused_tab_id: String) -> Self {
        let count = tabs.len();
        let clamped = if sizes.len() == count {
            Self::clamp(&sizes)
        } else {
            Self::even(count)
        };
        Self {
            id,
            tabs,
            sizes: clamped,
            focused_tab_id,
        }
    }

    pub fn left_tab_id(&self) -> &str {
        self.tabs.first().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn right_tab_id(&self) -> &str {
        self.tabs.last().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn ratio(&self) -> f32 {
        self.sizes.first().copied().unwrap_or(0.5)
    }

    pub fn is_full(&self) -> bool {
        self.tabs.len() >= Self::MOST
    }

    pub fn contains(&self, tab_id: &str) -> bool {
        self.tabs.iter().any(|t| t == tab_id)
    }

    pub fn least(count: usize) -> f32 {
        if count <= 2 { 0.20 } else { 0.10 }
    }

    pub fn even(count: usize) -> Vec<f32> {
        let count = count.max(1);
        vec![1.0 / count as f32; count]
    }

    pub fn clamp(sizes: &[f32]) -> Vec<f32> {
        let count = sizes.len();
        if count <= 1 {
            return vec![1.0];
        }
        if count == 2 {
            let first = sizes[0].clamp(0.20, 0.80);
            return vec![first, 1.0 - first];
        }
        let least = Self::least(count);
        let sum: f32 = sizes.iter().map(|&s| if s.is_finite() && s > 0.0 { s } else { 1.0 }).sum();
        let mut shares: Vec<f32> = if sum > 0.0 {
            sizes.iter().map(|&s| (if s.is_finite() && s > 0.0 { s } else { 1.0 }) / sum).collect()
        } else {
            Self::even(count)
        };

        let short: f32 = shares.iter().map(|&s| (least - s).max(0.0)).sum();
        if short > 0.0 {
            let spare: f32 = shares.iter().map(|&s| (s - least).max(0.0)).sum();
            if spare > 0.0 {
                shares = shares.iter().map(|&s| {
                    if s < least { least } else { s - (s - least) / spare * short }
                }).collect();
            }
        }
        shares
    }

    pub fn insert(&mut self, tab_id: String, beside: Option<&str>, before: bool) -> bool {
        if self.is_full() || self.contains(&tab_id) {
            return false;
        }
        let at = if let Some(target) = beside {
            self.tabs.iter().position(|t| t == target).map(|pos| if before { pos } else { pos + 1 }).unwrap_or(self.tabs.len())
        } else {
            self.tabs.len()
        };
        self.tabs.insert(at, tab_id);
        self.sizes = Self::even(self.tabs.len());
        true
    }

    pub fn remove(&mut self, tab_id: &str) -> bool {
        if let Some(pos) = self.tabs.iter().position(|t| t == tab_id) {
            self.tabs.remove(pos);
            if self.sizes.len() > pos {
                self.sizes.remove(pos);
            }
            if self.tabs.len() >= 2 {
                self.sizes = Self::clamp(&self.sizes);
                if self.focused_tab_id == tab_id {
                    self.focused_tab_id = self.tabs[0].clone();
                }
                return true;
            }
        }
        false
    }
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
    SetPinnedLetter(usize, String),
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
    NavigationStarted { tab_id: String, url: String },
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
    FullScreenChanged { tab_id: String, is_full: bool },
    PaneFocused(String),
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
    ToggleShield,
    ToggleSiteShield,
    ToggleSiteCard,
    #[allow(dead_code)]
    ToggleSiteKeepAwake,
    #[allow(dead_code)]
    ToggleSiteAutoplay,
    #[allow(dead_code)]
    SetSitePermission { perm: String, state: u8 },
    ClearSiteData,
    ConfirmClearSiteData,
    AcceptPermission(usize),
    CaptureScreenshot,
    ShareCurrentPage,
    DropTab {
        source: ui::DragSource,
        destination: ui::TabDropDestination,
    },
    OpenFolder(usize),
    OpenInSplitView(String),
    SplitWithNewTab,
    FocusOtherPane,
    SwapSplit,
    EvenSplit,
    SeparateSplit,
    CloseSplit,
    SplitRatioChanged(f32),
    SplitTabs(Vec<String>),
    SplitSizesChanged(Vec<f32>),
    OpenGlance {
        url: String,
        from_tab_id: Option<String>,
    },
    GlanceClose,
    GlanceOpenAsTab,
    GlanceOpenInSplit,
    GlanceTitleChanged {
        tab_id: String,
        title: String,
    },
    GlanceUrlChanged {
        tab_id: String,
        url: String,
    },
    GlanceFaviconChanged {
        tab_id: String,
        bytes: Vec<u8>,
    },
    GlanceNavigate {
        tab_id: String,
        url: String,
    },
    ToggleReader(Option<String>),
    ReaderMessage {
        tab_id: String,
        msg: ReaderMessage,
    },
    CycleReaderLook,
    CopyPageAsMarkdown(Option<String>),
    CopyMarkdownLink(Option<String>),
}

fn show_windows_share_ui(hwnd: HWND, title: &str, url: &str) -> windows::core::Result<()> {
    use windows::ApplicationModel::DataTransfer::{DataTransferManager, DataRequestedEventArgs};
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;
    use windows::Foundation::TypedEventHandler;
    use windows::core::HSTRING;

    unsafe {
        let interop: IDataTransferManagerInterop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
        let dtm: DataTransferManager = interop.GetForWindow(hwnd)?;

        let title_h = HSTRING::from(title);
        let url_h = HSTRING::from(url);

        let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(move |_sender, args| {
            if let Some(args) = args.as_ref() {
                let req = args.Request()?;
                let data = req.Data()?;
                let props = data.Properties()?;
                let _ = props.SetTitle(&title_h);
                if let Ok(uri) = windows::Foundation::Uri::CreateUri(&url_h) {
                    let _ = data.SetWebLink(&uri);
                }
                let _ = data.SetText(&url_h);
            }
            Ok(())
        });

        let _token = dtm.DataRequested(&handler)?;
        interop.ShowShareUIForWindow(hwnd)?;
    }
    Ok(())
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

fn capture_screenshot_to_clipboard(hwnd: HWND) {
    unsafe {
        use windows::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetDC, ReleaseDC,
            SelectObject, SRCCOPY,
        };
        use windows::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        let mut wr = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut wr);
        let w = wr.right - wr.left;
        let h = wr.bottom - wr.top;
        if w <= 0 || h <= 0 { return; }

        let hdc_window = GetDC(Some(hwnd));
        let hdc_mem = CreateCompatibleDC(Some(hdc_window));
        let hbmp = CreateCompatibleBitmap(hdc_window, w, h);
        let old_bmp = SelectObject(hdc_mem, hbmp.into());

        let _ = BitBlt(hdc_mem, 0, 0, w, h, Some(hdc_window), 0, 0, SRCCOPY);

        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(Some(hwnd), hdc_window);

        if OpenClipboard(Some(hwnd)).is_ok() {
            let _ = EmptyClipboard();
            let _ = SetClipboardData(2 /* CF_BITMAP */, Some(windows::Win32::Foundation::HANDLE(hbmp.0)));
            let _ = CloseClipboard();
        }
    }
}

struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab_idx: usize,
    pinned_shortcuts: Vec<TabItem>,
    folders: Vec<Folder>,
    splits: Vec<TabSplit>,
    next_tab_id: usize,
    window: Rc<BrowserWindow>,
    command_bar: Rc<RefCell<Option<CommandBar>>>,
    engine: Rc<WebViewEngine>,
    storage: Rc<RefCell<Storage>>,
    event_tx: mpsc::Sender<AppEvent>,
    favicon_cache: std::collections::HashMap<String, std::sync::Arc<ui::FaviconData>>,
    persist_dirty: bool,
    last_persist: Instant,
    closed_tabs: Vec<ClosedTab>,
    sidebar_before_fullscreen: Option<bool>,
    fullscreen_tab_id: Option<String>,
    last_anim_tick: Instant,
    shield_state: std::sync::Arc<engine::ShieldState>,
    glance_tab: Option<Tab>,
    glance_from_tab_id: Option<String>,
    glance_overlay: Option<Rc<ui::GlanceOverlay>>,
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
                        custom_letter: s.custom_letter,
                        is_split: false,
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
                    custom_letter: None,
                    is_split: false,
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
                    custom_letter: None,
                    is_split: false,
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
                    custom_letter: None,
                    is_split: false,
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
                custom_letter: s.custom_letter,
                is_muted: false,
                controller: None, // sleeping, will be activated if active tab
                last_active: Instant::now(),
                anim_progress: 1.0,
                is_closing: false,
                pending_permissions: Vec::new(),
                is_reader: false,
                seeking_reader_twin: false,
                reader_gen: 0,
                reader_markdown: None,
            });
        }

        let active_tab_idx = if !restored_tabs.is_empty() {
            saved_active_idx.min(restored_tabs.len() - 1)
        } else {
            0
        };

        let next_tab_id = max_id_num.max(restored_tabs.len()) + 1;

        let shield_enabled = storage.borrow().get_shield_enabled().unwrap_or(true);
        let shield_paused = storage.borrow().get_all_shield_paused().unwrap_or_default();
        let shield_state = std::sync::Arc::new(engine::ShieldState::new());
        shield_state.set_enabled(shield_enabled);
        for host in shield_paused {
            shield_state.set_paused(&host, true);
        }

        let saved_splits = if let Ok(st) = storage.try_borrow() {
            st.get_saved_splits().unwrap_or_default()
        } else {
            Vec::new()
        };
        let splits: Vec<TabSplit> = saved_splits
            .into_iter()
            .map(|s| {
                let tabs = s.get_tabs();
                let sizes = s.get_sizes();
                let focused = s.focused_tab_id.unwrap_or_else(|| tabs.first().cloned().unwrap_or_default());
                TabSplit::with_sizes(s.id, tabs, sizes, focused)
            })
            .collect();

        let mut app = Self {
            tabs: restored_tabs,
            active_tab_idx,
            pinned_shortcuts,
            folders,
            splits,
            next_tab_id,
            window,
            command_bar,
            engine,
            storage,
            event_tx,
            favicon_cache,
            persist_dirty: false,
            last_persist: Instant::now(),
            closed_tabs: Vec::new(),
            sidebar_before_fullscreen: None,
            fullscreen_tab_id: None,
            last_anim_tick: Instant::now(),
            shield_state,
            glance_tab: None,
            glance_from_tab_id: None,
            glance_overlay: None,
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
                    custom_letter: sc.custom_letter.clone(),
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
                        custom_letter: t.custom_letter.clone(),
                    });
                    pos += 1;
                }
            }
            let _ = st.save_tabs_state(&saved, self.active_tab_idx);

            let saved_splits: Vec<storage::SavedSplit> = self.splits.iter().map(|s| {
                storage::SavedSplit::new(s.id.clone(), s.tabs.clone(), s.sizes.clone(), Some(s.focused_tab_id.clone()))
            }).collect();
            let _ = st.save_splits(&saved_splits);
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

        let tab_id_new_win = tab_id.clone();
        let tx3 = self.event_tx.clone();
        let on_new_window = Box::new(move |target_url: String, is_shift: bool| {
            let ev = if is_shift {
                AppEvent::OpenGlance {
                    url: target_url,
                    from_tab_id: Some(tab_id_new_win.clone()),
                }
            } else {
                AppEvent::CreateTab(target_url)
            };
            let _ = tx3.send(ev);
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
                BrowserShortcut::ToggleReader => AppEvent::ToggleReader(None),
                BrowserShortcut::SplitView => AppEvent::SplitWithNewTab,
                BrowserShortcut::FocusOtherPane => AppEvent::FocusOtherPane,
                BrowserShortcut::SwapSplit => AppEvent::SwapSplit,
                BrowserShortcut::EvenSplit => AppEvent::EvenSplit,
                BrowserShortcut::SeparateSplit => AppEvent::SeparateSplit,
                BrowserShortcut::CloseSplit => AppEvent::CloseSplit,
                BrowserShortcut::CloseGlance => AppEvent::GlanceClose,
                BrowserShortcut::PromoteGlance => AppEvent::GlanceOpenAsTab,
            };
            let _ = tx4.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_start = tab_id.clone();
        let tx_start = self.event_tx.clone();
        let on_nav_start = Box::new(move |nav_url: String| {
            let _ = tx_start.send(AppEvent::NavigationStarted {
                tab_id: tab_id_clone_start.clone(),
                url: nav_url,
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

        let tab_id_clone_fs = tab_id.clone();
        let tx_fs = self.event_tx.clone();
        let on_fullscreen = Box::new(move |is_full: bool| {
            let _ = tx_fs.send(AppEvent::FullScreenChanged {
                tab_id: tab_id_clone_fs.clone(),
                is_full,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_focus = tab_id.clone();
        let tx_focus = self.event_tx.clone();
        let on_got_focus = Box::new(move || {
            let _ = tx_focus.send(AppEvent::PaneFocused(tab_id_clone_focus.clone()));
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

        let tab_id_clone_reader = tab_id.clone();
        let tx_reader = self.event_tx.clone();
        let on_reader = Box::new(move |msg: ReaderMessage| {
            let _ = tx_reader.send(AppEvent::ReaderMessage {
                tab_id: tab_id_clone_reader.clone(),
                msg,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let controller = match self.engine.create_controller(
            self.window.hwnd(),
            content_rect,
            url,
            Some(self.shield_state.clone()),
            Some(on_nav_start),
            Some(on_nav_complete),
            Some(on_title),
            Some(on_url),
            Some(on_favicon),
            Some(on_new_window),
            Some(on_shortcut),
            Some(on_fullscreen),
            Some(on_zapper),
            Some(on_reader),
            Some(on_got_focus),
        ) {
            Ok(c) => c,
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
            custom_letter: None,
            is_muted: false,
            controller: Some(controller),
            last_active: Instant::now(),
            anim_progress: if is_pinned { 1.0 } else { 0.05 },
            is_closing: false,
            pending_permissions: Vec::new(),
            is_reader: false,
            seeking_reader_twin: false,
            reader_gen: 0,
            reader_markdown: None,
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

        let tab_id_new_win = tab_id.clone();
        let tx3 = self.event_tx.clone();
        let on_new_window = Box::new(move |target_url: String, is_shift: bool| {
            let ev = if is_shift {
                AppEvent::OpenGlance {
                    url: target_url,
                    from_tab_id: Some(tab_id_new_win.clone()),
                }
            } else {
                AppEvent::CreateTab(target_url)
            };
            let _ = tx3.send(ev);
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
                BrowserShortcut::ToggleReader => AppEvent::ToggleReader(None),
                BrowserShortcut::SplitView => AppEvent::SplitWithNewTab,
                BrowserShortcut::FocusOtherPane => AppEvent::FocusOtherPane,
                BrowserShortcut::SwapSplit => AppEvent::SwapSplit,
                BrowserShortcut::EvenSplit => AppEvent::EvenSplit,
                BrowserShortcut::SeparateSplit => AppEvent::SeparateSplit,
                BrowserShortcut::CloseSplit => AppEvent::CloseSplit,
                BrowserShortcut::CloseGlance => AppEvent::GlanceClose,
                BrowserShortcut::PromoteGlance => AppEvent::GlanceOpenAsTab,
            };
            let _ = tx4.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_start = tab_id.clone();
        let tx_start = self.event_tx.clone();
        let on_nav_start = Box::new(move |nav_url: String| {
            let _ = tx_start.send(AppEvent::NavigationStarted {
                tab_id: tab_id_clone_start.clone(),
                url: nav_url,
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

        let tab_id_clone_fs = tab_id.clone();
        let tx_fs = self.event_tx.clone();
        let on_fullscreen = Box::new(move |is_full: bool| {
            let _ = tx_fs.send(AppEvent::FullScreenChanged {
                tab_id: tab_id_clone_fs.clone(),
                is_full,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tab_id_clone_focus = tab_id.clone();
        let tx_focus = self.event_tx.clone();
        let on_got_focus = Box::new(move || {
            let _ = tx_focus.send(AppEvent::PaneFocused(tab_id_clone_focus.clone()));
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

        let tab_id_clone_reader = tab_id.clone();
        let tx_reader = self.event_tx.clone();
        let on_reader = Box::new(move |msg: ReaderMessage| {
            let _ = tx_reader.send(AppEvent::ReaderMessage {
                tab_id: tab_id_clone_reader.clone(),
                msg,
            });
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        match self.engine.create_controller(
            self.window.hwnd(),
            content_rect,
            &url,
            Some(self.shield_state.clone()),
            Some(on_nav_start),
            Some(on_nav_complete),
            Some(on_title),
            Some(on_url),
            Some(on_favicon),
            Some(on_new_window),
            Some(on_shortcut),
            Some(on_fullscreen),
            Some(on_zapper),
            Some(on_reader),
            Some(on_got_focus),
        ) {
            Ok(c) => {
                if self.tabs[idx].is_muted {
                    let _ = c.set_muted(true);
                }
                self.tabs[idx].controller = Some(c);
            }
            Err(e) => {
                info!("Failed to create tab controller on wake: {}", e);
                self.window.set_loading(false, 0.0);
            }
        }
    }

    pub fn find_split_for_tab(&self, tab_id: &str) -> Option<&TabSplit> {
        self.splits.iter().find(|s| s.contains(tab_id))
    }

    pub fn find_split_for_tab_mut(&mut self, tab_id: &str) -> Option<&mut TabSplit> {
        self.splits.iter_mut().find(|s| s.contains(tab_id))
    }

    pub fn sync_split_bounds(&mut self) {
        if self.tabs.is_empty() {
            self.window.set_split_info(None);
            return;
        }

        // While a tab is HTML5-fullscreen (e.g. YouTube video fullscreen), only that tab is shown
        // across the entire client rect; all other split panes stay hidden until fullscreen ends.
        if let Some(fs_id) = self.fullscreen_tab_id.clone() {
            if let Some(fs_idx) = self.tabs.iter().position(|t| t.id == fs_id) {
                self.window.set_split_info(None);
                self.ensure_tab_controller(fs_idx);
                for (i, tab) in self.tabs.iter().enumerate() {
                    if i != fs_idx {
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.set_visible(false);
                        }
                    }
                }
                let mut client_rect = RECT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(
                        self.window.hwnd(),
                        &mut client_rect,
                    );
                }
                if let Some(ref ctrl) = self.tabs[fs_idx].controller {
                    let _ = ctrl.resume();
                    ctrl.set_fullscreen_mode(true);
                    let _ = ctrl.set_bounds(client_rect);
                    let _ = ctrl.set_visible(true);
                    ctrl.bring_to_top();
                }
                return;
            } else {
                self.fullscreen_tab_id = None;
            }
        }

        let active_id = self.tabs[self.active_tab_idx].id.clone();
        if let Some(s) = self.find_split_for_tab(&active_id).cloned() {
            let tab_indices: Option<Vec<usize>> = s.tabs.iter().map(|tid| self.tabs.iter().position(|t| t.id == *tid)).collect();

            if let Some(indices) = tab_indices {
                for &idx in &indices {
                    self.ensure_tab_controller(idx);
                }

                // Hide any tab controller that does not belong to the active split
                for (i, tab) in self.tabs.iter().enumerate() {
                    if !indices.contains(&i) {
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.set_visible(false);
                        }
                    }
                }

                let focused_idx = indices
                    .iter()
                    .position(|&idx| self.tabs[idx].id == s.focused_tab_id)
                    .map(|pos| indices[pos])
                    .unwrap_or(self.active_tab_idx);

                // Do not overwrite live divider sizes while the user is actively dragging a divider
                if !self.window.is_resizing_split() {
                    self.window.set_split_info(Some(ui::SplitInfo::new(
                        s.id.clone(),
                        indices.clone(),
                        s.sizes.clone(),
                        focused_idx,
                    )));
                }

                if let Some((pane_rects, _div_rects)) = self.window.get_split_layout_rects_multi() {
                    for (i, &idx) in indices.iter().enumerate() {
                        if let Some(ref ctrl) = self.tabs[idx].controller {
                            let _ = ctrl.resume();
                            ctrl.set_fullscreen_mode(false);
                            if i < pane_rects.len() {
                                let _ = ctrl.set_bounds(pane_rects[i]);
                            }
                            let _ = ctrl.set_visible(true);
                        }
                    }

                    if let Some(ref ctrl) = self.tabs[focused_idx].controller {
                        let zoom = ctrl.get_zoom_factor();
                        self.window.set_zoom(zoom);
                    }
                }
                self.sync_glance_bounds();
                return;
            } else {
                self.splits.retain(|sp| sp.id != s.id);
            }
        }

        self.window.set_split_info(None);
        for (i, tab) in self.tabs.iter().enumerate() {
            if i != self.active_tab_idx {
                if let Some(ref ctrl) = tab.controller {
                    let _ = ctrl.set_visible(false);
                }
            }
        }
        let (content_rect, _) = self.window.get_layout_rects();
        if let Some(ref ctrl) = self.tabs[self.active_tab_idx].controller {
            let _ = ctrl.resume();
            ctrl.set_fullscreen_mode(false);
            let _ = ctrl.set_bounds(content_rect);
            let _ = ctrl.set_visible(true);
            let zoom = ctrl.get_zoom_factor();
            self.window.set_zoom(zoom);
        }
        self.sync_glance_bounds();
    }

    pub fn open_glance(&mut self, url: &str, from_tab_id: Option<String>) {
        let cleaned = engine::clean_link::clean_url(url);
        if cleaned.trim().is_empty() {
            return;
        }
        if self.glance_tab.is_some() {
            self.close_glance();
        }

        let hwnd = self.window.hwnd();
        let hwnd_val = hwnd.0 as usize;

        if self.glance_overlay.is_none() {
            if let Ok(ov) = ui::GlanceOverlay::new(hwnd) {
                let tx = self.event_tx.clone();
                ov.set_on_action(move |action| {
                    let ev = match action {
                        ui::GlanceAction::Close => AppEvent::GlanceClose,
                        ui::GlanceAction::OpenAsTab => AppEvent::GlanceOpenAsTab,
                        ui::GlanceAction::OpenInSplit => AppEvent::GlanceOpenInSplit,
                    };
                    let _ = tx.send(ev);
                    unsafe {
                        let h = HWND(hwnd_val as *mut _);
                        let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
                    }
                });
                self.glance_overlay = Some(Rc::new(ov));
            }
        }

        let (content_rect, _) = self.window.get_layout_rects();
        let card_rect = ui::GlanceOverlay::compute_card_rect(hwnd, content_rect);

        let tab_id = format!("tab-{}", self.next_tab_id);
        self.next_tab_id += 1;

        let tid_title = tab_id.clone();
        let tx_title = self.event_tx.clone();
        let on_title = Box::new(move |title: String| {
            let _ = tx_title.send(AppEvent::GlanceTitleChanged {
                tab_id: tid_title.clone(),
                title,
            });
            unsafe {
                let h = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tid_url = tab_id.clone();
        let tx_url = self.event_tx.clone();
        let on_url = Box::new(move |new_url: String| {
            let _ = tx_url.send(AppEvent::GlanceUrlChanged {
                tab_id: tid_url.clone(),
                url: new_url,
            });
            unsafe {
                let h = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tid_fav = tab_id.clone();
        let tx_fav = self.event_tx.clone();
        let on_favicon = Box::new(move |bytes: Vec<u8>| {
            let _ = tx_fav.send(AppEvent::GlanceFaviconChanged {
                tab_id: tid_fav.clone(),
                bytes,
            });
            unsafe {
                let h = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tid_new = tab_id.clone();
        let tx_new = self.event_tx.clone();
        let on_new_window = Box::new(move |target_url: String, is_shift: bool| {
            let _ = tx_new.send(if is_shift {
                AppEvent::OpenGlance {
                    url: target_url,
                    from_tab_id: Some(tid_new.clone()),
                }
            } else {
                AppEvent::GlanceNavigate {
                    tab_id: tid_new.clone(),
                    url: target_url,
                }
            });
            unsafe {
                let h = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let tx_sc = self.event_tx.clone();
        let on_shortcut = Box::new(move |sc: BrowserShortcut| {
            let ev = match sc {
                BrowserShortcut::CloseGlance => AppEvent::GlanceClose,
                BrowserShortcut::PromoteGlance => AppEvent::GlanceOpenAsTab,
                BrowserShortcut::NewTab => AppEvent::OpenCommandBar(CommandBarMode::NewTab),
                BrowserShortcut::CloseTab => AppEvent::GlanceClose,
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
                BrowserShortcut::ToggleReader => AppEvent::ToggleReader(None),
                BrowserShortcut::SplitView => AppEvent::SplitWithNewTab,
                BrowserShortcut::FocusOtherPane => AppEvent::FocusOtherPane,
                BrowserShortcut::SwapSplit => AppEvent::SwapSplit,
                BrowserShortcut::EvenSplit => AppEvent::EvenSplit,
                BrowserShortcut::SeparateSplit => AppEvent::SeparateSplit,
                BrowserShortcut::CloseSplit => AppEvent::CloseSplit,
            };
            let _ = tx_sc.send(ev);
            unsafe {
                let h = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let web_host_hwnd = self
            .glance_overlay
            .as_ref()
            .map(|ov| ov.web_hwnd())
            .unwrap_or(hwnd);
        let local_card_rect = RECT {
            left: 0,
            top: 0,
            right: (card_rect.right - card_rect.left).max(10),
            bottom: (card_rect.bottom - card_rect.top).max(10),
        };

        let controller = match self.engine.create_controller(
            web_host_hwnd,
            local_card_rect,
            &cleaned,
            Some(self.shield_state.clone()),
            None,
            None,
            Some(on_title),
            Some(on_url),
            Some(on_favicon),
            Some(on_new_window),
            Some(on_shortcut),
            None,
            None,
            None,
            None,
        ) {
            Ok(c) => c,
            Err(e) => {
                info!("Failed to create glance controller: {}", e);
                return;
            }
        };

        controller.set_fullscreen_mode(true); // Sharp corners (no rounded region)
        let _ = controller.set_bounds(local_card_rect);
        let _ = controller.set_visible(true);
        controller.bring_to_top();

        self.glance_tab = Some(Tab {
            id: tab_id,
            title: friendly_title_for_url(&cleaned),
            url: cleaned.clone(),
            accent: accent_for_url(&cleaned),
            favicon: None,
            is_pinned: false,
            folder_id: None,
            custom_title: false,
            custom_letter: None,
            is_muted: false,
            controller: Some(controller),
            last_active: Instant::now(),
            anim_progress: 1.0,
            is_closing: false,
            pending_permissions: Vec::new(),
            is_reader: false,
            seeking_reader_twin: false,
            reader_gen: 0,
            reader_markdown: None,
        });
        self.glance_from_tab_id = from_tab_id;
        self.engine.set_glance_open(true);

        if let Some(ref ov) = self.glance_overlay {
            ov.show(content_rect, card_rect);
        }
        if let Some(ref ctrl) = self.glance_tab.as_ref().and_then(|t| t.controller.as_ref()) {
            let _ = ctrl.focus();
        }
    }

    pub fn close_glance(&mut self) {
        self.engine.set_glance_open(false);
        if let Some(ref ov) = self.glance_overlay {
            ov.hide();
        }
        if let Some(mut tab) = self.glance_tab.take() {
            if let Some(ref ctrl) = tab.controller {
                let _ = ctrl.set_visible(false);
            }
            tab.controller = None;
        }
        self.glance_from_tab_id = None;
        if let Some(ctrl) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
            let _ = ctrl.focus();
        }
    }

    pub fn promote_glance_to_tab(&mut self, in_split: bool) {
        self.engine.set_glance_open(false);
        if let Some(ref ov) = self.glance_overlay {
            ov.hide();
        }
        let Some(mut tab) = self.glance_tab.take() else {
            return;
        };
        let from_id = self.glance_from_tab_id.take();

        // Adopt source tab's folder if it's a regular tab
        let from_pos = from_id
            .as_ref()
            .and_then(|fid| self.tabs.iter().position(|t| t.id == *fid));
        if let Some(pos) = from_pos {
            if !self.tabs[pos].is_pinned {
                tab.folder_id = self.tabs[pos].folder_id.clone();
            }
        }

        let new_tab_id = tab.id.clone();
        let insert_idx = from_pos
            .map(|p| (p + 1).min(self.tabs.len()))
            .unwrap_or_else(|| (self.active_tab_idx + 1).min(self.tabs.len()));

        // Reparent the live WebView2 controller from the Glance popup host into main_hwnd
        if let Some(ref ctrl) = tab.controller {
            let _ = ctrl.set_parent_window(self.window.hwnd());
            ctrl.set_fullscreen_mode(false);
        }

        self.tabs.insert(insert_idx, tab);
        self.active_tab_idx = insert_idx;

        if in_split {
            if let Some(fid) = from_id.filter(|id| {
                self.tabs
                    .iter()
                    .any(|t| t.id == *id && !t.is_pinned)
            }) {
                if let Some(existing_idx) = self.splits.iter().position(|s| s.contains(&fid)) {
                    if !self.splits[existing_idx].is_full() {
                        let mut ordered = self.splits[existing_idx].tabs.clone();
                        if let Some(p) = ordered.iter().position(|id| id == &fid) {
                            ordered.insert(p + 1, new_tab_id.clone());
                        } else {
                            ordered.push(new_tab_id.clone());
                        }
                        self.handle_split_tabs(ordered);
                        return;
                    }
                }
                self.handle_split_tabs(vec![fid, new_tab_id]);
                return;
            }
        }

        self.sync_split_bounds();
        if let Some(ctrl) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
            ctrl.bring_to_top();
            let _ = ctrl.focus();
        }
        self.sync_tabs_ui();
        self.persist_state();
    }

    pub fn sync_glance_bounds(&self) {
        let Some(ref tab) = self.glance_tab else {
            return;
        };
        let (content_rect, _) = self.window.get_layout_rects();
        let card_rect = ui::GlanceOverlay::compute_card_rect(self.window.hwnd(), content_rect);
        if let Some(ref ov) = self.glance_overlay {
            ov.update_layout(content_rect, card_rect);
        }
        if let Some(ref ctrl) = tab.controller {
            let local_rect = RECT {
                left: 0,
                top: 0,
                right: (card_rect.right - card_rect.left).max(10),
                bottom: (card_rect.bottom - card_rect.top).max(10),
            };
            let _ = ctrl.set_bounds(local_rect);
            ctrl.bring_to_top();
        }
    }

    /// Group a split's tabs adjacently in `self.tabs` at the first member's position,
    /// adopting its `folder_id` (matching `Search`'s `pair` / `pairPickedTabs`) so
    /// the sidebar renders them cleanly inside one split container.
    fn group_split_tabs_in_list(&mut self, ordered_ids: &[String]) {
        if ordered_ids.len() < 2 {
            return;
        }
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let first_pos = match self.tabs.iter().position(|t| t.id == ordered_ids[0]) {
            Some(p) => p,
            None => return,
        };
        let target_folder = self.tabs[first_pos].folder_id.clone();

        let id_set: std::collections::HashSet<&str> = ordered_ids.iter().map(|s| s.as_str()).collect();
        let insert_at = self.tabs[..first_pos]
            .iter()
            .filter(|t| !id_set.contains(t.id.as_str()))
            .count();

        let mut grouped = Vec::with_capacity(ordered_ids.len());
        for tid in ordered_ids {
            if let Some(pos) = self.tabs.iter().position(|t| t.id == *tid) {
                let mut tab = self.tabs.remove(pos);
                tab.folder_id = target_folder.clone();
                tab.is_pinned = false;
                grouped.push(tab);
            }
        }

        let pos = insert_at.min(self.tabs.len());
        for (offset, tab) in grouped.into_iter().enumerate() {
            self.tabs.insert(pos + offset, tab);
        }

        if let Some(aid) = active_id {
            if let Some(new_idx) = self.tabs.iter().position(|t| t.id == aid) {
                self.active_tab_idx = new_idx;
            }
        }
    }

    fn start_split(&mut self, other_tab_id: Option<String>) {
        if self.tabs.is_empty() || self.active_tab_idx >= self.tabs.len() {
            return;
        }

        let current_tab_id = self.tabs[self.active_tab_idx].id.clone();

        // If the current tab is already in a split:
        if self.find_split_for_tab(&current_tab_id).is_some() {
            let is_full = self.find_split_for_tab(&current_tab_id).map_or(true, |s| s.is_full());
            if is_full {
                return;
            }

            let partner_id = match other_tab_id {
                Some(target_id) => {
                    if target_id == current_tab_id {
                        return;
                    }
                    if let Some(s) = self.find_split_for_tab(&current_tab_id) {
                        if s.contains(&target_id) {
                            return;
                        }
                    }
                    target_id
                }
                None => {
                    let tab_id = format!("tab-{}", self.next_tab_id);
                    self.next_tab_id += 1;
                    let default_url = "https://www.google.com".to_string();
                    let initial_title = friendly_title_for_url(&default_url);
                    let initial_accent = accent_for_url(&default_url);
                    let domain = ui::domain_of(&default_url);
                    let preloaded_favicon = self.get_cached_favicon(&domain);

                    let insert_pos = self.active_tab_idx + 1;
                    self.tabs.insert(insert_pos, Tab {
                        id: tab_id.clone(),
                        title: initial_title,
                        url: default_url,
                        accent: initial_accent,
                        favicon: preloaded_favicon,
                        is_pinned: false,
                        folder_id: self.tabs[self.active_tab_idx].folder_id.clone(),
                        custom_title: false,
                        custom_letter: None,
                        is_muted: false,
                        controller: None,
                        last_active: Instant::now(),
                        anim_progress: 1.0,
                        is_closing: false,
                        pending_permissions: Vec::new(),
                        is_reader: false,
                        seeking_reader_twin: false,
                        reader_gen: 0,
                        reader_markdown: None,
                    });
                    tab_id
                }
            };

            // Remove partner from any other split if present
            for other_s in self.splits.iter_mut() {
                if !other_s.contains(&current_tab_id) && other_s.contains(&partner_id) {
                    other_s.remove(&partner_id);
                }
            }
            self.splits.retain(|sp| sp.tabs.len() >= 2);

            let mut grouped_tabs = None;
            if let Some(s) = self.find_split_for_tab_mut(&current_tab_id) {
                s.insert(partner_id.clone(), None, false);
                s.focused_tab_id = partner_id.clone();
                grouped_tabs = Some(s.tabs.clone());
            }
            if let Some(ordered) = grouped_tabs {
                self.group_split_tabs_in_list(&ordered);
            }

            if let Some(new_active) = self.tabs.iter().position(|t| t.id == partner_id) {
                self.ensure_tab_controller(new_active);
                self.switch_tab(new_active);
            } else {
                self.sync_split_bounds();
                self.sync_tabs_ui();
            }
            self.persist_state();
            return;
        }

        let partner_id = match other_tab_id {
            Some(target_id) => {
                if target_id == current_tab_id {
                    return;
                }
                if let Some(pos) = self.tabs.iter().position(|t| t.id == target_id) {
                    let tid = self.tabs[pos].id.clone();
                    for other_s in self.splits.iter_mut() {
                        if other_s.contains(&tid) {
                            other_s.remove(&tid);
                        }
                    }
                    self.splits.retain(|sp| sp.tabs.len() >= 2);
                    tid
                } else {
                    return;
                }
            }
            None => {
                let tab_id = format!("tab-{}", self.next_tab_id);
                self.next_tab_id += 1;
                let default_url = "https://www.google.com".to_string();
                let initial_title = friendly_title_for_url(&default_url);
                let initial_accent = accent_for_url(&default_url);
                let domain = ui::domain_of(&default_url);
                let preloaded_favicon = self.get_cached_favicon(&domain);

                let insert_pos = self.active_tab_idx + 1;
                self.tabs.insert(insert_pos, Tab {
                    id: tab_id.clone(),
                    title: initial_title,
                    url: default_url,
                    accent: initial_accent,
                    favicon: preloaded_favicon,
                    is_pinned: false,
                    folder_id: self.tabs[self.active_tab_idx].folder_id.clone(),
                    custom_title: false,
                    custom_letter: None,
                    is_muted: false,
                    controller: None,
                    last_active: Instant::now(),
                    anim_progress: 1.0,
                    is_closing: false,
                    pending_permissions: Vec::new(),
                    is_reader: false,
                    seeking_reader_twin: false,
                    reader_gen: 0,
                    reader_markdown: None,
                });
                tab_id
            }
        };

        let split_id = format!("split-{}", self.next_tab_id);
        self.next_tab_id += 1;
        let pair_tabs = vec![current_tab_id, partner_id.clone()];
        self.splits.push(TabSplit::new(
            split_id,
            pair_tabs.clone(),
            partner_id.clone(),
        ));
        self.group_split_tabs_in_list(&pair_tabs);

        if let Some(new_active) = self.tabs.iter().position(|t| t.id == partner_id) {
            self.ensure_tab_controller(new_active);
            self.switch_tab(new_active);
        } else {
            self.sync_split_bounds();
            self.sync_tabs_ui();
        }
        self.persist_state();
    }

    fn swap_split(&mut self) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        let mut reversed_tabs = None;
        if let Some(s) = self.find_split_for_tab_mut(&active_id) {
            s.tabs.reverse();
            s.sizes.reverse();
            reversed_tabs = Some(s.tabs.clone());
        }
        if let Some(ordered) = reversed_tabs {
            self.group_split_tabs_in_list(&ordered);
        }
        self.sync_split_bounds();
        self.sync_tabs_ui();
        self.persist_state();
    }

    fn even_split(&mut self) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab_mut(&active_id) {
            s.sizes = TabSplit::even(s.tabs.len());
        }
        self.sync_split_bounds();
        self.persist_state();
    }

    fn separate_split(&mut self) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab(&active_id).cloned() {
            for tid in &s.tabs {
                if tid != &active_id {
                    if let Some(pos) = self.tabs.iter().position(|t| &t.id == tid) {
                        if let Some(ref ctrl) = self.tabs[pos].controller {
                            let _ = ctrl.set_visible(false);
                        }
                    }
                }
            }
            self.splits.retain(|sp| sp.id != s.id);
            self.sync_split_bounds();
            self.sync_tabs_ui();
            self.persist_state();
        }
    }

    fn close_split(&mut self) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab(&active_id).cloned() {
            self.splits.retain(|sp| sp.id != s.id);
            let tabs_to_close = s.tabs;
            for tid in tabs_to_close {
                if let Some(pos) = self.tabs.iter().position(|t| t.id == tid) {
                    self.close_tab(pos);
                }
            }
        }
    }

    fn focus_other_pane(&mut self) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab(&active_id) {
            if let Some(cur_pos) = s.tabs.iter().position(|t| t == &active_id) {
                let next_pos = (cur_pos + 1) % s.tabs.len();
                let next_id = &s.tabs[next_pos];
                if let Some(pos) = self.tabs.iter().position(|t| &t.id == next_id) {
                    self.switch_tab(pos);
                }
            }
        }
    }

    fn split_ratio_changed(&mut self, ratio: f32) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab_mut(&active_id) {
            let clamped = ratio.clamp(0.2, 0.8);
            if s.sizes.len() == 2 {
                s.sizes = vec![clamped, 1.0 - clamped];
            } else if !s.sizes.is_empty() {
                s.sizes[0] = clamped;
                s.sizes = TabSplit::clamp(&s.sizes);
            }
        }
        self.sync_split_bounds();
        self.mark_dirty();
    }

    fn split_sizes_changed(&mut self, sizes: Vec<f32>) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let Some(active_id) = active_id else { return };
        if let Some(s) = self.find_split_for_tab_mut(&active_id) {
            s.sizes = TabSplit::clamp(&sizes);
        }
        self.sync_split_bounds();
        self.mark_dirty();
    }

    fn handle_split_tabs(&mut self, tab_ids: Vec<String>) {
        if tab_ids.len() < 2 || tab_ids.len() > TabSplit::MOST {
            return;
        }

        let mut valid_ids: Vec<String> = Vec::new();
        for tid in tab_ids {
            if self.tabs.iter().any(|t| t.id == tid && !t.is_pinned) && !valid_ids.contains(&tid) {
                valid_ids.push(tid);
            }
        }

        if valid_ids.len() < 2 || valid_ids.len() > TabSplit::MOST {
            return;
        }

        // Remove these tabs from any existing splits
        for tid in &valid_ids {
            for s in self.splits.iter_mut() {
                if s.contains(tid) {
                    s.remove(tid);
                }
            }
        }
        self.splits.retain(|s| s.tabs.len() >= 2);

        self.group_split_tabs_in_list(&valid_ids);

        let split_id = format!("split-{}", self.next_tab_id);
        self.next_tab_id += 1;
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        let focused_id = active_id
            .filter(|aid| valid_ids.contains(aid))
            .unwrap_or_else(|| valid_ids[0].clone());
        self.splits.push(TabSplit::new(split_id, valid_ids.clone(), focused_id.clone()));

        if let Some(pos) = self.tabs.iter().position(|t| t.id == focused_id) {
            self.active_tab_idx = pos;
        }

        // sync_split_bounds ensures controllers for all split tabs, hides old non-split tabs,
        // and positions all 2..=4 panes cleanly.
        self.sync_split_bounds();

        if let Some(ctrl) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
            let _ = ctrl.focus();
        }

        self.sync_tabs_ui();
        self.persist_state();
    }

    fn switch_tab(&mut self, idx: usize) {
        if self.glance_tab.is_some() {
            self.close_glance();
        }
        if idx >= self.tabs.len() {
            return;
        }

        let old_idx = self.active_tab_idx;
        if let Some(ref fs_id) = self.fullscreen_tab_id.clone() {
            if self.tabs[idx].id != *fs_id {
                self.fullscreen_tab_id = None;
                let prev_vis = self.sidebar_before_fullscreen.take().unwrap_or(true);
                self.window.set_sidebar_visible_quiet(prev_vis);
            }
        }
        if old_idx == idx && self.tabs[idx].controller.is_some() {
            self.sync_split_bounds();
            self.sync_tabs_ui();
            return;
        }

        if old_idx < self.tabs.len() && old_idx != idx {
            let old_id = self.tabs[old_idx].id.clone();
            let new_id = self.tabs[idx].id.clone();
            let in_same_split = self.find_split_for_tab(&old_id).map_or(false, |s| {
                s.contains(&old_id) && s.contains(&new_id)
            });

            if !in_same_split {
                if let Some(old_split) = self.find_split_for_tab(&old_id).cloned() {
                    for tid in &old_split.tabs {
                        if let Some(pos) = self.tabs.iter().position(|t| t.id == *tid) {
                            if let Some(ref c) = self.tabs[pos].controller {
                                let _ = c.set_visible(false);
                            }
                        }
                    }
                } else if let Some(ref current) = self.tabs[old_idx].controller {
                    let _ = current.set_visible(false);
                }
            }

            if let Some(tab) = self.tabs.get_mut(old_idx) {
                tab.last_active = Instant::now();
            }
        }

        self.ensure_tab_controller(idx);
        self.active_tab_idx = idx;

        let active_id = self.tabs[idx].id.clone();
        if let Some(s) = self.find_split_for_tab_mut(&active_id) {
            s.focused_tab_id = active_id.clone();
        }

        self.sync_split_bounds();

        if let Some(ref ctrl) = self.tabs[idx].controller {
            let _ = ctrl.focus();
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

        let closed_tab_id = self.tabs[target_idx].id.clone();
        if self.fullscreen_tab_id.as_deref() == Some(closed_tab_id.as_str()) {
            self.fullscreen_tab_id = None;
            let prev_vis = self.sidebar_before_fullscreen.take().unwrap_or(true);
            self.window.set_sidebar_visible_quiet(prev_vis);
        }
        for s in self.splits.iter_mut() {
            if s.contains(&closed_tab_id) {
                s.remove(&closed_tab_id);
            }
        }
        self.splits.retain(|sp| sp.tabs.len() >= 2);

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
            self.sync_split_bounds();

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
                let tab_id = self.tabs[i].id.clone();
                for s in self.splits.iter_mut() {
                    if s.contains(&tab_id) {
                        s.remove(&tab_id);
                    }
                }
                self.splits.retain(|sp| sp.tabs.len() >= 2);
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
                self.sync_split_bounds();
            } else {
                self.window.set_split_info(None);
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
                    custom_letter: sc.custom_letter.clone().or_else(|| t.custom_letter.clone()),
                    is_split: false,
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
                    custom_letter: sc.custom_letter.clone(),
                    is_split: false,
                });
            }
        }

        for (i, t) in self.tabs.iter().enumerate() {
            if !t.is_pinned {
                if i == self.active_tab_idx {
                    active_regular = Some(regular_items.len());
                }
                let is_split = self.splits.iter().any(|s| s.contains(&t.id));
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
                    custom_letter: t.custom_letter.clone(),
                    is_split,
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

        let split_groups: Vec<ui::SidebarSplitGroup> = self.splits.iter().filter_map(|s| {
            let indices: Vec<usize> = s.tabs.iter().filter_map(|tid| {
                regular_items.iter().position(|item| item.id == *tid)
            }).collect();
            if indices.len() >= 2 {
                Some(ui::SidebarSplitGroup {
                    id: s.id.clone(),
                    tab_indices: indices,
                })
            } else {
                None
            }
        }).collect();

        let has_closed = !self.closed_tabs.is_empty();
        self.window.update_sidebar_data(regular_items, pinned_items, folder_items, split_groups, active, can_back, can_forward, has_closed);

        let (blocked, active_host) = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let host = tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
            let count = tab.controller.as_ref().map(|c| c.get_shield_blocked_count()).unwrap_or(0);
            (count, host)
        } else {
            (0, String::new())
        };
        let enabled = self.shield_state.is_enabled();
        let is_paused = self.shield_state.is_paused(&active_host);
        self.window.update_shield_status(enabled, blocked, is_paused);

        let pending_perms = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            tab.pending_permissions.clone()
        } else {
            Vec::new()
        };
        self.window.set_pending_permissions(pending_perms);

        let active_is_reader = self
            .tabs
            .get(self.active_tab_idx)
            .map_or(false, |t| t.is_reader);
        self.window.set_reader_active(active_is_reader);
    }

    fn effective_reader_look(&self) -> ReaderLook {
        if let Ok(st) = self.storage.try_borrow() {
            let s = st.get_reader_look().unwrap_or_else(|_| "system".to_string());
            ReaderLook::from_str(&s)
        } else {
            ReaderLook::System
        }
    }

    fn refresh_site_card_if_visible(&self) {
        if !self.window.is_site_card_visible() {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let active_host = tab
                .controller
                .as_ref()
                .map(|c| c.get_current_host())
                .unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
            let hidden_count = if let Ok(st) = self.storage.try_borrow() {
                st.get_hidden_elements(&active_host)
                    .map(|l| l.len())
                    .unwrap_or(0)
            } else {
                0
            };
            let card_data = SiteCardData {
                url: tab.url.clone(),
                host: active_host.clone(),
                is_secure: tab.url.starts_with("https://"),
                is_bookmarked: tab.is_pinned,
                is_reader: tab.is_reader,
                shield_enabled: self.shield_state.is_enabled(),
                shield_paused_for_site: self.shield_state.is_paused(&active_host),
                trackers_blocked_count: self.shield_state.get_blocked_count() as u32,
                hidden_elements_count: hidden_count,
            };
            self.window.update_site_card(card_data);
        }
    }

    fn resolve_tab_index_by_id(&self, target_tab_id: Option<&str>) -> Option<usize> {
        match target_tab_id {
            None => {
                if self.active_tab_idx < self.tabs.len() {
                    Some(self.active_tab_idx)
                } else {
                    None
                }
            }
            Some(tid) => {
                if let Some(pos) = self.tabs.iter().position(|t| t.id == tid) {
                    return Some(pos);
                }
                if let Some(sc) = self.pinned_shortcuts.iter().find(|s| s.id == tid) {
                    let sc_domain = ui::domain_of(&sc.url);
                    return self.tabs.iter().position(|t| {
                        t.is_pinned
                            && (t.url == sc.url
                                || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    });
                }
                None
            }
        }
    }

    fn reset_tab_reader_state(&mut self, pos: usize) {
        if let Some(tab) = self.tabs.get_mut(pos) {
            tab.is_reader = false;
            tab.seeking_reader_twin = false;
            tab.reader_markdown = None;
            tab.reader_gen = tab.reader_gen.wrapping_add(1);
        }
    }

    fn toggle_reader(&mut self, target_tab_id: Option<String>) {
        let Some(pos) = self.resolve_tab_index_by_id(target_tab_id.as_deref()) else {
            if target_tab_id.is_some() {
                self.window.show_toast("Tab is sleeping (open page first)");
            }
            return;
        };

        if self.tabs[pos].is_reader {
            self.reset_tab_reader_state(pos);
            if let Some(ref ctrl) = self.tabs[pos].controller {
                let _ = ctrl.reload();
            }
            self.refresh_site_card_if_visible();
            self.sync_tabs_ui();
            return;
        }

        if self.tabs[pos].controller.is_none() {
            self.window.show_toast("Tab is sleeping (open page first)");
            return;
        }

        let live_url = self.tabs[pos]
            .controller
            .as_ref()
            .and_then(|c| c.get_source().ok())
            .unwrap_or_default();
        let check_url = if !live_url.is_empty() && live_url != "about:blank" {
            live_url
        } else {
            self.tabs[pos].url.clone()
        };
        let lower_url = check_url.trim().to_ascii_lowercase();
        if !(lower_url.starts_with("http://")
            || lower_url.starts_with("https://")
            || lower_url.starts_with("file://"))
        {
            self.window.show_toast("Nothing to read on this page");
            return;
        }

        self.tabs[pos].reader_gen = self.tabs[pos].reader_gen.wrapping_add(1);
        let gen = self.tabs[pos].reader_gen;
        self.tabs[pos].reader_markdown = None;

        let reads_twins = if let Ok(st) = self.storage.try_borrow() {
            st.get_reader_reads_twins().unwrap_or(true)
        } else {
            true
        };
        let look = self.effective_reader_look();

        if reads_twins {
            self.tabs[pos].seeking_reader_twin = true;
            if let Some(ref ctrl) = self.tabs[pos].controller {
                let _ = ctrl.start_reader_twin_check(gen);
            }
        } else {
            self.tabs[pos].seeking_reader_twin = false;
            if let Some(ref ctrl) = self.tabs[pos].controller {
                let _ = ctrl.run_reader_heuristic(gen, look);
            }
        }
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
                            custom_letter: self.tabs[pos].custom_letter.clone(),
                            is_split: false,
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
                            custom_letter: removed.custom_letter,
                            is_muted: false,
                            controller: None, // Sleeping!
                            last_active: Instant::now(),
                            anim_progress: 1.0,
                            is_closing: false,
                            pending_permissions: Vec::new(),
                            is_reader: false,
                            seeking_reader_twin: false,
                            reader_gen: 0,
                            reader_markdown: None,
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
            AppEvent::SetPinnedLetter(idx, letter) => {
                if idx < self.pinned_shortcuts.len() {
                    let letter_val = if letter.trim().is_empty() {
                        None
                    } else {
                        // Take the first char as uppercase
                        let c = letter.trim().chars().next().map(|ch| ch.to_uppercase().to_string());
                        c
                    };
                    self.pinned_shortcuts[idx].custom_letter = letter_val.clone();
                    let sc_url = self.pinned_shortcuts[idx].url.clone();
                    let sc_domain = ui::domain_of(&sc_url);
                    if let Some(pos) = self.tabs.iter().position(|t| {
                        t.is_pinned && (t.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&t.url) == sc_domain))
                    }) {
                        self.tabs[pos].custom_letter = letter_val;
                    }
                    self.sync_tabs_ui();
                    self.mark_dirty();
                    self.persist_state();
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
                        self.reset_tab_reader_state(pos);
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
                    let title = if tab.title.trim().is_empty() {
                        "Share link".to_string()
                    } else {
                        tab.title.clone()
                    };
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let to_share = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    let clean = engine::clean_url(&to_share);
                    if let Err(e) = show_windows_share_ui(self.window.hwnd(), &title, &clean) {
                        tracing::warn!("Native Windows share UI failed: {:?}; copying link to clipboard", e);
                        copy_to_clipboard(self.window.hwnd(), &clean);
                        self.window.show_toast("Link copied to clipboard!");
                    }
                }
            }
            AppEvent::SharePinnedTab(idx) => {
                if let Some(sc) = self.pinned_shortcuts.get(idx) {
                    let clean = engine::clean_url(&sc.url);
                    if let Err(e) = show_windows_share_ui(self.window.hwnd(), &sc.title, &clean) {
                        tracing::warn!("Native Windows share UI failed: {:?}; copying link to clipboard", e);
                        copy_to_clipboard(self.window.hwnd(), &clean);
                        self.window.show_toast("Link copied to clipboard!");
                    }
                }
            }
            AppEvent::ShareCurrentPage => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let title = if tab.title.trim().is_empty() {
                        "Share link".to_string()
                    } else {
                        tab.title.clone()
                    };
                    let live_url = tab.controller.as_ref().and_then(|c| c.get_source().ok()).unwrap_or_default();
                    let to_share = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    let clean = engine::clean_url(&to_share);
                    if let Err(e) = show_windows_share_ui(self.window.hwnd(), &title, &clean) {
                        tracing::warn!("Native Windows share UI failed: {:?}; copying link to clipboard", e);
                        copy_to_clipboard(self.window.hwnd(), &clean);
                        self.window.show_toast("Link copied to clipboard!");
                    }
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
                    self.reset_tab_reader_state(pos);
                    if pos == self.active_tab_idx {
                        self.refresh_site_card_if_visible();
                        self.sync_tabs_ui();
                    }
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
                        self.reset_tab_reader_state(pos);
                        if pos == self.active_tab_idx {
                            self.refresh_site_card_if_visible();
                            self.sync_tabs_ui();
                        }
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
                    let is_same_doc_fragment = !url.is_empty()
                        && url != tab.url
                        && engine::strip_url_fragment(&url) == engine::strip_url_fragment(&tab.url);
                    if !is_same_doc_fragment {
                        tab.is_reader = false;
                        tab.seeking_reader_twin = false;
                        tab.reader_markdown = None;
                        tab.reader_gen = tab.reader_gen.wrapping_add(1);
                    }
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
                let pos = self.active_tab_idx;
                let was_reader = self.tabs.get(pos).map_or(false, |t| t.is_reader || t.seeking_reader_twin);
                self.reset_tab_reader_state(pos);
                if was_reader {
                    self.refresh_site_card_if_visible();
                    self.sync_tabs_ui();
                }
                if let Some(tab) = self.tabs.get(pos) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.reload();
                    }
                }
            }
            AppEvent::ToggleSidebar => {
                self.window.toggle_sidebar();
                self.sync_split_bounds();
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
            AppEvent::NavigationStarted { tab_id, url } => {
                let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                    let is_same_doc_fragment = !url.is_empty()
                        && url != self.tabs[pos].url
                        && engine::strip_url_fragment(&url) == engine::strip_url_fragment(&self.tabs[pos].url);
                    if !is_same_doc_fragment {
                        let was_reader = self.tabs[pos].is_reader || self.tabs[pos].seeking_reader_twin;
                        self.reset_tab_reader_state(pos);
                        if is_active && was_reader {
                            self.refresh_site_card_if_visible();
                            self.sync_tabs_ui();
                        }
                    }
                }
                if is_active {
                    self.window.set_loading(true, 0.45);
                }
            }
            AppEvent::NavigationCompleted { tab_id, .. } => {
                let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                if is_active {
                    self.window.set_loading(false, 1.0);
                }
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    let domain = ui::domain_of(&tab.url);
                    if !domain.is_empty() {
                        tab.pending_permissions.clear();
                        if let Ok(st) = self.storage.try_borrow() {
                            let notif_perm = st.get_site_permission(&domain, "notifications").unwrap_or(0);
                            if notif_perm == 0 && (domain.contains("youtube.com") || domain.contains("twitter.com") || domain.contains("x.com") || domain.contains("reddit.com")) {
                                tab.pending_permissions.push(ui::SitePermissionKind::Notifications);
                            }
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
                if is_active {
                    self.sync_tabs_ui();
                }
            }
            AppEvent::HardReload => {
                let pos = self.active_tab_idx;
                let was_reader = self.tabs.get(pos).map_or(false, |t| t.is_reader || t.seeking_reader_twin);
                self.reset_tab_reader_state(pos);
                if was_reader {
                    self.refresh_site_card_if_visible();
                    self.sync_tabs_ui();
                }
                if let Some(tab) = self.tabs.get(pos) {
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
                let mut is_pinned = false;
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.is_pinned = !tab.is_pinned;
                    is_pinned = tab.is_pinned;
                    if tab.is_pinned {
                        self.window.show_toast("Added to Favourite tabs");
                    } else {
                        self.window.show_toast("Removed from Favourite tabs");
                    }
                }
                if self.window.is_site_card_visible() {
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        let active_host = tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
                        let hidden_count = if let Ok(st) = self.storage.try_borrow() {
                            st.get_hidden_elements(&active_host).map(|l| l.len()).unwrap_or(0)
                        } else {
                            0
                        };
                        let card_data = SiteCardData {
                            url: tab.url.clone(),
                            host: active_host.clone(),
                            is_secure: tab.url.starts_with("https://"),
                            is_bookmarked: is_pinned,
                            is_reader: tab.is_reader,
                            shield_enabled: self.shield_state.is_enabled(),
                            shield_paused_for_site: self.shield_state.is_paused(&active_host),
                            trackers_blocked_count: self.shield_state.get_blocked_count() as u32,
                            hidden_elements_count: hidden_count,
                        };
                        self.window.update_site_card(card_data);
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
            AppEvent::FullScreenChanged { tab_id, is_full } => {
                let hwnd = self.window.hwnd();
                if is_full {
                    if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                        self.fullscreen_tab_id = Some(tab_id.clone());
                        if self.sidebar_before_fullscreen.is_none() {
                            self.sidebar_before_fullscreen = Some(self.window.is_sidebar_visible());
                        }
                        self.window.set_sidebar_visible_quiet(false);
                        self.active_tab_idx = pos;
                        if let Some(s) = self.find_split_for_tab_mut(&tab_id) {
                            s.focused_tab_id = tab_id.clone();
                        }
                        self.sync_split_bounds();
                        self.sync_tabs_ui();
                    }
                } else if self.fullscreen_tab_id.as_deref() == Some(tab_id.as_str()) || self.fullscreen_tab_id.is_some() {
                    self.fullscreen_tab_id = None;
                    let prev_vis = self.sidebar_before_fullscreen.take().unwrap_or(true);
                    self.window.set_sidebar_visible_quiet(prev_vis);
                    self.sync_split_bounds();
                    self.sync_tabs_ui();
                }
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
                }
            }
            AppEvent::PaneFocused(tab_id) => {
                if self.fullscreen_tab_id.is_some() {
                    return;
                }
                if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                    if pos != self.active_tab_idx && pos < self.tabs.len() && self.active_tab_idx < self.tabs.len() {
                        let cur_id = self.tabs[self.active_tab_idx].id.clone();
                        let in_same_split = self.find_split_for_tab(&cur_id)
                            .map_or(false, |s| s.contains(&tab_id));
                        if in_same_split {
                            self.active_tab_idx = pos;
                            if let Some(s) = self.find_split_for_tab_mut(&tab_id) {
                                s.focused_tab_id = tab_id.clone();
                            }
                            if let Some(s) = self.find_split_for_tab(&tab_id).cloned() {
                                let indices: Option<Vec<usize>> = s.tabs.iter()
                                    .map(|tid| self.tabs.iter().position(|t| t.id == *tid))
                                    .collect();
                                if let Some(indices) = indices {
                                    self.window.set_split_info(Some(ui::SplitInfo::new(
                                        s.id.clone(),
                                        indices,
                                        s.sizes.clone(),
                                        pos,
                                    )));
                                }
                            }
                            if let Some(ref ctrl) = self.tabs[pos].controller {
                                let zoom = ctrl.get_zoom_factor();
                                self.window.set_zoom(zoom);
                            }
                            if let Some(tab) = self.tabs.get_mut(pos) {
                                tab.last_active = Instant::now();
                            }
                            self.sync_tabs_ui();
                            self.mark_dirty();
                        }
                    }
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
            AppEvent::ToggleShield => {
                let new_enabled = !self.shield_state.is_enabled();
                self.shield_state.set_enabled(new_enabled);
                if let Ok(st) = self.storage.try_borrow() {
                    let _ = st.set_shield_enabled(new_enabled);
                }
                let msg = if new_enabled {
                    "Shield Active: Ad & Tracker Protection Enabled".to_string()
                } else {
                    "Shield Paused: Protection Disabled Globally".to_string()
                };
                self.window.show_toast(&msg);

                // Update SiteCard if visible so toggle knob reflects new state
                if self.window.is_site_card_visible() {
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        let active_host = tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
                        let is_bookmarked = tab.is_pinned;
                        let hidden_count = if let Ok(st) = self.storage.try_borrow() {
                            st.get_hidden_elements(&active_host).map(|l| l.len()).unwrap_or(0)
                        } else {
                            0
                        };
                        let card_data = SiteCardData {
                            url: tab.url.clone(),
                            host: active_host.clone(),
                            is_secure: tab.url.starts_with("https://"),
                            is_bookmarked,
                            is_reader: tab.is_reader,
                            shield_enabled: new_enabled,
                            shield_paused_for_site: self.shield_state.is_paused(&active_host),
                            trackers_blocked_count: self.shield_state.get_blocked_count() as u32,
                            hidden_elements_count: hidden_count,
                        };
                        self.window.update_site_card(card_data);
                    }
                }

                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let script = if new_enabled {
                            r#"
                                (() => {
                                    try {
                                        window.__BROWSER_SHIELD_ENABLED__ = true;
                                        localStorage.setItem('__browser_shield_global_enabled', 'true');
                                    } catch(e) {}
                                    location.reload();
                                })();
                            "#
                        } else {
                            r#"
                                (() => {
                                    try {
                                        window.__BROWSER_SHIELD_ENABLED__ = false;
                                        localStorage.setItem('__browser_shield_global_enabled', 'false');
                                        const s = document.getElementById('__browser_shield_cosmetic__');
                                        if (s) s.remove();
                                        const v = document.querySelector('video');
                                        if (v) {
                                            if (v.playbackRate === 16) v.playbackRate = 1.0;
                                            v.muted = false;
                                        }
                                    } catch(e) {}
                                    location.reload();
                                })();
                            "#
                        };
                        if ctrl.execute_script(script).is_err() {
                            let _ = ctrl.reload();
                        }
                    }
                }

                self.sync_tabs_ui();
            }
            AppEvent::ToggleSiteShield => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() {
                    self.window.show_toast("No active website to toggle Shield");
                    return;
                }
                let is_currently_paused = self.shield_state.is_paused(&active_host);
                let new_paused = !is_currently_paused;
                self.shield_state.set_paused(&active_host, new_paused);
                if let Ok(st) = self.storage.try_borrow() {
                    let _ = st.set_shield_paused(&active_host, new_paused);
                }
                let msg = if new_paused {
                    format!("Shield Paused for {}", active_host)
                } else {
                    format!("Shield Active for {}", active_host)
                };
                self.window.show_toast(&msg);

                // Update SiteCard if visible so toggle knob and status match
                if self.window.is_site_card_visible() {
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        let is_bookmarked = tab.is_pinned;
                        let hidden_count = if let Ok(st) = self.storage.try_borrow() {
                            st.get_hidden_elements(&active_host).map(|l| l.len()).unwrap_or(0)
                        } else {
                            0
                        };
                        let card_data = SiteCardData {
                            url: tab.url.clone(),
                            host: active_host.clone(),
                            is_secure: tab.url.starts_with("https://"),
                            is_bookmarked,
                            is_reader: tab.is_reader,
                            shield_enabled: self.shield_state.is_enabled(),
                            shield_paused_for_site: new_paused,
                            trackers_blocked_count: self.shield_state.get_blocked_count() as u32,
                            hidden_elements_count: hidden_count,
                        };
                        self.window.update_site_card(card_data);
                    }
                }

                // Update live page state and reload so ads load/strip according to updated policy
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let script = if new_paused {
                            r#"
                                (() => {
                                    try {
                                        window.__BROWSER_SHIELD_PAUSED__ = true;
                                        localStorage.setItem('__browser_shield_paused', 'true');
                                        const s = document.getElementById('__browser_shield_cosmetic__');
                                        if (s) s.remove();
                                        const v = document.querySelector('video');
                                        if (v) {
                                            if (v.playbackRate === 16) v.playbackRate = 1.0;
                                            v.muted = false;
                                        }
                                    } catch(e) {}
                                    location.reload();
                                })();
                            "#
                        } else {
                            r#"
                                (() => {
                                    try {
                                        window.__BROWSER_SHIELD_PAUSED__ = false;
                                        localStorage.setItem('__browser_shield_paused', 'false');
                                        localStorage.removeItem('__browser_shield_paused');
                                    } catch(e) {}
                                    location.reload();
                                })();
                            "#
                        };
                        if ctrl.execute_script(script).is_err() {
                            let _ = ctrl.reload();
                        }
                    }
                }

                self.sync_tabs_ui();
            }
            AppEvent::ToggleSiteCard => {
                if self.window.is_site_card_visible_or_recently_closed() {
                    self.window.hide_site_card();
                    return;
                }
                let (active_url, active_host, is_secure) = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let host = tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
                    let is_sec = tab.url.starts_with("https://");
                    (tab.url.clone(), host, is_sec)
                } else {
                    return;
                };

                let shield_paused = self.shield_state.is_paused(&active_host);
                let shield_enabled = self.shield_state.is_enabled();
                let blocked_count = self.shield_state.get_blocked_count() as u32;
                let is_bookmarked = self.tabs.get(self.active_tab_idx).map(|t| t.is_pinned).unwrap_or(false);
                let is_reader = self.tabs.get(self.active_tab_idx).map(|t| t.is_reader).unwrap_or(false);
                let (hidden_count, _) = if let Ok(st) = self.storage.try_borrow() {
                    let h_cnt = st.get_hidden_elements(&active_host).map(|l| l.len()).unwrap_or(0);
                    (h_cnt, ())
                } else {
                    (0, ())
                };

                let card_data = SiteCardData {
                    url: active_url,
                    host: active_host,
                    is_secure,
                    is_bookmarked,
                    is_reader,
                    shield_enabled,
                    shield_paused_for_site: shield_paused,
                    trackers_blocked_count: blocked_count,
                    hidden_elements_count: hidden_count,
                };

                let rect = self.window.get_url_screen_rect();
                let sender = self.event_tx.clone();
                let main_hwnd = self.window.hwnd();
                let on_action = Box::new(move |action: SiteCardAction| {
                    let ev = match action {
                        SiteCardAction::Share => AppEvent::ShareCurrentPage,
                        SiteCardAction::Bookmark => AppEvent::TogglePinCurrentTab,
                        SiteCardAction::ToggleReader => AppEvent::ToggleReader(None),
                        SiteCardAction::LaunchElementZapper => AppEvent::ToggleZapper,
                        SiteCardAction::Screenshot => AppEvent::CaptureScreenshot,
                        SiteCardAction::RestoreHiddenElements => AppEvent::RestoreAllHiddenElements,
                        SiteCardAction::ToggleShield => AppEvent::ToggleSiteShield,
                        SiteCardAction::ClearSiteData => AppEvent::ClearSiteData,
                        SiteCardAction::Close => return,
                    };
                    let _ = sender.send(ev);
                    unsafe {
                        let _ = PostMessageW(Some(main_hwnd), WM_APP, WPARAM(0), LPARAM(0));
                    }
                });

                self.window.show_site_card(rect.right, rect.bottom + 4, card_data, on_action);
            }
            AppEvent::ToggleSiteKeepAwake => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() { return; }
                if let Ok(st) = self.storage.try_borrow() {
                    let current = st.get_site_keep_awake(&active_host).unwrap_or(false);
                    let new_val = !current;
                    let _ = st.set_site_keep_awake(&active_host, new_val);
                    let msg = if new_val {
                        format!("Keep Awake enabled for {}", active_host)
                    } else {
                        format!("Keep Awake disabled for {}", active_host)
                    };
                    self.window.show_toast(&msg);
                }
            }
            AppEvent::ToggleSiteAutoplay => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() { return; }
                if let Ok(st) = self.storage.try_borrow() {
                    let current = st.get_site_autoplay(&active_host).unwrap_or(true);
                    let new_val = !current;
                    let _ = st.set_site_autoplay(&active_host, new_val);
                    let msg = if new_val {
                        format!("Sound Autoplay allowed for {}", active_host)
                    } else {
                        format!("Sound Autoplay blocked for {}", active_host)
                    };
                    self.window.show_toast(&msg);
                }
            }
            AppEvent::SetSitePermission { perm, state } => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() { return; }
                if let Ok(st) = self.storage.try_borrow() {
                    let _ = st.set_site_permission(&active_host, &perm, state);
                    let state_str = match state {
                        1 => "Allowed",
                        2 => "Blocked",
                        _ => "Ask",
                    };
                    self.window.show_toast(&format!("{}: {} for {}", perm, state_str, active_host));
                }
            }
            AppEvent::ClearSiteData => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() { return; }

                let sender = self.event_tx.clone();
                let main_hwnd = self.window.hwnd();
                self.window.show_clear_data_dialog(&active_host, Box::new(move |confirmed| {
                    if confirmed {
                        let _ = sender.send(AppEvent::ConfirmClearSiteData);
                        unsafe {
                            let _ = PostMessageW(Some(main_hwnd), WM_APP, WPARAM(0), LPARAM(0));
                        }
                    }
                }));
            }
            AppEvent::ConfirmClearSiteData => {
                let active_host = if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default())
                } else {
                    String::new()
                };
                if active_host.is_empty() { return; }
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref ctrl) = tab.controller {
                        let _ = ctrl.clear_site_data(&active_host);
                    }
                }
                self.window.show_toast(&format!("Cookies and site data cleared for {}", active_host));
            }
            AppEvent::AcceptPermission(idx) => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    if idx < tab.pending_permissions.len() {
                        let perm = tab.pending_permissions.remove(idx);
                        let host = tab.controller.as_ref().map(|c| c.get_current_host()).unwrap_or_else(|| engine::extract_host(&tab.url).unwrap_or_default());
                        let (perm_name, label) = match perm {
                            ui::SitePermissionKind::Notifications => ("notifications", "Notifications"),
                            ui::SitePermissionKind::Camera => ("camera", "Camera"),
                            ui::SitePermissionKind::Microphone => ("microphone", "Microphone"),
                            ui::SitePermissionKind::Location => ("location", "Location"),
                        };
                        if let Ok(st) = self.storage.try_borrow() {
                            let _ = st.set_site_permission(&host, perm_name, 1);
                        }
                        self.window.show_toast(&format!("{} allowed for {}", label, host));
                        self.sync_tabs_ui();
                    }
                }
            }
            AppEvent::CaptureScreenshot => {
                capture_screenshot_to_clipboard(self.window.hwnd());
                self.window.show_toast("Screenshot copied to clipboard");
            }
            AppEvent::OpenFolder(idx) => {
                if let Some(folder) = self.folders.get_mut(idx) {
                    if !folder.is_open {
                        folder.is_open = true;
                        self.window.start_tab_anim_timer();
                        self.sync_tabs_ui();
                        self.persist_state();
                    }
                }
            }
            AppEvent::DropTab { source, destination } => {
                self.handle_tab_drop(source, destination);
            }
            AppEvent::OpenInSplitView(tab_id) => {
                self.start_split(Some(tab_id));
            }
            AppEvent::SplitWithNewTab => {
                if let Some(tab_ids) = self.window.take_selected_split_tabs() {
                    self.handle_split_tabs(tab_ids);
                } else {
                    self.start_split(None);
                }
            }
            AppEvent::FocusOtherPane => {
                self.focus_other_pane();
            }
            AppEvent::SwapSplit => {
                self.swap_split();
            }
            AppEvent::EvenSplit => {
                self.even_split();
            }
            AppEvent::SeparateSplit => {
                self.separate_split();
            }
            AppEvent::CloseSplit => {
                self.close_split();
            }
            AppEvent::SplitRatioChanged(ratio) => {
                self.split_ratio_changed(ratio);
            }
            AppEvent::SplitTabs(tab_ids) => {
                self.handle_split_tabs(tab_ids);
            }
            AppEvent::SplitSizesChanged(sizes) => {
                self.split_sizes_changed(sizes);
            }
            AppEvent::OpenGlance { url, from_tab_id } => {
                self.open_glance(&url, from_tab_id);
            }
            AppEvent::GlanceClose => {
                self.close_glance();
            }
            AppEvent::GlanceOpenAsTab => {
                self.promote_glance_to_tab(false);
            }
            AppEvent::GlanceOpenInSplit => {
                self.promote_glance_to_tab(true);
            }
            AppEvent::GlanceTitleChanged { tab_id, title } => {
                if self.glance_tab.as_ref().map(|t| t.id == tab_id) == Some(true) {
                    if let Some(ref mut tab) = self.glance_tab {
                        if !title.is_empty() {
                            tab.title = title;
                        }
                    }
                } else {
                    self.handle_event(AppEvent::TitleChanged { tab_id, title });
                }
            }
            AppEvent::GlanceUrlChanged { tab_id, url: new_url } => {
                if self.glance_tab.as_ref().map(|t| t.id == tab_id) == Some(true) {
                    if let Some(ref mut tab) = self.glance_tab {
                        tab.url = new_url.clone();
                        tab.accent = accent_for_url(&new_url);
                    }
                } else {
                    self.handle_event(AppEvent::UrlChanged { tab_id, url: new_url });
                }
            }
            AppEvent::GlanceFaviconChanged { tab_id, bytes } => {
                if self.glance_tab.as_ref().map(|t| t.id == tab_id) == Some(true) {
                    if let Some(ref mut tab) = self.glance_tab {
                        if let Some(arc_fav) = ui::decode_favicon(&bytes) {
                            let domain = ui::domain_of(&tab.url);
                            if !domain.is_empty() {
                                self.favicon_cache.insert(domain, arc_fav.clone());
                            }
                            tab.favicon = Some(arc_fav);
                        }
                    }
                } else {
                    self.handle_event(AppEvent::FaviconChanged { tab_id, bytes });
                }
            }
            AppEvent::GlanceNavigate { tab_id, url: target_url } => {
                if self.glance_tab.as_ref().map(|t| t.id == tab_id) == Some(true) {
                    if let Some(ref tab) = self.glance_tab {
                        if let Some(ref ctrl) = tab.controller {
                            let cleaned = engine::clean_link::clean_url(&target_url);
                            let _ = ctrl.navigate(&cleaned);
                        }
                    }
                } else {
                    self.handle_event(AppEvent::CreateTab(target_url));
                }
            }
            AppEvent::ToggleReader(target_tab_id) => {
                self.toggle_reader(target_tab_id);
            }
            AppEvent::ReaderMessage { tab_id, msg } => {
                match msg {
                    ReaderMessage::TwinFound { gen, url, text, title, from } => {
                        let look = self.effective_reader_look();
                        if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                            if self.tabs[pos].seeking_reader_twin && self.tabs[pos].reader_gen == gen {
                                let parsed = engine::parse_markdown_page(&text, Some(&url));
                                let final_title = parsed
                                    .title
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(ToOwned::to_owned)
                                    .or_else(|| {
                                        let t = title.trim();
                                        if t.is_empty() {
                                            None
                                        } else {
                                            Some(t.to_string())
                                        }
                                    })
                                    .unwrap_or_else(|| friendly_title_for_url(&url));
                                let from_label = if from.trim().is_empty() {
                                    ui::domain_of(&url)
                                } else {
                                    from
                                };
                                self.tabs[pos].seeking_reader_twin = false;
                                self.tabs[pos].reader_markdown = Some(text);
                                if let Some(ref ctrl) = self.tabs[pos].controller {
                                    let _ = ctrl.show_reader_html(gen, &parsed.html, &final_title, &from_label, look);
                                }
                            }
                        }
                    }
                    ReaderMessage::TwinNone { gen } => {
                        let look = self.effective_reader_look();
                        if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                            if self.tabs[pos].seeking_reader_twin && self.tabs[pos].reader_gen == gen {
                                self.tabs[pos].seeking_reader_twin = false;
                                if let Some(ref ctrl) = self.tabs[pos].controller {
                                    let _ = ctrl.run_reader_heuristic(gen, look);
                                }
                            }
                        }
                    }
                    ReaderMessage::Result { gen, active } => {
                        let is_active = self.tabs.get(self.active_tab_idx).map_or(false, |t| t.id == tab_id);
                        if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                            if self.tabs[pos].reader_gen == gen {
                                self.tabs[pos].is_reader = active;
                                self.tabs[pos].seeking_reader_twin = false;
                                if !active {
                                    self.tabs[pos].reader_markdown = None;
                                    if is_active {
                                        self.window.show_toast("Nothing to read on this page");
                                    }
                                }
                                if is_active {
                                    self.refresh_site_card_if_visible();
                                    self.sync_tabs_ui();
                                }
                            }
                        }
                    }
                    ReaderMessage::LookChanged { look } => {
                        if let Ok(st) = self.storage.try_borrow() {
                            let _ = st.set_reader_follows_look(true);
                            let _ = st.set_reader_look(look.as_str());
                        }
                        for tab in &self.tabs {
                            if tab.is_reader {
                                if let Some(ref ctrl) = tab.controller {
                                    let _ = ctrl.apply_reader_look(look);
                                }
                            }
                        }
                    }
                    ReaderMessage::MarkdownExport { markdown } => {
                        if !markdown.trim().is_empty() {
                            copy_to_clipboard(self.window.hwnd(), &markdown);
                            self.window.show_toast("Copied page as Markdown");
                        } else {
                            self.window.show_toast("Nothing to export on this page");
                        }
                    }
                }
            }
            AppEvent::CycleReaderLook => {
                let current = if let Ok(st) = self.storage.try_borrow() {
                    let follows = st.get_reader_follows_look().unwrap_or(true);
                    if !follows {
                        ReaderLook::Light
                    } else {
                        ReaderLook::from_str(&st.get_reader_look().unwrap_or_else(|_| "system".to_string()))
                    }
                } else {
                    ReaderLook::System
                };
                let next = current.next();
                if let Ok(st) = self.storage.try_borrow() {
                    let _ = st.set_reader_follows_look(true);
                    let _ = st.set_reader_look(next.as_str());
                }
                for tab in &self.tabs {
                    if tab.is_reader {
                        if let Some(ref ctrl) = tab.controller {
                            let _ = ctrl.apply_reader_look(next);
                        }
                    }
                }
                self.window.show_toast(&format!("Reader look: {}", next.title()));
            }
            AppEvent::CopyPageAsMarkdown(target_tab_id) => {
                if let Some(pos) = self.resolve_tab_index_by_id(target_tab_id.as_deref()) {
                    if self.tabs[pos].is_reader {
                        if let Some(ref raw) = self.tabs[pos].reader_markdown {
                            copy_to_clipboard(self.window.hwnd(), raw);
                            self.window.show_toast("Copied page as Markdown");
                            return;
                        }
                    }
                    if let Some(ref ctrl) = self.tabs[pos].controller {
                        let _ = ctrl.export_reader_markdown();
                    } else {
                        self.window.show_toast("Tab is sleeping (load page first to export Markdown)");
                    }
                } else if target_tab_id.is_some() {
                    self.window.show_toast("Tab is sleeping (load page first to export Markdown)");
                }
            }
            AppEvent::CopyMarkdownLink(target_tab_id) => {
                if let Some(pos) = self.resolve_tab_index_by_id(target_tab_id.as_deref()) {
                    let tab = &self.tabs[pos];
                    let live_url = tab
                        .controller
                        .as_ref()
                        .and_then(|c| c.get_source().ok())
                        .unwrap_or_default();
                    let raw_url = if !live_url.is_empty() && live_url != "about:blank" {
                        live_url
                    } else {
                        tab.url.clone()
                    };
                    let clean = engine::clean_url(&raw_url);
                    let md_link = engine::format_markdown_link(&tab.title, &clean);
                    copy_to_clipboard(self.window.hwnd(), &md_link);
                    self.window.show_toast("Copied Markdown link");
                } else if let Some(tid) = target_tab_id.as_deref() {
                    if let Some(sc) = self.pinned_shortcuts.iter().find(|s| s.id == tid) {
                        let clean = engine::clean_url(&sc.url);
                        let md_link = engine::format_markdown_link(&sc.title, &clean);
                        copy_to_clipboard(self.window.hwnd(), &md_link);
                        self.window.show_toast("Copied Markdown link");
                    }
                }
            }
        }
    }

    fn move_tab_to_section_slot(
        &mut self,
        tab_id: &str,
        new_folder_id: Option<String>,
        slot: usize,
    ) {
        let active_id = self.tabs.get(self.active_tab_idx).map(|t| t.id.clone());
        if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
            let mut tab = self.tabs.remove(pos);
            tab.folder_id = new_folder_id.clone();
            tab.is_pinned = false;

            let matching_indices: Vec<usize> = self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_pinned && t.folder_id == new_folder_id)
                .map(|(i, _)| i)
                .collect();

            if matching_indices.is_empty() {
                self.tabs.push(tab);
            } else if slot < matching_indices.len() {
                self.tabs.insert(matching_indices[slot], tab);
            } else {
                let last_idx = *matching_indices.last().unwrap();
                self.tabs.insert(last_idx + 1, tab);
            }
        }
        if let Some(id) = active_id {
            if let Some(new_idx) = self.tabs.iter().position(|t| t.id == id) {
                self.active_tab_idx = new_idx;
            }
        }
    }

    fn handle_tab_drop(&mut self, source: ui::DragSource, dest: ui::TabDropDestination) {
        let target_tab_id = match source {
            ui::DragSource::Tab(tab_idx) => {
                self.tabs.iter().filter(|t| !t.is_pinned).nth(tab_idx).map(|t| t.id.clone())
            }
            ui::DragSource::Pinned(_) => None,
        };

        match dest {
            ui::TabDropDestination::Favorites(slot) => {
                match source {
                    ui::DragSource::Pinned(from_idx) => {
                        if from_idx < self.pinned_shortcuts.len() {
                            let item = self.pinned_shortcuts.remove(from_idx);
                            let insert_at = slot.min(self.pinned_shortcuts.len());
                            self.pinned_shortcuts.insert(insert_at, item);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                    ui::DragSource::Tab(_) => {
                        if let Some(tab_id) = target_tab_id {
                            if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
                                self.tabs[pos].is_pinned = true;
                                let sc_url = self.tabs[pos].url.clone();
                                let sc_domain = ui::domain_of(&sc_url);
                                let existing_idx = self.pinned_shortcuts.iter().position(|s| {
                                    s.url == sc_url || (!sc_domain.is_empty() && ui::domain_of(&s.url) == sc_domain)
                                });
                                let sc = if let Some(idx) = existing_idx {
                                    self.pinned_shortcuts.remove(idx)
                                } else {
                                    ui::TabItem {
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
                                        custom_letter: self.tabs[pos].custom_letter.clone(),
                                        is_split: false,
                                    }
                                };
                                let insert_at = slot.min(self.pinned_shortcuts.len());
                                self.pinned_shortcuts.insert(insert_at, sc);
                                self.window.show_toast("Added to Favourite tabs");
                                self.sync_tabs_ui();
                                self.persist_state();
                            }
                        }
                    }
                }
            }
            ui::TabDropDestination::Folder(folder_id) => {
                match source {
                    ui::DragSource::Pinned(fav_idx) => {
                        if fav_idx < self.pinned_shortcuts.len() {
                            let item = self.pinned_shortcuts.remove(fav_idx);
                            let rem_domain = ui::domain_of(&item.url);
                            let open_pos = self.tabs.iter().position(|t| {
                                t.is_pinned && (t.url == item.url || (!rem_domain.is_empty() && ui::domain_of(&t.url) == rem_domain))
                            });
                            let tab_id = if let Some(pos) = open_pos {
                                self.tabs[pos].is_pinned = false;
                                self.tabs[pos].id.clone()
                            } else {
                                let id = format!("tab-{}", self.next_tab_id);
                                self.next_tab_id += 1;
                                self.tabs.push(Tab {
                                    id: id.clone(),
                                    title: item.title,
                                    url: item.url,
                                    accent: item.accent,
                                    favicon: item.favicon,
                                    is_pinned: false,
                                    folder_id: None,
                                    custom_title: false,
                                    custom_letter: item.custom_letter,
                                    is_muted: false,
                                    controller: None,
                                    last_active: Instant::now(),
                                    anim_progress: 1.0,
                                    is_closing: false,
                                    pending_permissions: Vec::new(),
                                    is_reader: false,
                                    seeking_reader_twin: false,
                                    reader_gen: 0,
                                    reader_markdown: None,
                                });
                                id
                            };
                            self.move_tab_to_section_slot(&tab_id, Some(folder_id), usize::MAX);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                    ui::DragSource::Tab(_) => {
                        if let Some(tab_id) = target_tab_id {
                            self.move_tab_to_section_slot(&tab_id, Some(folder_id), usize::MAX);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                }
            }
            ui::TabDropDestination::FolderInsert { folder_id, insert_idx } => {
                match source {
                    ui::DragSource::Pinned(fav_idx) => {
                        if fav_idx < self.pinned_shortcuts.len() {
                            let item = self.pinned_shortcuts.remove(fav_idx);
                            let rem_domain = ui::domain_of(&item.url);
                            let open_pos = self.tabs.iter().position(|t| {
                                t.is_pinned && (t.url == item.url || (!rem_domain.is_empty() && ui::domain_of(&t.url) == rem_domain))
                            });
                            let tab_id = if let Some(pos) = open_pos {
                                self.tabs[pos].is_pinned = false;
                                self.tabs[pos].id.clone()
                            } else {
                                let id = format!("tab-{}", self.next_tab_id);
                                self.next_tab_id += 1;
                                self.tabs.push(Tab {
                                    id: id.clone(),
                                    title: item.title,
                                    url: item.url,
                                    accent: item.accent,
                                    favicon: item.favicon,
                                    is_pinned: false,
                                    folder_id: None,
                                    custom_title: false,
                                    custom_letter: item.custom_letter,
                                    is_muted: false,
                                    controller: None,
                                    last_active: Instant::now(),
                                    anim_progress: 1.0,
                                    is_closing: false,
                                    pending_permissions: Vec::new(),
                                    is_reader: false,
                                    seeking_reader_twin: false,
                                    reader_gen: 0,
                                    reader_markdown: None,
                                });
                                id
                            };
                            self.move_tab_to_section_slot(&tab_id, Some(folder_id), insert_idx);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                    ui::DragSource::Tab(_) => {
                        if let Some(tab_id) = target_tab_id {
                            self.move_tab_to_section_slot(&tab_id, Some(folder_id), insert_idx);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                }
            }
            ui::TabDropDestination::PinnedSection(slot) => {
                match source {
                    ui::DragSource::Pinned(fav_idx) => {
                        if fav_idx < self.pinned_shortcuts.len() {
                            let item = self.pinned_shortcuts.remove(fav_idx);
                            let rem_domain = ui::domain_of(&item.url);
                            let open_pos = self.tabs.iter().position(|t| {
                                t.is_pinned && (t.url == item.url || (!rem_domain.is_empty() && ui::domain_of(&t.url) == rem_domain))
                            });
                            let tab_id = if let Some(pos) = open_pos {
                                self.tabs[pos].is_pinned = false;
                                self.tabs[pos].id.clone()
                            } else {
                                let id = format!("tab-{}", self.next_tab_id);
                                self.next_tab_id += 1;
                                self.tabs.push(Tab {
                                    id: id.clone(),
                                    title: item.title,
                                    url: item.url,
                                    accent: item.accent,
                                    favicon: item.favicon,
                                    is_pinned: false,
                                    folder_id: None,
                                    custom_title: false,
                                    custom_letter: item.custom_letter,
                                    is_muted: false,
                                    controller: None,
                                    last_active: Instant::now(),
                                    anim_progress: 1.0,
                                    is_closing: false,
                                    pending_permissions: Vec::new(),
                                    is_reader: false,
                                    seeking_reader_twin: false,
                                    reader_gen: 0,
                                    reader_markdown: None,
                                });
                                id
                            };
                            self.move_tab_to_section_slot(&tab_id, Some(ui::PINNED_SECTION_ID.to_string()), slot);
                            self.window.show_toast("Added to Pinned tabs");
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                    ui::DragSource::Tab(_) => {
                        if let Some(tab_id) = target_tab_id {
                            self.move_tab_to_section_slot(&tab_id, Some(ui::PINNED_SECTION_ID.to_string()), slot);
                            self.window.show_toast("Added to Pinned tabs");
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                }
            }
            ui::TabDropDestination::NormalTabs(slot) => {
                match source {
                    ui::DragSource::Pinned(fav_idx) => {
                        if fav_idx < self.pinned_shortcuts.len() {
                            let item = self.pinned_shortcuts.remove(fav_idx);
                            let rem_domain = ui::domain_of(&item.url);
                            let open_pos = self.tabs.iter().position(|t| {
                                t.is_pinned && (t.url == item.url || (!rem_domain.is_empty() && ui::domain_of(&t.url) == rem_domain))
                            });
                            let tab_id = if let Some(pos) = open_pos {
                                self.tabs[pos].is_pinned = false;
                                self.tabs[pos].id.clone()
                            } else {
                                let id = format!("tab-{}", self.next_tab_id);
                                self.next_tab_id += 1;
                                self.tabs.push(Tab {
                                    id: id.clone(),
                                    title: item.title,
                                    url: item.url,
                                    accent: item.accent,
                                    favicon: item.favicon,
                                    is_pinned: false,
                                    folder_id: None,
                                    custom_title: false,
                                    custom_letter: item.custom_letter,
                                    is_muted: false,
                                    controller: None,
                                    last_active: Instant::now(),
                                    anim_progress: 1.0,
                                    is_closing: false,
                                    pending_permissions: Vec::new(),
                                    is_reader: false,
                                    seeking_reader_twin: false,
                                    reader_gen: 0,
                                    reader_markdown: None,
                                });
                                id
                            };
                            self.move_tab_to_section_slot(&tab_id, None, slot);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                    ui::DragSource::Tab(_) => {
                        if let Some(tab_id) = target_tab_id {
                            self.move_tab_to_section_slot(&tab_id, None, slot);
                            self.sync_tabs_ui();
                            self.persist_state();
                        }
                    }
                }
            }
            ui::TabDropDestination::SplitContainer { split_id } => {
                let dropped_tab_id = match source {
                    ui::DragSource::Tab(tab_idx) => {
                        self.tabs.iter().filter(|t| !t.is_pinned).nth(tab_idx).map(|t| t.id.clone())
                    }
                    ui::DragSource::Pinned(fav_idx) => {
                        self.pinned_shortcuts.get(fav_idx).cloned().map(|sc| {
                            let tab_id = format!("tab-{}", self.next_tab_id);
                            self.next_tab_id += 1;
                            self.tabs.push(Tab {
                                id: tab_id.clone(),
                                title: sc.title,
                                url: sc.url,
                                accent: sc.accent,
                                favicon: sc.favicon,
                                is_pinned: false,
                                folder_id: None,
                                custom_title: false,
                                custom_letter: sc.custom_letter,
                                is_muted: false,
                                controller: None,
                                last_active: Instant::now(),
                                anim_progress: 1.0,
                                is_closing: false,
                                pending_permissions: Vec::new(),
                                is_reader: false,
                                seeking_reader_twin: false,
                                reader_gen: 0,
                                reader_markdown: None,
                            });
                            tab_id
                        })
                    }
                };
                let Some(dropped_id) = dropped_tab_id else { return };

                // If already in this split or full, ignore
                if let Some(s) = self.splits.iter().find(|s| s.id == split_id) {
                    if s.contains(&dropped_id) || s.is_full() {
                        return;
                    }
                }

                // Remove from any other split
                for s in self.splits.iter_mut() {
                    if s.contains(&dropped_id) {
                        s.remove(&dropped_id);
                    }
                }
                self.splits.retain(|s| s.tabs.len() >= 2);

                let mut grouped_tabs = None;
                if let Some(s) = self.splits.iter_mut().find(|sp| sp.id == split_id) {
                    if !s.is_full() {
                        let last_tab = s.tabs.last().cloned();
                        s.insert(dropped_id.clone(), last_tab.as_deref(), false);
                        s.focused_tab_id = dropped_id.clone();
                        grouped_tabs = Some(s.tabs.clone());
                    }
                }
                if let Some(ordered) = grouped_tabs {
                    self.group_split_tabs_in_list(&ordered);
                }

                if let Some(pos) = self.tabs.iter().position(|t| t.id == dropped_id) {
                    self.active_tab_idx = pos;
                }

                self.sync_split_bounds();
                if let Some(ctrl) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                    let _ = ctrl.focus();
                }
                self.sync_tabs_ui();
                self.persist_state();
            }
            ui::TabDropDestination::WindowSplit { target_tab_id, on_left } => {
                let dropped_tab_id = match source {
                    ui::DragSource::Tab(tab_idx) => {
                        self.tabs.iter().filter(|t| !t.is_pinned).nth(tab_idx).map(|t| t.id.clone())
                    }
                    ui::DragSource::Pinned(fav_idx) => {
                        self.pinned_shortcuts.get(fav_idx).cloned().map(|sc| {
                            let tab_id = format!("tab-{}", self.next_tab_id);
                            self.next_tab_id += 1;
                            self.tabs.push(Tab {
                                id: tab_id.clone(),
                                title: sc.title,
                                url: sc.url,
                                accent: sc.accent,
                                favicon: sc.favicon,
                                is_pinned: false,
                                folder_id: None,
                                custom_title: false,
                                custom_letter: sc.custom_letter,
                                is_muted: false,
                                controller: None,
                                last_active: Instant::now(),
                                anim_progress: 1.0,
                                is_closing: false,
                                pending_permissions: Vec::new(),
                                is_reader: false,
                                seeking_reader_twin: false,
                                reader_gen: 0,
                                reader_markdown: None,
                            });
                            tab_id
                        })
                    }
                };
                let Some(dropped_id) = dropped_tab_id else { return };
                if self.tabs.is_empty() || self.active_tab_idx >= self.tabs.len() {
                    return;
                }
                let active_id = self.tabs[self.active_tab_idx].id.clone();
                if dropped_id == active_id {
                    return;
                }

                // If dropped_id is in any split, remove it from that split
                for s in self.splits.iter_mut() {
                    if s.contains(&dropped_id) {
                        s.remove(&dropped_id);
                    }
                }
                self.splits.retain(|s| s.tabs.len() >= 2);

                let grouped_tabs = if let Some(s) = self.find_split_for_tab_mut(&active_id) {
                    if !s.is_full() && !s.contains(&dropped_id) {
                        let beside_id = target_tab_id.as_deref().unwrap_or(&active_id);
                        s.insert(dropped_id.clone(), Some(beside_id), on_left);
                        s.focused_tab_id = dropped_id.clone();
                        Some(s.tabs.clone())
                    } else {
                        None
                    }
                } else {
                    let tabs = if on_left {
                        vec![dropped_id.clone(), active_id]
                    } else {
                        vec![active_id, dropped_id.clone()]
                    };
                    let split_id = format!("split-{}", self.next_tab_id);
                    self.next_tab_id += 1;
                    self.splits.push(TabSplit::new(split_id, tabs.clone(), dropped_id.clone()));
                    Some(tabs)
                };

                if let Some(ordered) = grouped_tabs {
                    self.group_split_tabs_in_list(&ordered);
                }

                if let Some(pos) = self.tabs.iter().position(|t| t.id == dropped_id) {
                    self.active_tab_idx = pos;
                }

                self.sync_split_bounds();
                if let Some(ctrl) = self.tabs.get(self.active_tab_idx).and_then(|t| t.controller.as_ref()) {
                    let _ = ctrl.focus();
                }
                self.sync_tabs_ui();
                self.persist_state();
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

    // Stop applying uBlock Origin for now so native ad shielding & YouTube fast-forwarding can be evaluated
    let _ = std::fs::remove_dir_all(app_data_dir.join("extensions"));

    // Create App state
    let app = Rc::new(RefCell::new(BrowserApp::new(
        window.clone(),
        command_bar_cell.clone(),
        engine.clone(),
        storage.clone(),
        event_tx.clone(),
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
                CommandAction::ToggleReader => AppEvent::ToggleReader(None),
                CommandAction::CycleReaderLook => AppEvent::CycleReaderLook,
                CommandAction::CopyPageAsMarkdown => AppEvent::CopyPageAsMarkdown(None),
                CommandAction::CopyMarkdownLink => AppEvent::CopyMarkdownLink(None),
                CommandAction::RestoreAllHidden => AppEvent::RestoreAllHiddenElements,
                CommandAction::CopyCleanLink => AppEvent::CopyCleanLink,
                CommandAction::ToggleShield => AppEvent::ToggleShield,
                CommandAction::ToggleSiteShield => AppEvent::ToggleSiteShield,
                CommandAction::ToggleSiteCard => AppEvent::ToggleSiteCard,
                CommandAction::OpenInSplitView => AppEvent::SplitWithNewTab,
                CommandAction::SwapSplit => AppEvent::SwapSplit,
                CommandAction::EvenSplit => AppEvent::EvenSplit,
                CommandAction::SeparateSplit => AppEvent::SeparateSplit,
                CommandAction::CloseSplit => AppEvent::CloseSplit,
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
            SidebarAction::ToggleReader(tid) => AppEvent::ToggleReader(tid),
            SidebarAction::CopyPageAsMarkdown(tid) => AppEvent::CopyPageAsMarkdown(tid),
            SidebarAction::CopyMarkdownLink(tid) => AppEvent::CopyMarkdownLink(tid),
            SidebarAction::RestoreAllHidden => AppEvent::RestoreAllHiddenElements,
            SidebarAction::ToggleShield => AppEvent::ToggleShield,
            SidebarAction::ToggleSiteShield => AppEvent::ToggleSiteShield,
            SidebarAction::ToggleSiteCard => AppEvent::ToggleSiteCard,
            SidebarAction::AcceptPermission(idx) => AppEvent::AcceptPermission(idx),
            SidebarAction::DropTab { source, destination } => AppEvent::DropTab { source, destination },
            SidebarAction::OpenFolder(idx) => AppEvent::OpenFolder(idx),
            SidebarAction::SetPinnedLetter(idx, letter) => AppEvent::SetPinnedLetter(idx, letter),
            SidebarAction::OpenInSplitView(tab_id) => AppEvent::OpenInSplitView(tab_id),
            SidebarAction::SplitWithNewTab => AppEvent::SplitWithNewTab,
            SidebarAction::FocusOtherPane => AppEvent::FocusOtherPane,
            SidebarAction::SwapSplit => AppEvent::SwapSplit,
            SidebarAction::EvenSplit => AppEvent::EvenSplit,
            SidebarAction::SeparateSplit => AppEvent::SeparateSplit,
            SidebarAction::CloseSplit => AppEvent::CloseSplit,
            SidebarAction::SplitRatioChanged(ratio) => AppEvent::SplitRatioChanged(ratio),
            SidebarAction::SplitTabs(tab_ids) => AppEvent::SplitTabs(tab_ids),
            SidebarAction::SplitSizesChanged(sizes) => AppEvent::SplitSizesChanged(sizes),
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
        if let Ok(mut app) = app_for_resize.try_borrow_mut() {
            app.sync_split_bounds();
        }
        if let Ok(cb_opt) = cb_for_resize.try_borrow() {
            if let Some(ref cb) = *cb_opt {
                if cb.is_visible() {
                    cb.recenter(content_rect);
                }
            }
        }
    });

    // Hook Window move to keep floating overlays (Glance, CommandBar) aligned
    let app_for_move = app.clone();
    let cb_for_move = command_bar_cell.clone();
    window.set_move_callback(move |content_rect, _sidebar_rect| {
        if let Ok(app) = app_for_move.try_borrow() {
            app.sync_glance_bounds();
        }
        if let Ok(cb_opt) = cb_for_move.try_borrow() {
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
        app.borrow_mut().sync_split_bounds();
        if let Some(ref active) = app.borrow().tabs.get(active_idx).and_then(|t| t.controller.as_ref()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_split_even_and_clamp() {
        assert_eq!(TabSplit::even(2), vec![0.5, 0.5]);
        assert_eq!(TabSplit::even(4), vec![0.25, 0.25, 0.25, 0.25]);

        let clamped_2 = TabSplit::clamp(&[0.05, 0.95]);
        assert_eq!(clamped_2, vec![0.20, 0.80]);

        let clamped_4 = TabSplit::clamp(&[0.05, 0.30, 0.30, 0.35]);
        assert!(clamped_4[0] >= 0.10);
        let sum: f32 = clamped_4.iter().sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_tab_split_insert_up_to_most() {
        let mut split = TabSplit::new(
            "split-1".to_string(),
            vec!["tab-1".to_string(), "tab-2".to_string()],
            "tab-1".to_string(),
        );
        assert_eq!(split.tabs.len(), 2);
        assert!(!split.is_full());

        // Insert 3rd tab
        assert!(split.insert("tab-3".to_string(), Some("tab-2"), false));
        assert_eq!(split.tabs, vec!["tab-1", "tab-2", "tab-3"]);
        assert_eq!(split.sizes.len(), 3);

        // Insert 4th tab (before tab-2)
        assert!(split.insert("tab-4".to_string(), Some("tab-2"), true));
        assert_eq!(split.tabs, vec!["tab-1", "tab-4", "tab-2", "tab-3"]);
        assert_eq!(split.sizes.len(), 4);
        assert!(split.is_full());

        // 5th tab rejected (max 4)
        assert!(!split.insert("tab-5".to_string(), None, false));
        assert_eq!(split.tabs.len(), 4);

        // Duplicate tab rejected
        assert!(!split.insert("tab-1".to_string(), None, false));
    }

    #[test]
    fn test_tab_split_remove() {
        let mut split = TabSplit::new(
            "split-1".to_string(),
            vec!["tab-1".to_string(), "tab-2".to_string(), "tab-3".to_string()],
            "tab-1".to_string(),
        );

        // Remove tab-2: 2 tabs remain, split stays valid
        assert!(split.remove("tab-2"));
        assert_eq!(split.tabs, vec!["tab-1", "tab-3"]);
        assert_eq!(split.sizes.len(), 2);

        // Remove tab-3: only 1 tab remains, split dissolve signal returned (false)
        assert!(!split.remove("tab-3"));
        assert_eq!(split.tabs, vec!["tab-1"]);
    }

    #[test]
    fn test_tab_split_multi_direct() {
        let split_3 = TabSplit::new(
            "split-3way".to_string(),
            vec!["tab-1".to_string(), "tab-2".to_string(), "tab-3".to_string()],
            "tab-1".to_string(),
        );
        assert_eq!(split_3.tabs.len(), 3);
        assert_eq!(split_3.sizes.len(), 3);
        assert!(!split_3.is_full());
        let sum: f32 = split_3.sizes.iter().sum();
        assert!((sum - 1.0).abs() < 0.001);
    }
}

