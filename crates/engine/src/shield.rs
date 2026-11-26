//! Native Ad & Tracker Shield Engine (F06)
//!
//! Directly ported and enhanced from macOS Search's `Shield.swift` and `YouTubeAds.swift`.
//! Provides zero-latency network request blocking for tracking beacons and ad networks,
//! cosmetic stylesheet injection for ad slot collapse, and dedicated YouTube player
//! ad schedule pruning and 16x fast-forward fallback.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Third-party domains whose primary function is tracking, behavioral surveillance, or ads.
/// Directly mirrors and extends `Shield.unwanted` from macOS Search.
pub static UNWANTED_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "googletagservices.com",
    "google-analytics.com",
    "googletagmanager.com",
    "adservice.google.com",
    "amazon-adsystem.com",
    "adnxs.com",
    "adsrvr.org",
    "criteo.com",
    "criteo.net",
    "taboola.com",
    "outbrain.com",
    "rubiconproject.com",
    "pubmatic.com",
    "openx.net",
    "casalemedia.com",
    "smartadserver.com",
    "sharethrough.com",
    "indexww.com",
    "bidswitch.net",
    "33across.com",
    "teads.tv",
    "moatads.com",
    "adroll.com",
    "scorecardresearch.com",
    "quantserve.com",
    "chartbeat.com",
    "hotjar.com",
    "mouseflow.com",
    "fullstory.com",
    "clarity.ms",
    "mixpanel.com",
    "amplitude.com",
    "segment.com",
    "segment.io",
    "branch.io",
    "appsflyer.com",
    "adjust.com",
    "analytics.tiktok.com",
    "connect.facebook.net",
    "ads-twitter.com",
    "analytics.twitter.com",
];

/// YouTube hosts where the player and ad scripts operate.
pub static YOUTUBE_HOSTS: &[&str] = &[
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];

/// Known YouTube ad endpoints and reporting paths from `YouTubeAds.swift`.
pub static YOUTUBE_AD_PATHS: &[&str] = &[
    "/api/stats/ads",
    "/pagead/",
    "/ptracking",
    "/get_midroll_info",
    "/youtubei/v1/player/ad_break",
    "/pcs/activeview",
];

/// Extracts the hostname from an HTTP/HTTPS URL string in lowercase, without port or user info.
pub fn extract_host(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let without_scheme = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return None;
    };

    let authority = without_scheme.split('/').next()?.split('?').next()?.split('#').next()?;
    let host_part = if let Some((_, h)) = authority.split_once('@') {
        h
    } else {
        authority
    };

    let host = if let Some((h, _port)) = host_part.split_once(':') {
        h
    } else {
        host_part
    };

    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Checks whether `candidate_host` matches or is a subdomain of `target_domain`.
pub fn host_matches_domain(candidate_host: &str, target_domain: &str) -> bool {
    if candidate_host == target_domain {
        return true;
    }
    if candidate_host.ends_with(target_domain) {
        let prefix_len = candidate_host.len() - target_domain.len();
        if prefix_len > 0 && candidate_host.as_bytes()[prefix_len - 1] == b'.' {
            return true;
        }
    }
    false
}

/// Checks whether the given host is an unwanted tracking or ad domain.
pub fn is_unwanted_host(host: &str) -> bool {
    for &unwanted in UNWANTED_DOMAINS {
        if host_matches_domain(host, unwanted) {
            return true;
        }
    }
    false
}

/// Checks whether the URL is a YouTube ad telemetry or segment request.
pub fn is_youtube_ad_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    for &path in YOUTUBE_AD_PATHS {
        if lower.contains(path) {
            return true;
        }
    }
    if lower.contains("/api/stats/qoe?") && lower.contains("adformat=") {
        return true;
    }
    if lower.contains("imasdk.googleapis.com") {
        return true;
    }
    false
}

/// Determines whether a request to `request_url` from a page on `page_host` is third-party.
pub fn is_third_party(request_host: &str, page_host: &str) -> bool {
    if page_host.is_empty() {
        return true;
    }
    let p_clean = page_host.trim_start_matches("www.");
    let r_clean = request_host.trim_start_matches("www.");
    if p_clean == r_clean || r_clean.ends_with(&format!(".{p_clean}")) {
        return false;
    }
    true
}

/// Shield decision engine: tests whether an outgoing web resource request should be blocked.
pub fn should_block_request(
    request_url: &str,
    page_host: &str,
    shield_enabled: bool,
    is_paused_on_page: bool,
) -> bool {
    if !shield_enabled || is_paused_on_page {
        return false;
    }

    let req_host = match extract_host(request_url) {
        Some(h) => h,
        None => return false,
    };

    // YouTube ad endpoints blocked when navigating on or from YouTube
    let is_on_yt = YOUTUBE_HOSTS.iter().any(|&h| host_matches_domain(page_host, h));
    if is_on_yt && is_youtube_ad_url(request_url) {
        return true;
    }

    // Unwanted domains are blocked if they are third-party (or ad networks)
    if is_unwanted_host(&req_host) {
        if is_third_party(&req_host, page_host) {
            return true;
        }
        // Even if embedded, primary ad networks like doubleclick or googlesyndication are blocked
        if req_host.contains("doubleclick.net")
            || req_host.contains("googlesyndication.com")
            || req_host.contains("adservice.google.com")
            || req_host.contains("amazon-adsystem.com")
            || req_host.contains("criteo.")
            || req_host.contains("taboola.com")
            || req_host.contains("outbrain.com")
        {
            return true;
        }
    }

    false
}

/// Shared Shield State for a webview tab instance.
#[derive(Debug, Clone)]
pub struct ShieldState {
    pub enabled: Arc<AtomicBool>,
    pub paused_hosts: Arc<RwLock<HashSet<String>>>,
    pub blocked_count: Arc<AtomicU64>,
}

impl Default for ShieldState {
    fn default() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            paused_hosts: Arc::new(RwLock::new(HashSet::new())),
            blocked_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl ShieldState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_shared(
        enabled: Arc<AtomicBool>,
        paused_hosts: Arc<RwLock<HashSet<String>>>,
    ) -> Self {
        Self {
            enabled,
            paused_hosts,
            blocked_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, val: bool) {
        self.enabled.store(val, Ordering::Relaxed);
    }

    pub fn is_paused(&self, host: &str) -> bool {
        if host.is_empty() {
            return false;
        }
        let clean = host.trim_start_matches("www.").to_ascii_lowercase();
        if let Ok(guard) = self.paused_hosts.read() {
            if guard.contains(&clean) || guard.contains(host) {
                return true;
            }
            for paused in guard.iter() {
                if host_matches_domain(&clean, paused) {
                    return true;
                }
            }
            false
        } else {
            false
        }
    }

    pub fn set_paused(&self, host: &str, paused: bool) {
        let clean = host.trim_start_matches("www.").to_ascii_lowercase();
        if clean.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.paused_hosts.write() {
            if paused {
                guard.insert(clean);
            } else {
                guard.remove(&clean);
            }
        }
    }

    pub fn increment_blocked(&self) -> u64 {
        self.blocked_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn get_blocked_count(&self) -> u64 {
        self.blocked_count.load(Ordering::Relaxed)
    }

    pub fn reset_blocked(&self) {
        self.blocked_count.store(0, Ordering::Relaxed);
    }
}

/// YouTube Ad Stripper & 16x Fast-Forward Fallback Script.
/// Ported directly from `Search/Sources/Search/YouTubeAds.swift`.
pub const YOUTUBE_AD_STRIPPER_SCRIPT: &str = r#"
(() => {
    'use strict';
    if (window !== window.top) return;
    const hosts = [
        "youtube.com", "www.youtube.com", "m.youtube.com",
        "music.youtube.com", "youtube-nocookie.com", "www.youtube-nocookie.com"
    ];
    if (!hosts.some(h => location.hostname === h || location.hostname.endsWith('.' + h))) return;

    function isShieldDisabled() {
        try {
            if (window.__BROWSER_SHIELD_PAUSED__ === true) return true;
            if (window.__BROWSER_SHIELD_ENABLED__ === false) return true;
            if (localStorage.getItem('__browser_shield_paused') === 'true') return true;
            if (localStorage.getItem('__browser_shield_global_enabled') === 'false') return true;
        } catch(e) {}
        return false;
    }

    if (isShieldDisabled()) return;

    // ──────────────── 1. In-Page Player Data Pruning ────────────────
    // Intercepts and prunes ad schedule objects before YouTube player initializes
    try {
        var apply = Reflect.apply, owns = Object.prototype.hasOwnProperty,
            define = Object.defineProperty, isArray = Array.isArray;
        var ads = ['adPlacements', 'adSlots', 'playerAds', 'adBreakHeartbeatParams'];
        var marks = ['responseContext', 'playabilityStatus', 'streamingData', 'videoDetails'];

        function plain(o) { return o !== null && typeof o === 'object' && !isArray(o); }
        function isYouTubePayload(o) {
            for (var i = 0; i < marks.length; i++) {
                if (apply(owns, o, [marks[i]])) return true;
            }
            return false;
        }
        function stripAds(o) {
            if (!plain(o) || !isYouTubePayload(o)) return;
            for (var i = 0; i < ads.length; i++) {
                if (apply(owns, o, [ads[i]])) delete o[ads[i]];
            }
        }
        function pruneOne(o) {
            if (!plain(o)) return;
            stripAds(o);
            if (apply(owns, o, ['playerResponse'])) stripAds(o.playerResponse);
        }
        function prune(val) {
            try {
                if (isArray(val)) {
                    for (var i = 0; i < val.length && i < 16; i++) pruneOne(val[i]);
                } else {
                    pruneOne(val);
                }
            } catch(e) {}
            return val;
        }

        // Intercept JSON.parse
        var parse = JSON.parse;
        if (typeof parse === 'function') {
            define(JSON, 'parse', {
                value: ({ parse(text, reviver) { return prune(apply(parse, JSON, arguments)); } }).parse,
                writable: true, enumerable: false, configurable: true
            });
        }

        // Intercept Response.prototype.json
        if (typeof Response === 'function' && Response.prototype.json) {
            var rawJson = Response.prototype.json;
            var then = Promise.prototype.then;
            define(Response.prototype, 'json', {
                value: ({ json() { return apply(then, apply(rawJson, this, []), [prune]); } }).json,
                writable: true, enumerable: true, configurable: true
            });
        }

        // Intercept early window properties: ytInitialPlayerResponse, ytInitialData
        ['ytInitialPlayerResponse', 'ytInitialData'].forEach(function(name) {
            try {
                if (apply(owns, window, [name])) return;
                var heldVal;
                define(window, name, {
                    get: function() { return heldVal; },
                    set: function(val) { heldVal = prune(val); },
                    enumerable: true, configurable: true
                });
            } catch(e) {}
        });
    } catch(e) {}

    // ──────────────── 2. Video Player Fast-Forward Fallback (16x & Mute) ────────────────
    // When an ad plays despite payload pruning, mutes and accelerates at 16x
    try {
        var stateMap = new WeakMap();

        function handleVideo(event) {
            if (isShieldDisabled()) {
                var v = event.target || document.querySelector('video');
                if (v instanceof HTMLVideoElement) {
                    if (v.playbackRate === 16) {
                        v.playbackRate = 1.0;
                    }
                    v.muted = false;
                }
                return;
            }
            var video = event.target || document.querySelector('video');
            if (!(video instanceof HTMLVideoElement)) return;
            var player = video.closest('.html5-video-player') || document.querySelector('.html5-video-player');
            var isAd = !!player && (
                player.classList.contains('ad-showing') ||
                player.classList.contains('ad-interrupting')
            );
            var prev = stateMap.get(video);

            try {
                if (isAd) {
                    if (!prev) {
                        prev = { muted: video.muted, rate: video.playbackRate };
                        stateMap.set(video, prev);
                        video.muted = true;
                    }
                    // Instant skip: jump to end of ad if duration is finite and valid
                    if (isFinite(video.duration) && video.duration > 0 && video.currentTime < video.duration - 0.1) {
                        try {
                            video.currentTime = video.duration;
                        } catch(e) {}
                    }
                    if (video.playbackRate !== 16) {
                        video.playbackRate = 16;
                    }
                    // If video is paused by YouTube ad engine, try resume immediately
                    if (video.paused) {
                        video.play().catch(() => {});
                    }
                    // Auto-click skip button if rendered
                    var skipBtn = document.querySelector(
                        '.ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-skip-ad-button, .ytp-ad-skip-button-container button, button.ytp-ad-skip-button-modern, .ytp-ad-overlay-close-button'
                    );
                    if (skipBtn) {
                        skipBtn.click();
                    }
                } else if (prev) {
                    stateMap.delete(video);
                    video.playbackRate = prev.rate;
                    video.muted = prev.muted;
                }
            } catch(e) {}
        }

        function dismissInterruptionDialogs() {
            try {
                // Remove enforcement message dialogs ("Experiencing interruptions?")
                const dialogs = document.querySelectorAll(
                    'ytd-enforcement-message-view-model, tp-yt-paper-dialog:has(ytd-enforcement-message-view-model), #error-screen:has(ytd-enforcement-message-view-model)'
                );
                const hadDialogs = dialogs.length > 0;
                for (let i = 0; i < dialogs.length; i++) {
                    dialogs[i].remove();
                }
                if (hadDialogs) {
                    const backdrops = document.querySelectorAll('tp-yt-iron-overlay-backdrop');
                    for (let i = 0; i < backdrops.length; i++) {
                        backdrops[i].remove();
                    }
                }
                const dismissBtn = document.querySelector(
                    'ytd-enforcement-message-view-model button, tp-yt-paper-dialog:has(ytd-enforcement-message-view-model) #dismiss-button button'
                );
                if (dismissBtn) {
                    try { dismissBtn.click(); } catch (e) {}
                }
                // Only resume playback if an enforcement dialog actually interrupted the player just now;
                // never override a user-initiated pause.
                if (hadDialogs || dismissBtn) {
                    var v = document.querySelector('video');
                    if (v && v.paused && !document.hidden) {
                        var player = v.closest('.html5-video-player');
                        if (player && !player.classList.contains('ad-showing')) {
                            v.play().catch(() => {});
                        }
                    }
                }
            } catch(e) {}
        }

        function neutralizeYtConfig() {
            try {
                if (window.yt && window.yt.config_) {
                    window.yt.config_.EXPERIENCE_INTERRUPTIONS_MODE = false;
                    if (window.yt.config_.EXPERIMENT_FLAGS) {
                        window.yt.config_.EXPERIMENT_FLAGS.web_enable_ab_rsp_cl = false;
                        window.yt.config_.EXPERIMENT_FLAGS.web_enable_ab_reg_app = false;
                    }
                    if (window.yt.config_.openPopupConfig && window.yt.config_.openPopupConfig.supportedPopups) {
                        window.yt.config_.openPopupConfig.supportedPopups.adBlockMessageViewModel = false;
                    }
                }
            } catch(e) {}
        }

        ['playing', 'timeupdate', 'durationchange', 'emptied'].forEach(function(type) {
            document.addEventListener(type, handleVideo, true);
        });

        // Periodic watchdog in case events are throttled
        setInterval(function() {
            if (isShieldDisabled()) {
                var v = document.querySelector('video');
                if (v && v.playbackRate === 16) {
                    v.playbackRate = 1.0;
                    v.muted = false;
                }
                return;
            }
            neutralizeYtConfig();
            dismissInterruptionDialogs();
            var v = document.querySelector('video');
            if (v) handleVideo({ target: v });
        }, 200);
    } catch(e) {}

    // ──────────────── 3. Anti-Adblock & Interruption Neutralization ────────────────
    try {
        if (window.yt && window.yt.config_) {
            window.yt.config_.EXPERIENCE_INTERRUPTIONS_MODE = false;
            if (window.yt.config_.EXPERIMENT_FLAGS) {
                window.yt.config_.EXPERIMENT_FLAGS.web_enable_ab_rsp_cl = false;
                window.yt.config_.EXPERIMENT_FLAGS.web_enable_ab_reg_app = false;
            }
            if (window.yt.config_.openPopupConfig && window.yt.config_.openPopupConfig.supportedPopups) {
                window.yt.config_.openPopupConfig.supportedPopups.adBlockMessageViewModel = false;
            }
        }
    } catch(e) {}
})();
"#;

/// Comprehensive Cosmetic Ad & Tracker Hiding Script.
/// Ports `Shield.slots`, `Shield.slotsBySite`, and `Shield.reddit` from macOS Search.
pub const COSMETIC_SHIELD_CSS_SCRIPT: &str = r#"
(() => {
    if (window !== window.top) return;
    // Exclude YouTube so we don't interfere with its custom polymer layout
    if (location.hostname.endsWith('youtube.com')) return;

    try {
        if (window.__BROWSER_SHIELD_PAUSED__ === true) return;
        if (window.__BROWSER_SHIELD_ENABLED__ === false) return;
        if (localStorage.getItem('__browser_shield_paused') === 'true') return;
        if (localStorage.getItem('__browser_shield_global_enabled') === 'false') return;
    } catch(e) {}

    const css = `
        /* Generic ad slots and ad container identifiers */
        .adsbygoogle, ins.adsbygoogle, [id^="google_ads_"], [id^="div-gpt-ad"],
        [id^="taboola-"], #taboola-below-article, iframe[src*="doubleclick.net"],
        iframe[src*="googlesyndication"], iframe[src*="amazon-adsystem"],
        .ad-slot, .ad-slot-container, .top-banner-ad-container,
        .ad-leaderboard, .ad-billboard, .ad-giga, .ad-mpu, .ad-mrec,
        .ad-unit, .adunit, .adslot, .dfp-ad, .gpt-ad, .w_ad,
        .ad-container, .advertisement, .ad-banner, .ad-wrapper,
        .taboola-placeholder, #outbrain_widget_0, .OUTBRAIN,
        [data-ad-unit], [data-ad-client], [data-ad-slot],
        div[class*="SponsoredMessage"], div[class*="ad-placeholder"],
        /* Reddit promoted posts and inline ads */
        shreddit-ad-post, shreddit-comments-page-ad, shreddit-sidebar-ad,
        shreddit-dynamic-ad-link, comment-tree-ad, .promotedlink,
        /* Specific publisher ad wrappers */
        #billboard-wrapper, .ad-slot-header__wrapper, .top-fronts-banner-ad-container {
            display: none !important;
            visibility: hidden !important;
            height: 0 !important;
            width: 0 !important;
            opacity: 0 !important;
            pointer-events: none !important;
        }
    `;

    function inject() {
        if (document.getElementById('__browser_shield_cosmetic__')) return;
        const style = document.createElement('style');
        style.id = '__browser_shield_cosmetic__';
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_host() {
        assert_eq!(extract_host("https://google.com/search?q=test"), Some("google.com".into()));
        assert_eq!(extract_host("http://sub.example.co.uk:8080/path"), Some("sub.example.co.uk".into()));
        assert_eq!(extract_host("https://user:pass@ads.doubleclick.net/ad"), Some("ads.doubleclick.net".into()));
        assert_eq!(extract_host("about:blank"), None);
    }

    #[test]
    fn test_host_matches_domain() {
        assert!(host_matches_domain("doubleclick.net", "doubleclick.net"));
        assert!(host_matches_domain("ad.doubleclick.net", "doubleclick.net"));
        assert!(host_matches_domain("sub.ad.doubleclick.net", "doubleclick.net"));
        assert!(!host_matches_domain("notdoubleclick.net", "doubleclick.net"));
        assert!(!host_matches_domain("example.com", "doubleclick.net"));
    }

    #[test]
    fn test_is_unwanted_host() {
        assert!(is_unwanted_host("doubleclick.net"));
        assert!(is_unwanted_host("stats.g.doubleclick.net"));
        assert!(is_unwanted_host("google-analytics.com"));
        assert!(is_unwanted_host("ssl.google-analytics.com"));
        assert!(is_unwanted_host("criteo.com"));
        assert!(is_unwanted_host("criteo.net"));
        assert!(is_unwanted_host("analytics.tiktok.com"));
        assert!(!is_unwanted_host("wikipedia.org"));
        assert!(!is_unwanted_host("github.com"));
    }

    #[test]
    fn test_is_youtube_ad_url() {
        assert!(is_youtube_ad_url("https://www.youtube.com/api/stats/ads?v=xyz"));
        assert!(is_youtube_ad_url("https://www.youtube.com/pagead/lvz?ai=abc"));
        assert!(is_youtube_ad_url("https://www.youtube.com/ptracking?val=123"));
        assert!(is_youtube_ad_url("https://www.youtube.com/api/stats/qoe?adformat=1_2&other=3"));
        assert!(is_youtube_ad_url("https://imasdk.googleapis.com/js/sdkloader/ima3.js"));
        assert!(!is_youtube_ad_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(!is_youtube_ad_url("https://www.youtube.com/s/player/abcdef/player_ias.vflset/en_US/base.js"));
    }

    #[test]
    fn test_is_third_party() {
        assert!(is_third_party("doubleclick.net", "theverge.com"));
        assert!(!is_third_party("theverge.com", "theverge.com"));
        assert!(!is_third_party("cdn.theverge.com", "theverge.com"));
        assert!(!is_third_party("www.theverge.com", "theverge.com"));
    }

    #[test]
    fn test_should_block_request() {
        // Active shield blocks third-party trackers
        assert!(should_block_request(
            "https://ad.doubleclick.net/pagead/ad.js",
            "nytimes.com",
            true,
            false,
        ));
        assert!(should_block_request(
            "https://www.google-analytics.com/analytics.js",
            "example.com",
            true,
            false,
        ));

        // When paused for a site, nothing is blocked
        assert!(!should_block_request(
            "https://ad.doubleclick.net/pagead/ad.js",
            "nytimes.com",
            true,
            true, // paused!
        ));

        // When shield globally disabled, nothing is blocked
        assert!(!should_block_request(
            "https://ad.doubleclick.net/pagead/ad.js",
            "nytimes.com",
            false,
            false,
        ));

        // Legitimate requests to first-party are not blocked
        assert!(!should_block_request(
            "https://nytimes.com/assets/main.js",
            "nytimes.com",
            true,
            false,
        ));

        // YouTube ad endpoints blocked on YouTube when active, but allowed when paused
        assert!(should_block_request(
            "https://www.youtube.com/api/stats/ads?v=test",
            "youtube.com",
            true,
            false,
        ));
        assert!(!should_block_request(
            "https://www.youtube.com/api/stats/ads?v=test",
            "youtube.com",
            true,
            true,
        ));
    }

    #[test]
    fn test_shield_state_operations() {
        let state = ShieldState::new();
        assert!(state.is_enabled());
        assert_eq!(state.get_blocked_count(), 0);

        assert_eq!(state.increment_blocked(), 1);
        assert_eq!(state.increment_blocked(), 2);
        assert_eq!(state.get_blocked_count(), 2);

        state.reset_blocked();
        assert_eq!(state.get_blocked_count(), 0);

        assert!(!state.is_paused("nytimes.com"));
        state.set_paused("www.nytimes.com", true);
        assert!(state.is_paused("nytimes.com"));
        assert!(state.is_paused("www.nytimes.com"));
        assert!(state.is_paused("sub.nytimes.com"));

        state.set_paused("youtube.com", true);
        assert!(state.is_paused("youtube.com"));
        assert!(state.is_paused("www.youtube.com"));
        assert!(state.is_paused("m.youtube.com"));
        assert!(state.is_paused("music.youtube.com"));

        state.set_paused("nytimes.com", false);
        assert!(!state.is_paused("nytimes.com"));
        assert!(!state.is_paused("sub.nytimes.com"));
    }
}
