use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use engine::{init_com, BrowserShortcut, WebViewController, WebViewEngine};
use storage::Storage;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ui::{
    BrowserWindow, CommandAction, CommandBar, SidebarAction, TabItemView, WindowConfig,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

fn get_app_data_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("ArcBrowser")
    } else {
        PathBuf::from(".arc_browser")
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

struct Tab {
    id: String,
    title: String,
    url: String,
    controller: WebViewController,
}

enum AppEvent {
    CreateTab(String),
    SwitchTab(usize),
    CloseTab(usize),
    Navigate(String),
    NavBack,
    NavForward,
    NavReload,
    ToggleSidebar,
    OpenCommandBar,
    TitleChanged { tab_id: String, title: String },
    UrlChanged { tab_id: String, url: String },
}

struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab_idx: usize,
    next_tab_id: usize,
    window: Rc<BrowserWindow>,
    command_bar: Rc<RefCell<Option<CommandBar>>>,
    engine: Rc<WebViewEngine>,
    event_tx: mpsc::Sender<AppEvent>,
}

impl BrowserApp {
    fn new(
        window: Rc<BrowserWindow>,
        command_bar: Rc<RefCell<Option<CommandBar>>>,
        engine: Rc<WebViewEngine>,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_idx: 0,
            next_tab_id: 1,
            window,
            command_bar,
            engine,
            event_tx,
        }
    }

    fn create_tab(&mut self, url: &str) {
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
                BrowserShortcut::NewTab => AppEvent::OpenCommandBar,
                BrowserShortcut::CloseTab => AppEvent::CloseTab(usize::MAX), // indicates active tab
                BrowserShortcut::ToggleSidebar => AppEvent::ToggleSidebar,
                BrowserShortcut::Reload => AppEvent::NavReload,
            };
            let _ = tx4.send(ev);
            unsafe {
                let hwnd = HWND(hwnd_val as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            }
        });

        let controller = match self.engine.create_controller(
            self.window.hwnd(),
            content_rect,
            url,
            None,
            Some(on_title),
            Some(on_url),
            Some(on_new_window),
            Some(on_shortcut),
        ) {
            Ok(c) => c,
            Err(e) => {
                info!("Failed to create tab controller: {}", e);
                return;
            }
        };

        // Hide currently active tab if any
        if let Some(current) = self.tabs.get(self.active_tab_idx) {
            let _ = current.controller.set_visible(false);
        }

        let new_idx = self.tabs.len();
        self.tabs.push(Tab {
            id: tab_id,
            title: "New Tab".to_string(),
            url: url.to_string(),
            controller,
        });

        self.switch_tab(new_idx);
    }

    fn switch_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }

        if self.active_tab_idx < self.tabs.len() && self.active_tab_idx != idx {
            let _ = self.tabs[self.active_tab_idx].controller.set_visible(false);
        }

        self.active_tab_idx = idx;
        let (content_rect, _) = self.window.get_layout_rects();
        let _ = self.tabs[idx].controller.set_bounds(content_rect);
        let _ = self.tabs[idx].controller.set_visible(true);

        self.sync_tabs_ui();
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

        if self.tabs.len() <= 1 {
            // If only one tab left, navigate to google instead of leaving empty browser
            let _ = self.tabs[0].controller.navigate("https://www.google.com");
            return;
        }

        // Hide and remove the tab
        let _ = self.tabs[target_idx].controller.set_visible(false);
        self.tabs.remove(target_idx);

        if self.active_tab_idx >= self.tabs.len() {
            self.active_tab_idx = self.tabs.len().saturating_sub(1);
        } else if self.active_tab_idx > target_idx {
            self.active_tab_idx -= 1;
        }

        let (content_rect, _) = self.window.get_layout_rects();
        if let Some(active) = self.tabs.get(self.active_tab_idx) {
            let _ = active.controller.set_bounds(content_rect);
            let _ = active.controller.set_visible(true);
        }

        self.sync_tabs_ui();
    }

    fn sync_tabs_ui(&self) {
        let items: Vec<TabItemView> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabItemView {
                id: t.id.clone(),
                title: t.title.clone(),
                is_active: i == self.active_tab_idx,
            })
            .collect();
        self.window.update_tabs(items);
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::CreateTab(url) => {
                self.create_tab(&url);
            }
            AppEvent::SwitchTab(idx) => {
                self.switch_tab(idx);
            }
            AppEvent::CloseTab(idx) => {
                self.close_tab(idx);
            }
            AppEvent::Navigate(url) => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.url = url.clone();
                    let _ = tab.controller.navigate(&url);
                }
            }
            AppEvent::NavBack => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let _ = tab.controller.go_back();
                }
            }
            AppEvent::NavForward => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let _ = tab.controller.go_forward();
                }
            }
            AppEvent::NavReload => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let _ = tab.controller.reload();
                }
            }
            AppEvent::ToggleSidebar => {
                self.window.toggle_sidebar();
            }
            AppEvent::OpenCommandBar => {
                let (content_rect, _) = self.window.get_layout_rects();
                if let Some(ref cb) = *self.command_bar.borrow() {
                    cb.toggle(content_rect);
                }
            }
            AppEvent::TitleChanged { tab_id, title } => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.title = title;
                    self.sync_tabs_ui();
                }
            }
            AppEvent::UrlChanged { tab_id, url } => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.url = url;
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = Instant::now();

    // Initialize tracing
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();

    info!("Starting Arc-Style Browser for Windows...");

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
    let _storage = Storage::open(&storage_db_path)?;
    let t_after_storage = Instant::now();
    log_timing(
        "2. Storage (SQLite) Init",
        t_after_storage.duration_since(t_before_storage).as_secs_f64() * 1000.0,
        t_after_storage.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 3: Window Creation
    let t_before_window = Instant::now();
    let config = WindowConfig {
        title: "Arc Browser".to_string(),
        width: 1280,
        height: 800,
        sidebar_width: 260,
        caption_height: 48,
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

    // Create App state
    let app = Rc::new(RefCell::new(BrowserApp::new(
        window.clone(),
        command_bar_cell.clone(),
        engine.clone(),
        event_tx.clone(),
    )));

    // Create Command Bar
    let hwnd = window.hwnd();
    let tx_cb = event_tx.clone();
    let command_bar = CommandBar::new(
        hwnd,
        Box::new(move |action| {
            let ev = match action {
                CommandAction::Navigate(url) => AppEvent::Navigate(url),
                CommandAction::NewTab => AppEvent::CreateTab("https://www.google.com".to_string()),
                CommandAction::CloseTab => AppEvent::CloseTab(usize::MAX),
                CommandAction::ToggleSidebar => AppEvent::ToggleSidebar,
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
            SidebarAction::NavNewTab => AppEvent::OpenCommandBar,
            SidebarAction::ToggleSidebar => AppEvent::ToggleSidebar,
            SidebarAction::SelectTab(idx) => AppEvent::SwitchTab(idx),
            SidebarAction::CloseTab(idx) => AppEvent::CloseTab(idx),
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
                let _ = tab.controller.set_bounds(content_rect);
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
        }
    });

    // Initial tab
    let target_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://www.google.com".to_string());

    let t_before_tab = Instant::now();
    app.borrow_mut().create_tab(&target_url);
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

    info!("Arc Browser exiting.");
    Ok(())
}
