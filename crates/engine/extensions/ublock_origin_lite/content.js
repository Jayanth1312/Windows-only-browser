// uBlock Origin Lite - Bundled Ad Overlay Dismisser (safe, non-intrusive)
(() => {
    if (!location.hostname.endsWith('youtube.com')) return;

    // Only click skip/dismiss buttons when they appear - no video manipulation
    // YouTube's anti-adblock detects playbackRate/currentTime/muted changes and
    // will break the entire player or refuse to load if those are tampered with.
    function dismissAdOverlays() {
        const skipButtons = [
            '.ytp-skip-ad-button',
            '.ytp-ad-skip-button',
            '.ytp-ad-skip-button-modern',
            '.ytp-ad-overlay-close-button',
            'button.ytp-ad-skip-button-modern',
        ];
        for (const sel of skipButtons) {
            const btn = document.querySelector(sel);
            if (btn && btn.offsetParent !== null) {
                try { btn.click(); } catch (e) {}
            }
        }
    }

    // Use MutationObserver instead of setInterval for lower overhead
    let debounceTimer = null;
    const observer = new MutationObserver(() => {
        if (debounceTimer) return;
        debounceTimer = setTimeout(() => {
            debounceTimer = null;
            dismissAdOverlays();
        }, 300);
    });

    function start() {
        const player = document.getElementById('movie_player') || document.body;
        observer.observe(player, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', start);
    } else {
        start();
    }
})();
