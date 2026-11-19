use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use engine::{init_com, WebViewEngine};
use storage::Storage;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ui::{BrowserWindow, WindowConfig};

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

    // Stage 1: COM Initialization
    let t_before_com = Instant::now();
    init_com()?;
    let t_after_com = Instant::now();
    log_timing(
        "1. COM Init",
        t_after_com.duration_since(t_before_com).as_secs_f64() * 1000.0,
        t_after_com.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 2: Storage Initialization (SQLite)
    let t_before_storage = Instant::now();
    let _storage = Storage::open(&storage_db_path)?;
    let t_after_storage = Instant::now();
    log_timing(
        "2. Storage (SQLite) Init",
        t_after_storage.duration_since(t_before_storage).as_secs_f64() * 1000.0,
        t_after_storage.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 3: Window Creation & Frame Setup
    let t_before_window = Instant::now();
    let config = WindowConfig {
        title: "Arc Browser".to_string(),
        width: 1280,
        height: 800,
        sidebar_width: 260,
        caption_height: 48,
    };
    let window = BrowserWindow::new(config)?;
    let t_after_window = Instant::now();
    log_timing(
        "3. Window & Custom Frame Creation",
        t_after_window.duration_since(t_before_window).as_secs_f64() * 1000.0,
        t_after_window.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 4: WebView2 Environment
    let t_before_env = Instant::now();
    let engine = WebViewEngine::new(Some(&webview_data_dir))?;
    let t_after_env = Instant::now();
    log_timing(
        "4. WebView2 Environment Created",
        t_after_env.duration_since(t_before_env).as_secs_f64() * 1000.0,
        t_after_env.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Stage 5: WebView2 Controller Creation
    let (content_rect, _sidebar_rect) = window.get_layout_rects();
    let target_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://www.google.com".to_string());

    let t_start_clone = t0;
    let nav_completed_cb = Box::new(move |success| {
        let t_nav_done = Instant::now();
        let total_ms = t_nav_done.duration_since(t_start_clone).as_secs_f64() * 1000.0;
        log_timing(
            if success {
                "6. First Navigation Completed"
            } else {
                "6. First Navigation Failed"
            },
            0.0,
            total_ms,
        );
        info!(">>> COLD START COMPLETED IN {:.2} ms <<<", total_ms);
    });

    let t_before_ctrl = Instant::now();
    let controller = engine.create_controller(
        window.hwnd(),
        content_rect,
        &target_url,
        Some(nav_completed_cb),
    )?;
    let t_after_ctrl = Instant::now();
    log_timing(
        "5. WebView2 Controller Created",
        t_after_ctrl.duration_since(t_before_ctrl).as_secs_f64() * 1000.0,
        t_after_ctrl.duration_since(t0).as_secs_f64() * 1000.0,
    );

    // Hook resize to keep WebView2 bounds synchronized with content area
    let ctrl_cell = Rc::new(RefCell::new(controller));
    let ctrl_for_resize = ctrl_cell.clone();
    window.set_resize_callback(move |content_rect, _sidebar_rect| {
        if let Ok(ctrl) = ctrl_for_resize.try_borrow() {
            let _ = ctrl.set_bounds(content_rect);
        }
    });

    // Show window
    window.show();
    let t_shown = Instant::now();
    log_timing(
        "*. Window Visible (Interactive)",
        t_shown.duration_since(t_after_ctrl).as_secs_f64() * 1000.0,
        t_shown.duration_since(t0).as_secs_f64() * 1000.0,
    );

    info!("Entering message loop...");
    window.run_message_loop();

    info!("Arc Browser exiting.");
    Ok(())
}
