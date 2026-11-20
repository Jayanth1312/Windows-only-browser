use std::path::Path;
use std::string::FromUtf16Error;
use std::sync::mpsc;
use thiserror::Error;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler,
    FaviconChangedEventHandler, GetFaviconCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    ProfileAddBrowserExtensionCompletedHandler, SourceChangedEventHandler,
    WebMessageReceivedEventHandler,
};
use windows::core::{HSTRING, Interface, PCWSTR};
use windows::Win32::Foundation::{E_POINTER, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::System::Com::{CoInitializeEx, IStream, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Windows COM/System error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("WebView2 COM error: {0}")]
    WebView2(#[from] webview2_com::Error),
    #[error("UTF-16 conversion error: {0}")]
    Utf16(#[from] FromUtf16Error),
    #[error("Channel error: {0}")]
    Channel(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserShortcut {
    NewTab,
    CloseTab,
    ToggleSidebar,
    Reload,
    HardReload,
    NavBack,
    NavForward,
    CopyUrl,
    FocusUrlBar,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    PinTab,
    NextTab,
    PrevTab,
    SelectTab(usize),
    ReopenClosedTab,
}

pub fn init_com() -> Result<(), EngineError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    Ok(())
}

pub struct WebViewEngine {
    environment: ICoreWebView2Environment,
}

pub struct WebViewController {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    last_bounds: std::cell::Cell<(i32, i32, i32, i32)>,
}

const MODERN_SCROLLBAR_CSS_SCRIPT: &str = r#"
(() => {
    const css = `
        ::-webkit-scrollbar {
            width: 8px !important;
            height: 8px !important;
        }
        ::-webkit-scrollbar-track {
            background: transparent !important;
        }
        ::-webkit-scrollbar-thumb {
            background: rgba(140, 140, 150, 0.35) !important;
            border-radius: 9999px !important;
            border: 2px solid transparent !important;
            background-clip: padding-box !important;
        }
        ::-webkit-scrollbar-thumb:hover {
            background: rgba(180, 180, 190, 0.65) !important;
            border-radius: 9999px !important;
            border: 2px solid transparent !important;
            background-clip: padding-box !important;
        }
        ::-webkit-scrollbar-corner {
            background: transparent !important;
        }
        ::-webkit-scrollbar-button {
            display: none !important;
            width: 0 !important;
            height: 0 !important;
        }
    `;
    function inject() {
        if (document.getElementById('__arc_modern_scrollbar__')) return;
        const style = document.createElement('style');
        style.id = '__arc_modern_scrollbar__';
        style.textContent = css;
        if (document.head) {
            document.head.appendChild(style);
        } else if (document.documentElement) {
            document.documentElement.appendChild(style);
        }
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', inject);
    } else {
        inject();
    }
})();
"#;

const COSMETIC_ADBLOCK_CSS_SCRIPT: &str = r#"
(() => {
    // Exclude YouTube so its internal player DOM & anti-adblock integrity are not disrupted
    if (location.hostname.endsWith('youtube.com')) return;
    const css = `
        .adsbygoogle, [id^="google_ads_"], [id^="div-gpt-ad"], iframe[id^="google_ads_"],
        .ad-container, .advertisement, .ad-banner, .ad-wrapper, .ad-slot,
        .taboola-placeholder, #outbrain_widget_0, .OUTBRAIN,
        [data-ad-unit], [data-ad-client], [data-ad-slot],
        div[class*="SponsoredMessage"], div[class*="ad-placeholder"] {
            display: none !important;
            visibility: hidden !important;
            height: 0 !important;
            width: 0 !important;
            opacity: 0 !important;
            pointer-events: none !important;
        }
    `;
    function inject() {
        if (document.getElementById('__arc_adblock_cosmetic__')) return;
        const style = document.createElement('style');
        style.id = '__arc_adblock_cosmetic__';
        style.textContent = css;
        if (document.head) {
            document.head.appendChild(style);
        } else if (document.documentElement) {
            document.documentElement.appendChild(style);
        }
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', inject);
    } else {
        inject();
    }
})();
"#;

const YT_HIDE_INTERRUPTIONS_SCRIPT: &str = r#"
(() => {
    if (!location.hostname.endsWith('youtube.com')) return;
    let timer = null;
    function sweep() {
        timer = null;
        const pc = document.querySelector('ytd-popup-container');
        if (!pc || !pc.textContent || !pc.textContent.includes('Experiencing interruptions')) return;
        for (const el of pc.children) {
            if (el.textContent && el.textContent.includes('Experiencing interruptions')) {
                el.style.setProperty('display', 'none', 'important');
            }
        }
    }
    function start() {
        new MutationObserver(() => {
            if (!timer) timer = setTimeout(sweep, 400);
        }).observe(document.documentElement, { childList: true, subtree: true });
    }
    if (document.documentElement) start();
    else document.addEventListener('DOMContentLoaded', start);
})();
"#;

fn read_istream(stream: &IStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut bytes_read = 0u32;
        let hr = unsafe {
            stream.Read(
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
                Some(&mut bytes_read),
            )
        };
        if hr.is_err() || bytes_read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..bytes_read as usize]);
    }
    out
}

fn fetch_favicon(
    webview15: &ICoreWebView2_15,
    cb: std::sync::Arc<Box<dyn Fn(Vec<u8>) + Send + 'static>>,
) {
    let handler = GetFaviconCompletedHandler::create(Box::new(move |error_code, stream| {
        if error_code.is_ok() {
            if let Some(stream) = stream {
                let bytes = read_istream(&stream);
                if !bytes.is_empty() {
                    cb(bytes);
                }
            }
        }
        Ok(())
    }));
    unsafe {
        let _ = webview15.GetFavicon(COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG, &handler);
    }
}

impl WebViewEngine {
    pub fn new(user_data_folder: Option<&Path>) -> Result<Self, EngineError> {
        let (tx, rx) = mpsc::channel();

        let user_data_str: Option<HSTRING> = user_data_folder.map(|p| HSTRING::from(p.as_os_str()));
        let user_data_pcwstr = match &user_data_str {
            Some(hs) => PCWSTR::from_raw(hs.as_ptr()),
            None => PCWSTR::null(),
        };

        // Enable Windows 11 Fluent Overlay Scrollbars & smooth modern scrolling with performance flags & extension support
        let options_raw = CoreWebView2EnvironmentOptions::default();
        unsafe {
            options_raw.set_are_browser_extensions_enabled(true);
            options_raw.set_scroll_bar_style(COREWEBVIEW2_SCROLLBAR_STYLE_FLUENT_OVERLAY);
            options_raw.set_additional_browser_arguments(
                "--enable-features=OverlayScrollbar,FluentOverlayScrollbar,FluentScrollbar --disable-features=CalculateNativeWinOcclusion".to_string(),
            );
        }
        let options: ICoreWebView2EnvironmentOptions = options_raw.into();

        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    user_data_pcwstr,
                    &options,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error_code, env| {
                if let Err(e) = error_code {
                    let _ = tx.send(Err(EngineError::Windows(e)));
                    return Ok(());
                }
                match env {
                    Some(e) => {
                        let _ = tx.send(Ok(e));
                    }
                    None => {
                        let _ = tx.send(Err(EngineError::Windows(windows::core::Error::from(
                            E_POINTER,
                        ))));
                    }
                }
                Ok(())
            }),
        )?;

        let environment = rx
            .recv()
            .map_err(|e| EngineError::Channel(e.to_string()))??;

        Ok(Self { environment })
    }

    pub fn create_controller(
        &self,
        parent: HWND,
        bounds: RECT,
        initial_url: &str,
        on_nav_starting: Option<Box<dyn Fn() + Send + 'static>>,
        on_nav_completed: Option<Box<dyn Fn(bool) + Send + 'static>>,
        on_title_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_url_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_favicon_changed: Option<Box<dyn Fn(Vec<u8>) + Send + 'static>>,
        on_new_window: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_shortcut: Option<Box<dyn Fn(BrowserShortcut) + Send + 'static>>,
    ) -> Result<WebViewController, EngineError> {
        let (tx, rx) = mpsc::channel();
        let env_clone = self.environment.clone();

        CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                env_clone
                    .CreateCoreWebView2Controller(parent, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error_code, ctrl| {
                if let Err(e) = error_code {
                    let _ = tx.send(Err(EngineError::Windows(e)));
                    return Ok(());
                }
                match ctrl {
                    Some(c) => {
                        let _ = tx.send(Ok(c));
                    }
                    None => {
                        let _ = tx.send(Err(EngineError::Windows(windows::core::Error::from(
                            E_POINTER,
                        ))));
                    }
                }
                Ok(())
            }),
        )?;

        let controller = rx
            .recv()
            .map_err(|e| EngineError::Channel(e.to_string()))??;

        let webview = unsafe { controller.CoreWebView2()? };

        // Set bounds and make visible
        unsafe {
            controller.SetBounds(bounds)?;
            controller.SetIsVisible(true)?;
        }

        // Configure basic settings
        if let Ok(settings) = unsafe { webview.Settings() } {
            unsafe {
                let _ = settings.SetIsStatusBarEnabled(false);
                let _ = settings.SetAreDevToolsEnabled(true);
                let _ = settings.SetAreDefaultContextMenusEnabled(true);
            }
        }

        // Inject modern minimal overlay scrollbar script into all documents
        {
            let script_hstring = HSTRING::from(MODERN_SCROLLBAR_CSS_SCRIPT);
            let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                |_res, _id| Ok(()),
            ));
            unsafe {
                let _ = webview.AddScriptToExecuteOnDocumentCreated(
                    PCWSTR::from_raw(script_hstring.as_ptr()),
                    &handler,
                );
            }
        }

        // Inject instant cosmetic ad-blocking CSS script into all documents
        {
            let script_hstring = HSTRING::from(COSMETIC_ADBLOCK_CSS_SCRIPT);
            let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                |_res, _id| Ok(()),
            ));
            unsafe {
                let _ = webview.AddScriptToExecuteOnDocumentCreated(
                    PCWSTR::from_raw(script_hstring.as_ptr()),
                    &handler,
                );
            }
        }

        // Hide YouTube's "Experiencing interruptions?" popup
        {
            let script_hstring = HSTRING::from(YT_HIDE_INTERRUPTIONS_SCRIPT);
            let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                |_res, _id| Ok(()),
            ));
            unsafe {
                let _ = webview.AddScriptToExecuteOnDocumentCreated(
                    PCWSTR::from_raw(script_hstring.as_ptr()),
                    &handler,
                );
            }
        }

        // Inject SPA and dynamic title observer script (e.g. YouTube video changes)
        {
            const PAGE_META_OBSERVER_SCRIPT: &str = r#"
            (() => {
                let curTitle = '';
                let curUrl = '';

                function reportMeta() {
                    try {
                        const t = document.title || '';
                        const u = location.href || '';
                        if (t !== curTitle || u !== curUrl) {
                            curTitle = t;
                            curUrl = u;
                            if (window.chrome && window.chrome.webview) {
                                window.chrome.webview.postMessage("meta:" + t + "|||" + u);
                            }
                        }
                    } catch(e) {}
                }

                function hookTitle() {
                    const titleEl = document.querySelector('title');
                    if (titleEl) {
                        const obs = new MutationObserver(() => reportMeta());
                        obs.observe(titleEl, { childList: true, characterData: true, subtree: true });
                    }
                    reportMeta();
                }

                if (document.readyState === 'loading') {
                    document.addEventListener('DOMContentLoaded', hookTitle);
                } else {
                    hookTitle();
                }

                window.addEventListener('popstate', () => {
                    setTimeout(reportMeta, 50);
                    setTimeout(reportMeta, 300);
                });

                window.addEventListener('yt-navigate-finish', () => {
                    setTimeout(reportMeta, 50);
                    setTimeout(reportMeta, 300);
                });

                document.addEventListener('yt-page-data-updated', () => {
                    setTimeout(reportMeta, 50);
                    setTimeout(reportMeta, 300);
                });

                try {
                    const origPush = history.pushState;
                    if (origPush) {
                        history.pushState = function(...args) {
                            const res = origPush.apply(this, args);
                            setTimeout(reportMeta, 50);
                            setTimeout(reportMeta, 300);
                            return res;
                        };
                    }
                } catch(e) {}

                try {
                    const origReplace = history.replaceState;
                    if (origReplace) {
                        history.replaceState = function(...args) {
                            const res = origReplace.apply(this, args);
                            setTimeout(reportMeta, 50);
                            setTimeout(reportMeta, 300);
                            return res;
                        };
                    }
                } catch(e) {}
            })();
            "#;
            let script_hstring = HSTRING::from(PAGE_META_OBSERVER_SCRIPT);
            let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                |_res, _id| Ok(()),
            ));
            unsafe {
                let _ = webview.AddScriptToExecuteOnDocumentCreated(
                    PCWSTR::from_raw(script_hstring.as_ptr()),
                    &handler,
                );
            }
        }

        let title_cb_shared = on_title_changed.map(|cb| std::sync::Arc::new(cb));
        let url_cb_shared = on_url_changed.map(|cb| std::sync::Arc::new(cb));

        // Hook WebMessageReceived for dynamic SPA title updates (e.g. YouTube video clicks)
        {
            let meta_title_cb = title_cb_shared.clone();
            let meta_url_cb = url_cb_shared.clone();
            let mut msg_token = 0;
            let msg_handler = WebMessageReceivedEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut str_ptr = windows::core::PWSTR::null();
                        if unsafe { args.TryGetWebMessageAsString(&mut str_ptr) }.is_ok()
                            && !str_ptr.is_null()
                        {
                            if let Ok(msg) = unsafe { str_ptr.to_string() } {
                                unsafe {
                                    windows::Win32::System::Com::CoTaskMemFree(Some(
                                        str_ptr.as_ptr() as _,
                                    ));
                                }
                                if let Some(payload) = msg.strip_prefix("meta:") {
                                    if let Some((t, u)) = payload.split_once("|||") {
                                        if !t.is_empty() {
                                            if let Some(ref cb) = meta_title_cb {
                                                cb(t.to_string());
                                            }
                                        }
                                        if !u.is_empty() {
                                            if let Some(ref cb) = meta_url_cb {
                                                cb(u.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_WebMessageReceived(&msg_handler, &mut msg_token);
            }
        }

        if let Some(start_cb) = on_nav_starting {
            let mut start_token = 0;
            let start_handler = NavigationStartingEventHandler::create(Box::new(
                move |_sender, _args| {
                    start_cb();
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_NavigationStarting(&start_handler, &mut start_token);
            }
        }

        // Hook navigation completed and favicon
        let wv15_opt = webview.cast::<ICoreWebView2_15>().ok();
        let fav_cb_shared = on_favicon_changed.map(|cb| std::sync::Arc::new(cb));

        let nav_title_cb = title_cb_shared.clone();
        let nav_webview = webview.clone();
        let nav_wv15 = wv15_opt.clone();
        let nav_fav_cb = fav_cb_shared.clone();
        let mut nav_token = 0;
        let nav_handler = NavigationCompletedEventHandler::create(Box::new(
            move |_sender, args| {
                if let Some(args) = args {
                    let mut success = windows::core::BOOL(0);
                    let ok = unsafe { args.IsSuccess(&mut success) }.is_ok() && success.as_bool();
                    if let Some(ref cb) = on_nav_completed {
                        cb(ok);
                    }
                    if ok {
                        if let Some(ref t_cb) = nav_title_cb {
                            let mut title_ptr = windows::core::PWSTR::null();
                            if unsafe { nav_webview.DocumentTitle(&mut title_ptr) }.is_ok()
                                && !title_ptr.is_null()
                            {
                                if let Ok(title) = unsafe { title_ptr.to_string() } {
                                    unsafe {
                                        windows::Win32::System::Com::CoTaskMemFree(Some(
                                            title_ptr.as_ptr() as _,
                                        ));
                                    }
                                    if !title.is_empty() {
                                        t_cb(title);
                                    }
                                }
                            }
                        }
                        if let (Some(ref wv15), Some(ref fav_cb)) = (&nav_wv15, &nav_fav_cb) {
                            fetch_favicon(wv15, fav_cb.clone());
                        }
                    }
                }
                Ok(())
            },
        ));
        unsafe {
            let _ = webview.add_NavigationCompleted(&nav_handler, &mut nav_token);
        }

        // Hook document title changed event
        if let Some(t_cb) = title_cb_shared {
            let mut token = 0;
            let webview_clone = webview.clone();
            let handler = DocumentTitleChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    let mut title_ptr = windows::core::PWSTR::null();
                    if unsafe { webview_clone.DocumentTitle(&mut title_ptr) }.is_ok()
                        && !title_ptr.is_null()
                    {
                        if let Ok(title) = unsafe { title_ptr.to_string() } {
                            unsafe {
                                windows::Win32::System::Com::CoTaskMemFree(Some(
                                    title_ptr.as_ptr() as _,
                                ));
                            }
                            if !title.is_empty() {
                                t_cb(title);
                            }
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_DocumentTitleChanged(&handler, &mut token);
            }
        }

        // Hook source changed event
        if let Some(cb) = url_cb_shared {
            let mut token = 0;
            let webview_clone = webview.clone();
            let handler = SourceChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    let mut uri_ptr = windows::core::PWSTR::null();
                    if unsafe { webview_clone.Source(&mut uri_ptr) }.is_ok()
                        && !uri_ptr.is_null()
                    {
                        if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                            unsafe {
                                windows::Win32::System::Com::CoTaskMemFree(Some(
                                    uri_ptr.as_ptr() as _,
                                ));
                            }
                            cb(uri);
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_SourceChanged(&handler, &mut token);
            }
        }

        // Hook favicon changed event (ICoreWebView2_15)
        if let (Some(ref webview15), Some(ref fav_cb)) = (&wv15_opt, &fav_cb_shared) {
            let wv15_clone = webview15.clone();
            let fav_cb_clone = fav_cb.clone();
            let mut token = 0;
            let handler = FaviconChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    fetch_favicon(&wv15_clone, fav_cb_clone.clone());
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview15.add_FaviconChanged(&handler, &mut token);
            }
        }

        // Hook new window requested (target="_blank" -> open in our tabs!)
        if let Some(cb) = on_new_window {
            let mut token = 0;
            let handler = NewWindowRequestedEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut uri_ptr = windows::core::PWSTR::null();
                        if unsafe { args.Uri(&mut uri_ptr) }.is_ok() && !uri_ptr.is_null() {
                            if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                                unsafe {
                                    windows::Win32::System::Com::CoTaskMemFree(Some(
                                        uri_ptr.as_ptr() as _,
                                    ));
                                }
                                let _ = unsafe { args.SetHandled(true) };
                                cb(uri);
                            }
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_NewWindowRequested(&handler, &mut token);
            }
        }

        // Hook accelerator keys for browser shortcuts
        if let Some(cb) = on_shortcut {
            let mut token = 0;
            let handler = AcceleratorKeyPressedEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut key_event_kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                        let mut virtual_key = 0u32;
                        let _ = unsafe { args.KeyEventKind(&mut key_event_kind) };
                        let _ = unsafe { args.VirtualKey(&mut virtual_key) };

                        if key_event_kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
                            let ctrl = unsafe {
                                (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0
                                    || (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0
                            };
                            let shift = unsafe {
                                (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0
                                    || (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0
                            };
                            let alt = unsafe {
                                (GetKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0
                                    || (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0
                            };

                            let mut shortcut = None;

                            if alt {
                                if virtual_key == 0x25 { // Left arrow -> Back
                                    shortcut = Some(BrowserShortcut::NavBack);
                                } else if virtual_key == 0x27 { // Right arrow -> Forward
                                    shortcut = Some(BrowserShortcut::NavForward);
                                }
                            }

                            if ctrl {
                                if shift {
                                    match virtual_key {
                                        0x43 => shortcut = Some(BrowserShortcut::CopyUrl), // 'C'
                                        0x58 => shortcut = Some(BrowserShortcut::PinTab),  // 'X'
                                        0x52 => shortcut = Some(BrowserShortcut::HardReload), // 'R'
                                        0x09 => shortcut = Some(BrowserShortcut::PrevTab), // Tab
                                        0x54 => shortcut = Some(BrowserShortcut::ReopenClosedTab), // 'T' -> Reopen Closed Tab!
                                        _ => {}
                                    }
                                } else {
                                    match virtual_key {
                                        0x54 => shortcut = Some(BrowserShortcut::NewTab), // 'T'
                                        0x57 => shortcut = Some(BrowserShortcut::CloseTab), // 'W'
                                        0x53 | 0x42 => shortcut = Some(BrowserShortcut::ToggleSidebar), // 'S' / 'B'
                                        0x52 => shortcut = Some(BrowserShortcut::Reload), // 'R'
                                        0x4C => shortcut = Some(BrowserShortcut::FocusUrlBar), // 'L'
                                        0xBC => shortcut = Some(BrowserShortcut::OpenSettings), // ','
                                        0xBB | 0x6B => shortcut = Some(BrowserShortcut::ZoomIn), // '+'
                                        0xBD | 0x6D => shortcut = Some(BrowserShortcut::ZoomOut), // '-'
                                        0x30 | 0x60 => shortcut = Some(BrowserShortcut::ZoomReset), // '0'
                                        0x44 => shortcut = Some(BrowserShortcut::PinTab), // 'D'
                                        0x09 => shortcut = Some(BrowserShortcut::NextTab), // Tab
                                        0x31..=0x39 => shortcut = Some(BrowserShortcut::SelectTab((virtual_key - 0x31) as usize)),
                                        _ => {}
                                    }
                                }
                            } else if virtual_key == 0x74 { // F5
                                shortcut = Some(BrowserShortcut::Reload);
                            }

                            if let Some(sc) = shortcut {
                                let _ = unsafe { args.SetHandled(true) };
                                cb(sc);
                            }
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = controller.add_AcceleratorKeyPressed(&handler, &mut token);
            }
        }

        // Initial navigation
        let url_hstring = HSTRING::from(initial_url);
        unsafe {
            webview.Navigate(PCWSTR::from_raw(url_hstring.as_ptr()))?;
        }

        Ok(WebViewController {
            controller,
            webview,
            last_bounds: std::cell::Cell::new((0, 0, 0, 0)),
        })
    }
}

impl WebViewController {
    pub fn set_bounds(&self, bounds: RECT) -> Result<(), EngineError> {
        let new_bounds = (bounds.left, bounds.top, bounds.right, bounds.bottom);
        let old_bounds = self.last_bounds.get();

        unsafe {
            self.controller.SetBounds(bounds)?;
            self.controller.NotifyParentWindowPositionChanged()?;
        }

        // Only redo the expensive child-window region clipping when size actually changes
        let w = bounds.right - bounds.left;
        let h = bounds.bottom - bounds.top;
        let old_w = old_bounds.2 - old_bounds.0;
        let old_h = old_bounds.3 - old_bounds.1;

        if w > 0 && h > 0 && (w != old_w || h != old_h) {
            self.last_bounds.set(new_bounds);
            unsafe {
                let mut parent = HWND::default();
                if self.controller.ParentWindow(&mut parent).is_ok() {
                    let mut children: Vec<HWND> = Vec::new();
                    unsafe extern "system" fn enum_child(child: HWND, lparam: LPARAM) -> windows::core::BOOL {
                        let list = &mut *(lparam.0 as *mut Vec<HWND>);
                        list.push(child);
                        windows::core::BOOL(1)
                    }
                    let _ = EnumChildWindows(
                        Some(parent),
                        Some(enum_child),
                        LPARAM(&mut children as *mut _ as isize),
                    );
                    let radius = 16;
                    for child in children {
                        let mut class_buf = [0u16; 64];
                        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(child, &mut class_buf);
                        if len > 0 {
                            let cname = String::from_utf16_lossy(&class_buf[..len as usize]);
                            if cname.contains("Intermediate") {
                                continue;
                            }
                        }
                        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
                        let _ = SetWindowRgn(child, Some(rgn), false);
                    }
                }
            }
        } else if w > 0 && h > 0 {
            self.last_bounds.set(new_bounds);
        }
        Ok(())
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), EngineError> {
        unsafe {
            self.controller.SetIsVisible(visible)?;
        }
        Ok(())
    }

    pub fn focus(&self) -> Result<(), EngineError> {
        unsafe {
            let _ = self.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
        Ok(())
    }

    pub fn load_extension(&self, extension_path: &Path) -> Result<(), EngineError> {
        if let Ok(wv13) = self.webview.cast::<ICoreWebView2_13>() {
            if let Ok(profile) = unsafe { wv13.Profile() } {
                if let Ok(profile7) = profile.cast::<ICoreWebView2Profile7>() {
                    let path_str = extension_path.to_string_lossy().to_string();
                    let path_hstring = HSTRING::from(path_str);
                    let handler = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(
                        |res, _ext| {
                            if let Err(e) = res {
                                tracing::warn!("Failed to load browser extension: {:?}", e);
                            } else {
                                tracing::info!("Browser extension loaded successfully!");
                            }
                            Ok(())
                        },
                    ));
                    unsafe {
                        let _ = profile7.AddBrowserExtension(
                            PCWSTR::from_raw(path_hstring.as_ptr()),
                            &handler,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn navigate(&self, url: &str) -> Result<(), EngineError> {
        let url_hstring = HSTRING::from(url);
        unsafe {
            self.webview.Navigate(PCWSTR::from_raw(url_hstring.as_ptr()))?;
        }
        Ok(())
    }

    pub fn get_source(&self) -> Result<String, EngineError> {
        let mut uri_ptr = windows::core::PWSTR::null();
        unsafe {
            self.webview.Source(&mut uri_ptr)?;
            if !uri_ptr.is_null() {
                let s = uri_ptr.to_string().map_err(|e| EngineError::Channel(e.to_string()))?;
                windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _));
                Ok(s)
            } else {
                Ok(String::new())
            }
        }
    }

    pub fn reload(&self) -> Result<(), EngineError> {
        unsafe {
            self.webview.Reload()?;
        }
        Ok(())
    }

    pub fn go_back(&self) -> Result<(), EngineError> {
        unsafe {
            self.webview.GoBack()?;
        }
        Ok(())
    }

    pub fn go_forward(&self) -> Result<(), EngineError> {
        unsafe {
            self.webview.GoForward()?;
        }
        Ok(())
    }

    pub fn can_go_back(&self) -> bool {
        let mut can = windows::core::BOOL(0);
        unsafe {
            let _ = self.webview.CanGoBack(&mut can);
        }
        can.as_bool()
    }

    pub fn can_go_forward(&self) -> bool {
        let mut can = windows::core::BOOL(0);
        unsafe {
            let _ = self.webview.CanGoForward(&mut can);
        }
        can.as_bool()
    }

    pub fn get_zoom_factor(&self) -> f64 {
        let mut zoom = 1.0f64;
        unsafe {
            let _ = self.controller.ZoomFactor(&mut zoom);
        }
        zoom
    }

    pub fn set_zoom_factor(&self, zoom: f64) -> Result<(), EngineError> {
        unsafe {
            self.controller.SetZoomFactor(zoom)?;
        }
        Ok(())
    }

    pub fn get_url(&self) -> Result<String, EngineError> {
        let mut uri_ptr = windows::core::PWSTR::null();
        unsafe {
            self.webview.Source(&mut uri_ptr)?;
            if uri_ptr.is_null() {
                Ok(String::new())
            } else {
                let s = uri_ptr.to_string()?;
                windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _));
                Ok(s)
            }
        }
    }

    pub fn get_title(&self) -> Result<String, EngineError> {
        let mut title_ptr = windows::core::PWSTR::null();
        unsafe {
            self.webview.DocumentTitle(&mut title_ptr)?;
            if title_ptr.is_null() {
                Ok(String::new())
            } else {
                let s = title_ptr.to_string()?;
                windows::Win32::System::Com::CoTaskMemFree(Some(title_ptr.as_ptr() as _));
                Ok(s)
            }
        }
    }

    pub fn is_muted(&self) -> bool {
        if let Ok(wv8) = self.webview.cast::<ICoreWebView2_8>() {
            let mut muted = windows::core::BOOL(0);
            if unsafe { wv8.IsMuted(&mut muted) }.is_ok() {
                return muted.as_bool();
            }
        }
        false
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), EngineError> {
        if let Ok(wv8) = self.webview.cast::<ICoreWebView2_8>() {
            unsafe {
                wv8.SetIsMuted(muted)?;
            }
        }
        Ok(())
    }
}

impl Drop for WebViewController {
    fn drop(&mut self) {
        unsafe {
            let _ = self.controller.Close();
        }
    }
}