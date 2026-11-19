use std::path::Path;
use std::string::FromUtf16Error;
use std::sync::mpsc;
use thiserror::Error;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler,
};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{E_POINTER, HWND, RECT};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

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

impl WebViewEngine {
    pub fn new(user_data_folder: Option<&Path>) -> Result<Self, EngineError> {
        let (tx, rx) = mpsc::channel();

        let user_data_str: Option<HSTRING> = user_data_folder.map(|p| HSTRING::from(p.as_os_str()));
        let user_data_pcwstr = match &user_data_str {
            Some(hs) => PCWSTR::from_raw(hs.as_ptr()),
            None => PCWSTR::null(),
        };

        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    user_data_pcwstr,
                    None,
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

        // Navigate
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
