// Reader Mode & Markdown Exporter
//
// Faithful port of Search's `Reader.swift`:
// - Typographic `#office-reader` stylesheet with Light / Dark / System look
// - Markdown twin detection (direct markdown MIME / `.md`, `<link rel="alternate" type="text/markdown">`,
//   and same-origin `.md` / `index.html.md` probing)
// - Link-penalized prose extraction heuristic + lead media / `.office-above` / iframe chart & video preservation
// - Isolated closed Shadow DOM `#office-reader-look` switcher pill
// - Clean HTML-to-Markdown exporter + `[title](url)` formatter

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderLook {
    Light,
    Dark,
    System,
}

impl ReaderLook {
    pub fn as_str(self) -> &'static str {
        match self {
            ReaderLook::Light => "light",
            ReaderLook::Dark => "dark",
            ReaderLook::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => ReaderLook::Light,
            "dark" => ReaderLook::Dark,
            _ => ReaderLook::System,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ReaderLook::Light => ReaderLook::Dark,
            ReaderLook::Dark => ReaderLook::System,
            ReaderLook::System => ReaderLook::Light,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ReaderLook::Light => "Light",
            ReaderLook::Dark => "Dark",
            ReaderLook::System => "System",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderMessage {
    TwinFound {
        gen: u64,
        url: String,
        text: String,
        title: String,
        from: String,
    },
    TwinNone {
        gen: u64,
    },
    Result {
        gen: u64,
        active: bool,
    },
    MarkdownExport {
        markdown: String,
    },
    LookChanged {
        look: ReaderLook,
    },
}

const READER_DARK_VARS: &str = "color-scheme:dark;--r-ground:#1c1c1c;--r-ink:#e0e0e0;--r-quiet:#8a8a8a;--r-aside:#a8a8a8;--r-rule:#333;--r-code:#2a2a2a;--r-picture:brightness(.92)";

pub fn reader_sheet() -> String {
    format!(
        concat!(
            ":root{{color-scheme:light;--r-ground:#fff;--r-ink:#171717;--r-quiet:#a3a3a3;--r-aside:#555;--r-rule:#e8e8e8;--r-code:#f5f5f5;--r-picture:none}}",
            "@media (prefers-color-scheme:dark){{:root:not([data-office-look=light]){{{dark}}}}}",
            ":root[data-office-look=dark]{{{dark}}}",
            "html,body{{background:var(--r-ground) !important;margin:0 !important;padding:0 !important}}",
            "#office-reader{{max-width:38em;margin:0 auto;padding:72px 24px 160px;font:400 18px/1.72 ui-serif,Georgia,\"Times New Roman\",serif;color:var(--r-ink)}}",
            "#office-reader h1{{font:600 30px/1.24 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:0 0 8px;letter-spacing:-0.01em}}",
            "#office-reader .office-from{{font:400 12px/1 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;color:var(--r-quiet);margin:0 0 40px;text-transform:uppercase;letter-spacing:.06em}}",
            "#office-reader p{{margin:0 0 1.35em}}",
            "#office-reader img,#office-reader video,#office-reader iframe{{max-width:100%;height:auto;border-radius:6px;margin:1.6em 0;display:block}}",
            "#office-reader img{{filter:var(--r-picture)}}",
            "#office-reader iframe{{width:100%;aspect-ratio:16/9;height:auto;border:0}}",
            "#office-reader svg{{max-width:100%}}",
            "#office-reader iframe.office-chart{{aspect-ratio:auto}}",
            "#office-reader figure{{margin:1.8em 0}}",
            "#office-reader figcaption{{font:400 13px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;color:var(--r-quiet);margin-top:.6em}}",
            "#office-reader figcaption > * + *{{margin-left:.35em}}",
            "#office-reader .office-above{{font:400 14px/1.55 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;color:var(--r-aside);margin:0 0 1.6em}}",
            "#office-reader .office-above ul{{padding-left:1.2em}}",
            "#office-reader .office-above h2{{margin:.4em 0}}",
            "#office-reader a{{color:var(--r-ink);text-underline-offset:3px}}",
            "#office-reader h2,#office-reader h3{{font:600 20px/1.3 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:2em 0 .6em}}",
            "#office-reader h4,#office-reader h5,#office-reader h6{{font:600 16px/1.4 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:1.8em 0 .5em}}",
            "#office-reader pre,#office-reader code,#office-reader kbd{{font-family:ui-monospace,Consolas,\"Cascadia Mono\",monospace;font-size:14px}}",
            "#office-reader pre{{background:var(--r-code);padding:14px;border-radius:8px;overflow:auto}}",
            "#office-reader blockquote{{margin:1.6em 0;padding-left:1.2em;border-left:2px solid var(--r-rule);color:var(--r-aside)}}",
            "#office-reader ul,#office-reader ol{{margin:0 0 1.35em;padding-left:1.4em}}",
            "#office-reader ul{{list-style:disc outside}}",
            "#office-reader ol{{list-style:decimal outside}}",
            "#office-reader li>ul{{list-style-type:circle}}",
            "#office-reader :not(pre)>code{{background:var(--r-code);padding:.1em .35em;border-radius:4px}}",
            "#office-reader li{{margin:.3em 0}}",
            "#office-reader li>ul,#office-reader li>ol{{margin-bottom:0}}",
            "#office-reader input[type=\"checkbox\"]{{margin-right:.45em;vertical-align:middle}}",
            "#office-reader hr{{border:0;border-top:1px solid var(--r-rule);margin:2.4em 0}}",
            "#office-reader .office-wide{{overflow-x:auto;margin:1.6em 0}}",
            "#office-reader table{{border-collapse:collapse;font:400 15px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif}}",
            "#office-reader th,#office-reader td{{border-bottom:1px solid var(--r-rule);padding:8px 12px 8px 0;text-align:left;vertical-align:top}}",
            "#office-reader th{{font-weight:600}}",
            "#office-reader .office-part{{font:600 16px/1.4 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:1.8em 0 .5em}}",
            "#office-reader details{{margin:0 0 1.35em}}",
            "#office-reader summary{{cursor:pointer}}"
        ),
        dark = READER_DARK_VARS
    )
}

const MOUNT_LOOK_PILL_JS: &str = r#"
function mountLookSwitcher(initialLook) {
  var old = document.getElementById('office-reader-look');
  if (old) old.remove();

  var host = document.createElement('div');
  host.id = 'office-reader-look';
  host.style.cssText = 'all:initial !important;position:fixed !important;top:14px !important;right:14px !important;z-index:2147483645 !important;';

  var shadow = host.attachShadow({ mode: 'closed' });
  var style = document.createElement('style');
  style.textContent = [
    ':host{all:initial;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;}',
    '.pill{display:inline-flex;align-items:center;gap:2px;padding:3px;border-radius:9999px;',
    'background:var(--p-bg,#ffffff);border:1px solid var(--p-line,rgba(0,0,0,0.10));',
    'box-shadow:0 2px 10px rgba(0,0,0,0.12);opacity:0.55;transition:opacity 140ms ease,background 140ms ease;}',
    '.pill:hover{opacity:1;}',
    'button{appearance:none;border:0;background:transparent;width:24px;height:22px;padding:0;margin:0;',
    'border-radius:9999px;display:inline-flex;align-items:center;justify-content:center;',
    'color:var(--p-muted,#737373);cursor:pointer;transition:background 120ms ease,color 120ms ease;}',
    'button:hover{color:var(--p-ink,#171717);}',
    'button[aria-pressed="true"]{background:var(--p-wash,rgba(0,0,0,0.08));color:var(--p-ink,#171717);}',
    'svg{width:13px;height:13px;display:block;}',
    '.pill[data-effective="dark"]{--p-bg:#242424;--p-line:rgba(255,255,255,0.14);--p-ink:#f0f0f0;--p-muted:#9a9a9a;--p-wash:rgba(255,255,255,0.14);box-shadow:0 2px 12px rgba(0,0,0,0.45);}'
  ].join('');

  var pill = document.createElement('div');
  pill.className = 'pill';
  pill.setAttribute('role', 'group');
  pill.setAttribute('aria-label', 'Reader Appearance');

  var items = [
    {
      id: 'light',
      label: 'Light',
      svg: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>'
    },
    {
      id: 'dark',
      label: 'Dark',
      svg: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>'
    },
    {
      id: 'system',
      label: 'As the window is',
      svg: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor" stroke="none"/></svg>'
    }
  ];

  var buttons = {};
  function syncPill(chosen) {
    var darkSys = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
    var eff = chosen === 'system' ? (darkSys ? 'dark' : 'light') : chosen;
    pill.setAttribute('data-effective', eff);
    Object.keys(buttons).forEach(function (k) {
      buttons[k].setAttribute('aria-pressed', k === chosen ? 'true' : 'false');
    });
  }

  window.__officeReaderSyncLook = function (look) {
    var chosen = (look === 'light' || look === 'dark' || look === 'system') ? look : 'system';
    if (chosen === 'system') document.documentElement.removeAttribute('data-office-look');
    else document.documentElement.setAttribute('data-office-look', chosen);
    syncPill(chosen);
  };

  items.forEach(function (it) {
    var btn = document.createElement('button');
    btn.type = 'button';
    btn.title = it.label;
    btn.setAttribute('aria-label', it.label);
    btn.innerHTML = it.svg;
    btn.addEventListener('click', function (e) {
      e.preventDefault();
      e.stopPropagation();
      window.__officeReaderSyncLook(it.id);
      if (window.chrome && window.chrome.webview) {
        window.chrome.webview.postMessage('reader:' + JSON.stringify({ type: 'look', look: it.id }));
      }
    });
    buttons[it.id] = btn;
    pill.appendChild(btn);
  });

  if (window.matchMedia) {
    try {
      var mq = window.matchMedia('(prefers-color-scheme: dark)');
      var onMedia = function () {
        var attr = document.documentElement.getAttribute('data-office-look');
        syncPill(attr || 'system');
      };
      if (mq.addEventListener) mq.addEventListener('change', onMedia);
    } catch (e) {}
  }

  shadow.appendChild(style);
  shadow.appendChild(pill);
  document.documentElement.appendChild(host);
  syncPill(initialLook);
}
"#;

const PRESENT_JS: &str = r#"
function present(wrap, title, from, sheet, readerLook) {
  var chosen = typeof readerLook === 'string' ? readerLook : 'light';
  if (chosen === 'system') document.documentElement.removeAttribute('data-office-look');
  else document.documentElement.setAttribute('data-office-look', chosen);
  var theirs = document.querySelectorAll('style, link[rel~="stylesheet" i]');
  for (var i = 0; i < theirs.length; i++) {
    var id = theirs[i].id || '';
    if (id.indexOf('office-') !== 0 && id.indexOf('__browser_') !== 0 && id.indexOf('browser-') !== 0) {
      theirs[i].remove();
    }
  }
  document.documentElement.removeAttribute('style');
  document.body.removeAttribute('style');
  var style = document.createElement('style');
  style.id = 'office-reader-sheet';
  style.textContent = sheet;
  var top = document.createElement('h1');
  top.textContent = title;
  var line = document.createElement('p');
  line.className = 'office-from';
  line.textContent = from;
  wrap.id = 'office-reader';
  document.body.innerHTML = '';
  (document.head || document.documentElement).appendChild(style);
  wrap.insertBefore(line, wrap.firstChild);
  wrap.insertBefore(top, wrap.firstChild);
  document.body.appendChild(wrap);
  mountLookSwitcher(chosen);
  window.scrollTo(0, 0);
}
"#;

const HEURISTIC_BODY_JS: &str = r#"
  function prose(el) {
    var paragraphs = el.querySelectorAll('p');
    if (paragraphs.length < 2) return 0;
    var letters = 0;
    for (var i = 0; i < paragraphs.length; i++) {
      letters += (paragraphs[i].innerText || '').length;
    }
    if (letters < 400) return 0;
    var whole = (el.textContent || '').length;
    if (!whole) return 0;
    var inside = 0, links = el.querySelectorAll('a');
    for (var j = 0; j < links.length; j++) {
      inside += (links[j].textContent || '').length;
    }
    return letters * (1 - Math.min(inside / whole, 1));
  }

  function best() {
    var candidates = document.querySelectorAll(
      'article, main, [role="main"], .post, .entry, .article, .content, #content, div, section'
    );
    var top = null, mark = 0;
    for (var i = 0; i < candidates.length; i++) {
      var el = candidates[i], score = prose(el);
      if (!score) continue;
      if (!top) { top = el; mark = score; continue; }
      if (top.contains(el)) {
        if (score * 1.25 > mark) { top = el; mark = score; }
      } else if (el.contains(top)) {
        if (score > mark * 1.25) { top = el; mark = score; }
      } else if (score > mark) {
        top = el; mark = score;
      }
    }
    return top;
  }

  if (document.getElementById('office-reader')) return 'read';

  var article = best();
  if (!article) return 'none';

  var lead = null;
  for (var up = article.parentElement, depth = 0;
       up && up !== document.body && depth < 4 && !lead;
       up = up.parentElement, depth++) {
    var shown = up.querySelectorAll('figure, picture');
    for (var l = 0; l < shown.length && !lead; l++) {
      var face = shown[l].querySelector('img');
      if (face && article.compareDocumentPosition(shown[l]) === Node.DOCUMENT_POSITION_PRECEDING &&
          !shown[l].closest('nav, aside, footer, [role="banner"]') &&
          face.getBoundingClientRect().width >= 400) lead = shown[l];
    }
  }

  var framed = article.querySelectorAll('iframe');
  for (var z = 0; z < framed.length; z++) {
    framed[z].setAttribute('data-office-height', Math.round(framed[z].getBoundingClientRect().height));
  }

  var drawings = article.querySelectorAll('svg');
  for (var dr = 0; dr < drawings.length; dr++) {
    if (drawings[dr].parentElement && drawings[dr].parentElement.closest('svg')) continue;
    var drawn = drawings[dr].getBoundingClientRect();
    drawings[dr].setAttribute('width', Math.round(drawn.width));
    drawings[dr].setAttribute('height', Math.round(drawn.height));
  }

  var heading = document.querySelector('h1');
  var above = [];
  if (heading) {
    var found = document.querySelectorAll(
      '[class*="byline" i],[class*="takeaways" i],[class*="lede" i],h2'
    );
    for (var a = 0; a < found.length; a++) {
      var part = found[a];
      if (part.contains(article) || article.contains(part) ||
          above.some(function (o) { return o.contains(part); })) continue;
      if (!(heading.compareDocumentPosition(part) & Node.DOCUMENT_POSITION_FOLLOWING)) continue;
      if (!(article.compareDocumentPosition(part) & Node.DOCUMENT_POSITION_PRECEDING)) continue;
      if (part.closest('nav, aside, footer, [role="banner"]') || !part.getClientRects().length) continue;
      above.push(part);
    }
  }

  var late = ['data-src', 'data-original', 'data-lazy-src', 'data-lazy',
              'data-full-src', 'data-hi-res-src', 'data-image', 'data-echo'];
  var pictures = Array.prototype.slice.call(article.querySelectorAll('img'));
  if (lead) pictures = pictures.concat(Array.prototype.slice.call(lead.querySelectorAll('img')));
  above.forEach(function (o) {
    pictures = pictures.concat(Array.prototype.slice.call(o.querySelectorAll('img')));
  });
  for (var i = 0; i < pictures.length; i++) {
    var picture = pictures[i];
    picture.setAttribute('loading', 'eager');
    var real = picture.currentSrc || picture.getAttribute('src') || '';
    if (!real || real.indexOf('data:image') === 0 || picture.naturalWidth <= 2) {
      for (var k = 0; k < late.length; k++) {
        var kept = picture.getAttribute(late[k]);
        if (kept) { real = kept; break; }
      }
    }
    if (real) picture.setAttribute('src', real);
    var lateSet = picture.getAttribute('data-srcset');
    if (lateSet && !picture.getAttribute('srcset')) {
      picture.setAttribute('srcset', lateSet);
    }
  }

  var inherent = /^(SOURCE|TRACK|PARAM|AREA|TEMPLATE|BR|WBR|SCRIPT|STYLE|NOSCRIPT)$/;
  var all = [];
  [article].concat(above).forEach(function (root) {
    all = all.concat(Array.prototype.slice.call(root.querySelectorAll('*')));
  });
  for (var h = 0; h < all.length; h++) {
    var el = all[h];
    if (inherent.test(el.tagName) || el.closest('details:not([open])')) continue;
    var look = getComputedStyle(el);
    var gone = look.display === 'none' || look.visibility === 'hidden';
    if (!gone && (look.overflow === 'hidden' || look.clip !== 'auto')) {
      var box = el.getBoundingClientRect();
      gone = box.width <= 1 && box.height <= 1;
    }
    if (gone && !(look.display === 'none' && el.querySelector('p'))) el.setAttribute('data-office-hidden', '');
  }

  var title = (heading && heading.innerText.trim()) || document.title;

  var wrap = document.createElement('div');
  wrap.innerHTML = article.innerHTML;
  for (var o = above.length - 1; o >= 0; o--) {
    var copy = above[o].cloneNode(true);
    copy.classList.add('office-above');
    wrap.insertBefore(copy, wrap.firstChild);
  }

  var hidden = wrap.querySelectorAll('[data-office-hidden]');
  for (var d = 0; d < hidden.length; d++) hidden[d].remove();
  var styled = wrap.querySelectorAll('[style]');
  for (var y = 0; y < styled.length; y++) styled[y].removeAttribute('style');
  if (lead) {
    var opening = lead.cloneNode(true);
    var parts = [opening].concat(Array.prototype.slice.call(opening.querySelectorAll('*')));
    for (var x = 0; x < parts.length; x++) {
      parts[x].removeAttribute('class');
      parts[x].removeAttribute('style');
    }
    var after = wrap.firstChild;
    while (after && after.classList && after.classList.contains('office-above')) after = after.nextSibling;
    wrap.insertBefore(opening, after);
  }

  var pressed = wrap.querySelectorAll('button');
  for (var b = 0; b < pressed.length; b++) {
    if (!pressed[b].querySelector('img, picture, video')) continue;
    while (pressed[b].firstChild) pressed[b].parentNode.insertBefore(pressed[b].firstChild, pressed[b]);
    pressed[b].remove();
  }

  var clutter = wrap.querySelectorAll(
    'script,style,noscript,form,nav,aside,footer,button,input,select,textarea,' +
    '[role="complementary"],[role="navigation"],[role="banner"],[aria-hidden="true"],' +
    '.video-js > [class*="vjs-"]:not(video)'
  );
  for (var c = 0; c < clutter.length; c++) clutter[c].remove();

  var invites = wrap.querySelectorAll('[class*="newsletter" i]');
  for (var n = 0; n < invites.length; n++) {
    if (invites[n].querySelectorAll('p').length < 3) invites[n].remove();
  }

  var players = /youtube|youtu\.be|vimeo|dailymotion|loom\.com|streamable|wistia|ted\.com/i;
  var charts = /flourish|flo\.uri\.sh|datawrapper|dwcdn\.net|infogram|tableau|ig\.ft\.com|google\.com\/maps/i;
  var frames = wrap.querySelectorAll('iframe');
  for (var f = 0; f < frames.length; f++) {
    var where = frames[f].getAttribute('src') || frames[f].getAttribute('data-src') || '';
    if (players.test(where)) {
      frames[f].setAttribute('src', where);
      frames[f].removeAttribute('height');
      frames[f].removeAttribute('width');
    } else if (where && charts.test(where)) {
      var tall = +frames[f].getAttribute('data-office-height');
      frames[f].setAttribute('src', where);
      frames[f].setAttribute('style', 'height:' + (tall > 60 ? tall : 420) + 'px');
      frames[f].className = 'office-chart';
    } else {
      frames[f].remove();
    }
  }

  var videos = wrap.querySelectorAll('video'), posters = [];
  for (var v = 0; v < videos.length; v++) {
    if (videos[v].poster) posters.push(videos[v].poster);
  }
  var stills = wrap.querySelectorAll('img');
  for (var t = 0; t < stills.length; t++) {
    if (posters.indexOf(stills[t].src) >= 0 && !stills[t].closest('video')) stills[t].remove();
  }

  for (var q = 0; q < videos.length; q++) {
    var video = videos[q], fed = /^blob:/.test(video.getAttribute('src') || ''), plain = '';
    var named = video.querySelectorAll('source');
    for (var m = 0; m < named.length; m++) {
      if (/^blob:/.test(named[m].getAttribute('src') || '')) fed = true;
      else if (!plain && /^https?:/.test(named[m].src)) plain = named[m].src;
    }
    if (!fed) continue;
    if (plain) {
      for (var r = 0; r < named.length; r++) named[r].remove();
      video.setAttribute('preload', 'metadata');
      video.setAttribute('src', plain);
      video.setAttribute('controls', '');
      video.setAttribute('playsinline', '');
      video.removeAttribute('autoplay');
      var player = video.parentNode && video.parentNode.closest ? video.parentNode.closest('.video-js') : null;
      if (player) player.parentNode.replaceChild(video, player);
    } else if (video.poster) {
      var still = document.createElement('img');
      still.setAttribute('src', video.poster);
      video.parentNode.replaceChild(still, video);
    } else {
      video.remove();
    }
  }

  var kept = wrap.querySelectorAll('img');
  for (var g = 0; g < kept.length; g++) {
    var src = kept[g].getAttribute('src') || '';
    if (!src || src.indexOf('data:image') === 0) kept[g].remove();
  }

  var boxes = wrap.querySelectorAll('figure, picture, div, section, p');
  for (var e = boxes.length - 1; e >= 0; e--) {
    if (!boxes[e].textContent.trim() &&
        !boxes[e].querySelector('img, video, audio, iframe, svg, canvas, table')) boxes[e].remove();
  }

  var own = wrap.querySelector('h1');
  if (own && own.textContent.replace(/\s+/g, ' ').trim().toLowerCase() ===
      title.replace(/\s+/g, ' ').toLowerCase()) own.remove();

  present(wrap, title, location.host.replace(/^www\./, ''), sheet, readerLook);
  return 'read';
"#;

pub fn start_twin_script(gen: u64) -> String {
    format!(
        r#"(async function() {{
  var gen = {gen};
  async function findTwin() {{
    var markdown = ['text/markdown', 'text/x-markdown'];
    function kind(type) {{ return (type || '').toLowerCase().split(';')[0].trim(); }}
    var heading = document.querySelector('h1');
    var about = {{
      title: (heading && heading.innerText.trim()) || document.title || '',
      from: (location.host || '').replace(/^www\./, '')
    }};

    var own = kind(document.contentType);
    if (markdown.indexOf(own) >= 0 ||
        (own === 'text/plain' && /\.(md|markdown)$/i.test(location.pathname))) {{
      return {{ url: location.href, text: document.body ? (document.body.textContent || '') : '',
               title: about.title, from: about.from }};
    }}
    if (!/^https?:$/.test(location.protocol)) return null;

    function home(address) {{
      try {{ return new URL(address, location.href).origin === location.origin; }} catch (e) {{ return false; }}
    }}
    async function read(address, patience, named) {{
      if (!home(address)) return null;
      var stop = new AbortController();
      var timer = setTimeout(function () {{ stop.abort(); }}, patience);
      try {{
        var answer = await fetch(address, {{
          headers: {{ Accept: 'text/markdown, text/plain;q=0.5' }},
          cache: 'no-store', credentials: 'same-origin', signal: stop.signal
        }});
        var type = kind(answer.headers.get('content-type'));
        var fits = markdown.indexOf(type) >= 0 || (named && type === 'text/plain');
        if (!answer.ok || !fits || (answer.url && !home(answer.url))) {{
          if (answer.body) try {{ answer.body.cancel(); }} catch (ce) {{}}
          return null;
        }}
        var text = await answer.text();
        if (text.length > 4000000 || /^\s*<(!doctype|html)/i.test(text)) return null;
        return {{ url: answer.url || address, text: text, title: about.title, from: about.from }};
      }} catch (e) {{
        return null;
      }} finally {{
        clearTimeout(timer);
      }}
    }}

    var links = document.querySelectorAll('link[rel~="alternate" i][type]');
    for (var i = 0; i < links.length; i++) {{
      if (markdown.indexOf(kind(links[i].type)) < 0 || !/^https?:/.test(links[i].href) || !home(links[i].href)) continue;
      var named = await read(links[i].href, 5000, true);
      if (named) return named;
      break;
    }}

    var guess = new URL(location.href);
    guess.hash = '';
    if (/\.(md|markdown)$/i.test(guess.pathname)) return null;
    guess.pathname += /\/$/.test(guess.pathname) ? 'index.html.md' : '.md';
    return await read(guess.href, 1500, false);
  }}

  var result = null;
  try {{ result = await findTwin(); }} catch (e) {{ result = null; }}
  if (window.chrome && window.chrome.webview) {{
    if (result && typeof result.text === 'string') {{
      window.chrome.webview.postMessage('reader:' + JSON.stringify({{
        type: 'twin_found',
        gen: gen,
        url: result.url || '',
        text: result.text,
        title: result.title || '',
        from: result.from || ''
      }}));
    }} else {{
      window.chrome.webview.postMessage('reader:' + JSON.stringify({{
        type: 'twin_none',
        gen: gen
      }}));
    }}
  }}
}})();"#,
        gen = gen
    )
}

pub fn show_html_script(gen: u64, html: &str, title: &str, from: &str, look: ReaderLook) -> String {
    let html_js = encode_js_string(html);
    let title_js = encode_js_string(title);
    let from_js = encode_js_string(from);
    let sheet_js = encode_js_string(&reader_sheet());
    let look_js = encode_js_string(look.as_str());

    format!(
        r#"(function() {{
  var gen = {gen};
  var html = {html_js};
  var title = {title_js};
  var from = {from_js};
  var sheet = {sheet_js};
  var readerLook = {look_js};
  {MOUNT_LOOK_PILL_JS}
  {PRESENT_JS}
  var wrap = document.createElement('div');
  wrap.innerHTML = html;
  present(wrap, title, from, sheet, readerLook);
  if (window.chrome && window.chrome.webview) {{
    window.chrome.webview.postMessage('reader:' + JSON.stringify({{ type: 'result', gen: gen, active: true }}));
  }}
}})();"#,
        gen = gen,
        html_js = html_js,
        title_js = title_js,
        from_js = from_js,
        sheet_js = sheet_js,
        look_js = look_js,
    )
}

pub fn heuristic_script(gen: u64, look: ReaderLook) -> String {
    let sheet_js = encode_js_string(&reader_sheet());
    let look_js = encode_js_string(look.as_str());

    format!(
        r#"(function() {{
  var gen = {gen};
  var sheet = {sheet_js};
  var readerLook = {look_js};
  {MOUNT_LOOK_PILL_JS}
  {PRESENT_JS}
  function runReader() {{
    {HEURISTIC_BODY_JS}
  }}
  var outcome = 'none';
  try {{ outcome = runReader(); }} catch (e) {{ outcome = 'none'; }}
  if (window.chrome && window.chrome.webview) {{
    window.chrome.webview.postMessage('reader:' + JSON.stringify({{
      type: 'result',
      gen: gen,
      active: outcome === 'read'
    }}));
  }}
}})();"#,
        gen = gen,
        sheet_js = sheet_js,
        look_js = look_js,
    )
}

pub fn apply_look_script(look: ReaderLook) -> String {
    let look_js = encode_js_string(look.as_str());
    format!(
        r#"(function() {{
  var look = {look_js};
  if (window.__officeReaderSyncLook) {{
    window.__officeReaderSyncLook(look);
  }} else {{
    if (look === 'system') document.documentElement.removeAttribute('data-office-look');
    else document.documentElement.setAttribute('data-office-look', look);
  }}
}})();"#,
        look_js = look_js
    )
}

pub fn export_markdown_script() -> String {
    r#"(function() {
  function pickRoot() {
    var r = document.getElementById('office-reader');
    if (r) return { root: r, inReader: true };
    var candidates = document.querySelectorAll('article, main, [role="main"], .post, .entry, .article, .content, #content');
    var best = null, bestScore = 0;
    for (var i = 0; i < candidates.length; i++) {
      var el = candidates[i];
      var ps = el.querySelectorAll('p');
      var letters = 0;
      for (var j = 0; j < ps.length; j++) letters += (ps[j].innerText || '').length;
      if (letters > bestScore) { best = el; bestScore = letters; }
    }
    return { root: best || document.body, inReader: false };
  }

  function cleanInline(s) {
    return (s || '').replace(/\s+/g, ' ');
  }

  function inlineMd(node) {
    if (!node) return '';
    if (node.nodeType === 3) return node.nodeValue || '';
    if (node.nodeType !== 1) return '';
    var tag = node.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'noscript' || node.id === 'office-reader-look') return '';
    if (node.classList && node.classList.contains('office-from')) return '';
    if (tag === 'br') return '  \n';
    if (tag === 'img') {
      var src = node.getAttribute('src') || node.currentSrc || '';
      if (!src || src.indexOf('data:image') === 0) return '';
      var alt = (node.getAttribute('alt') || '').replace(/[\[\]]/g, '');
      return '![' + alt + '](' + src + ')';
    }
    if (tag === 'code' && (!node.parentElement || node.parentElement.tagName.toLowerCase() !== 'pre')) {
      return '`' + (node.textContent || '').replace(/`/g, '\\`') + '`';
    }
    if (tag === 'input' && node.type === 'checkbox') {
      return node.checked ? '[x] ' : '[ ] ';
    }
    var inner = '';
    for (var i = 0; i < node.childNodes.length; i++) inner += inlineMd(node.childNodes[i]);
    if (tag === 'strong' || tag === 'b') {
      var t = inner.trim();
      return t ? '**' + t + '**' : '';
    }
    if (tag === 'em' || tag === 'i') {
      var t2 = inner.trim();
      return t2 ? '*' + t2 + '*' : '';
    }
    if (tag === 'del' || tag === 's') {
      var t3 = inner.trim();
      return t3 ? '~~' + t3 + '~~' : '';
    }
    if (tag === 'a') {
      var href = node.getAttribute('href') || '';
      var label = cleanInline(inner).trim();
      if (!label) return '';
      if (!href || /^javascript:/i.test(href)) return label;
      return '[' + label.replace(/\[/g, '\\[').replace(/\]/g, '\\]') + '](' + href + ')';
    }
    return inner;
  }

  function blockMd(node, listDepth) {
    if (!node) return '';
    if (node.nodeType === 3) {
      var txt = (node.nodeValue || '').trim();
      return txt ? cleanInline(txt) + '\n\n' : '';
    }
    if (node.nodeType !== 1) return '';
    var tag = node.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'noscript' ||
        tag === 'nav' || tag === 'aside' || tag === 'footer' ||
        node.id === 'office-reader-look') return '';
    if (node.classList && node.classList.contains('office-from')) return '';

    if (/^h[1-6]$/.test(tag)) {
      var lvl = parseInt(tag.slice(1), 10);
      var txt2 = cleanInline(inlineMd(node)).trim();
      if (!txt2) return '';
      return '#'.repeat(lvl) + ' ' + txt2 + '\n\n';
    }
    if (tag === 'hr') return '---\n\n';
    if (tag === 'pre') {
      var codeEl = node.querySelector('code');
      var cls = codeEl ? (codeEl.className || '') : '';
      var m = cls.match(/language-([A-Za-z0-9_+-]+)/);
      var lang = m ? m[1] : '';
      var raw = (codeEl || node).textContent || '';
      return '```' + lang + '\n' + raw.replace(/\n+$/, '') + '\n```\n\n';
    }
    if (tag === 'blockquote') {
      var inner = '';
      for (var i = 0; i < node.childNodes.length; i++) inner += blockMd(node.childNodes[i], listDepth);
      var lines = inner.trim().split('\n');
      return lines.map(function (l) { return l ? '> ' + l : '>'; }).join('\n') + '\n\n';
    }
    if (tag === 'ul' || tag === 'ol') {
      var ordered = tag === 'ol';
      var start = ordered ? (parseInt(node.getAttribute('start') || '1', 10) || 1) : 1;
      var out = '';
      var idx = 0;
      for (var k = 0; k < node.children.length; k++) {
        var li = node.children[k];
        if (li.tagName.toLowerCase() !== 'li') continue;
        var indent = '  '.repeat(listDepth);
        var bullet = ordered ? (start + idx) + '. ' : '- ';
        idx++;
        var textPart = '';
        var subPart = '';
        for (var c = 0; c < li.childNodes.length; c++) {
          var ch = li.childNodes[c];
          if (ch.nodeType === 1 && (ch.tagName.toLowerCase() === 'ul' || ch.tagName.toLowerCase() === 'ol')) {
            subPart += blockMd(ch, listDepth + 1);
          } else {
            textPart += inlineMd(ch);
          }
        }
        out += indent + bullet + cleanInline(textPart).trim() + '\n';
        if (subPart) out += subPart;
      }
      return out + (listDepth === 0 ? '\n' : '');
    }
    if (tag === 'table') {
      var rows = Array.prototype.slice.call(node.querySelectorAll('tr'));
      if (!rows.length) return '';
      var md = '';
      for (var r = 0; r < rows.length; r++) {
        var cells = Array.prototype.slice.call(rows[r].querySelectorAll('th, td'));
        var vals = cells.map(function (c) { return cleanInline(inlineMd(c)).trim().replace(/\|/g, '\\|'); });
        md += '| ' + vals.join(' | ') + ' |\n';
        if (r === 0) {
          md += '| ' + vals.map(function () { return '---'; }).join(' | ') + ' |\n';
        }
      }
      return md + '\n';
    }
    if (tag === 'p' || tag === 'figcaption') {
      var pTxt = cleanInline(inlineMd(node)).trim();
      return pTxt ? pTxt + '\n\n' : '';
    }
    if (tag === 'img' || tag === 'figure') {
      if (tag === 'img') {
        var im = inlineMd(node);
        return im ? im + '\n\n' : '';
      }
    }
    var acc = '';
    for (var j = 0; j < node.childNodes.length; j++) {
      acc += blockMd(node.childNodes[j], listDepth);
    }
    return acc;
  }

  var picked = pickRoot();
  var out = '';
  if (picked.root) {
    if (!picked.inReader) {
      var h1 = document.querySelector('h1');
      var title = (h1 && h1.innerText.trim()) || document.title || '';
      if (title) out += '# ' + title.trim() + '\n\n';
    }
    out += blockMd(picked.root, 0);
  }
  out = out.replace(/\n{3,}/g, '\n\n').trim();
  if (!out) out = (document.title ? '# ' + document.title + '\n\n' : '') + location.href;
  if (window.chrome && window.chrome.webview) {
    window.chrome.webview.postMessage('reader:' + JSON.stringify({
      type: 'markdown',
      markdown: out
    }));
  }
})();"#.to_string()
}

pub fn format_markdown_link(title: &str, url: &str) -> String {
    let trimmed = title.trim();
    let label = if trimmed.is_empty() { url } else { trimmed };
    let escaped = label.replace('[', "\\[").replace(']', "\\]");
    format!("[{}]({})", escaped, url)
}

pub fn strip_url_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

pub fn encode_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn parse_reader_message(payload: &str) -> Option<ReaderMessage> {
    let fields = parse_json_object(payload)?;
    let msg_type = match fields.get("type") {
        Some(JsonVal::Str(s)) => s.as_str(),
        _ => return None,
    };

    match msg_type {
        "twin_found" => {
            let gen = match fields.get("gen") {
                Some(JsonVal::Num(n)) => *n,
                _ => return None,
            };
            let url = match fields.get("url") {
                Some(JsonVal::Str(s)) => s.clone(),
                _ => String::new(),
            };
            let text = match fields.get("text") {
                Some(JsonVal::Str(s)) => s.clone(),
                _ => return None,
            };
            let title = match fields.get("title") {
                Some(JsonVal::Str(s)) => s.clone(),
                _ => String::new(),
            };
            let from = match fields.get("from") {
                Some(JsonVal::Str(s)) => s.clone(),
                _ => String::new(),
            };
            Some(ReaderMessage::TwinFound {
                gen,
                url,
                text,
                title,
                from,
            })
        }
        "twin_none" => {
            let gen = match fields.get("gen") {
                Some(JsonVal::Num(n)) => *n,
                _ => return None,
            };
            Some(ReaderMessage::TwinNone { gen })
        }
        "result" => {
            let gen = match fields.get("gen") {
                Some(JsonVal::Num(n)) => *n,
                _ => return None,
            };
            let active = match fields.get("active") {
                Some(JsonVal::Bool(b)) => *b,
                _ => false,
            };
            Some(ReaderMessage::Result { gen, active })
        }
        "markdown" => {
            let markdown = match fields.get("markdown") {
                Some(JsonVal::Str(s)) => s.clone(),
                _ => return None,
            };
            Some(ReaderMessage::MarkdownExport { markdown })
        }
        "look" => {
            let look_str = match fields.get("look") {
                Some(JsonVal::Str(s)) => s.as_str(),
                _ => return None,
            };
            Some(ReaderMessage::LookChanged {
                look: ReaderLook::from_str(look_str),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum JsonVal {
    Str(String),
    Num(u64),
    Bool(bool),
    Null,
}

fn parse_json_object(input: &str) -> Option<std::collections::HashMap<String, JsonVal>> {
    let bytes = input.as_bytes();
    let mut i = 0;
    skip_ws(bytes, &mut i);
    if *bytes.get(i)? != b'{' {
        return None;
    }
    i += 1;
    let mut map = std::collections::HashMap::new();
    skip_ws(bytes, &mut i);
    if bytes.get(i) == Some(&b'}') {
        return Some(map);
    }
    loop {
        skip_ws(bytes, &mut i);
        let key = parse_json_str(input, &mut i)?;
        skip_ws(bytes, &mut i);
        if *bytes.get(i)? != b':' {
            return None;
        }
        i += 1;
        skip_ws(bytes, &mut i);
        let val = parse_json_val(input, &mut i)?;
        map.insert(key, val);
        skip_ws(bytes, &mut i);
        match bytes.get(i) {
            Some(&b',') => i += 1,
            Some(&b'}') => break,
            _ => return None,
        }
    }
    Some(map)
}

fn skip_ws(bytes: &[u8], i: &mut usize) {
    while *i < bytes.len() && matches!(bytes[*i], b' ' | b'\n' | b'\r' | b'\t') {
        *i += 1;
    }
}

fn parse_json_str(input: &str, i: &mut usize) -> Option<String> {
    let bytes = input.as_bytes();
    if *bytes.get(*i)? != b'"' {
        return None;
    }
    *i += 1;
    let mut out = String::new();
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == b'"' {
            *i += 1;
            return Some(out);
        }
        if b == b'\\' {
            *i += 1;
            let esc = *bytes.get(*i)?;
            *i += 1;
            match esc {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\x08'),
                b'f' => out.push('\x0c'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let code1 = parse_hex4(bytes.get(*i..*i + 4)?)?;
                    *i += 4;
                    if (0xD800..=0xDBFF).contains(&code1)
                        && bytes.get(*i..*i + 2) == Some(b"\\u")
                    {
                        if let Some(code2) =
                            bytes.get(*i + 2..*i + 6).and_then(parse_hex4)
                        {
                            if (0xDC00..=0xDFFF).contains(&code2) {
                                *i += 6;
                                let combined = 0x10000
                                    + (((code1 as u32 - 0xD800) << 10) | (code2 as u32 - 0xDC00));
                                if let Some(ch) = char::from_u32(combined) {
                                    out.push(ch);
                                    continue;
                                }
                            }
                        }
                    }
                    if let Some(ch) = char::from_u32(code1 as u32) {
                        out.push(ch);
                    }
                }
                _ => return None,
            }
        } else {
            let ch = input[*i..].chars().next()?;
            out.push(ch);
            *i += ch.len_utf8();
        }
    }
    None
}

fn parse_hex4(slice: &[u8]) -> Option<u16> {
    if slice.len() != 4 {
        return None;
    }
    let s = std::str::from_utf8(slice).ok()?;
    u16::from_str_radix(s, 16).ok()
}

fn parse_json_val(input: &str, i: &mut usize) -> Option<JsonVal> {
    let bytes = input.as_bytes();
    let b = *bytes.get(*i)?;
    match b {
        b'"' => parse_json_str(input, i).map(JsonVal::Str),
        b't' if input[*i..].starts_with("true") => {
            *i += 4;
            Some(JsonVal::Bool(true))
        }
        b'f' if input[*i..].starts_with("false") => {
            *i += 5;
            Some(JsonVal::Bool(false))
        }
        b'n' if input[*i..].starts_with("null") => {
            *i += 4;
            Some(JsonVal::Null)
        }
        b'0'..=b'9' => {
            let start = *i;
            while *i < bytes.len() && bytes[*i].is_ascii_digit() {
                *i += 1;
            }
            let num: u64 = input[start..*i].parse().ok()?;
            Some(JsonVal::Num(num))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reader_messages() {
        let twin = r##"{"type":"twin_found","gen":7,"url":"https://docs.rs/foo.md","text":"# Hello\n\nWorld \uD83D\uDE80","title":"Fallback","from":"docs.rs"}"##;
        let msg = parse_reader_message(twin).unwrap();
        assert_eq!(
            msg,
            ReaderMessage::TwinFound {
                gen: 7,
                url: "https://docs.rs/foo.md".to_string(),
                text: "# Hello\n\nWorld 🚀".to_string(),
                title: "Fallback".to_string(),
                from: "docs.rs".to_string(),
            }
        );

        let none = parse_reader_message(r#"{"type":"twin_none","gen":3}"#).unwrap();
        assert_eq!(none, ReaderMessage::TwinNone { gen: 3 });

        let res = parse_reader_message(r#"{"type":"result","gen":3,"active":true}"#).unwrap();
        assert_eq!(res, ReaderMessage::Result { gen: 3, active: true });

        let look = parse_reader_message(r#"{"type":"look","look":"dark"}"#).unwrap();
        assert_eq!(look, ReaderMessage::LookChanged { look: ReaderLook::Dark });

        let md = parse_reader_message(r#"{"type":"markdown","markdown":"Line 1\n\"quoted\""}"#).unwrap();
        assert_eq!(
            md,
            ReaderMessage::MarkdownExport {
                markdown: "Line 1\n\"quoted\"".to_string()
            }
        );
    }

    #[test]
    fn test_reader_look_cycle_and_markdown_link() {
        assert_eq!(ReaderLook::Light.next(), ReaderLook::Dark);
        assert_eq!(ReaderLook::Dark.next(), ReaderLook::System);
        assert_eq!(ReaderLook::System.next(), ReaderLook::Light);

        assert_eq!(
            format_markdown_link("Title [v2]", "https://example.com/a"),
            "[Title \\[v2\\]](https://example.com/a)"
        );
        assert_eq!(
            format_markdown_link("   ", "https://example.com/"),
            "[https://example.com/](https://example.com/)"
        );
        assert_eq!(
            strip_url_fragment("https://example.com/docs#section-2"),
            "https://example.com/docs"
        );
    }

    #[test]
    fn test_generated_scripts_contain_expected_symbols() {
        let sheet = reader_sheet();
        assert!(sheet.contains("#office-reader"));
        assert!(sheet.contains("data-office-look=dark"));

        let s_twin = start_twin_script(42);
        assert!(s_twin.contains("var gen = 42;"));
        assert!(s_twin.contains("text/markdown"));
        assert!(s_twin.contains("index.html.md"));

        let s_heur = heuristic_script(9, ReaderLook::Dark);
        assert!(s_heur.contains("var gen = 9;"));
        assert!(s_heur.contains("var readerLook = \"dark\";"));
        assert!(s_heur.contains("office-reader-look"));

        let s_show = show_html_script(11, "<p>Hi</p>", "Title", "example.com", ReaderLook::System);
        assert!(s_show.contains("var gen = 11;"));
        assert!(s_show.contains("var readerLook = \"system\";"));
    }
}
