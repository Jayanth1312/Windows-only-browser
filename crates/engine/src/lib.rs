pub mod clean_link;
pub mod markdown;
pub mod reader;
pub mod shield;
pub mod webview;

pub use clean_link::{clean_url, has_tracking_params, is_signature, tracks};
pub use markdown::{parse_markdown_page, MarkdownPage};
pub use reader::{format_markdown_link, strip_url_fragment, ReaderLook, ReaderMessage};
pub use shield::{
    extract_host, is_third_party, is_unwanted_host, is_youtube_ad_url, should_block_request,
    ShieldState, COSMETIC_SHIELD_CSS_SCRIPT, UNWANTED_DOMAINS, YOUTUBE_AD_STRIPPER_SCRIPT,
};
pub use webview::{
    init_com, BrowserShortcut, EngineError, WebViewController, WebViewEngine, ZapperMessage,
};
