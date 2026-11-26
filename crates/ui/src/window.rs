use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint,
    FillRect, FillRgn, FrameRgn, InvalidateRect, ScreenToClient, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW, IsZoomed, KillTimer, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, CS_DBLCLKS, CW_USEDEFAULT, GWLP_USERDATA,
    HTCLIENT, IDC_ARROW, IDC_HAND, IDC_SIZEWE, MSG, SWP_FRAMECHANGED, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    WM_APP, WM_CHAR, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_INITMENUPOPUP, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW,
};

use crate::frame::{enable_modern_frame, handle_nccalcsize, handle_nchittest};
use crate::sidebar::{
    ActiveTab, CaptionButton, SidebarHit, SidebarRenderer, SidebarState, TabItem,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Error, Debug)]
pub enum UiError {
    #[error("Windows error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Window creation failed")]
    WindowCreationFailed,
}

#[derive(Debug, Clone)]
pub enum SidebarAction {
    NavBack,
    NavForward,
    NavReload,
    NavNewTab,
    ToggleSidebar,
    UrlBarClicked,
    SelectTab(usize),
    CloseTab(usize),
    SelectPinned(usize),
    ClosePinnedTab(usize),
    PinTab(usize),
    UnpinTab(usize),
    DuplicateTab(usize),
    PromptRenameTab(usize),
    RenameTab(usize, String),
    SetPinnedLetter(usize, String),
    CopyUrl,
    CopyCleanLink(usize),
    CopyCleanPinnedTab(usize),
    CopyCleanLinkActive,
    TogglePinCurrentTab,
    OpenSettings,
    OpenDownloads,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextTab,
    PrevTab,
    HardReload,
    ReopenClosedTab,
    ToggleMuteTab(usize),
    ToggleMutePinnedTab(usize),
    UnloadTab(usize),
    ShareTab(usize),
    SharePinnedTab(usize),
    CloseOtherTabs(usize),
    NewTabBelow(usize),
    ReloadSpecificTab(usize),
    ReloadSpecificPinnedTab(usize),
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
    RestoreAllHidden,
    ToggleShield,
    ToggleSiteShield,
    ToggleSiteCard,
    AcceptPermission(usize),
    DropTab {
        source: crate::sidebar::DragSource,
        destination: crate::sidebar::TabDropDestination,
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
}

#[derive(Debug, Clone)]
pub struct SplitInfo {
    pub id: String,
    pub tab_indices: Vec<usize>,
    pub sizes: Vec<f32>,
    pub focused_tab_idx: usize,
}

impl SplitInfo {
    pub fn new(id: String, tab_indices: Vec<usize>, sizes: Vec<f32>, focused_tab_idx: usize) -> Self {
        Self {
            id,
            tab_indices,
            sizes,
            focused_tab_idx,
        }
    }

    pub fn left_tab_idx(&self) -> usize {
        self.tab_indices.first().copied().unwrap_or(0)
    }

    pub fn right_tab_idx(&self) -> usize {
        self.tab_indices.last().copied().unwrap_or(0)
    }

    pub fn ratio(&self) -> f32 {
        self.sizes.first().copied().unwrap_or(0.5)
    }

    pub fn is_left_focused(&self) -> bool {
        self.focused_tab_idx == self.left_tab_idx()
    }
}

const TAB_ANIM_TIMER_ID: usize = 0x4203;
const INLINE_CARET_TIMER_ID: usize = 0x4301;
const DRAG_HOVER_TIMER_ID: usize = 0x4205;

pub struct WindowConfig {
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
    pub caption_height: i32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Browser".to_string(),
            width: 1280,
            height: 800,
            sidebar_width: 280,
            caption_height: 32,
        }
    }
}

pub type ResizeCallback = Rc<dyn Fn(RECT, RECT)>;
pub type ActionCallback = Rc<dyn Fn(SidebarAction)>;
pub type CustomMsgHandler = Rc<dyn Fn(u32, WPARAM, LPARAM)>;

struct WindowState {
    config: WindowConfig,
    sidebar: SidebarRenderer,
    sidebar_state: SidebarState,
    sidebar_visible: bool,
    is_resizing_sidebar: bool,
    drag_start_x: i32,
    drag_start_width: i32,
    is_loading: bool,
    loading_progress: f32,
    pressed_hit: SidebarHit,
    on_resize: Option<ResizeCallback>,
    on_action: Option<ActionCallback>,
    on_custom_msg: Option<CustomMsgHandler>,
    is_tracking_mouse: bool,
    context_menu: Option<crate::context_menu::ContextMenu>,
    toast_overlay: Option<crate::toast::ToastOverlay>,
    folder_flyout: Option<crate::folder_flyout::FolderFlyout>,
    icon_picker: Option<crate::icon_picker::IconPicker>,
    site_card: Option<crate::site_card::SiteCard>,
    clear_data_dialog: Option<crate::confirm_dialog::ClearDataDialog>,
    drag_candidate: Option<crate::sidebar::DragCandidate>,
    split_info: Option<SplitInfo>,
    is_resizing_split: bool,
    active_split_divider_idx: usize,
    split_drag_start_x: i32,
    split_drag_start_sizes: Vec<f32>,
    is_hovering_split_divider: bool,
    split_drop_overlay: Option<crate::split_overlay::SplitDropOverlay>,
}

#[inline]
unsafe fn ensure_dark_menus() {
    crate::frame::enable_process_dark_mode();
}

pub struct BrowserWindow {
    hwnd: HWND,
    state: Rc<RefCell<WindowState>>,
}

impl BrowserWindow {
    pub fn new(config: WindowConfig) -> Result<Self, UiError> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            ensure_dark_menus();
        }

        let class_name = w!("BrowserWindowClass");

        unsafe {
            let hinstance = GetModuleHandleW(None)?.into();
            let arrow_cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                hCursor: arrow_cursor,
                lpszClassName: class_name,
                ..Default::default()
            };

            let _ = RegisterClassW(&wc);

            let initial_state = SidebarState::default();

            let state = Rc::new(RefCell::new(WindowState {
                config,
                sidebar: SidebarRenderer::new(),
                sidebar_state: initial_state,
                sidebar_visible: true,
                is_resizing_sidebar: false,
                drag_start_x: 0,
                drag_start_width: 280,
                is_loading: false,
                loading_progress: 0.0,
                pressed_hit: SidebarHit::None,
                on_resize: None,
                on_action: None,
                on_custom_msg: None,
                is_tracking_mouse: false,
                context_menu: None,
                toast_overlay: None,
                folder_flyout: None,
                icon_picker: None,
                site_card: None,
                clear_data_dialog: None,
                drag_candidate: None,
                split_info: None,
                is_resizing_split: false,
                active_split_divider_idx: 0,
                split_drag_start_x: 0,
                split_drag_start_sizes: Vec::new(),
                is_hovering_split_divider: false,
                split_drop_overlay: None,
            }));

            let raw_state = Rc::into_raw(state.clone()) as *mut _;

            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("Browser"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1280,
                800,
                None,
                None,
                Some(hinstance),
                Some(raw_state),
            )?;

            if hwnd.0 == std::ptr::null_mut() {
                return Err(UiError::WindowCreationFailed);
            }

            let menu = crate::context_menu::ContextMenu::new(hwnd).ok();
            state.borrow_mut().context_menu = menu;

            let toast = crate::toast::ToastOverlay::new(hwnd).ok();
            state.borrow_mut().toast_overlay = toast;

            let flyout = crate::folder_flyout::FolderFlyout::new(hwnd).ok();
            state.borrow_mut().folder_flyout = flyout;

            let picker = crate::icon_picker::IconPicker::new(hwnd).ok();
            state.borrow_mut().icon_picker = picker;

            let sc = crate::site_card::SiteCard::new(hwnd).ok();
            state.borrow_mut().site_card = sc;

            let cdd = crate::confirm_dialog::ClearDataDialog::new(hwnd).ok();
            state.borrow_mut().clear_data_dialog = cdd;

            let drop_overlay = crate::split_overlay::SplitDropOverlay::new(hwnd).ok();
            state.borrow_mut().split_drop_overlay = drop_overlay;

            Ok(Self { hwnd, state })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_resize_callback<F>(&self, callback: F)
    where
        F: Fn(RECT, RECT) + 'static,
    {
        self.state.borrow_mut().on_resize = Some(Rc::new(callback));
        self.trigger_resize();
    }

    pub fn set_action_callback<F>(&self, callback: F)
    where
        F: Fn(SidebarAction) + 'static,
    {
        self.state.borrow_mut().on_action = Some(Rc::new(callback));
    }

    pub fn set_custom_msg_handler<F>(&self, handler: F)
    where
        F: Fn(u32, WPARAM, LPARAM) + 'static,
    {
        self.state.borrow_mut().on_custom_msg = Some(Rc::new(handler));
    }

    pub fn update_tabs(&self, tabs: Vec<TabItem>, active_idx: usize) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.tabs = tabs;
            if active_idx < b.sidebar_state.tabs.len() {
                b.sidebar_state.active = Some(ActiveTab::Tab(active_idx));
            } else if b.sidebar_state.tabs.is_empty() {
                b.sidebar_state.active = None;
            }
        }
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn update_sidebar_data(
        &self,
        tabs: Vec<TabItem>,
        pinned: Vec<TabItem>,
        folders: Vec<crate::sidebar::FolderItem>,
        splits: Vec<crate::sidebar::SidebarSplitGroup>,
        active: Option<ActiveTab>,
        can_go_back: bool,
        can_go_forward: bool,
        has_closed_tabs: bool,
    ) {
        let active_changed = {
            let mut b = self.state.borrow_mut();
            let changed = b.sidebar_state.active != active;
            b.sidebar_state.tabs = tabs;
            b.sidebar_state.pinned = pinned;
            b.sidebar_state.folders = folders;
            b.sidebar_state.splits = splits;
            b.sidebar_state.active = active;
            b.sidebar_state.can_go_back = can_go_back;
            b.sidebar_state.can_go_forward = can_go_forward;
            b.sidebar_state.has_closed_tabs = has_closed_tabs;
            changed
        };
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            if active_changed {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            } else {
                invalidate_sidebar(self.hwnd, &self.state);
            }
        }
    }

    pub fn update_shield_status(&self, enabled: bool, blocked_count: u64, is_paused: bool) {
        let changed = {
            let mut b = self.state.borrow_mut();
            if b.sidebar_state.shield_enabled != enabled
                || b.sidebar_state.shield_blocked_count != blocked_count
                || b.sidebar_state.shield_paused_site != is_paused
            {
                b.sidebar_state.shield_enabled = enabled;
                b.sidebar_state.shield_blocked_count = blocked_count;
                b.sidebar_state.shield_paused_site = is_paused;
                true
            } else {
                false
            }
        };
        if changed {
            unsafe {
                invalidate_sidebar(self.hwnd, &self.state);
            }
        }
    }

    pub fn start_inline_rename_folder(&self, folder_id: &str, current_name: &str) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                target: crate::sidebar::InlineEditTarget::Folder(folder_id.to_string()),
                text: current_name.to_string(),
                cursor: current_name.len(),
                selection: Some((0, current_name.len())),
                caret_visible: true,
            });
        }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), INLINE_CARET_TIMER_ID, 500, None);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn start_inline_rename_tab(&self, tab_idx: usize, current_title: &str) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                target: crate::sidebar::InlineEditTarget::Tab(tab_idx),
                text: current_title.to_string(),
                cursor: current_title.len(),
                selection: Some((0, current_title.len())),
                caret_visible: true,
            });
        }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), INLINE_CARET_TIMER_ID, 500, None);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn start_inline_letter_pinned(&self, pinned_idx: usize, current_letter: &str) {
        {
            let mut b = self.state.borrow_mut();
            b.sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                target: crate::sidebar::InlineEditTarget::PinnedLetter(pinned_idx),
                text: current_letter.to_string(),
                cursor: current_letter.len(),
                selection: Some((0, current_letter.len())),
                caret_visible: true,
            });
        }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), INLINE_CARET_TIMER_ID, 500, None);
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn update_tabs_and_pinned(
        &self,
        tabs: Vec<TabItem>,
        pinned: Vec<TabItem>,
        active: Option<ActiveTab>,
        can_go_back: bool,
        can_go_forward: bool,
        has_closed_tabs: bool,
    ) {
        let folders = self.state.borrow().sidebar_state.folders.clone();
        let splits = self.state.borrow().sidebar_state.splits.clone();
        self.update_sidebar_data(tabs, pinned, folders, splits, active, can_go_back, can_go_forward, has_closed_tabs);
    }

    pub fn is_sidebar_visible(&self) -> bool {
        self.state.borrow().sidebar_visible
    }

    pub fn set_sidebar_visible(&self, visible: bool) {
        self.state.borrow_mut().sidebar_visible = visible;
        self.trigger_resize();
    }

    pub fn toggle_sidebar(&self) {
        let cur = self.is_sidebar_visible();
        self.set_sidebar_visible(!cur);
    }

    pub fn start_tab_anim_timer(&self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TAB_ANIM_TIMER_ID, 16, None);
        }
    }

    pub fn stop_tab_anim_timer(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TAB_ANIM_TIMER_ID);
        }
    }

    pub fn update_pinned_favicon(&self, url: &str, favicon: std::sync::Arc<crate::sidebar::FaviconData>) {
        let domain = crate::sidebar::domain_of(url);
        if domain.is_empty() {
            return;
        }
        let changed = {
            let mut b = self.state.borrow_mut();
            let mut changed = false;
            for p in b.sidebar_state.pinned.iter_mut() {
                let p_domain = crate::sidebar::domain_of(&p.url);
                if !p_domain.is_empty() && p_domain == domain {
                    p.favicon = Some(favicon.clone());
                    changed = true;
                }
            }
            changed
        };
        if changed {
            unsafe {
                invalidate_sidebar(self.hwnd, &self.state);
            }
        }
    }

    pub fn get_layout_rects(&self) -> (RECT, RECT) {
        compute_layout_rects(self.hwnd, &self.state)
    }

    pub fn set_split_info(&self, info: Option<SplitInfo>) {
        {
            let mut b = self.state.borrow_mut();
            b.split_info = info;
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn get_split_layout_rects(&self) -> Option<(RECT, RECT, RECT)> {
        compute_split_layout_rects(self.hwnd, &self.state)
    }

    pub fn get_split_layout_rects_multi(&self) -> Option<(Vec<RECT>, Vec<RECT>)> {
        compute_split_layout_rects_multi(self.hwnd, &self.state)
    }

    pub fn trigger_resize(&self) {
        let (content_rect, sidebar_rect) = self.get_layout_rects();
        let cb = self.state.borrow().on_resize.clone();
        if let Some(cb) = cb {
            cb(content_rect, sidebar_rect);
        }
        unsafe {
            clamp_scroll(self.hwnd, &self.state);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn set_loading(&self, is_loading: bool, progress: f32) {
        {
            let mut b = self.state.borrow_mut();
            b.is_loading = is_loading;
            b.loading_progress = progress.clamp(0.0, 1.0);
        }
        unsafe {
            let (content_rect, _) = self.get_layout_rects();
            let top_strip = RECT {
                left: content_rect.left,
                top: 0,
                right: content_rect.right,
                bottom: content_rect.top + 8,
            };
            let _ = InvalidateRect(Some(self.hwnd), Some(&top_strip), false);
        }
    }

    pub fn set_zoom(&self, zoom: f64) {
        self.state.borrow_mut().sidebar_state.zoom = zoom;
        unsafe {
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn set_pending_permissions(&self, perms: Vec<crate::sidebar::SitePermissionKind>) {
        self.state.borrow_mut().sidebar_state.pending_permissions = perms;
        unsafe {
            invalidate_sidebar(self.hwnd, &self.state);
        }
    }

    pub fn show_toast(&self, message: &str) {
        if let Some(ref t) = self.state.borrow().toast_overlay {
            t.show(message);
        }
    }

    pub fn show_site_card(
        &self,
        anchor_x: i32,
        anchor_y: i32,
        data: crate::site_card::SiteCardData,
        on_action: Box<dyn Fn(crate::site_card::SiteCardAction)>,
    ) {
        if let Some(ref sc) = self.state.borrow().site_card {
            sc.show(anchor_x, anchor_y, data, on_action);
        }
    }

    pub fn update_site_card(&self, data: crate::site_card::SiteCardData) {
        if let Some(ref sc) = self.state.borrow().site_card {
            sc.update_data(data);
        }
    }

    pub fn hide_site_card(&self) {
        if let Some(ref sc) = self.state.borrow().site_card {
            sc.hide();
        }
    }

    pub fn is_site_card_visible(&self) -> bool {
        self.state
            .borrow()
            .site_card
            .as_ref()
            .map(|sc| sc.is_visible())
            .unwrap_or(false)
    }

    pub fn is_site_card_visible_or_recently_closed(&self) -> bool {
        self.state
            .borrow()
            .site_card
            .as_ref()
            .map(|sc| sc.is_visible_or_recently_closed())
            .unwrap_or(false)
    }

    pub fn show_clear_data_dialog(&self, domain: &str, on_choice: Box<dyn Fn(bool)>) {
        if let Some(ref dlg) = self.state.borrow().clear_data_dialog {
            dlg.show(domain, on_choice);
        }
    }

    pub fn hide_clear_data_dialog(&self) {
        if let Some(ref dlg) = self.state.borrow().clear_data_dialog {
            dlg.hide();
        }
    }

    pub fn get_url_screen_rect(&self) -> RECT {
        let st = self.state.borrow();
        let mut wr = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut wr);
        }
        let (_, sidebar_rect) = compute_layout_rects(self.hwnd, &self.state);
        let s = SidebarRenderer::scale(self.hwnd);
        let sw = sidebar_rect.right - sidebar_rect.left;
        let l = st.sidebar.compute_layout(s, sw, wr.bottom - wr.top, &st.sidebar_state);
        crate::sidebar::rc(
            wr.left + sidebar_rect.left + l.url.left,
            wr.top + sidebar_rect.top + l.url.top,
            wr.left + sidebar_rect.left + l.url.right,
            wr.top + sidebar_rect.top + l.url.bottom,
        )
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
    }

    pub fn run_message_loop(&self) {
        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

// ───────────────────────────── Helpers ─────────────────────────────

fn caption_of(hit: SidebarHit) -> CaptionButton {
    match hit {
        SidebarHit::Caption(c) => c,
        _ => CaptionButton::None,
    }
}

fn compute_layout_rects(hwnd: HWND, state: &RefCell<WindowState>) -> (RECT, RECT) {
    let mut client_rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client_rect);
    }

    let width = client_rect.right - client_rect.left;
    let height = client_rect.bottom - client_rect.top;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = dpi as f32 / 96.0;

    let sidebar_visible = state.try_borrow().map(|s| s.sidebar_visible).unwrap_or(true);
    let pad = (8.0 * scale).round() as i32;

    if sidebar_visible {
        let sidebar_width_cfg = state.try_borrow().map(|s| s.config.sidebar_width).unwrap_or(280);
        let sidebar_width = (sidebar_width_cfg as f32 * scale).round() as i32;
        let available_content_w = (width - sidebar_width).max(0);

        let content_rect = RECT {
            left: pad,
            top: pad,
            right: (available_content_w - pad).max(pad),
            bottom: (height - pad).max(pad),
        };

        let sidebar_rect = RECT {
            left: available_content_w,
            top: 0,
            right: width,
            bottom: height,
        };

        (content_rect, sidebar_rect)
    } else {
        let content_rect = RECT {
            left: pad,
            top: pad,
            right: (width - pad).max(pad),
            bottom: (height - pad).max(pad),
        };

        let sidebar_rect = RECT {
            left: width,
            top: 0,
            right: width,
            bottom: height,
        };

        (content_rect, sidebar_rect)
    }
}

/// Sidebar rect in client coordinates.
unsafe fn sidebar_rect_of(hwnd: HWND, state: &RefCell<WindowState>) -> RECT {
    let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
    sidebar_rect
}

/// Hit-test a point given in client coordinates.
unsafe fn hit_at(hwnd: HWND, state: &RefCell<WindowState>, x: i32, y: i32) -> SidebarHit {
    let b = state.borrow();
    if !b.sidebar_visible {
        return SidebarHit::None;
    }

    let r = sidebar_rect_of(hwnd, state);
    if x < r.left || x >= r.right || y < r.top || y >= r.bottom {
        return SidebarHit::None;
    }

    b.sidebar.hit_test(
        hwnd,
        x - r.left,
        y - r.top,
        r.right - r.left,
        r.bottom - r.top,
        &b.sidebar_state,
    )
}

unsafe fn invalidate_sidebar(hwnd: HWND, state: &RefCell<WindowState>) {
    let r = sidebar_rect_of(hwnd, state);
    let _ = InvalidateRect(Some(hwnd), Some(&r), false);
}

fn is_on_sidebar_edge(hwnd: HWND, state: &RefCell<WindowState>, x: i32, _y: i32) -> bool {
    let b = state.borrow();
    if !b.sidebar_visible {
        return false;
    }
    drop(b);
    let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
    let border_x = sidebar_rect.left;
    (x - border_x).abs() <= 5
}

pub fn calculate_split_rects_multi(content_rect: RECT, sizes: &[f32], scale: f32) -> Option<(Vec<RECT>, Vec<RECT>)> {
    let count = sizes.len();
    if count < 2 {
        return None;
    }
    let gutter = (7.0 * scale).round().max(6.0) as i32;
    let total_w = content_rect.right - content_rect.left;
    let total_gutters = (count as i32 - 1) * gutter;
    if total_w <= total_gutters + 50 {
        return None;
    }
    let avail_w = total_w - total_gutters;

    // Normalize sizes to sum to 1.0
    let sum: f32 = sizes.iter().map(|&s| if s.is_finite() && s > 0.0 { s } else { 1.0 }).sum();
    let norm_sizes: Vec<f32> = if sum > 0.0 {
        sizes.iter().map(|&s| (if s.is_finite() && s > 0.0 { s } else { 1.0 }) / sum).collect()
    } else {
        vec![1.0 / count as f32; count]
    };

    let mut pane_rects = Vec::with_capacity(count);
    let mut div_rects = Vec::with_capacity(count - 1);
    let mut cur_x = content_rect.left;

    for i in 0..count {
        let is_last = i == count - 1;
        let pane_w = if is_last {
            content_rect.right - cur_x
        } else {
            ((avail_w as f32) * norm_sizes[i]).round() as i32
        };

        let pane_right = if is_last {
            content_rect.right
        } else {
            cur_x + pane_w
        };

        pane_rects.push(RECT {
            left: cur_x,
            top: content_rect.top,
            right: pane_right,
            bottom: content_rect.bottom,
        });

        if !is_last {
            let div_left = pane_right;
            let div_right = div_left + gutter;
            div_rects.push(RECT {
                left: div_left,
                top: content_rect.top,
                right: div_right,
                bottom: content_rect.bottom,
            });
            cur_x = div_right;
        }
    }

    Some((pane_rects, div_rects))
}

pub fn calculate_split_rects(content_rect: RECT, ratio: f32, scale: f32) -> Option<(RECT, RECT, RECT)> {
    let clamped_ratio = ratio.clamp(0.20, 0.80);
    let (panes, divs) = calculate_split_rects_multi(content_rect, &[clamped_ratio, 1.0 - clamped_ratio], scale)?;
    if panes.len() == 2 && divs.len() == 1 {
        Some((panes[0], divs[0], panes[1]))
    } else {
        None
    }
}

pub fn snap_split_ratio(x: i32, content_left: i32, avail_w: i32, scale: f32) -> f32 {
    let raw_ratio = (x - content_left) as f32 / avail_w.max(1) as f32;
    let clamped = raw_ratio.clamp(0.20, 0.80);
    let snap_px = (15.0 * scale).round() as i32;
    let center_x = content_left + avail_w / 2;
    if (x - center_x).abs() <= snap_px {
        0.50
    } else {
        clamped
    }
}

fn compute_split_layout_rects_multi(hwnd: HWND, state: &RefCell<WindowState>) -> Option<(Vec<RECT>, Vec<RECT>)> {
    let b = state.try_borrow().ok()?;
    let split = b.split_info.as_ref()?;
    let sizes = split.sizes.clone();
    drop(b);

    let (content_rect, _) = compute_layout_rects(hwnd, state);
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = dpi as f32 / 96.0;

    calculate_split_rects_multi(content_rect, &sizes, scale)
}

fn compute_split_layout_rects(hwnd: HWND, state: &RefCell<WindowState>) -> Option<(RECT, RECT, RECT)> {
    let (panes, divs) = compute_split_layout_rects_multi(hwnd, state)?;
    if panes.len() >= 2 && !divs.is_empty() {
        Some((panes[0], divs[0], panes[1]))
    } else {
        None
    }
}

fn which_split_divider(hwnd: HWND, state: &RefCell<WindowState>, x: i32, y: i32) -> Option<usize> {
    if let Some((_panes, divs)) = compute_split_layout_rects_multi(hwnd, state) {
        for (i, div) in divs.iter().enumerate() {
            if x >= div.left - 3 && x <= div.right + 3 && y >= div.top && y <= div.bottom {
                return Some(i);
            }
        }
    }
    None
}

fn is_on_split_divider(hwnd: HWND, state: &RefCell<WindowState>, x: i32, y: i32) -> bool {
    which_split_divider(hwnd, state, x, y).is_some()
}

unsafe fn clamp_scroll(hwnd: HWND, state: &RefCell<WindowState>) {
    let r = sidebar_rect_of(hwnd, state);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let mut b = state.borrow_mut();
    if b.sidebar_visible {
        let max = b.sidebar.max_scroll(hwnd, h, w, &b.sidebar_state);
        let cur = b.sidebar_state.scroll;
        b.sidebar_state.scroll = cur.clamp(0, max);
        let max_top = b.sidebar.max_scroll_top(hwnd, h, w, &b.sidebar_state);
        let cur_top = b.sidebar_state.scroll_top;
        b.sidebar_state.scroll_top = cur_top.clamp(0, max_top);
    }
}

// ───────────────────────────── Window proc ─────────────────────────────

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<WindowState>;

    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let raw_state = create_struct.lpCreateParams as *mut RefCell<WindowState>;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        enable_modern_frame(hwnd);
        return LRESULT(0);
    }

    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = &*state_ptr;

    match msg {
        WM_NCCALCSIZE => {
            if let Some(res) = handle_nccalcsize(hwnd, wparam, lparam) {
                return res;
            }
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let hit = hit_at(hwnd, state, pt.x, pt.y);
            let is_edge = is_on_sidebar_edge(hwnd, state, pt.x, pt.y);

            let (sidebar_w, caption_h, sidebar_vis) = {
                let b = state.borrow();
                let dpi = GetDpiForWindow(hwnd);
                let sw = (b.config.sidebar_width as f32 * (dpi as f32 / 96.0)).round() as i32;
                let ch = (32.0 * (dpi as f32 / 96.0)).round() as i32;
                (sw, ch, b.sidebar_visible)
            };

            // Sidebar client area must always be HTCLIENT to prevent non-client hover/flicker loops
            let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
            if sidebar_vis && pt.x >= sidebar_rect.left && pt.y >= caption_h {
                return LRESULT(HTCLIENT as isize);
            }

            let is_interactive = is_edge || !matches!(hit, SidebarHit::None);
            return handle_nchittest(hwnd, lparam, sidebar_w, caption_h, sidebar_vis, is_interactive);
        }
        WM_SETCURSOR => {
            let hit_test = (lparam.0 & 0xffff) as i16 as u32;
            if hit_test == HTCLIENT {
                if state.borrow().sidebar_state.drag_state.is_some() {
                    let arrow_c = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
                    let _ = SetCursor(Some(arrow_c));
                    return LRESULT(1);
                }
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                let is_resizing = state.borrow().is_resizing_sidebar;
                let is_resizing_split = state.borrow().is_resizing_split;
                if is_resizing || is_on_sidebar_edge(hwnd, state, pt.x, pt.y)
                    || is_resizing_split || is_on_split_divider(hwnd, state, pt.x, pt.y) {
                    if let Ok(c) = LoadCursorW(None, IDC_SIZEWE) {
                        let _ = SetCursor(Some(c));
                        return LRESULT(1);
                    }
                } else {
                    let hit = hit_at(hwnd, state, pt.x, pt.y);
                    let is_interactive = matches!(
                        hit,
                        SidebarHit::Pinned(_)
                            | SidebarHit::Folder(_)
                            | SidebarHit::FolderIcon(_)
                            | SidebarHit::JumpToActiveTab
                            | SidebarHit::Tab(_)
                            | SidebarHit::TabClose(_)
                            | SidebarHit::NewTab
                            | SidebarHit::Back
                            | SidebarHit::Forward
                            | SidebarHit::Reload
                            | SidebarHit::ToggleSidebar
                            | SidebarHit::CleanLinkBtn
                            | SidebarHit::SiteSettings
                            | SidebarHit::PermissionIcon(_)
                            | SidebarHit::UrlBar
                            | SidebarHit::ZoomReset
                            | SidebarHit::Shield
                            | SidebarHit::Downloads
                            | SidebarHit::Settings
                    );
                    let cursor_id = if is_interactive { IDC_HAND } else { IDC_ARROW };
                    if let Ok(c) = LoadCursorW(None, cursor_id) {
                        let _ = SetCursor(Some(c));
                        return LRESULT(1);
                    }
                }
            }
        }
        WM_ERASEBKGND => {
            return LRESULT(1);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let b = state.borrow();

            let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);

            // Fill window background / frame padding with the unified dark chassis color (matches sidebar)
            let brush = CreateSolidBrush(b.sidebar.theme.bg);
            let frame_rect = if b.sidebar_visible {
                RECT {
                    left: 0,
                    top: 0,
                    right: sidebar_rect.left,
                    bottom: client.bottom,
                }
            } else {
                client
            };
            let _ = FillRect(hdc, &frame_rect, brush);
            let _ = DeleteObject(brush.into());

            // If empty (no active tabs), draw the stage card with reduced darkness (#1e1e23) inside content_rect
            if b.sidebar_state.active.is_none() {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let corner_r = (16.0 * scale).round() as i32;
                let stage_rgn = CreateRoundRectRgn(
                    content_rect.left,
                    content_rect.top,
                    content_rect.right,
                    content_rect.bottom,
                    corner_r,
                    corner_r,
                );
                let stage_brush = CreateSolidBrush(crate::sidebar::rgb(23, 23, 26));
                let _ = FillRgn(hdc, stage_rgn, stage_brush);
                let _ = DeleteObject(stage_brush.into());

                let border_brush = CreateSolidBrush(crate::sidebar::rgb(35, 35, 40));
                let _ = FrameRgn(hdc, stage_rgn, border_brush, 1, 1);
                let _ = DeleteObject(border_brush.into());
                let _ = DeleteObject(stage_rgn.into());
            }

            // If loading, draw sleek capsule pill loading progress bar with rounded corners in #14dcff
            if b.is_loading && b.loading_progress > 0.0 {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let bar_h = (3.5 * scale).round().max(3.0) as i32;
                let bar_y = (content_rect.top - bar_h - 2).max(1);
                let max_w = (content_rect.right - content_rect.left).max(0);
                let current_w = (max_w as f32 * b.loading_progress).round().max(bar_h as f32 * 2.0) as i32;
                let bar_left = content_rect.left;
                let bar_right = (content_rect.left + current_w).min(content_rect.right);
                if bar_right > bar_left {
                    let rgn = CreateRoundRectRgn(bar_left, bar_y, bar_right, bar_y + bar_h, bar_h, bar_h);
                    let load_brush = CreateSolidBrush(COLORREF(0x00ffdc14)); // #14dcff (RGB: 20, 220, 255)
                    let _ = FillRgn(hdc, rgn, load_brush);
                    let _ = DeleteObject(load_brush.into());
                    let _ = DeleteObject(rgn.into());
                }
            }

            // If split view is active, render the draggable dividers between panes
            if let Some((_pane_rects, div_rects)) = compute_split_layout_rects_multi(hwnd, state) {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;

                for (di, div_rect) in div_rects.iter().enumerate() {
                    // Fill divider gutter with the unified chassis background
                    let div_bg_brush = CreateSolidBrush(crate::sidebar::rgb(22, 22, 25));
                    let _ = FillRect(hdc, div_rect, div_bg_brush);
                    let _ = DeleteObject(div_bg_brush.into());

                    // Subtle hairline borders on left and right edges of divider
                    let border_pen_brush = CreateSolidBrush(crate::sidebar::rgb(35, 35, 40));
                    let left_edge = RECT { left: div_rect.left, top: div_rect.top, right: div_rect.left + 1, bottom: div_rect.bottom };
                    let right_edge = RECT { left: div_rect.right - 1, top: div_rect.top, right: div_rect.right, bottom: div_rect.bottom };
                    let _ = FillRect(hdc, &left_edge, border_pen_brush);
                    let _ = FillRect(hdc, &right_edge, border_pen_brush);
                    let _ = DeleteObject(border_pen_brush.into());

                    // Focus accent line indicating which adjacent pane is currently focused
                    let focused_idx = b.split_info.as_ref().map(|sp| sp.focused_tab_idx).unwrap_or(0);
                    let is_left_of_div = focused_idx == di;
                    let is_right_of_div = focused_idx == di + 1;
                    if is_left_of_div || is_right_of_div {
                        let focus_brush = CreateSolidBrush(crate::sidebar::rgb(140, 140, 150)); // subtle neutral silver
                        let focus_edge = if is_left_of_div {
                            RECT { left: div_rect.left, top: div_rect.top, right: div_rect.left + 2, bottom: div_rect.bottom }
                        } else {
                            RECT { left: div_rect.right - 2, top: div_rect.top, right: div_rect.right, bottom: div_rect.bottom }
                        };
                        let _ = FillRect(hdc, &focus_edge, focus_brush);
                        let _ = DeleteObject(focus_brush.into());
                    }

                    // Centered rounded grip handle pill
                    let grip_w = (3.0 * scale).round().max(2.0) as i32;
                    let grip_h = (36.0 * scale).round().max(24.0) as i32;
                    let grip_x = div_rect.left + (div_rect.right - div_rect.left - grip_w) / 2;
                    let grip_y = div_rect.top + (div_rect.bottom - div_rect.top - grip_h) / 2;
                    let grip_rect = RECT { left: grip_x, top: grip_y, right: grip_x + grip_w, bottom: grip_y + grip_h };

                    let grip_color = if b.is_resizing_split && b.active_split_divider_idx == di {
                        crate::sidebar::rgb(220, 220, 225) // bright white-silver when dragging
                    } else if b.is_hovering_split_divider {
                        crate::sidebar::rgb(180, 180, 190) // silver on hover
                    } else {
                        crate::sidebar::rgb(75, 75, 85) // subtle grey idle
                    };
                    let grip_brush = CreateSolidBrush(grip_color);
                    let grip_rgn = CreateRoundRectRgn(grip_rect.left, grip_rect.top, grip_rect.right, grip_rect.bottom, grip_w, grip_w);
                    let _ = FillRgn(hdc, grip_rgn, grip_brush);
                    let _ = DeleteObject(grip_brush.into());
                    let _ = DeleteObject(grip_rgn.into());
                }
            }

            // Render sidebar smoothly from offscreen memory DC
            if b.sidebar_visible {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let base_sidebar_w = (b.config.sidebar_width as f32 * scale).round() as i32;
                let hovered = caption_of(b.sidebar_state.hovered);
                let pressed = caption_of(b.pressed_hit);
                b.sidebar.render(hwnd, hdc, sidebar_rect, base_sidebar_w, &b.sidebar_state, hovered, pressed);
            }

            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if state.borrow().is_resizing_split {
                let (active_div, start_sizes) = {
                    let b = state.borrow();
                    (b.active_split_divider_idx, b.split_drag_start_sizes.clone())
                };

                let count = start_sizes.len();
                if count >= 2 && active_div + 1 < count {
                    let (content_rect, _) = compute_layout_rects(hwnd, state);
                    let dpi = GetDpiForWindow(hwnd);
                    let scale = dpi as f32 / 96.0;
                    let gutter = (7.0 * scale).round().max(6.0) as i32;
                    let total_gutters = (count as i32 - 1) * gutter;
                    let avail_w = (content_rect.right - content_rect.left - total_gutters).max(50);

                    // Compute the start X of the pair being resized
                    let start_share: f32 = start_sizes[..active_div].iter().sum();
                    let pair_start_x = content_rect.left + (start_share * avail_w as f32).round() as i32 + (active_div as i32 * gutter);
                    let both_fraction = start_sizes[active_div] + start_sizes[active_div + 1];
                    let both_px = both_fraction * avail_w as f32;

                    // Mouse X relative to the pair
                    let left_px = (x - gutter / 2 - pair_start_x) as f32;
                    let least_fraction = if count <= 2 { 0.20 } else { 0.15 };
                    let least_px = least_fraction * avail_w as f32;
                    let clamped_left_px = left_px.clamp(least_px, (both_px - least_px).max(least_px));

                    // 15px snap to even 50/50 of this pair
                    let even_px = both_px / 2.0;
                    let snapped_left_px = if (clamped_left_px - even_px).abs() <= 15.0 * scale {
                        even_px
                    } else {
                        clamped_left_px
                    };

                    let first_fraction = snapped_left_px / avail_w as f32;
                    let second_fraction = (both_px - snapped_left_px) / avail_w as f32;

                    let mut new_sizes = start_sizes.clone();
                    new_sizes[active_div] = first_fraction;
                    new_sizes[active_div + 1] = second_fraction;

                    let changed = {
                        let mut b = state.borrow_mut();
                        if let Some(ref mut sp) = b.split_info {
                            sp.sizes = new_sizes.clone();
                            true
                        } else {
                            false
                        }
                    };

                    if changed {
                        let on_resize = state.borrow().on_resize.clone();
                        if let Some(cb) = on_resize {
                            let (content, sidebar) = compute_layout_rects(hwnd, state);
                            cb(content, sidebar);
                        }
                        let _ = InvalidateRect(Some(hwnd), Some(&content_rect), false);
                    }
                }
                return LRESULT(0);
            }

            let is_on_div = is_on_split_divider(hwnd, state, x, y);
            let div_hover_changed = {
                let mut b = state.borrow_mut();
                if b.is_hovering_split_divider != is_on_div {
                    b.is_hovering_split_divider = is_on_div;
                    true
                } else {
                    false
                }
            };
            if div_hover_changed {
                if let Some((_, divs)) = compute_split_layout_rects_multi(hwnd, state) {
                    for div in divs {
                        let _ = InvalidateRect(Some(hwnd), Some(&div), false);
                    }
                }
            }

            if state.borrow().is_resizing_sidebar {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;

                let (start_x, start_w) = {
                    let b = state.borrow();
                    (b.drag_start_x, b.drag_start_width)
                };

                let delta_x = start_x - x;
                let delta_logical = ((delta_x as f32) / scale).round() as i32;
                let clamped_w = (start_w + delta_logical).clamp(200, 520);

                let changed = {
                    let mut b = state.borrow_mut();
                    if b.config.sidebar_width != clamped_w {
                        b.config.sidebar_width = clamped_w;
                        true
                    } else {
                        false
                    }
                };

                if changed {
                    let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);
                    let cb = state.borrow().on_resize.clone();
                    if let Some(cb) = cb {
                        cb(content_rect, sidebar_rect);
                    }
                    clamp_scroll(hwnd, state);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                }
                return LRESULT(0);
            }

            // Check if starting or continuing a drag
            let start_drag_info = {
                let b = state.borrow();
                if b.sidebar_state.drag_state.is_none() {
                    if let Some(cand) = b.drag_candidate {
                        let dist = (((x - cand.start_x).pow(2) + (y - cand.start_y).pow(2)) as f32).sqrt();
                        if dist >= 4.0 {
                            let item = match cand.source {
                                crate::sidebar::DragSource::Tab(idx) => {
                                    b.sidebar_state.tabs.get(idx).map(|t| crate::sidebar::DragCarriedItem {
                                        title: t.title.clone(),
                                        url: t.url.clone(),
                                        accent: t.accent,
                                        favicon: t.favicon.clone(),
                                        is_loaded: t.is_loaded,
                                        custom_letter: t.custom_letter.clone(),
                                    })
                                }
                                crate::sidebar::DragSource::Pinned(idx) => {
                                    b.sidebar_state.pinned.get(idx).map(|p| crate::sidebar::DragCarriedItem {
                                        title: p.title.clone(),
                                        url: p.url.clone(),
                                        accent: p.accent,
                                        favicon: p.favicon.clone(),
                                        is_loaded: p.is_loaded,
                                        custom_letter: p.custom_letter.clone(),
                                    })
                                }
                            };
                            item.map(|it| (cand.source, cand.start_x, cand.start_y, it))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((src, sx, sy, it)) = start_drag_info {
                let mut b = state.borrow_mut();
                b.sidebar_state.drag_state = Some(crate::sidebar::DragState {
                    source: src,
                    item: it,
                    start_x: sx,
                    start_y: sy,
                    cur_x: x,
                    cur_y: y,
                    target: None,
                    folder_hover_start: None,
                });
                let _ = SetCapture(hwnd);
            }

            let is_dragging = state.borrow().sidebar_state.drag_state.is_some();
            if is_dragging {
                let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);
                let is_in_content = x >= content_rect.left && x <= content_rect.right && y >= content_rect.top && y <= content_rect.bottom;
                let target = if is_in_content {
                    let b = state.borrow();
                    let split_info_opt = b.split_info.clone();
                    if let Some(sp) = split_info_opt {
                        if sp.tab_indices.len() >= 4 {
                            // Max 4 vertical split panes reached
                            if let Some(ref ov) = b.split_drop_overlay {
                                ov.hide();
                            }
                            None
                        } else {
                            let (panes, _) = compute_split_layout_rects_multi(hwnd, state).unwrap_or_default();
                            let mut target_pane_idx = 0;
                            for (pi, pr) in panes.iter().enumerate() {
                                if x >= pr.left && x <= pr.right {
                                    target_pane_idx = pi;
                                    break;
                                }
                            }
                            if target_pane_idx < panes.len() {
                                let pr = panes[target_pane_idx];
                                let mid = pr.left + (pr.right - pr.left) / 2;
                                let on_left = x < mid;
                                let drop_rect = if on_left {
                                    RECT { left: pr.left, top: pr.top, right: mid, bottom: pr.bottom }
                                } else {
                                    RECT { left: mid, top: pr.top, right: pr.right, bottom: pr.bottom }
                                };
                                let target_tab_id = b.sidebar_state.tabs.get(sp.tab_indices[target_pane_idx]).map(|t| t.id.clone());
                                let label = if on_left { "Split Left" } else { "Split Right" };
                                let accent = crate::sidebar::rgb(220, 220, 225);
                                if let Some(ref ov) = b.split_drop_overlay {
                                    ov.show(drop_rect, label, accent);
                                }
                                Some(crate::sidebar::DragTarget::WindowSplit { target_tab_id, on_left })
                            } else {
                                None
                            }
                        }
                    } else {
                        // Single active tab
                        let mid = content_rect.left + (content_rect.right - content_rect.left) / 2;
                        let on_left = x < mid;
                        let drop_rect = if on_left {
                            RECT { left: content_rect.left, top: content_rect.top, right: mid, bottom: content_rect.bottom }
                        } else {
                            RECT { left: mid, top: content_rect.top, right: content_rect.right, bottom: content_rect.bottom }
                        };
                        let active_tab_id = match b.sidebar_state.active {
                            Some(crate::sidebar::ActiveTab::Tab(i)) => b.sidebar_state.tabs.get(i).map(|t| t.id.clone()),
                            _ => None,
                        };
                        let label = if on_left { "Split Left" } else { "Split Right" };
                        let accent = crate::sidebar::rgb(220, 220, 225);
                        if let Some(ref ov) = b.split_drop_overlay {
                            ov.show(drop_rect, label, accent);
                        }
                        Some(crate::sidebar::DragTarget::WindowSplit { target_tab_id: active_tab_id, on_left })
                    }
                } else {
                    let b = state.borrow();
                    if let Some(ref ov) = b.split_drop_overlay {
                        ov.hide();
                    }
                    b.sidebar.compute_drag_target(hwnd, &b.sidebar_state, x, y, sidebar_rect)
                };

                let folder_to_open = {
                    let mut b = state.borrow_mut();
                    let SidebarState { ref mut drag_state, ref folders, .. } = b.sidebar_state;
                    if let Some(ref mut ds) = drag_state {
                        ds.cur_x = x;
                        ds.cur_y = y;
                        ds.target = target;

                        // Check folder hover auto-open
                        if let Some(crate::sidebar::DragTarget::Folder(f_idx)) = ds.target {
                            let is_closed = folders.get(f_idx).map_or(false, |f| !f.is_open);
                            if is_closed {
                                match ds.folder_hover_start {
                                    Some((idx, start)) if idx == f_idx => {
                                        if start.elapsed() >= std::time::Duration::from_millis(200) {
                                            ds.folder_hover_start = None;
                                            Some(f_idx)
                                        } else {
                                            None
                                        }
                                    }
                                    _ => {
                                        ds.folder_hover_start = Some((f_idx, std::time::Instant::now()));
                                        unsafe { let _ = SetTimer(Some(hwnd), DRAG_HOVER_TIMER_ID, 60, None); }
                                        None
                                    }
                                }
                            } else {
                                ds.folder_hover_start = None;
                                None
                            }
                        } else {
                            ds.folder_hover_start = None;
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(f_idx) = folder_to_open {
                    let on_action = state.borrow().on_action.clone();
                    if let Some(ref cb) = on_action {
                        cb(SidebarAction::OpenFolder(f_idx));
                    }
                }

                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }

            if !state.borrow().is_tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                state.borrow_mut().is_tracking_mouse = true;
            }

            let (_, sidebar_rect) = compute_layout_rects(hwnd, state);
            let is_over_sidebar = x >= sidebar_rect.left && x <= sidebar_rect.right && y >= sidebar_rect.top && y <= sidebar_rect.bottom;
            let over_changed = {
                let mut b = state.borrow_mut();
                if b.sidebar_state.is_mouse_over_sidebar != is_over_sidebar {
                    b.sidebar_state.is_mouse_over_sidebar = is_over_sidebar;
                    true
                } else {
                    false
                }
            };

            let hit = hit_at(hwnd, state, x, y);
            let hit_changed = {
                let mut b = state.borrow_mut();
                if hit != b.sidebar_state.hovered {
                    b.sidebar_state.hovered = hit;
                    true
                } else {
                    false
                }
            };
            if over_changed || hit_changed {
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            state.borrow_mut().is_tracking_mouse = false;
            let changed = {
                let b = state.borrow();
                b.sidebar_state.hovered != SidebarHit::None || b.pressed_hit != SidebarHit::None || b.sidebar_state.is_mouse_over_sidebar
            };
            if changed {
                {
                    let mut b = state.borrow_mut();
                    b.sidebar_state.hovered = SidebarHit::None;
                    b.pressed_hit = SidebarHit::None;
                    b.sidebar_state.is_mouse_over_sidebar = false;
                }
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if let Some(ref menu) = state.borrow().context_menu {
                if menu.is_visible() {
                    menu.hide();
                }
            }
            if let Some(ref ip) = state.borrow().icon_picker {
                if ip.is_visible() {
                    ip.hide();
                }
            }

            let had_edit = {
                let mut b = state.borrow_mut();
                b.sidebar_state.inline_edit.take()
            };
            if let Some(ed) = had_edit {
                let trimmed = ed.text.trim().to_string();
                if !trimmed.is_empty() {
                    let on_action = state.borrow().on_action.clone();
                    if let Some(ref cb) = on_action {
                        match ed.target {
                            crate::sidebar::InlineEditTarget::Folder(fid) => {
                                cb(SidebarAction::RenameFolder(fid, trimmed));
                            }
                            crate::sidebar::InlineEditTarget::Tab(idx) => {
                                cb(SidebarAction::RenameTab(idx, trimmed));
                            }
                            crate::sidebar::InlineEditTarget::PinnedLetter(idx) => {
                                cb(SidebarAction::SetPinnedLetter(idx, trimmed));
                            }
                        }
                    }
                }
                unsafe {
                    let _ = KillTimer(Some(hwnd), INLINE_CARET_TIMER_ID);
                    invalidate_sidebar(hwnd, state);
                }
            }

            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if let Some(div_idx) = which_split_divider(hwnd, state, x, y) {
                if msg == WM_LBUTTONDBLCLK {
                    {
                        let mut b = state.borrow_mut();
                        if let Some(ref mut sp) = b.split_info {
                            let count = sp.tab_indices.len().max(1);
                            sp.sizes = vec![1.0 / count as f32; count];
                        }
                    }
                    let on_action = state.borrow().on_action.clone();
                    if let Some(ref cb) = on_action {
                        cb(SidebarAction::EvenSplit);
                    }
                    if let Some((_l, divs)) = compute_split_layout_rects_multi(hwnd, state) {
                        for div in divs {
                            let _ = InvalidateRect(Some(hwnd), Some(&div), false);
                        }
                    }
                    return LRESULT(0);
                }
                let mut b = state.borrow_mut();
                let start_sizes = b.split_info.as_ref().map(|sp| sp.sizes.clone());
                if let Some(sizes) = start_sizes {
                    b.is_resizing_split = true;
                    b.active_split_divider_idx = div_idx;
                    b.split_drag_start_x = x;
                    b.split_drag_start_sizes = sizes;
                }
                drop(b);
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

            if is_on_sidebar_edge(hwnd, state, x, y) {
                let mut b = state.borrow_mut();
                b.is_resizing_sidebar = true;
                b.drag_start_x = x;
                b.drag_start_width = b.config.sidebar_width;
                drop(b);
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

            let hit = hit_at(hwnd, state, x, y);
            if msg == WM_LBUTTONDBLCLK {
                if let SidebarHit::Pinned(i) = hit {
                    state.borrow_mut().drag_candidate = None;
                    let cur_letter = state.borrow().sidebar_state.pinned.get(i).and_then(|p| p.custom_letter.clone()).unwrap_or_default();
                    state.borrow_mut().sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                        target: crate::sidebar::InlineEditTarget::PinnedLetter(i),
                        text: cur_letter.clone(),
                        cursor: cur_letter.len(),
                        selection: Some((0, cur_letter.len())),
                        caret_visible: true,
                    });
                    unsafe {
                        let _ = SetTimer(Some(hwnd), INLINE_CARET_TIMER_ID, 500, None);
                        invalidate_sidebar(hwnd, state);
                    }
                    return LRESULT(0);
                }
            }
            if hit != SidebarHit::None {
                match hit {
                    SidebarHit::Tab(i) => {
                        state.borrow_mut().drag_candidate = Some(crate::sidebar::DragCandidate {
                            source: crate::sidebar::DragSource::Tab(i),
                            start_x: x,
                            start_y: y,
                        });
                    }
                    SidebarHit::Pinned(i) => {
                        state.borrow_mut().drag_candidate = Some(crate::sidebar::DragCandidate {
                            source: crate::sidebar::DragSource::Pinned(i),
                            start_x: x,
                            start_y: y,
                        });
                    }
                    _ => {
                        state.borrow_mut().drag_candidate = None;
                    }
                }
                state.borrow_mut().pressed_hit = hit;
                let _ = SetCapture(hwnd);
                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if state.borrow().is_resizing_split {
                state.borrow_mut().is_resizing_split = false;
                let _ = ReleaseCapture();
                let sizes = state.borrow().split_info.as_ref().map(|sp| sp.sizes.clone()).unwrap_or_default();
                let on_action = state.borrow().on_action.clone();
                if let Some(ref cb) = on_action {
                    cb(SidebarAction::SplitSizesChanged(sizes));
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }

            if state.borrow().is_resizing_sidebar {
                state.borrow_mut().is_resizing_sidebar = false;
                let _ = ReleaseCapture();
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }

            let drag = {
                let mut b = state.borrow_mut();
                b.drag_candidate = None;
                if let Some(ref ov) = b.split_drop_overlay {
                    ov.hide();
                }
                b.sidebar_state.drag_state.take()
            };

            if let Some(drag) = drag {
                let _ = ReleaseCapture();
                unsafe { let _ = KillTimer(Some(hwnd), DRAG_HOVER_TIMER_ID); }
                state.borrow_mut().pressed_hit = SidebarHit::None;

                if let Some(target) = drag.target {
                    let dest = {
                        let b = state.borrow();
                        match target {
                            crate::sidebar::DragTarget::Favorites(slot) => {
                                Some(crate::sidebar::TabDropDestination::Favorites(slot))
                            }
                            crate::sidebar::DragTarget::Folder(f_idx) => {
                                b.sidebar_state.folders.get(f_idx).map(|f| crate::sidebar::TabDropDestination::Folder(f.id.clone()))
                            }
                            crate::sidebar::DragTarget::FolderInsert { folder_idx, insert_idx } => {
                                b.sidebar_state.folders.get(folder_idx).map(|f| {
                                    crate::sidebar::TabDropDestination::FolderInsert {
                                        folder_id: f.id.clone(),
                                        insert_idx,
                                    }
                                })
                            }
                            crate::sidebar::DragTarget::PinnedSection(slot) => {
                                Some(crate::sidebar::TabDropDestination::PinnedSection(slot))
                            }
                            crate::sidebar::DragTarget::NormalTabs(slot) => {
                                Some(crate::sidebar::TabDropDestination::NormalTabs(slot))
                            }
                            crate::sidebar::DragTarget::SplitContainer { split_id } => {
                                Some(crate::sidebar::TabDropDestination::SplitContainer { split_id })
                            }
                            crate::sidebar::DragTarget::WindowSplit { target_tab_id, on_left } => {
                                Some(crate::sidebar::TabDropDestination::WindowSplit { target_tab_id, on_left })
                            }
                        }
                    };

                    if let Some(destination) = dest {
                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref cb) = on_action {
                            cb(SidebarAction::DropTab {
                                source: drag.source,
                                destination,
                            });
                        }
                    }
                }

                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }

            let pressed = state.borrow().pressed_hit;
            if pressed != SidebarHit::None {
                let _ = ReleaseCapture();
                state.borrow_mut().pressed_hit = SidebarHit::None;
                let released = hit_at(hwnd, state, x, y);
                invalidate_sidebar(hwnd, state);

                if pressed == released {
                    match pressed {
                        SidebarHit::Caption(CaptionButton::Minimize) => {
                            let _ = ShowWindow(hwnd, SW_MINIMIZE);
                        }
                        SidebarHit::Caption(CaptionButton::Maximize) => {
                            if IsZoomed(hwnd).as_bool() {
                                let _ = ShowWindow(hwnd, SW_RESTORE);
                            } else {
                                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                            }
                        }
                        SidebarHit::Caption(CaptionButton::Close) => {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                        SidebarHit::ZoomReset => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ZoomReset);
                            }
                        }
                        SidebarHit::Shield => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleSiteShield);
                            }
                        }
                        SidebarHit::ToggleSidebar => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleSidebar);
                            }
                        }
                        SidebarHit::Back => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavBack);
                            }
                        }
                        SidebarHit::Forward => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavForward);
                            }
                        }
                        SidebarHit::Reload => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavReload);
                            }
                        }
                        SidebarHit::CleanLinkBtn => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::CopyCleanLinkActive);
                            }
                        }
                        SidebarHit::PermissionIcon(idx) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::AcceptPermission(idx));
                            }
                        }
                        SidebarHit::SiteSettings => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleSiteCard);
                            }
                        }
                        SidebarHit::UrlBar => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::UrlBarClicked);
                            }
                        }
                        SidebarHit::NewTab => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::NavNewTab);
                            }
                        }
                        SidebarHit::Tab(i) => {
                            let ctrl_pressed = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32) } < 0;
                            let shift_pressed = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(windows::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT.0 as i32) } < 0;

                            if ctrl_pressed {
                                let mut b = state.borrow_mut();
                                if b.sidebar_state.selected.is_empty() {
                                    if let Some(crate::sidebar::ActiveTab::Tab(act)) = b.sidebar_state.active {
                                        b.sidebar_state.selected.push(act);
                                    }
                                }
                                if let Some(pos) = b.sidebar_state.selected.iter().position(|&idx| idx == i) {
                                    b.sidebar_state.selected.remove(pos);
                                } else {
                                    b.sidebar_state.selected.push(i);
                                }
                                drop(b);
                                unsafe { invalidate_sidebar(hwnd, state); }
                            } else if shift_pressed {
                                let mut b = state.borrow_mut();
                                let anchor = b.sidebar_state.selected.first().copied()
                                    .or_else(|| if let Some(crate::sidebar::ActiveTab::Tab(act)) = b.sidebar_state.active { Some(act) } else { None })
                                    .unwrap_or(i);
                                let start = anchor.min(i);
                                let end = anchor.max(i);
                                b.sidebar_state.selected = (start..=end).collect();
                                drop(b);
                                unsafe { invalidate_sidebar(hwnd, state); }
                            } else {
                                state.borrow_mut().sidebar_state.selected.clear();
                                let on_action = state.borrow().on_action.clone();
                                if let Some(cb) = on_action {
                                    cb(SidebarAction::SelectTab(i));
                                }
                            }
                        }
                        SidebarHit::Downloads => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::OpenDownloads);
                            }
                        }
                        SidebarHit::Settings => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::OpenSettings);
                            }
                        }
                        SidebarHit::TabClose(i) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::CloseTab(i));
                            }
                        }
                        SidebarHit::Folder(i) | SidebarHit::FolderIcon(i) => {
                            {
                                let mut b = state.borrow_mut();
                                if let Some(folder) = b.sidebar_state.folders.get_mut(i) {
                                    folder.is_open = !folder.is_open;
                                }
                            }
                            unsafe {
                                let _ = SetTimer(Some(hwnd), TAB_ANIM_TIMER_ID, 16, None);
                                invalidate_sidebar(hwnd, state);
                            }
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::ToggleFolder(i));
                            }
                        }
                        SidebarHit::JumpToActiveTab => {
                            let target = state.borrow().sidebar.last_layout.borrow().as_ref().map(|l| l.jump_target_scroll).unwrap_or(0);
                            state.borrow_mut().sidebar_state.scroll = target;
                            unsafe {
                                invalidate_sidebar(hwnd, state);
                            }
                        }
                        SidebarHit::Pinned(i) => {
                            let on_action = state.borrow().on_action.clone();
                            if let Some(cb) = on_action {
                                cb(SidebarAction::SelectPinned(i));
                            }
                        }
                        _ => {}
                    }
                }
                return LRESULT(0);
            }
        }
        WM_RBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            if is_on_split_divider(hwnd, state, x, y) {
                let mut pt = POINT { x, y };
                unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                let items = vec![
                    crate::context_menu::ContextMenuItem::item(1, "Swap Left and Right Panes", Some("Ctrl+Alt+S")),
                    crate::context_menu::ContextMenuItem::item(2, "Even Out (50 / 50)", Some("Double-Click")),
                    crate::context_menu::ContextMenuItem::separator(),
                    crate::context_menu::ContextMenuItem::item(3, "Separate Split", Some("Ctrl+Alt+U")),
                    crate::context_menu::ContextMenuItem::item(4, "Close Split (Both Panes)", Some("Ctrl+Alt+W")),
                ];
                let on_action = state.borrow().on_action.clone();
                if let Some(ref menu) = state.borrow().context_menu {
                    menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                        if let Some(ref cb) = on_action {
                            match cmd {
                                1 => cb(SidebarAction::SwapSplit),
                                2 => cb(SidebarAction::EvenSplit),
                                3 => cb(SidebarAction::SeparateSplit),
                                4 => cb(SidebarAction::CloseSplit),
                                _ => {}
                            }
                        }
                    }));
                }
                return LRESULT(0);
            }

            let hit = hit_at(hwnd, state, x, y);
            let has_closed = state.borrow().sidebar_state.has_closed_tabs;

            match hit {
                SidebarHit::Tab(idx) => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }

                    let selected_tabs = {
                        let mut b = state.borrow_mut();
                        if !b.sidebar_state.selected.contains(&idx) {
                            b.sidebar_state.selected.clear();
                        }
                        b.sidebar_state.selected.clone()
                    };

                    let tab_opt = state.borrow().sidebar_state.tabs.get(idx).cloned();
                    if let Some(tab) = tab_opt {
                        let mute_label = if tab.is_muted { "Unmute Tab" } else { "Mute Tab" };
                        let unload_item = if tab.is_loaded {
                            crate::context_menu::ContextMenuItem::item(5, "Unload Tab", None)
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(5, "Unloaded (Sleeping)", None)
                        };
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };

                        let is_pin_section = tab.folder_id.as_deref() == Some(crate::sidebar::PINNED_SECTION_ID);
                        let pin_label = if is_pin_section { "Unpin Tab" } else { "Pin Tab" };

                        let mut items = vec![
                            crate::context_menu::ContextMenuItem::item(11, "New Tab Below", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(1, "Reload Tab", Some("Ctrl+R")),
                            crate::context_menu::ContextMenuItem::item(2, mute_label, None),
                            crate::context_menu::ContextMenuItem::item(3, "Favourite Tab", Some("Ctrl+Shift+X")),
                            crate::context_menu::ContextMenuItem::item(12, pin_label, None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(4, "Rename...", Some("F2")),
                        ];

                        if selected_tabs.len() >= 2 && selected_tabs.len() <= 4 && selected_tabs.contains(&idx) {
                            let count = selected_tabs.len();
                            let split_label = format!("Open {} Tabs in Split View", count);
                            items.insert(0, crate::context_menu::ContextMenuItem::item(24, &split_label, Some("Ctrl+Alt+N")));
                            items.insert(1, crate::context_menu::ContextMenuItem::separator());
                        }

                        let folders = state.borrow().sidebar_state.folders.clone();
                        if !folders.is_empty() {
                            items.push(crate::context_menu::ContextMenuItem::separator());
                            if tab.folder_id.is_some() && !is_pin_section {
                                items.push(crate::context_menu::ContextMenuItem::item(199, "Remove from Folder", None));
                            }
                            for (fi, f) in folders.iter().enumerate() {
                                items.push(crate::context_menu::ContextMenuItem::item(
                                    200 + fi as u32,
                                    &format!("Move to {}", f.name),
                                    None,
                                ));
                            }
                        }

                        items.extend(vec![
                            crate::context_menu::ContextMenuItem::separator(),
                            unload_item,
                            crate::context_menu::ContextMenuItem::item(6, "Duplicate Tab", None),
                            crate::context_menu::ContextMenuItem::item(7, "Share Tab (Copy Link)", None),
                            crate::context_menu::ContextMenuItem::item(15, "Copy Clean Link", Some("Ctrl+Shift+C")),
                            crate::context_menu::ContextMenuItem::item(16, "Toggle Ad & Tracker Shield", None),
                            crate::context_menu::ContextMenuItem::separator(),
                        ]);

                        if tab.is_split {
                            items.extend(vec![
                                crate::context_menu::ContextMenuItem::item(25, "Add New Tab to Split View", Some("Ctrl+Alt+N")),
                                crate::context_menu::ContextMenuItem::item(20, "Swap Split Panes", Some("Ctrl+Alt+S")),
                                crate::context_menu::ContextMenuItem::item(21, "Even Out Split (50 / 50)", Some("Ctrl+Alt+E")),
                                crate::context_menu::ContextMenuItem::item(22, "Separate Split", Some("Ctrl+Alt+U")),
                                crate::context_menu::ContextMenuItem::separator(),
                            ]);
                        } else {
                            items.extend(vec![
                                crate::context_menu::ContextMenuItem::item(23, "Open in Split View", Some("Ctrl+Alt+N")),
                                crate::context_menu::ContextMenuItem::separator(),
                            ]);
                        }

                        items.extend(vec![
                            crate::context_menu::ContextMenuItem::item(13, "Element Zapper", Some("Ctrl+Shift+H")),
                            crate::context_menu::ContextMenuItem::item(14, "Restore Hidden Elements", None),
                            crate::context_menu::ContextMenuItem::item(16, "Toggle Ad & Tracker Shield", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(8, "Close Tab", Some("Ctrl+W")),
                            crate::context_menu::ContextMenuItem::item(9, "Close Other Tabs", None),
                            reopen_item,
                        ]);

                        let on_action = state.borrow().on_action.clone();
                        let state_rc = state;
                        let folders_rc = folders.clone();
                        let tab_title_rename = tab.title.clone();
                        let tab_id_split = tab.id.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if cmd == 4 {
                                    // Inline rename directly on tab!
                                    {
                                        let mut b = state_rc.borrow_mut();
                                        b.sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                                            target: crate::sidebar::InlineEditTarget::Tab(idx),
                                            text: tab_title_rename.clone(),
                                            cursor: tab_title_rename.len(),
                                            selection: Some((0, tab_title_rename.len())),
                                            caret_visible: true,
                                        });
                                    }
                                    unsafe {
                                        let _ = SetTimer(Some(hwnd), INLINE_CARET_TIMER_ID, 500, None);
                                        invalidate_sidebar(hwnd, &state_rc);
                                    }
                                    return;
                                }
                                if cmd == 199 {
                                    if let Some(ref cb) = on_action {
                                        cb(SidebarAction::MoveTabToFolder(idx, None));
                                    }
                                    return;
                                }
                                if cmd >= 200 && (cmd - 200) < folders_rc.len() as u32 {
                                    let fi = (cmd - 200) as usize;
                                    let folder_id = folders_rc[fi].id.clone();
                                    if let Some(ref cb) = on_action {
                                        cb(SidebarAction::MoveTabToFolder(idx, Some(folder_id)));
                                    }
                                    return;
                                }
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::ReloadSpecificTab(idx)),
                                        2 => cb(SidebarAction::ToggleMuteTab(idx)),
                                        3 => cb(SidebarAction::PinTab(idx)),
                                        5 => cb(SidebarAction::UnloadTab(idx)),
                                        6 => cb(SidebarAction::DuplicateTab(idx)),
                                        7 => cb(SidebarAction::ShareTab(idx)),
                                        15 => cb(SidebarAction::CopyCleanLink(idx)),
                                        8 => cb(SidebarAction::CloseTab(idx)),
                                        9 => cb(SidebarAction::CloseOtherTabs(idx)),
                                        10 => cb(SidebarAction::ReopenClosedTab),
                                        11 => cb(SidebarAction::NewTabBelow(idx)),
                                        12 => cb(SidebarAction::MoveTabToFolder(
                                            idx,
                                            if is_pin_section { None } else { Some(crate::sidebar::PINNED_SECTION_ID.to_string()) },
                                        )),
                                        13 => cb(SidebarAction::ToggleZapper),
                                        14 => cb(SidebarAction::RestoreAllHidden),
                                        16 => cb(SidebarAction::ToggleSiteShield),
                                        20 => cb(SidebarAction::SwapSplit),
                                        21 => cb(SidebarAction::EvenSplit),
                                        22 => cb(SidebarAction::SeparateSplit),
                                        23 => cb(SidebarAction::OpenInSplitView(tab_id_split.clone())),
                                        24 => {
                                            let tab_ids: Vec<String> = {
                                                let b = state_rc.borrow();
                                                selected_tabs.iter().filter_map(|&i| b.sidebar_state.tabs.get(i).map(|t| t.id.clone())).collect()
                                            };
                                            cb(SidebarAction::SplitTabs(tab_ids));
                                        }
                                        25 => cb(SidebarAction::SplitWithNewTab),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                    }
                    return LRESULT(0);
                }
                SidebarHit::Folder(idx) | SidebarHit::FolderIcon(idx) => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let folder_opt = state.borrow().sidebar_state.folders.get(idx).cloned();
                    if let Some(folder) = folder_opt {
                        let folder_id_rename = folder.id.clone();
                        let folder_name_rename = folder.name.clone();
                        let folder_id_icon = folder.id.clone();
                        let folder_id_new_tab = folder.id.clone();
                        let folder_id_del = folder.id.clone();

                        let items = vec![
                            crate::context_menu::ContextMenuItem::item(1, "Rename Folder", Some("F2")),
                            crate::context_menu::ContextMenuItem::item(2, "Change Icon...", None),
                            crate::context_menu::ContextMenuItem::item(3, "New Tab in Folder", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(4, "Delete Folder", None),
                        ];

                        let on_action = state.borrow().on_action.clone();
                        let state_rc = state;
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                match cmd {
                                    1 => {
                                        // Inline rename
                                        {
                                            let mut b = state_rc.borrow_mut();
                                            b.sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                                                target: crate::sidebar::InlineEditTarget::Folder(folder_id_rename.clone()),
                                                text: folder_name_rename.clone(),
                                                cursor: folder_name_rename.len(),
                                                selection: Some((0, folder_name_rename.len())),
                                                caret_visible: true,
                                            });
                                        }
                                        unsafe {
                                            let _ = SetTimer(Some(hwnd), INLINE_CARET_TIMER_ID, 500, None);
                                            invalidate_sidebar(hwnd, &state_rc);
                                        }
                                    }
                                    2 => {
                                        // Change Icon
                                        let (folder_rect, sidebar_left) = {
                                            let b = state_rc.borrow();
                                            let frect = b.sidebar.last_layout.borrow().as_ref().and_then(|l| l.folders.get(idx).copied()).unwrap_or_default();
                                            let (_, srect) = compute_layout_rects(hwnd, &state_rc);
                                            (frect, srect.left)
                                        };
                                        let mut sidebar_screen_pt = POINT { x: sidebar_left, y: 0 };
                                        let mut folder_screen_pt = POINT { x: 0, y: folder_rect.top };
                                        unsafe {
                                            let _ = ClientToScreen(hwnd, &mut sidebar_screen_pt);
                                            let _ = ClientToScreen(hwnd, &mut folder_screen_pt);
                                        }
                                        if let Some(ref ip) = state_rc.borrow().icon_picker {
                                            let on_act = on_action.clone();
                                            let s = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
                                            let card_w = (252.0 * s).round() as i32;
                                            let target_x = sidebar_screen_pt.x - card_w - (8.0 * s).round() as i32;
                                            let target_y = if folder_rect.top > 0 { folder_screen_pt.y } else { pt.y - 40 };
                                            ip.show(target_x, target_y, &folder_id_icon, Box::new(move |fid, icon| {
                                                if let Some(ref cb) = on_act {
                                                    cb(SidebarAction::ChangeFolderIcon(fid, icon));
                                                }
                                            }));
                                        }
                                    }
                                    3 => {
                                        if let Some(ref cb) = on_action {
                                            cb(SidebarAction::NewTabInFolder(folder_id_new_tab.clone()));
                                        }
                                    }
                                    4 => {
                                        if let Some(ref cb) = on_action {
                                            cb(SidebarAction::DeleteFolder(folder_id_del.clone()));
                                        }
                                    }
                                    _ => {}
                                }
                            }));
                        }
                    }
                    return LRESULT(0);
                }
                SidebarHit::Pinned(idx) => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let pin_opt = state.borrow().sidebar_state.pinned.get(idx).cloned();
                    if let Some(pin) = pin_opt {
                        let mute_label = if pin.is_muted { "Unmute Tab" } else { "Mute Tab" };
                        let unload_item = if pin.is_loaded {
                            crate::context_menu::ContextMenuItem::item(5, "Unload Tab", None)
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(5, "Unloaded (Sleeping)", None)
                        };
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(10, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };

                        let has_custom_letter = pin.custom_letter.is_some();
                        let letter_label = if has_custom_letter { "Change Letter..." } else { "Assign Letter..." };

                        let mut items = vec![
                            crate::context_menu::ContextMenuItem::item(1, "Reload Tab", Some("Ctrl+R")),
                            crate::context_menu::ContextMenuItem::item(2, mute_label, None),
                            crate::context_menu::ContextMenuItem::item(3, "Unfavourite Tab", Some("Ctrl+Shift+X")),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(17, letter_label, None),
                        ];
                        if has_custom_letter {
                            items.push(crate::context_menu::ContextMenuItem::item(18, "Reset to Website Favicon", None));
                        }
                        items.extend(vec![
                            crate::context_menu::ContextMenuItem::item(4, "Change Label...", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            unload_item,
                            crate::context_menu::ContextMenuItem::item(6, "Duplicate Tab", None),
                            crate::context_menu::ContextMenuItem::item(7, "Share Tab (Copy Link)", None),
                            crate::context_menu::ContextMenuItem::item(15, "Copy Clean Link", Some("Ctrl+Shift+C")),
                            crate::context_menu::ContextMenuItem::item(16, "Toggle Ad & Tracker Shield", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            crate::context_menu::ContextMenuItem::item(8, "Close Tab", Some("Ctrl+W")),
                            reopen_item,
                        ]);

                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::ReloadSpecificPinnedTab(idx)),
                                        2 => cb(SidebarAction::ToggleMutePinnedTab(idx)),
                                        3 => cb(SidebarAction::UnpinTab(idx)),
                                        4 => cb(SidebarAction::PromptRenameTab(idx)),
                                        17 => {
                                            let cur_letter = state.borrow().sidebar_state.pinned.get(idx).and_then(|p| p.custom_letter.clone()).unwrap_or_default();
                                            state.borrow_mut().sidebar_state.inline_edit = Some(crate::sidebar::InlineEditState {
                                                target: crate::sidebar::InlineEditTarget::PinnedLetter(idx),
                                                text: cur_letter.clone(),
                                                cursor: cur_letter.len(),
                                                selection: Some((0, cur_letter.len())),
                                                caret_visible: true,
                                            });
                                            unsafe {
                                                let _ = SetTimer(Some(hwnd), INLINE_CARET_TIMER_ID, 500, None);
                                                invalidate_sidebar(hwnd, state);
                                            }
                                        }
                                        18 => cb(SidebarAction::SetPinnedLetter(idx, String::new())),
                                        5 => cb(SidebarAction::UnloadTab(idx)),
                                        6 => cb(SidebarAction::DuplicateTab(idx)),
                                        7 => cb(SidebarAction::SharePinnedTab(idx)),
                                        15 => cb(SidebarAction::CopyCleanPinnedTab(idx)),
                                        16 => cb(SidebarAction::ToggleSiteShield),
                                        8 => cb(SidebarAction::ClosePinnedTab(idx)),
                                        10 => cb(SidebarAction::ReopenClosedTab),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                    }
                    return LRESULT(0);
                }
                _ => {
                    let mut pt = POINT { x, y };
                    unsafe { let _ = ClientToScreen(hwnd, &mut pt); }
                    let r = sidebar_rect_of(hwnd, state);
                    if pt.x >= r.left && state.borrow().sidebar_visible {
                        let reopen_item = if has_closed {
                            crate::context_menu::ContextMenuItem::item(2, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        } else {
                            crate::context_menu::ContextMenuItem::item_disabled(2, "Reopen Closed Tab", Some("Ctrl+Shift+T"))
                        };
                        let items = vec![
                            crate::context_menu::ContextMenuItem::item(1, "New Tab", Some("Ctrl+T")),
                            crate::context_menu::ContextMenuItem::item(3, "New Folder", None),
                            crate::context_menu::ContextMenuItem::separator(),
                            reopen_item,
                        ];
                        let on_action = state.borrow().on_action.clone();
                        if let Some(ref menu) = state.borrow().context_menu {
                            menu.show(pt.x, pt.y, items, Box::new(move |cmd| {
                                if let Some(ref cb) = on_action {
                                    match cmd {
                                        1 => cb(SidebarAction::NavNewTab),
                                        2 => cb(SidebarAction::ReopenClosedTab),
                                        3 => cb(SidebarAction::CreateFolder),
                                        _ => {}
                                    }
                                }
                            }));
                        }
                        return LRESULT(0);
                    }
                }
            }
            return LRESULT(0);
        }
        WM_INITMENUPOPUP => {
            unsafe {
                if let Ok(menu_hwnd) = FindWindowW(w!("#32768"), windows::core::PCWSTR::null()) {
                    if menu_hwnd.0 != std::ptr::null_mut() {
                        crate::frame::allow_dark_mode_for_window(menu_hwnd);
                    }
                }
            }
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as i32;
            let mut pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let r = sidebar_rect_of(hwnd, state);
            if pt.x >= r.left && state.borrow().sidebar_visible {
                let dpi = GetDpiForWindow(hwnd);
                let step = (48.0 * dpi as f32 / 96.0) as i32;
                {
                    let mut b = state.borrow_mut();
                    let cur = b.sidebar_state.scroll;
                    b.sidebar_state.scroll = cur - delta * step / 120;
                }
                clamp_scroll(hwnd, state);
                invalidate_sidebar(hwnd, state);
                return LRESULT(0);
            }
        }
        WM_SIZE => {
            let (content_rect, sidebar_rect) = compute_layout_rects(hwnd, state);

            let on_resize = state.borrow().on_resize.clone();
            if let Some(cb) = on_resize {
                cb(content_rect, sidebar_rect);
            }

            clamp_scroll(hwnd, state);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let suggested_rect = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested_rect.left,
                suggested_rect.top,
                suggested_rect.right - suggested_rect.left,
                suggested_rect.bottom - suggested_rect.top,
                SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            if (wparam.0 as i32) == 0x1B {
                let was_drag = {
                    let mut b = state.borrow_mut();
                    b.drag_candidate = None;
                    if let Some(ref ov) = b.split_drop_overlay {
                        ov.hide();
                    }
                    b.sidebar_state.drag_state.take().is_some()
                };
                if was_drag {
                    unsafe {
                        let _ = ReleaseCapture();
                        let _ = KillTimer(Some(hwnd), DRAG_HOVER_TIMER_ID);
                    }
                    state.borrow_mut().pressed_hit = SidebarHit::None;
                    unsafe { invalidate_sidebar(hwnd, state); }
                    return LRESULT(0);
                }
            }
            let has_edit = state.borrow().sidebar_state.inline_edit.is_some();
            if has_edit {
                match wparam.0 as i32 {
                    0x0D => {
                        // VK_RETURN -> Commit rename
                        let edit_opt = state.borrow_mut().sidebar_state.inline_edit.take();
                        if let Some(ed) = edit_opt {
                            let trimmed = ed.text.trim().to_string();
                            let on_action = state.borrow().on_action.clone();
                            if let Some(ref cb) = on_action {
                                match ed.target {
                                    crate::sidebar::InlineEditTarget::Folder(fid) => {
                                        if !trimmed.is_empty() {
                                            cb(SidebarAction::RenameFolder(fid, trimmed));
                                        }
                                    }
                                    crate::sidebar::InlineEditTarget::Tab(idx) => {
                                        if !trimmed.is_empty() {
                                            cb(SidebarAction::RenameTab(idx, trimmed));
                                        }
                                    }
                                    crate::sidebar::InlineEditTarget::PinnedLetter(idx) => {
                                        cb(SidebarAction::SetPinnedLetter(idx, trimmed));
                                    }
                                }
                            }
                            unsafe {
                                let _ = KillTimer(Some(hwnd), INLINE_CARET_TIMER_ID);
                                invalidate_sidebar(hwnd, state);
                            }
                        }
                        return LRESULT(0);
                    }
                    0x1B => {
                        // VK_ESCAPE -> Cancel rename
                        state.borrow_mut().sidebar_state.inline_edit = None;
                        unsafe {
                            let _ = KillTimer(Some(hwnd), INLINE_CARET_TIMER_ID);
                            invalidate_sidebar(hwnd, state);
                        }
                        return LRESULT(0);
                    }
                    0x08 => {
                        // VK_BACK
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            if let Some((s_idx, e_idx)) = ed.selection.take() {
                                let (min_i, max_i) = if s_idx <= e_idx { (s_idx, e_idx) } else { (e_idx, s_idx) };
                                let min_i = min_i.min(ed.text.len());
                                let max_i = max_i.min(ed.text.len());
                                ed.text.drain(min_i..max_i);
                                ed.cursor = min_i;
                            } else if ed.cursor > 0 {
                                let mut prev = ed.cursor - 1;
                                while prev > 0 && !ed.text.is_char_boundary(prev) {
                                    prev -= 1;
                                }
                                ed.text.drain(prev..ed.cursor);
                                ed.cursor = prev;
                            }
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x2E => {
                        // VK_DELETE
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            if let Some((s_idx, e_idx)) = ed.selection.take() {
                                let (min_i, max_i) = if s_idx <= e_idx { (s_idx, e_idx) } else { (e_idx, s_idx) };
                                let min_i = min_i.min(ed.text.len());
                                let max_i = max_i.min(ed.text.len());
                                ed.text.drain(min_i..max_i);
                                ed.cursor = min_i;
                            } else if ed.cursor < ed.text.len() {
                                let mut next = ed.cursor + 1;
                                while next < ed.text.len() && !ed.text.is_char_boundary(next) {
                                    next += 1;
                                }
                                ed.text.drain(ed.cursor..next);
                            }
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x25 => {
                        // VK_LEFT
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            ed.selection = None;
                            if ed.cursor > 0 {
                                let mut prev = ed.cursor - 1;
                                while prev > 0 && !ed.text.is_char_boundary(prev) {
                                    prev -= 1;
                                }
                                ed.cursor = prev;
                            }
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x27 => {
                        // VK_RIGHT
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            ed.selection = None;
                            if ed.cursor < ed.text.len() {
                                let mut next = ed.cursor + 1;
                                while next < ed.text.len() && !ed.text.is_char_boundary(next) {
                                    next += 1;
                                }
                                ed.cursor = next;
                            }
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x24 => {
                        // VK_HOME
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            ed.selection = None;
                            ed.cursor = 0;
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x23 => {
                        // VK_END
                        let mut b = state.borrow_mut();
                        if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                            ed.selection = None;
                            ed.cursor = ed.text.len();
                            ed.caret_visible = true;
                        }
                        unsafe { invalidate_sidebar(hwnd, state); }
                        return LRESULT(0);
                    }
                    0x41 => {
                        // 'A' with Ctrl
                        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
                        let ctrl = (unsafe { GetKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0;
                        if ctrl {
                            let mut b = state.borrow_mut();
                            if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                                ed.selection = Some((0, ed.text.len()));
                                ed.cursor = ed.text.len();
                                ed.caret_visible = true;
                            }
                            unsafe { invalidate_sidebar(hwnd, state); }
                            return LRESULT(0);
                        }
                    }
                    _ => {}
                }
            }

            use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetAsyncKeyState, VK_CONTROL, VK_SHIFT, VK_MENU};
            let ctrl = (unsafe { GetKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0;
            let shift = (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0;
            let alt = (unsafe { GetKeyState(VK_MENU.0 as i32) } as u16 & 0x8000) != 0
                || (unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as u16 & 0x8000) != 0;

            let on_action = state.borrow().on_action.clone();
            let vk = wparam.0 as u32;

            if ctrl && alt {
                match vk {
                    0x4E => { // 'N' -> Split View (Ctrl+Alt+N)
                        let selected = state.borrow().sidebar_state.selected.clone();
                        if selected.len() >= 2 && selected.len() <= 4 {
                            let tab_ids: Vec<String> = {
                                let b = state.borrow();
                                selected.iter().filter_map(|&i| b.sidebar_state.tabs.get(i).map(|t| t.id.clone())).collect()
                            };
                            if tab_ids.len() >= 2 {
                                state.borrow_mut().sidebar_state.selected.clear();
                                unsafe { invalidate_sidebar(hwnd, state); }
                                if let Some(ref cb) = on_action { cb(SidebarAction::SplitTabs(tab_ids)); }
                                return LRESULT(0);
                            }
                        }
                        if let Some(ref cb) = on_action { cb(SidebarAction::SplitWithNewTab); }
                        return LRESULT(0);
                    }
                    0x25 | 0x27 | 0xDC => { // Left/Right Arrow or '\' -> Focus Other Pane
                        if let Some(ref cb) = on_action { cb(SidebarAction::FocusOtherPane); }
                        return LRESULT(0);
                    }
                    0x53 => { // 'S' -> Swap Split (Ctrl+Alt+S)
                        if let Some(ref cb) = on_action { cb(SidebarAction::SwapSplit); }
                        return LRESULT(0);
                    }
                    0x45 => { // 'E' -> Even Split (Ctrl+Alt+E)
                        if let Some(ref cb) = on_action { cb(SidebarAction::EvenSplit); }
                        return LRESULT(0);
                    }
                    0x55 => { // 'U' -> Separate Split (Ctrl+Alt+U)
                        if let Some(ref cb) = on_action { cb(SidebarAction::SeparateSplit); }
                        return LRESULT(0);
                    }
                    0x57 => { // 'W' -> Close Split (Ctrl+Alt+W)
                        if let Some(ref cb) = on_action { cb(SidebarAction::CloseSplit); }
                        return LRESULT(0);
                    }
                    _ => {}
                }
            } else if alt {
                if vk == 0x25 { // Left arrow -> Back
                    if let Some(ref cb) = on_action { cb(SidebarAction::NavBack); }
                    return LRESULT(0);
                } else if vk == 0x27 { // Right arrow -> Forward
                    if let Some(ref cb) = on_action { cb(SidebarAction::NavForward); }
                    return LRESULT(0);
                }
            }

            if ctrl {
                if shift {
                    match vk {
                        0x43 => { // 'C' -> Copy Clean Link (Ctrl+Shift+C)
                            if let Some(ref cb) = on_action { cb(SidebarAction::CopyCleanLinkActive); }
                            return LRESULT(0);
                        }
                        0x58 => { // 'X' -> Pin Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::TogglePinCurrentTab); }
                            return LRESULT(0);
                        }
                        0x52 => { // 'R' -> Hard Reload
                            if let Some(ref cb) = on_action { cb(SidebarAction::HardReload); }
                            return LRESULT(0);
                        }
                        0x09 => { // Tab -> Prev Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::PrevTab); }
                            return LRESULT(0);
                        }
                        0x54 => { // 'T' -> Reopen Closed Tab!
                            if let Some(ref cb) = on_action { cb(SidebarAction::ReopenClosedTab); }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                } else {
                    match vk {
                        0x54 => { // 'T' -> New Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::NavNewTab); }
                            return LRESULT(0);
                        }
                        0x57 => { // 'W' -> Close Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::CloseTab(usize::MAX)); }
                            return LRESULT(0);
                        }
                        0x53 | 0x42 => { // 'S' or 'B' -> Toggle Sidebar
                            if let Some(ref cb) = on_action { cb(SidebarAction::ToggleSidebar); }
                            return LRESULT(0);
                        }
                        0x52 => { // 'R' -> Reload
                            if let Some(ref cb) = on_action { cb(SidebarAction::NavReload); }
                            return LRESULT(0);
                        }
                        0x4C => { // 'L' -> Focus URL Bar
                            if let Some(ref cb) = on_action { cb(SidebarAction::UrlBarClicked); }
                            return LRESULT(0);
                        }
                        0xBC => { // ',' -> Settings
                            if let Some(ref cb) = on_action { cb(SidebarAction::OpenSettings); }
                            return LRESULT(0);
                        }
                        0xBB | 0x6B => { // '+' or '=' -> Zoom In
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomIn); }
                            return LRESULT(0);
                        }
                        0xBD | 0x6D => { // '-' -> Zoom Out
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomOut); }
                            return LRESULT(0);
                        }
                        0x30 | 0x60 => { // '0' -> Reset Zoom
                            if let Some(ref cb) = on_action { cb(SidebarAction::ZoomReset); }
                            return LRESULT(0);
                        }
                        0x44 => { // 'D' -> Pin Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::TogglePinCurrentTab); }
                            return LRESULT(0);
                        }
                        0x09 => { // Tab -> Next Tab
                            if let Some(ref cb) = on_action { cb(SidebarAction::NextTab); }
                            return LRESULT(0);
                        }
                        0x31..=0x39 => { // '1'..'9' -> Select Tab
                            let idx = (vk - 0x31) as usize;
                            if let Some(ref cb) = on_action { cb(SidebarAction::SelectTab(idx)); }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
            } else if vk == 0x74 { // F5 -> Reload
                if let Some(ref cb) = on_action { cb(SidebarAction::NavReload); }
                return LRESULT(0);
            }
        }
        WM_CHAR => {
            let ch = wparam.0 as u16;
            if ch >= 32 {
                if let Some(c) = char::from_u32(ch as u32) {
                    let mut b = state.borrow_mut();
                    if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                        if matches!(ed.target, crate::sidebar::InlineEditTarget::PinnedLetter(_)) {
                            let upper = c.to_uppercase().to_string();
                            ed.text = upper.chars().take(1).collect();
                            ed.cursor = ed.text.len();
                            ed.selection = None;
                            ed.caret_visible = true;
                        } else {
                            if let Some((s_idx, e_idx)) = ed.selection.take() {
                                let (min_i, max_i) = if s_idx <= e_idx { (s_idx, e_idx) } else { (e_idx, s_idx) };
                                let min_i = min_i.min(ed.text.len());
                                let max_i = max_i.min(ed.text.len());
                                ed.text.drain(min_i..max_i);
                                ed.cursor = min_i;
                            }
                            ed.text.insert(ed.cursor, c);
                            ed.cursor += c.len_utf8();
                            ed.caret_visible = true;
                        }
                    }
                    drop(b);
                    unsafe {
                        invalidate_sidebar(hwnd, state);
                    }
                    return LRESULT(0);
                }
            }
        }
        WM_TIMER if wparam.0 == INLINE_CARET_TIMER_ID => {
            let has_edit = {
                let mut b = state.borrow_mut();
                if let Some(ref mut ed) = b.sidebar_state.inline_edit {
                    ed.caret_visible = !ed.caret_visible;
                    true
                } else {
                    false
                }
            };
            if has_edit {
                unsafe {
                    invalidate_sidebar(hwnd, state);
                }
            } else {
                unsafe {
                    let _ = KillTimer(Some(hwnd), INLINE_CARET_TIMER_ID);
                }
            }
            return LRESULT(0);
        }
        msg if msg >= WM_APP => {
            let handler = state.borrow().on_custom_msg.clone();
            if let Some(handler) = handler {
                handler(msg, wparam, lparam);
            }
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == DRAG_HOVER_TIMER_ID => {
            let folder_to_open = {
                let mut b = state.borrow_mut();
                let SidebarState { ref mut drag_state, ref folders, .. } = b.sidebar_state;
                if let Some(ref mut ds) = drag_state {
                    if let Some(crate::sidebar::DragTarget::Folder(f_idx)) = ds.target {
                        let is_closed = folders.get(f_idx).map_or(false, |f| !f.is_open);
                        if is_closed {
                            if let Some((idx, start)) = ds.folder_hover_start {
                                if idx == f_idx && start.elapsed() >= std::time::Duration::from_millis(200) {
                                    ds.folder_hover_start = None;
                                    Some(f_idx)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            ds.folder_hover_start = None;
                            None
                        }
                    } else {
                        ds.folder_hover_start = None;
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(f_idx) = folder_to_open {
                unsafe { let _ = KillTimer(Some(hwnd), DRAG_HOVER_TIMER_ID); }
                let on_action = state.borrow().on_action.clone();
                if let Some(ref cb) = on_action {
                    cb(SidebarAction::OpenFolder(f_idx));
                }
            }
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == 0x4201 => {
            unsafe {
                let _ = KillTimer(Some(hwnd), 0x4201);
                state.borrow_mut().sidebar_state.toast = None;
                invalidate_sidebar(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TAB_ANIM_TIMER_ID => {
            let on_action = state.borrow().on_action.clone();
            if let Some(cb) = on_action {
                cb(SidebarAction::TabAnimTick);
            }
            return LRESULT(0);
        }
        WM_CLOSE => {
            let handler = state.borrow().on_custom_msg.clone();
            if let Some(handler) = handler {
                handler(WM_CLOSE, wparam, lparam);
            }
            let _ = DestroyWindow(hwnd);
            return LRESULT(0);
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_rects_50_50() {
        let content_rect = RECT { left: 100, top: 0, right: 1100, bottom: 800 };
        let (left, div, right) = calculate_split_rects(content_rect, 0.5, 1.0).expect("layout should succeed");
        // total_w = 1000, gutter = 7. avail_w = 993.
        // left_w = round(993 * 0.5) = 497.
        // left: 100 .. 597
        // div: 597 .. 604 (width 7)
        // right: 604 .. 1100 (width 496)
        assert_eq!(left.left, 100);
        assert_eq!(left.right, 597);
        assert_eq!(div.left, 597);
        assert_eq!(div.right, 604);
        assert_eq!(div.right - div.left, 7);
        assert_eq!(right.left, 604);
        assert_eq!(right.right, 1100);
        assert_eq!(left.top, 0);
        assert_eq!(left.bottom, 800);
    }

    #[test]
    fn test_split_rects_ratio_clamping() {
        let content_rect = RECT { left: 0, top: 0, right: 1000, bottom: 600 };
        // Extreme left (< 0.20) should clamp to 0.20
        let (left_min, _, _) = calculate_split_rects(content_rect, 0.05, 1.0).unwrap();
        // avail_w = 993, 993 * 0.20 = 199
        assert_eq!(left_min.right - left_min.left, 199);

        // Extreme right (> 0.80) should clamp to 0.80
        let (left_max, _, _) = calculate_split_rects(content_rect, 0.95, 1.0).unwrap();
        // avail_w = 993, 993 * 0.80 = 794
        assert_eq!(left_max.right - left_max.left, 794);
    }

    #[test]
    fn test_snap_split_ratio() {
        let content_left = 100;
        let avail_w = 1000;
        let scale = 1.0;
        // center is 100 + 500 = 600.
        // snap range is within 15px: 585 .. 615

        // Within snap zone -> 0.50
        assert_eq!(snap_split_ratio(605, content_left, avail_w, scale), 0.50);
        assert_eq!(snap_split_ratio(590, content_left, avail_w, scale), 0.50);

        // Outside snap zone -> proportional ratio
        let r1 = snap_split_ratio(400, content_left, avail_w, scale);
        assert!((r1 - 0.30).abs() < 0.01);

        // Beyond clamp bounds -> clamped
        assert_eq!(snap_split_ratio(150, content_left, avail_w, scale), 0.20);
        assert_eq!(snap_split_ratio(950, content_left, avail_w, scale), 0.80);
    }

    #[test]
    fn test_split_rects_multi_3_panes() {
        let content_rect = RECT { left: 0, top: 0, right: 1200, bottom: 800 };
        let (panes, divs) = calculate_split_rects_multi(content_rect, &[1.0, 1.0, 1.0], 1.0).expect("3 panes");
        assert_eq!(panes.len(), 3);
        assert_eq!(divs.len(), 2);
        assert_eq!(panes[0].left, 0);
        assert_eq!(panes[2].right, 1200);
        assert_eq!(divs[0].left, panes[0].right);
        assert_eq!(divs[0].right, panes[1].left);
        assert_eq!(divs[1].left, panes[1].right);
        assert_eq!(divs[1].right, panes[2].left);
    }

    #[test]
    fn test_split_rects_multi_4_panes() {
        let content_rect = RECT { left: 0, top: 0, right: 1600, bottom: 900 };
        let (panes, divs) = calculate_split_rects_multi(content_rect, &[0.25, 0.25, 0.25, 0.25], 1.0).expect("4 panes");
        assert_eq!(panes.len(), 4);
        assert_eq!(divs.len(), 3);
        assert_eq!(panes[0].left, 0);
        assert_eq!(panes[3].right, 1600);
    }
}