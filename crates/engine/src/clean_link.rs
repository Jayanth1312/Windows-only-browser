//! CleanLink: Tracking parameter stripping engine.
//!
//! Directly ported and enhanced from macOS Search's `CleanLink.swift`.
//! Removes invasive analytics, marketing, and surveillance identifiers
//! (`utm_*`, `fbclid`, `gclid`, `twclid`, `igshid`, `msclkid`, `si`, `mc_eid`)
//! while preserving functional parameters, relative parameter order, URL fragments,
//! and crucially protecting cryptographically signed URLs (AWS S3, Google Cloud, CloudFront, Azure).

/// Parameter names that track and do nothing else on any site.
/// `utm_` covers all Google Analytics / Urchin Tracking Module tags.
pub static UNIVERSAL_TRACKING: &[&str] = &[
    "fbclid",   // Meta / Facebook Click Identifier
    "gclid",    // Google AdWords Click Identifier
    "dclid",    // DoubleClick Click Identifier
    "gbraid",   // Google Ads iOS Web-to-App Conversion Identifier
    "wbraid",   // Google Ads iOS App-to-Web Conversion Identifier
    "msclkid",  // Microsoft / Bing Ads Click Identifier
    "yclid",    // Yandex Click Identifier
    "twclid",   // Twitter / X Click Identifier
    "ttclid",   // TikTok Click Identifier
    "mc_eid",   // Mailchimp Email Identifier
    "igshid",   // Instagram Share Identifier
    "igsh",     // Instagram Share Identifier (short)
    "_hsenc",   // HubSpot Encoded Tracking Identifier
    "_hsmi",    // HubSpot Marketing Email Identifier
    "mkt_tok",  // Marketo Tracking Token
];

/// Parameters that track on specific sites but may have functional meaning on others.
/// For example, `si` on YouTube or Spotify identifies who shared the link.
pub static SITE_SPECIFIC_TRACKING: &[(&str, &[&str])] = &[
    ("si", &["youtube.com", "youtu.be", "spotify.com"]),
];

/// Parameter names indicating a cryptographically signed address (S3, GCS, CloudFront, Azure).
/// An address that carries a signature MUST be left completely untouched,
/// since removing or re-ordering any parameter would invalidate the signature and break the link.
pub static SIGNATURE_PARAMS: &[&str] = &[
    "x-amz-signature",  // Amazon Web Services S3 Pre-signed URL
    "x-goog-signature", // Google Cloud Storage Signed URL
    "signature",        // Generic / CloudFront / Cloudinary signed URL
    "sig",              // Azure Blob Storage Shared Access Signature (SAS)
];

/// Returns true if the query parameter name indicates a cryptographic signature.
#[inline]
pub fn is_signature(name: &str) -> bool {
    SIGNATURE_PARAMS.contains(&name)
}

/// Helper to decode percent-encoding and '+' in parameter names for accurate matching.
fn decode_param_name(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            s.push(' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte_val) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..=i + 2]).unwrap_or(""),
                16,
            ) {
                s.push(byte_val as char);
                i += 3;
            } else {
                s.push('%');
                i += 1;
            }
        } else {
            s.push(bytes[i] as char);
            i += 1;
        }
    }
    s.to_ascii_lowercase()
}

/// Checks whether a parameter name is known to be a tracking parameter on the given host.
pub fn tracks(name: &str, host: &str) -> bool {
    if name.starts_with("utm_") || UNIVERSAL_TRACKING.contains(&name) {
        return true;
    }
    for &(param, sites) in SITE_SPECIFIC_TRACKING {
        if name == param {
            for &site in sites {
                if host == site || host.ends_with(&format!(".{site}")) {
                    return true;
                }
            }
        }
    }
    false
}

/// Checks whether the provided URL contains any tracking parameters that would be stripped.
pub fn has_tracking_params(url: &str) -> bool {
    clean_url(url) != url
}

/// The core Clean Link cleaner: returns the address without its tracking parameters,
/// or the exact same address when there is nothing to take out or when touching it
/// would break a cryptographic signature.
pub fn clean_url(url: &str) -> String {
    let trimmed = url.trim();
    let scheme_end = match trimmed.find("://") {
        Some(idx) => idx,
        None => return trimmed.to_string(),
    };

    let scheme = &trimmed[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return trimmed.to_string();
    }

    // Host extraction
    let after_scheme = &trimmed[scheme_end + 3..];
    let host_end = after_scheme
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_scheme.len());
    let mut host = &after_scheme[..host_end];
    // Strip userinfo if present: "user:pass@host"
    if let Some(at_idx) = host.rfind('@') {
        host = &host[at_idx + 1..];
    }
    // Strip port if present: "host:8080"
    if let Some(colon_idx) = host.find(':') {
        host = &host[..colon_idx];
    }
    let host_lower = host.to_ascii_lowercase();

    // Query separation
    let query_start = match trimmed.find('?') {
        Some(idx) => idx,
        None => return trimmed.to_string(),
    };

    let before_query = &trimmed[..query_start];
    let query_and_frag = &trimmed[query_start + 1..];

    let (query, fragment) = match query_and_frag.find('#') {
        Some(frag_idx) => (&query_and_frag[..frag_idx], Some(&query_and_frag[frag_idx..])),
        None => (query_and_frag, None),
    };

    if query.is_empty() {
        return trimmed.to_string();
    }

    let pieces: Vec<&str> = query.split('&').collect();
    let mut names: Vec<String> = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        let raw_name = piece.split('=').next().unwrap_or("");
        names.push(decode_param_name(raw_name));
    }

    // If any parameter in the query is a signature, do not touch anything!
    if names.iter().any(|name| is_signature(name)) {
        return trimmed.to_string();
    }

    let mut kept_pieces: Vec<&str> = Vec::with_capacity(pieces.len());
    for (piece, name) in pieces.iter().zip(names.iter()) {
        if !tracks(name, &host_lower) {
            kept_pieces.push(piece);
        }
    }

    // If no parameters were stripped, return the original URL unmodified
    if kept_pieces.len() == pieces.len() {
        return trimmed.to_string();
    }

    let mut result = String::with_capacity(trimmed.len());
    result.push_str(before_query);
    if !kept_pieces.is_empty() {
        result.push('?');
        result.push_str(&kept_pieces.join("&"));
    }
    if let Some(frag) = fragment {
        result.push_str(frag);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utm_parameters_stripped() {
        let input = "https://example.com/article?utm_source=twitter&utm_medium=social&utm_campaign=spring2026";
        let output = clean_url(input);
        assert_eq!(output, "https://example.com/article");
        assert!(has_tracking_params(input));
    }

    #[test]
    fn test_mixed_parameters_preserved() {
        let input = "https://example.com/search?q=rust+lang&utm_source=newsletter&page=2";
        let output = clean_url(input);
        assert_eq!(output, "https://example.com/search?q=rust+lang&page=2");
    }

    #[test]
    fn test_fragment_preserved() {
        let input = "https://example.com/docs?utm_content=button&topic=async#section-4";
        let output = clean_url(input);
        assert_eq!(output, "https://example.com/docs?topic=async#section-4");
    }

    #[test]
    fn test_all_universal_click_ids() {
        let input = "https://shop.com/item?id=99&fbclid=123&gclid=456&msclkid=789&twclid=abc&ttclid=def&mc_eid=ghi";
        let output = clean_url(input);
        assert_eq!(output, "https://shop.com/item?id=99");
    }

    #[test]
    fn test_site_specific_youtube_si() {
        let yt = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&si=trackingShareToken";
        assert_eq!(clean_url(yt), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");

        let yt_short = "https://youtu.be/dQw4w9WgXcQ?si=trackingShareToken&t=42";
        assert_eq!(clean_url(yt_short), "https://youtu.be/dQw4w9WgXcQ?t=42");

        let spotify = "https://open.spotify.com/track/12345?si=userShareCode";
        assert_eq!(clean_url(spotify), "https://open.spotify.com/track/12345");

        // On other sites, `si` is kept!
        let other = "https://example.com/api?si=system_index";
        assert_eq!(clean_url(other), "https://example.com/api?si=system_index");
    }

    #[test]
    fn test_cryptographic_signatures_protected() {
        // AWS S3 pre-signed URL with utm parameter: MUST NOT BE TOUCHED
        let s3 = "https://mybucket.s3.amazonaws.com/report.pdf?x-amz-signature=abcdef123456&utm_source=email";
        assert_eq!(clean_url(s3), s3);

        // Google Cloud Storage
        let gcs = "https://storage.googleapis.com/bucket/file?x-goog-signature=secret&gclid=ads";
        assert_eq!(clean_url(gcs), gcs);

        // CloudFront / generic signature
        let cf = "https://cdn.example.com/video.mp4?signature=validSignature&utm_campaign=promo";
        assert_eq!(clean_url(cf), cf);

        // Azure SAS
        let azure = "https://account.blob.core.windows.net/container/blob?sig=sasToken&fbclid=fake";
        assert_eq!(clean_url(azure), azure);
    }

    #[test]
    fn test_non_http_urls_untouched() {
        assert_eq!(clean_url("about:blank"), "about:blank");
        assert_eq!(clean_url("file:///C:/doc.html?utm_source=foo"), "file:///C:/doc.html?utm_source=foo");
        assert_eq!(clean_url("javascript:void(0)"), "javascript:void(0)");
    }

    #[test]
    fn test_urls_without_queries() {
        assert_eq!(clean_url("https://example.com"), "https://example.com");
        assert_eq!(clean_url("https://example.com/path#hash"), "https://example.com/path#hash");
    }
}
