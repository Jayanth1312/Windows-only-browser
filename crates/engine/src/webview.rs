use std::path::Path;
use std::string::FromUtf16Error;
use std::sync::mpsc;
use thiserror::Error;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::*,
    DocumentTitleChangedEventHandler, NavigationCompletedEventHandler,
    NewWindowRequestedEventHandler, SourceChangedEventHandler,
};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{E_POINTER, HWND, RECT};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};

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
    NewTab,        // Ctrl+T
    CloseTab,      // Ctrl+W
    ToggleSidebar, // Ctrl+S
    Reload,        // Ctrl+R
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

impl WebViewEngine {
    pub fn new(user_data_folder: Option<&Path>) -> Result<Self, EngineError> {
        let (tx, rx) = mpsc::channel();

        let user_data_str: Option<HSTRING> = user_data_folder.map(|p| HSTRING::from(p.as_os_str()));
        let user_data_pcwstr = match &user_data_str {
            Some(hs) => PCWSTR::from_raw(hs.as_ptr()),
            None => PCWSTR::null(),
        };

        // Enable Windows 11 Fluent Overlay Scrollbars & smooth modern scrolling
        let options_raw = CoreWebView2EnvironmentOptions::default();
        unsafe {
            options_raw.set_scroll_bar_style(COREWEBVIEW2_SCROLLBAR_STYLE_FLUENT_OVERLAY);
            options_raw.set_additional_browser_arguments(
                "--enable-features=OverlayScrollbar,FluentOverlayScrollbar,FluentScrollbar".to_string(),
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
                        let _ = tx.send(Err(EngineError::Windows(windows::core::Error::from(E_POINTER))));
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
        on_nav_completed: Option<Box<dyn Fn(bool) + Send + 'static>>,
        on_title_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_url_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
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
                        let _ = tx.send(Err(EngineError::Windows(windows::core::Error::from(E_POINTER))));
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

        // Hook navigation completed if callback provided
        if let Some(cb) = on_nav_completed {
            let mut token = 0;
            let handler = NavigationCompletedEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut success = windows::core::BOOL(0);
                        if unsafe { args.IsSuccess(&mut success) }.is_ok() {
                            cb(success.as_bool());
                        }
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_NavigationCompleted(&handler, &mut token);
            }
        }

        // Hook document title changed event
        if let Some(cb) = on_title_changed {
            let mut token = 0;
            let webview_clone = webview.clone();
            let handler = DocumentTitleChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    let mut title_ptr = windows::core::PWSTR::null();
                    if unsafe { webview_clone.DocumentTitle(&mut title_ptr) }.is_ok() && !title_ptr.is_null() {
                        if let Ok(title) = unsafe { title_ptr.to_string() } {
                            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(title_ptr.as_ptr() as _)); }
                            cb(title);
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
        if let Some(cb) = on_url_changed {
            let mut token = 0;
            let webview_clone = webview.clone();
            let handler = SourceChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    let mut uri_ptr = windows::core::PWSTR::null();
                    if unsafe { webview_clone.Source(&mut uri_ptr) }.is_ok() && !uri_ptr.is_null() {
                        if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _)); }
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

        // Hook new window requested (target="_blank" -> open in our tabs!)
        if let Some(cb) = on_new_window {
            let mut token = 0;
            let handler = NewWindowRequestedEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut uri_ptr = windows::core::PWSTR::null();
                        if unsafe { args.Uri(&mut uri_ptr) }.is_ok() && !uri_ptr.is_null() {
                            if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                                unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _)); }
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
                            let ctrl = unsafe { (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 };
                            if ctrl {
                                match virtual_key {
                                    0x54 => { // 'T' -> New Tab / Command Bar
                                        let _ = unsafe { args.SetHandled(true) };
                                        cb(BrowserShortcut::NewTab);
                                    }
                                    0x57 => { // 'W' -> Close Tab
                                        let _ = unsafe { args.SetHandled(true) };
                                        cb(BrowserShortcut::CloseTab);
                                    }
                                    0x53 => { // 'S' -> Toggle Sidebar
                                        let _ = unsafe { args.SetHandled(true) };
                                        cb(BrowserShortcut::ToggleSidebar);
                                    }
                                    0x52 => { // 'R' -> Reload
                                        let _ = unsafe { args.SetHandled(true) };
                                        cb(BrowserShortcut::Reload);
                                    }
                                    _ => {}
                                }
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
        })
    }
}

impl WebViewController {
    pub fn set_bounds(&self, bounds: RECT) -> Result<(), EngineError> {
        unsafe {
            self.controller.SetBounds(bounds)?;
            self.controller.NotifyParentWindowPositionChanged()?;
        }
        Ok(())
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), EngineError> {
        unsafe {
            self.controller.SetIsVisible(visible)?;
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
}

impl Drop for WebViewController {
    fn drop(&mut self) {
        unsafe {
            let _ = self.controller.Close();
        }
    }
}
