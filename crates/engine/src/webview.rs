use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::string::FromUtf16Error;
use std::sync::mpsc;
use thiserror::Error;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    ContainsFullScreenElementChangedEventHandler,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler,
    ExecuteScriptCompletedHandler,
    FaviconChangedEventHandler, GetFaviconCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    ProfileAddBrowserExtensionCompletedHandler, SourceChangedEventHandler,
    TrySuspendCompletedHandler, WebMessageReceivedEventHandler,
    WebResourceRequestedEventHandler,
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
    CopyCleanLink,
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
    ToggleZapper,
}

#[derive(Debug, Clone)]
pub enum ZapperMessage {
    Zap {
        selector: String,
        label: String,
        note: String,
    },
    Undo,
    Status {
        active: bool,
    },
    RequestCss,
}

pub fn init_com() -> Result<(), EngineError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    Ok(())
}

pub struct WebViewEngine {
    environment: ICoreWebView2Environment,
    spare_controller: Rc<RefCell<Option<(ICoreWebView2Controller, ICoreWebView2)>>>,
    is_prewarming: Rc<Cell<bool>>,
}

pub struct WebViewController {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    last_bounds: std::cell::Cell<(i32, i32, i32, i32)>,
    shield_state: std::sync::Arc<crate::shield::ShieldState>,
    current_host: std::sync::Arc<std::sync::RwLock<String>>,
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
        if (document.getElementById('__browser_modern_scrollbar__')) return;
        const style = document.createElement('style');
        style.id = '__browser_modern_scrollbar__';
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

    // Fallback periodic sync without monkey-patching native history APIs
    setInterval(reportMeta, 500);
})();
"#;

const SELECTION_SEARCH_POPUP_SCRIPT: &str = r#"
(function() {
    if (window.__zenSearchPopupInjected) return;
    window.__zenSearchPopupInjected = true;

    let host = null;
    let pill = null;
    let currentSelectedText = '';
    let isVisible = false;

    function initPopup() {
        if (host) return;
        host = document.createElement('div');
        host.id = 'zen-search-popup-host';
        host.style.cssText = 'all: initial !important; position: absolute !important; top: 0 !important; left: 0 !important; z-index: 2147483647 !important; pointer-events: none !important;';

        const shadow = host.attachShadow({ mode: 'closed' });

        const style = document.createElement('style');
        style.textContent = `
            * {
                box-sizing: border-box;
                margin: 0;
                padding: 0;
            }
            .zen-search-pill {
                position: fixed;
                display: inline-flex;
                align-items: center;
                justify-content: center;
                background: #1c1c1c;
                color: #ffffff;
                font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
                font-size: 13px;
                font-weight: 400;
                line-height: 1;
                padding: 7px 14px 7px 11px;
                border-radius: 7px;
                border: 1px solid rgba(255, 255, 255, 0.12);
                box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4), 0 1px 3px rgba(0, 0, 0, 0.2);
                cursor: pointer;
                pointer-events: auto;
                user-select: none;
                -webkit-user-select: none;
                opacity: 0;
                visibility: hidden;
                transform: translate(-50%, -75%) scale(0.92);
                transition: opacity 0.14s cubic-bezier(0.16, 1, 0.3, 1), transform 0.14s cubic-bezier(0.16, 1, 0.3, 1), visibility 0.14s, background 0.1s ease;
                white-space: nowrap;
                z-index: 2147483647;
            }
            .zen-search-pill.visible {
                opacity: 1;
                visibility: visible;
                transform: translate(-50%, -100%) scale(1);
            }
            .zen-search-pill.bottom-placed.visible {
                opacity: 1;
                visibility: visible;
                transform: translate(-50%, 0%) scale(1);
            }
            .zen-search-pill:hover {
                background: #282828;
                border-color: rgba(255, 255, 255, 0.20);
                box-shadow: 0 6px 20px rgba(0, 0, 0, 0.55), 0 2px 5px rgba(0, 0, 0, 0.3);
            }
            .zen-search-pill:active {
                transform: translate(-50%, -100%) scale(0.97);
                background: #161616;
            }
            .zen-search-pill.bottom-placed:active {
                transform: translate(-50%, 0%) scale(0.97);
            }
            .zen-search-icon {
                width: 13px;
                height: 13px;
                margin-right: 7px;
                flex-shrink: 0;
                stroke: currentColor;
                stroke-width: 2.2;
                fill: none;
                stroke-linecap: round;
                stroke-linejoin: round;
                opacity: 0.9;
            }
            .zen-search-text {
                letter-spacing: 0.1px;
            }
        `;

        pill = document.createElement('div');
        pill.className = 'zen-search-pill';
        pill.innerHTML = `
            <svg class="zen-search-icon" viewBox="0 0 24 24">
                <circle cx="11" cy="11" r="7"></circle>
                <line x1="21" y1="21" x2="16.5" y2="16.5"></line>
            </svg>
            <span class="zen-search-text">Search</span>
        `;

        shadow.appendChild(style);
        shadow.appendChild(pill);

        pill.addEventListener('pointerdown', (e) => {
            e.preventDefault();
            e.stopPropagation();
        });

        pill.addEventListener('mousedown', (e) => {
            e.preventDefault();
            e.stopPropagation();
        });

        pill.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            if (currentSelectedText && window.chrome && window.chrome.webview) {
                window.chrome.webview.postMessage("search:" + encodeURIComponent(currentSelectedText));
            }
            hide();
        });
    }

    function ensureAttached() {
        initPopup();
        if (document.documentElement && !document.documentElement.contains(host)) {
            document.documentElement.appendChild(host);
        }
    }

    function hide() {
        if (!isVisible || !pill) return;
        isVisible = false;
        pill.classList.remove('visible', 'bottom-placed');
        currentSelectedText = '';
    }

    function showForRect(rect, text) {
        ensureAttached();
        currentSelectedText = text;

        let centerX = rect.left + rect.width / 2;
        centerX = Math.max(55, Math.min(window.innerWidth - 55, centerX));

        const spaceAbove = rect.top;
        let topY = 0;
        let isBottom = false;

        if (spaceAbove >= 44) {
            topY = rect.top - 8;
            isBottom = false;
        } else {
            topY = rect.bottom + 8;
            isBottom = true;
        }

        pill.style.left = Math.round(centerX) + 'px';
        pill.style.top = Math.round(topY) + 'px';

        if (isBottom) {
            pill.classList.add('bottom-placed');
        } else {
            pill.classList.remove('bottom-placed');
        }

        pill.classList.add('visible');
        isVisible = true;
    }

    function checkSelection() {
        const sel = window.getSelection();
        if (!sel || sel.isCollapsed || sel.rangeCount === 0) {
            hide();
            return;
        }

        const raw = sel.toString();
        const text = raw.replace(/\s+/g, ' ').trim();
        if (!text || text.length > 500) {
            hide();
            return;
        }

        try {
            const range = sel.getRangeAt(0);
            const rect = range.getBoundingClientRect();
            if (rect.width <= 0 && rect.height <= 0) {
                hide();
                return;
            }
            showForRect(rect, text);
        } catch(e) {
            hide();
        }
    }

    document.addEventListener('mouseup', (e) => {
        if (host && (e.target === host || host.contains(e.target))) return;
        setTimeout(checkSelection, 20);
    }, true);

    document.addEventListener('keyup', (e) => {
        if (e.key === 'Shift' || e.key.startsWith('Arrow') || (e.ctrlKey && e.key.toLowerCase() === 'a')) {
            setTimeout(checkSelection, 20);
        } else if (e.key === 'Escape') {
            hide();
        }
    }, true);

    document.addEventListener('mousedown', (e) => {
        if (host && (e.target === host || host.contains(e.target))) return;
        hide();
    }, true);

    document.addEventListener('contextmenu', hide, true);
    window.addEventListener('scroll', hide, { passive: true });
    window.addEventListener('resize', hide, { passive: true });
})();
"#;

const ELEMENT_ZAPPER_SCRIPT: &str = r#"
(() => {
    if (window.__officeVeil) return;

    var frame = null;
    var tag = null;
    var target = null;
    var live = false;

    function sheet(id) {
        var s = document.getElementById(id);
        if (!s) {
            s = document.createElement('style');
            s.id = id;
            (document.head || document.documentElement).appendChild(s);
        }
        return s;
    }

    function chrome() {
        if (frame) return frame;
        frame = document.createElement('div');
        frame.id = '__zapper_frame';
        frame.style.cssText = 'position:fixed;z-index:2147483646;pointer-events:none;' +
            'border:2px solid #ff4444;background:rgba(255,68,68,0.12);' +
            'border-radius:4px;transition:all .06s ease-out;display:none;' +
            'box-shadow:0 0 12px rgba(255,68,68,0.35);';

        tag = document.createElement('div');
        tag.style.cssText = 'position:absolute;font:600 11px -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;' +
            'color:#ffffff;background:#181818;border:1px solid rgba(255,255,255,0.18);padding:3px 8px;' +
            'border-radius:5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;' +
            'box-shadow:0 2px 8px rgba(0,0,0,0.6);letter-spacing:0.2px;';

        frame.appendChild(tag);
        (document.body || document.documentElement).appendChild(frame);
        return frame;
    }

    function place(el) {
        var box = chrome();
        var r = el.getBoundingClientRect();
        box.style.display = 'block';
        box.style.left = r.left + 'px';
        box.style.top = r.top + 'px';
        box.style.width = r.width + 'px';
        box.style.height = r.height + 'px';

        var n = name(el);
        var s = shape(el);
        tag.textContent = n + ' · ' + s;

        // Above if room, inside top edge if flush with top
        tag.style.top = r.top >= 28 ? '-24px' : '4px';
        tag.style.left = Math.max(2, -r.left + 4) + 'px';
        tag.style.maxWidth = Math.max(100, window.innerWidth - Math.max(0, r.left) - 20) + 'px';
    }

    var known = {
        nav: 'Navigation', header: 'Header', footer: 'Footer', aside: 'Sidebar',
        form: 'Form', dialog: 'Dialog', video: 'Video', img: 'Image',
        button: 'Button', iframe: 'Embed', figure: 'Figure', table: 'Table',
        section: 'Section', article: 'Article', main: 'Main'
    };

    function name(el) {
        var said = el.getAttribute && (el.getAttribute('aria-label') || el.getAttribute('title'));
        if (said && said.trim()) return clip(said.trim(), 35);
        var tagName = el.tagName.toLowerCase();
        if (known[tagName]) return known[tagName];
        var role = el.getAttribute && el.getAttribute('role');
        if (role) return role.charAt(0).toUpperCase() + role.slice(1);
        var text = (el.innerText || '').trim().replace(/\s+/g, ' ');
        return text ? clip(text, 35) : tagName;
    }

    function shape(el) {
        var r = el.getBoundingClientRect();
        var cx = r.left + r.width / 2, cy = r.top + r.height / 2;
        var side = cx < window.innerWidth / 3 ? 'left'
                 : (cx > window.innerWidth * 2 / 3 ? 'right' : 'center');
        var band = cy < window.innerHeight / 3 ? 'top'
                 : (cy > window.innerHeight * 2 / 3 ? 'bottom' : 'middle');
        return Math.round(r.width) + '×' + Math.round(r.height) + ' · ' + band + ' ' + side;
    }

    function clip(text, n) {
        return text.length > n ? text.slice(0, n) + '…' : text;
    }

    function steady(c) {
        return /^[a-zA-Z][\w-]{2,29}$/.test(c) && !/\d{3,}/.test(c) &&
               !/^(css|sc|jsx|emotion|svelte|styles?)-/.test(c);
    }

    function unique(sel) {
        try {
            return document.querySelectorAll(sel).length === 1;
        } catch (e) {
            return false;
        }
    }

    function selectorFor(el) {
        if (el.id && unique('#' + CSS.escape(el.id))) {
            return '#' + CSS.escape(el.id);
        }

        var hooks = ['data-testid', 'data-test', 'data-qa', 'data-cy', 'aria-label', 'name', 'role'];
        for (var i = 0; i < hooks.length; i++) {
            var v = el.getAttribute && el.getAttribute(hooks[i]);
            if (v) {
                var s = el.tagName.toLowerCase() + '[' + hooks[i] + '="' + CSS.escape(v) + '"]';
                if (unique(s)) return s;
            }
        }

        var classes = (el.className && typeof el.className === 'string')
            ? el.className.trim().split(/\s+/).filter(steady) : [];
        if (classes.length) {
            var byClass = el.tagName.toLowerCase() + '.' + classes.map(CSS.escape).join('.');
            if (unique(byClass)) return byClass;
        }

        var parts = [], node = el;
        while (node && node.nodeType === 1 && node !== document.documentElement) {
            if (node.id && unique('#' + CSS.escape(node.id))) {
                parts.unshift('#' + CSS.escape(node.id));
                break;
            }
            var tagName = node.tagName.toLowerCase();
            var parent = node.parentElement;
            if (!parent) {
                parts.unshift(tagName);
                break;
            }
            var kin = Array.prototype.filter.call(parent.children, function (c) {
                return c.tagName === node.tagName;
            });
            parts.unshift(kin.length > 1
                ? tagName + ':nth-of-type(' + (kin.indexOf(node) + 1) + ')'
                : tagName);
            node = parent;
        }
        return parts.join(' > ');
    }

    function postMsg(action, selector, label, note) {
        if (window.chrome && window.chrome.webview) {
            window.chrome.webview.postMessage(
                "zapper:" + action + "|||" + (selector || "") + "|||" + (label || "") + "|||" + (note || "")
            );
        }
    }

    function onMove(e) {
        if (!live) return;
        var el = document.elementFromPoint(e.clientX, e.clientY);
        if (!el || el === frame || el === document.documentElement || el === document.body) return;
        target = el;
        place(el);
    }

    function swallow(e) {
        if (!live) return;
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
    }

    function onPress(e) {
        if (!live) return;
        swallow(e);
        var el = target || document.elementFromPoint(e.clientX, e.clientY);
        if (!el || el === frame || el === document.documentElement || el === document.body) return;
        var sel = selectorFor(el);
        var lbl = name(el);
        var shp = shape(el);

        // Immediate visual hiding in page
        try {
            var curStyle = sheet('browser-zapper');
            curStyle.textContent += '\n' + sel + ' { display: none !important; }';
        } catch(err) {}

        postMsg('zap', sel, lbl, shp);
        target = null;
        if (frame) frame.style.display = 'none';
    }

    function onKeyDown(e) {
        if (!live) return;
        if (e.key === 'Escape') {
            window.__officeVeil.off();
        } else if (e.key === 'z' && (e.ctrlKey || e.metaKey)) {
            postMsg('undo');
        }
    }

    var presses = ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click',
                   'dblclick', 'contextmenu', 'touchstart'];

    window.__officeVeil = {
        on: function () {
            if (live) return;
            live = true;
            chrome();
            document.documentElement.style.cursor = 'crosshair';
            document.addEventListener('mousemove', onMove, true);
            document.addEventListener('pointermove', onMove, true);
            document.addEventListener('keydown', onKeyDown, true);
            presses.forEach(function (kind) {
                document.addEventListener(kind, kind === 'pointerdown' ? onPress : swallow, true);
            });
            postMsg('status', '', '', 'true');
        },
        off: function () {
            if (!live) return;
            live = false;
            target = null;
            if (frame) frame.style.display = 'none';
            document.documentElement.style.cursor = '';
            document.removeEventListener('mousemove', onMove, true);
            document.removeEventListener('pointermove', onMove, true);
            document.removeEventListener('keydown', onKeyDown, true);
            presses.forEach(function (kind) {
                document.removeEventListener(kind, kind === 'pointerdown' ? onPress : swallow, true);
            });
            postMsg('status', '', '', 'false');
        },
        isActive: function () {
            return live;
        },
        applyRules: function (css) {
            sheet('browser-zapper').textContent = css;
        },
        peek: function (css, sel) {
            sheet('browser-zapper').textContent = css;
            sheet('browser-peek').textContent = sel +
                ' { outline: 2px solid #ff4444 !important; outline-offset: 2px !important; }';
            try {
                var el = document.querySelector(sel);
                if (el) el.scrollIntoView({ block: 'center', behavior: 'smooth' });
            } catch (e) {}
        },
        unpeek: function (css) {
            sheet('browser-zapper').textContent = css;
            sheet('browser-peek').textContent = '';
        }
    };

    // Request initial site CSS on load
    postMsg('request_css');
})();
"#;

fn inject_default_scripts(webview: &ICoreWebView2) {
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
        let script_hstring = HSTRING::from(crate::shield::COSMETIC_SHIELD_CSS_SCRIPT);
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

    // Inject YouTube ad stripper, 16x fast-forward skipper, and anti-interruption script
    {
        let script_hstring = HSTRING::from(crate::shield::YOUTUBE_AD_STRIPPER_SCRIPT);
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

    // Inject text selection search popup script into all documents
    {
        let script_hstring = HSTRING::from(SELECTION_SEARCH_POPUP_SCRIPT);
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

    // Inject element zapper picker script into all documents
    {
        let script_hstring = HSTRING::from(ELEMENT_ZAPPER_SCRIPT);
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
}

fn configure_webview_settings(webview: &ICoreWebView2) {
    if let Ok(settings) = unsafe { webview.Settings() } {
        unsafe {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetAreDevToolsEnabled(true);
            let _ = settings.SetAreDefaultContextMenusEnabled(true);
            if let Ok(settings2) = settings.cast::<ICoreWebView2Settings2>() {
                let ua = HSTRING::from(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
                );
                let _ = settings2.SetUserAgent(PCWSTR::from_raw(ua.as_ptr()));
            }
        }
    }
}

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

        Ok(Self {
            environment,
            spare_controller: Rc::new(RefCell::new(None)),
            is_prewarming: Rc::new(Cell::new(false)),
        })
    }

    pub fn prewarm(&self, parent: HWND) {
        if self.spare_controller.borrow().is_some() || self.is_prewarming.get() {
            return;
        }
        self.is_prewarming.set(true);

        let env = self.environment.clone();
        let spare_cell = self.spare_controller.clone();
        let prewarm_flag = self.is_prewarming.clone();

        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |error_code, ctrl| {
            prewarm_flag.set(false);
            if error_code.is_ok() {
                if let Some(controller) = ctrl {
                    if let Ok(webview) = unsafe { controller.CoreWebView2() } {
                        unsafe {
                            let _ = controller.SetIsVisible(false);
                        }
                        configure_webview_settings(&webview);
                        inject_default_scripts(&webview);
                        *spare_cell.borrow_mut() = Some((controller, webview));
                        tracing::info!("Pre-warmed spare WebView2 controller ready");
                    }
                }
            }
            Ok(())
        }));

        unsafe {
            let _ = env.CreateCoreWebView2Controller(parent, &handler);
        }
    }

    pub fn create_controller(
        &self,
        parent: HWND,
        bounds: RECT,
        initial_url: &str,
        shield_state: Option<std::sync::Arc<crate::shield::ShieldState>>,
        on_nav_starting: Option<Box<dyn Fn() + Send + 'static>>,
        on_nav_completed: Option<Box<dyn Fn(bool) + Send + 'static>>,
        on_title_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_url_changed: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_favicon_changed: Option<Box<dyn Fn(Vec<u8>) + Send + 'static>>,
        on_new_window: Option<Box<dyn Fn(String) + Send + 'static>>,
        on_shortcut: Option<Box<dyn Fn(BrowserShortcut) + Send + 'static>>,
        on_fullscreen_changed: Option<Box<dyn Fn(bool) + Send + 'static>>,
        on_zapper_message: Option<Box<dyn Fn(ZapperMessage) + Send + 'static>>,
    ) -> Result<WebViewController, EngineError> {
        let (controller, webview, is_spare) = if let Some((ctrl, wv)) = self.spare_controller.borrow_mut().take() {
            (ctrl, wv, true)
        } else {
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
            (controller, webview, false)
        };

        let shield = shield_state.unwrap_or_else(|| std::sync::Arc::new(crate::shield::ShieldState::new()));
        let init_host = crate::shield::extract_host(initial_url).unwrap_or_default();
        let current_host = std::sync::Arc::new(std::sync::RwLock::new(init_host));

        // Set bounds and make visible
        unsafe {
            controller.SetBounds(bounds)?;
            controller.SetIsVisible(true)?;
        }

        if !is_spare {
            configure_webview_settings(&webview);
            inject_default_scripts(&webview);
        }

        // Register WebResourceRequested filter for zero-latency ad & tracker blocking
        let filter_hstring = HSTRING::from("*");
        unsafe {
            let _ = webview.AddWebResourceRequestedFilter(
                PCWSTR::from_raw(filter_hstring.as_ptr()),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
        }

        let env_req = self.environment.clone();
        let shield_req = shield.clone();
        let cur_host_req = current_host.clone();
        let mut resource_token = 0;
        let resource_handler = WebResourceRequestedEventHandler::create(Box::new(
            move |_sender, args| {
                if let Some(args) = args {
                    if let Ok(req) = unsafe { args.Request() } {
                        let mut uri_ptr = windows::core::PWSTR::null();
                        if unsafe { req.Uri(&mut uri_ptr) }.is_ok() && !uri_ptr.is_null() {
                            if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                                unsafe {
                                    windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _));
                                }
                                let page_host = cur_host_req.read().map(|h| h.clone()).unwrap_or_default();
                                let enabled = shield_req.is_enabled();
                                let is_paused = shield_req.is_paused(&page_host);

                                if crate::shield::should_block_request(&uri, &page_host, enabled, is_paused) {
                                    let reason = HSTRING::from("Blocked by Shield");
                                    let headers = HSTRING::from("Content-Length: 0\r\nConnection: close");
                                    if let Ok(resp) = unsafe {
                                        env_req.CreateWebResourceResponse(
                                            None,
                                            403,
                                            PCWSTR::from_raw(reason.as_ptr()),
                                            PCWSTR::from_raw(headers.as_ptr()),
                                        )
                                    } {
                                        let _ = unsafe { args.SetResponse(&resp) };
                                        shield_req.increment_blocked();
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
        ));
        unsafe {
            let _ = webview.add_WebResourceRequested(&resource_handler, &mut resource_token);
        }

        let title_cb_shared = on_title_changed.map(|cb| std::sync::Arc::new(cb));
        let url_cb_shared = on_url_changed.map(|cb| std::sync::Arc::new(cb));
        let new_win_cb_shared = on_new_window.map(|cb| std::sync::Arc::new(cb));
        let zapper_cb_shared = on_zapper_message.map(|cb| std::sync::Arc::new(cb));

        // Hook WebMessageReceived for dynamic SPA title updates & selection search
        {
            let meta_title_cb = title_cb_shared.clone();
            let meta_url_cb = url_cb_shared.clone();
            let cur_host_meta = current_host.clone();
            let search_cb = new_win_cb_shared.clone();
            let zapper_cb = zapper_cb_shared.clone();
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
                                            if let Some(h) = crate::shield::extract_host(u) {
                                                if let Ok(mut lock) = cur_host_meta.write() {
                                                    *lock = h;
                                                }
                                            }
                                            if let Some(ref cb) = meta_url_cb {
                                                cb(u.to_string());
                                            }
                                        }
                                    }
                                } else if let Some(query) = msg.strip_prefix("search:") {
                                    if let Some(ref cb) = search_cb {
                                        let search_url = format!("https://www.google.com/search?q={}", query);
                                        cb(search_url);
                                    }
                                } else if let Some(url) = msg.strip_prefix("open_tab:") {
                                    if let Some(ref cb) = search_cb {
                                        cb(url.to_string());
                                    }
                                } else if let Some(payload) = msg.strip_prefix("zapper:") {
                                    let mut parts = payload.split("|||");
                                    let action = parts.next().unwrap_or("");
                                    match action {
                                        "zap" => {
                                            let selector = parts.next().unwrap_or("").to_string();
                                            let label = parts.next().unwrap_or("").to_string();
                                            let note = parts.next().unwrap_or("").to_string();
                                            if let Some(ref cb) = zapper_cb {
                                                cb(ZapperMessage::Zap { selector, label, note });
                                            }
                                        }
                                        "undo" => {
                                            if let Some(ref cb) = zapper_cb {
                                                cb(ZapperMessage::Undo);
                                            }
                                        }
                                        "status" => {
                                            let active = parts.next().unwrap_or("false") == "true";
                                            if let Some(ref cb) = zapper_cb {
                                                cb(ZapperMessage::Status { active });
                                            }
                                        }
                                        "request_css" => {
                                            if let Some(ref cb) = zapper_cb {
                                                cb(ZapperMessage::RequestCss);
                                            }
                                        }
                                        _ => {}
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

        // Hook navigation starting to update current host and reset blocked count for new page load
        {
            let cur_host_nav = current_host.clone();
            let shield_nav = shield.clone();
            let mut start_token = 0;
            let start_handler = NavigationStartingEventHandler::create(Box::new(
                move |_sender, args| {
                    if let Some(args) = args {
                        let mut uri_ptr = windows::core::PWSTR::null();
                        if unsafe { args.Uri(&mut uri_ptr) }.is_ok() && !uri_ptr.is_null() {
                            if let Ok(uri) = unsafe { uri_ptr.to_string() } {
                                unsafe {
                                    windows::Win32::System::Com::CoTaskMemFree(Some(uri_ptr.as_ptr() as _));
                                }
                                if let Some(h) = crate::shield::extract_host(&uri) {
                                    if let Ok(mut lock) = cur_host_nav.write() {
                                        *lock = h;
                                    }
                                }
                                shield_nav.reset_blocked();
                            }
                        }
                    }
                    if let Some(ref start_cb) = on_nav_starting {
                        start_cb();
                    }
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
            let cur_host_source = current_host.clone();
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
                            if let Some(h) = crate::shield::extract_host(&uri) {
                                if let Ok(mut lock) = cur_host_source.write() {
                                    *lock = h;
                                }
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
        if let Some(cb) = new_win_cb_shared {
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

        // Hook HTML5 fullscreen changes (e.g. YouTube video full screen)
        if let Some(cb) = on_fullscreen_changed {
            let mut fs_token = 0;
            let webview_clone = webview.clone();
            let handler = ContainsFullScreenElementChangedEventHandler::create(Box::new(
                move |_sender, _args| {
                    let mut full = windows::core::BOOL(0);
                    if unsafe { webview_clone.ContainsFullScreenElement(&mut full) }.is_ok() {
                        cb(full.as_bool());
                    }
                    Ok(())
                },
            ));
            unsafe {
                let _ = webview.add_ContainsFullScreenElementChanged(&handler, &mut fs_token);
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
                                        0x43 => shortcut = Some(BrowserShortcut::CopyCleanLink), // 'C' -> Copy Clean Link (Ctrl+Shift+C)
                                        0x48 => shortcut = Some(BrowserShortcut::ToggleZapper), // 'H' -> Element Zapper!
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

        // Asynchronously pre-warm the next spare controller for instant tab creation!
        self.prewarm(parent);

        Ok(WebViewController {
            controller,
            webview,
            last_bounds: std::cell::Cell::new((0, 0, 0, 0)),
            shield_state: shield,
            current_host,
        })
    }
}

impl Drop for WebViewEngine {
    fn drop(&mut self) {
        if let Some((ctrl, _)) = self.spare_controller.borrow_mut().take() {
            unsafe {
                let _ = ctrl.Close();
            }
        }
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

    pub fn set_fullscreen_mode(&self, fullscreen: bool) {
        unsafe {
            let mut parent = HWND::default();
            if self.controller.ParentWindow(&mut parent).is_ok() && !parent.0.is_null() {
                let mut children: Vec<HWND> = Vec::new();
                unsafe extern "system" fn enum_child(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
                    let list = &mut *(lparam.0 as *mut Vec<HWND>);
                    list.push(hwnd);
                    windows::core::BOOL(1)
                }
                let _ = EnumChildWindows(
                    Some(parent),
                    Some(enum_child),
                    LPARAM(&mut children as *mut _ as isize),
                );
                let (left, top, right, bottom) = self.last_bounds.get();
                let w = right - left;
                let h = bottom - top;
                for child in children {
                    if fullscreen {
                        let _ = SetWindowRgn(child, None, true);
                    } else if w > 0 && h > 0 {
                        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 16, 16);
                        let _ = SetWindowRgn(child, Some(rgn), true);
                    }
                }
            }
        }
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
        if let Some(h) = crate::shield::extract_host(url) {
            if let Ok(mut lock) = self.current_host.write() {
                *lock = h;
            }
        }
        self.shield_state.reset_blocked();
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

    pub fn get_clean_source(&self) -> Result<String, EngineError> {
        let raw = self.get_source()?;
        Ok(crate::clean_link::clean_url(&raw))
    }

    pub fn shield_state(&self) -> &std::sync::Arc<crate::shield::ShieldState> {
        &self.shield_state
    }

    pub fn get_shield_blocked_count(&self) -> u64 {
        self.shield_state.get_blocked_count()
    }

    pub fn reset_shield_blocked_count(&self) {
        self.shield_state.reset_blocked();
    }

    pub fn set_shield_enabled(&self, enabled: bool) {
        self.shield_state.set_enabled(enabled);
    }

    pub fn is_shield_enabled(&self) -> bool {
        self.shield_state.is_enabled()
    }

    pub fn set_shield_site_paused(&self, host: &str, paused: bool) {
        self.shield_state.set_paused(host, paused);
    }

    pub fn is_shield_site_paused(&self, host: &str) -> bool {
        self.shield_state.is_paused(host)
    }

    pub fn get_current_host(&self) -> String {
        self.current_host.read().map(|h| h.clone()).unwrap_or_default()
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

    pub fn try_suspend(&self) -> Result<(), EngineError> {
        if let Ok(wv3) = self.webview.cast::<ICoreWebView2_3>() {
            let handler = TrySuspendCompletedHandler::create(Box::new(move |_error_code, _is_successful| {
                Ok(())
            }));
            unsafe {
                let _ = wv3.TrySuspend(&handler);
            }
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<(), EngineError> {
        if let Ok(wv3) = self.webview.cast::<ICoreWebView2_3>() {
            unsafe {
                let _ = wv3.Resume();
            }
        }
        Ok(())
    }

    pub fn is_suspended(&self) -> bool {
        if let Ok(wv3) = self.webview.cast::<ICoreWebView2_3>() {
            let mut suspended = windows::core::BOOL(0);
            if unsafe { wv3.IsSuspended(&mut suspended) }.is_ok() {
                return suspended.as_bool();
            }
        }
        false
    }

    pub fn execute_script(&self, script: &str) -> Result<(), EngineError> {
        let script_hstring = HSTRING::from(script);
        let handler = ExecuteScriptCompletedHandler::create(Box::new(|_res, _str| Ok(())));
        unsafe {
            let _ = self.webview.ExecuteScript(
                PCWSTR::from_raw(script_hstring.as_ptr()),
                &handler,
            );
        }
        Ok(())
    }

    pub fn toggle_zapper(&self) -> Result<(), EngineError> {
        self.execute_script(
            "if(window.__officeVeil){if(window.__officeVeil.isActive())window.__officeVeil.off();else window.__officeVeil.on();}"
        )
    }

    pub fn set_zapper_active(&self, active: bool) -> Result<(), EngineError> {
        let script = if active {
            "if(window.__officeVeil){window.__officeVeil.on();}"
        } else {
            "if(window.__officeVeil){window.__officeVeil.off();}"
        };
        self.execute_script(script)
    }

    pub fn apply_zapper_rules(&self, css: &str) -> Result<(), EngineError> {
        let escaped = css
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let script = format!("if(window.__officeVeil){{window.__officeVeil.applyRules(`{}`);}}", escaped);
        self.execute_script(&script)
    }

    pub fn peek_zapper_element(&self, css: &str, selector: &str) -> Result<(), EngineError> {
        let escaped_css = css
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let escaped_sel = selector
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let script = format!("if(window.__officeVeil){{window.__officeVeil.peek(`{}`,`{}`);}}", escaped_css, escaped_sel);
        self.execute_script(&script)
    }

    pub fn unpeek_zapper_element(&self, css: &str) -> Result<(), EngineError> {
        let escaped = css
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let script = format!("if(window.__officeVeil){{window.__officeVeil.unpeek(`{}`);}}", escaped);
        self.execute_script(&script)
    }
}

impl Drop for WebViewController {
    fn drop(&mut self) {
        unsafe {
            let _ = self.controller.Close();
        }
    }
}