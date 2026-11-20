# Arc-Style Browser for Windows: Build Plan

A 7-part plan for building a small, fast, open-source, no-AI, Arc-style browser for Windows using Rust and WebView2. Each plan is sized to hand to T3 Code as its own spec.

## Core decisions (locked before Plan 1)

- **Engine:** WebView2, one controller per tab, all inside one shared environment.
- **Shell:** Rust with the `windows` crate and `webview2-com`, using a custom-drawn frame.
- **Sidebar rendering:** Direct2D and DirectWrite in a child HWND. Raw Win32 controls are painful for a custom Arc-style list, so draw it yourself.
- **Spaces:** use WebView2's `Profile` API (one profile per space) rather than separate user-data folders, which spawn extra browser processes.
- **Storage:** SQLite (`rusqlite`) for tabs, pins, folders, history and settings.
- **Floating command bar:** an owned popup window, not a child HWND, so it layers cleanly over the webview.
- **Shortcuts:** intercept them via WebView2's `AcceleratorKeyPressed`, so they fire even when a page has focus.

Note: if a Rune (GPU-rendered Rust UI framework) spike turns out clean, Rune could be a nicer way to build the UI. The Win32 route is simply the safest for embedding WebView2, so this plan is built around it.

## Plan 1: Foundation (\~15%)

- Cargo workspace layout: `app`, `ui`, `engine`, `storage`.
- Borderless window with a custom frame (`WM_NCCALCSIZE`, `WM_NCHITTEST`), DWM shadow and Windows 11 rounded corners.
- WebView2 environment and one webview filling the content area.
- A right-side sidebar region (empty, solid color).
- Startup timing harness. Measure cold start now so you can protect it later.

**Done when:** the window opens fast, loads a page, resizes without flicker, and cold-start time is logged.

## Plan 2: Tabs and Sidebar Core (\~20%)

- Tab model (id, URL, title, favicon, state) and one WebView2 controller per tab, with only the active one visible.
- Sidebar tab list with hover, active and close states.
- Window controls (minimize, maximize, close) drawn in the sidebar, with Win11 snap-layout support on the maximize button.
- Sidebar toggle: when hidden, controls hide too. You'll need a fallback so the window isn't uncloseable. Suggested: a thin hover strip on the right edge that reveals the sidebar, plus a draggable top edge.
- Back, forward and reload, title and favicon events, new-window handling, lazy tab loading.

**Done when:** you can open, switch and close tabs and move or close the window from the sidebar.

## Plan 3: Command Bar and Shortcuts (\~15%)

- Floating command bar (Ctrl+T): URL vs search detection, default search engine, suggestions from history and open tabs.
- Shortcut system: a central registry, with Ctrl+S (sidebar), Ctrl+T, Ctrl+W, Ctrl+H, Ctrl+L, Ctrl+R, Ctrl+Tab, Ctrl+Shift+T (reopen closed tab) and Ctrl+F.
- Find in page (WebView2 has a find API).
- Everything rebindable from day one. Ctrl+S will clash with sites like Google Docs, so make it easy to change.

**Done when:** you can browse using only the keyboard.

## Plan 4: The Arc Features (\~20%)

- Pinned sites: a grid at the top of the sidebar, persistent, with a favicon per pin and a "reset to pinned URL" action.
- Folders: nesting, expand and collapse, and drag-and-drop for tabs and pins.
- Spaces: each has a name, color and its own profile (cookies and logins), plus switching with a dropdown and shortcuts.
- Persistence through SQLite.

**Done when:** pins, folders and spaces survive a restart and each space keeps its own logins.

## Plan 5: Browser Essentials (\~15%)

- History: storage, a page opened by Ctrl+H, search and delete.
- Downloads: a download manager UI and tracking.
- Permissions prompts, such as camera and location.
- Session restore, tab suspension for memory, and tab search.
- Context menus (link, image, page).

**Done when:** you could use it as your daily browser for a week.

## Plan 6: Settings and Polish (\~10%)

- Settings page: search engine, startup behavior, themes (light, dark, per-space accent), shortcut rebinding, clear data, default-browser registration.
- Mini-player or picture-in-picture, if WebView2 supports it for your needs.
- Accessibility and DPI checks, plus keyboard navigation inside the sidebar.

## Plan 7: Ship It (\~5%)

- Installer (MSI or MSIX) and a WebView2 runtime check, an auto-updater, and crash logging.
- Binary size and startup tuning (`lto`, `opt-level="z"`, `panic=abort`, and profiling the init path).
- CI, README, contribution guide, license, and a release workflow for the open-source launch.

## Known Risks

- **Extensions:** WebView2's extension support is limited. Check Microsoft's current docs before promising any.
- **Custom frame edge cases:** multi-monitor, DPI changes, snap layouts and dragging. Budget extra time in Plan 2.
- **Memory:** many WebView2 controllers add up, which is why tab suspension is in Plan 5.

## Working with T3 Code

For each plan, write a spec with scope, out-of-scope, files to touch, and acceptance criteria, then run them in order so no plan builds on unfinished work.
