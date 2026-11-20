// Runs in the page's MAIN world on youtube.com at document_start.
// Removes ad data from player responses BEFORE the player sees it.
// Does not touch the <video> element, so playback state stays untouched.
(() => {
    if (window.__arcYtAdStrip) return;
    window.__arcYtAdStrip = true;

    const AD_KEYS = ['adPlacements', 'playerAds', 'adSlots', 'adBreakHeartbeatParams'];

    function strip(obj) {
        try {
            if (!obj || typeof obj !== 'object') return obj;
            for (const k of AD_KEYS) {
                if (k in obj) delete obj[k];
            }
            if (obj.playerResponse && typeof obj.playerResponse === 'object') {
                strip(obj.playerResponse);
            }
            if (obj.auxiliaryUi && obj.auxiliaryUi.messageRenderers) {
                // leave as is
            }
        } catch (e) {}
        return obj;
    }

    // 1) Initial page data: window.ytInitialPlayerResponse is assigned by an inline script.
    try {
        let _value;
        Object.defineProperty(window, 'ytInitialPlayerResponse', {
            configurable: true,
            enumerable: true,
            get() { return _value; },
            set(v) { _value = strip(v); },
        });
    } catch (e) {}

    // 2) SPA navigations: player/next requests made with fetch().
    try {
        const origFetch = window.fetch;
        window.fetch = async function (...args) {
            const res = await origFetch.apply(this, args);
            try {
                const req = args[0];
                const url = typeof req === 'string' ? req : (req && req.url) || '';
                if (url.includes('/youtubei/v1/player') || url.includes('/youtubei/v1/get_watch')) {
                    const text = await res.clone().text();
                    const data = JSON.parse(text);
                    strip(data);
                    return new Response(JSON.stringify(data), {
                        status: res.status,
                        statusText: res.statusText,
                        headers: res.headers,
                    });
                }
            } catch (e) {}
            return res;
        };
    } catch (e) {}
})();
