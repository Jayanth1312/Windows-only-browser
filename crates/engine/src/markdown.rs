// Markdown, turned into the HTML Reader Mode shows.
//
// Ported from Search's `Markdown.swift`: CommonMark + GitHub additions
// (tables, strikethrough, task lists, bare links) + MDX tolerance
// (four-space indent is NOT a code block; callout components render as
// blockquotes; unknown components collapse to their title/href or nothing).
//
// Security: every piece of text is HTML-escaped, every HTML tag is rebuilt
// strictly from an allowlist with safe attributes only, and every URL must
// resolve to `http:`, `https:`, `mailto:`, or an in-page `#fragment`.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPage {
    pub title: Option<String>,
    pub html: String,
}

pub fn parse_markdown_page(text: &str, base_url: Option<&str>) -> MarkdownPage {
    let mut lines = normalized(text);
    let mut title = front_matter(&mut lines);
    let refs = references(&mut lines);
    let mut parser = Parser::new(base_url, refs);
    let mut blocks = parser.blocks(&lines);

    // Lift the first top-level H1 heading as the page title so Reader Mode
    // presents it uniformly above the provenance line.
    if let Some(first) = blocks
        .iter()
        .position(|b| matches!(b, Block::Heading(_, _)))
    {
        if let Block::Heading(1, text) = &blocks[first] {
            title = Some(plain(text));
            blocks.remove(first);
        }
    }

    let html = parser.render_blocks(&blocks, false);
    let title = title.and_then(|t| if t.trim().is_empty() { None } else { Some(t) });
    MarkdownPage { title, html }
}

fn normalized(text: &str) -> Vec<String> {
    let mut s = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
        .replace('\u{E000}', "")
        .replace('\u{E001}', "");
    if s.starts_with('\u{FEFF}') {
        s.remove(0);
    }
    s.split('\n').map(|l| l.to_string()).collect()
}

fn front_matter(lines: &mut Vec<String>) -> Option<String> {
    if lines.first().map(|s| s.trim() == "---") != Some(true) {
        return None;
    }
    if lines.len() <= 1 || !lines[1].contains(':') {
        return None;
    }
    let end = lines.iter().enumerate().skip(1).find_map(|(idx, l)| {
        let t = l.trim();
        if t == "---" || t == "..." {
            Some(idx)
        } else {
            None
        }
    })?;
    let mut title = None;
    for line in &lines[1..end] {
        if let Some(rest) = line.strip_prefix("title:") {
            let mut value = rest.trim().to_string();
            let chars: Vec<char> = value.chars().collect();
            if chars.len() >= 2
                && (chars[0] == '"' || chars[0] == '\'')
                && chars[chars.len() - 1] == chars[0]
            {
                value = chars[1..chars.len() - 1].iter().collect();
            }
            title = Some(value);
            break;
        }
    }
    lines.drain(0..=end);
    title
}

fn references(lines: &mut Vec<String>) -> HashMap<String, (String, Option<String>)> {
    let mut found = HashMap::new();
    let mut fenced = false;
    lines.retain(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced || !trimmed.starts_with('[') {
            return true;
        }
        let Some(close) = trimmed.find("]:") else {
            return true;
        };
        let name = &trimmed[1..close];
        if name.is_empty() || name.contains(']') {
            return true;
        }
        let after = &trimmed[close + 2..];
        let mut scanner = Scanner::new(after);
        let Some((address, title)) = scanner.destination(None) else {
            return true;
        };
        let key = label(name);
        found.entry(key).or_insert((address, title));
        false
    });
    found
}

pub fn label(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn plain(text: &str) -> String {
    let empty_refs = HashMap::new();
    let html = Inline {
        base: None,
        references: &empty_refs,
    }
    .render(text);
    let mut bare = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => bare.push(ch),
            _ => {}
        }
    }
    unescape(&bare).trim().to_string()
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Validates and resolves a Markdown URL against `base`, allowing only
/// `http:`, `https:`, `mailto:`, and in-page `#fragment` links.
pub fn address(raw: &str, base: Option<&str>) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with('#') {
        return Some(encode_fragment_url(raw));
    }

    if let Some((scheme, _)) = split_scheme(raw) {
        let lower = scheme.to_ascii_lowercase();
        if lower == "http" || lower == "https" || lower == "mailto" {
            if lower == "mailto" {
                return Some(encode_url_chars(raw));
            }
            // Must have authority for http/https
            if raw[scheme.len() + 1..].starts_with("//") {
                return Some(encode_url_chars(raw));
            }
            return None;
        }
        return None;
    }

    let base_str = base?.trim();
    let (base_scheme, base_rest) = split_scheme(base_str)?;
    let lower_base_scheme = base_scheme.to_ascii_lowercase();
    if lower_base_scheme != "http" && lower_base_scheme != "https" {
        return None;
    }
    let after_slashes = base_rest.strip_prefix("//")?;

    // Protocol-relative URL (`//example.com/path`)
    if raw.starts_with("//") {
        let resolved = format!("{}:{}", lower_base_scheme, raw);
        return Some(encode_url_chars(&resolved));
    }

    // Parse base URL components: authority, path, query
    let (base_no_frag, _) = after_slashes.split_once('#').unwrap_or((after_slashes, ""));
    let (base_before_q, _base_q) = base_no_frag.split_once('?').unwrap_or((base_no_frag, ""));
    let (authority, base_path) = match base_before_q.find('/') {
        Some(idx) => (&base_before_q[..idx], &base_before_q[idx..]),
        None => (base_before_q, "/"),
    };
    if authority.is_empty() {
        return None;
    }

    let resolved = if raw.starts_with('?') {
        format!("{}://{}{}{}", lower_base_scheme, authority, base_path, raw)
    } else if raw.starts_with('/') {
        let (path_part, suffix) = split_query_or_frag(raw);
        let norm = remove_dot_segments(path_part);
        format!("{}://{}{}{}", lower_base_scheme, authority, norm, suffix)
    } else {
        let (path_part, suffix) = split_query_or_frag(raw);
        let dir = match base_path.rfind('/') {
            Some(pos) => &base_path[..=pos],
            None => "/",
        };
        let merged = format!("{}{}", dir, path_part);
        let norm = remove_dot_segments(&merged);
        format!("{}://{}{}{}", lower_base_scheme, authority, norm, suffix)
    };

    Some(encode_url_chars(&resolved))
}

fn split_scheme(s: &str) -> Option<(&str, &str)> {
    let colon = s.find(':')?;
    if colon == 0 {
        return None;
    }
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    Some((scheme, &s[colon + 1..]))
}

fn split_query_or_frag(s: &str) -> (&str, &str) {
    let q = s.find('?');
    let f = s.find('#');
    let cut = match (q, f) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return (s, ""),
    };
    (&s[..cut], &s[cut..])
}

fn remove_dot_segments(path: &str) -> String {
    let trailing_slash = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..") || path == "." || path == "..";
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    if segs.is_empty() {
        "/".to_string()
    } else {
        let mut out = format!("/{}", segs.join("/"));
        if trailing_slash {
            out.push('/');
        }
        out
    }
}

fn encode_fragment_url(raw: &str) -> String {
    encode_url_chars(raw)
}

fn encode_url_chars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if is_url_allowed(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn is_url_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '.' | '_' | '~' | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' | ':' | '@' | '/' | '?' | '#' | '%' | '[' | ']'
        )
}

pub fn slug(text: &str) -> String {
    let p = plain(text).to_lowercase();
    let kept: String = p
        .chars()
        .filter(|&c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        .collect();
    kept.replace(' ', "-")
}

// MARK: - Blocks

#[derive(Debug, Clone)]
enum Block {
    Heading(usize, String),
    Paragraph(String),
    Code(Option<String>, String),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: usize,
        items: Vec<Vec<Block>>,
        checks: Vec<Option<bool>>,
        tight: bool,
    },
    Rule,
    Table {
        head: Vec<String>,
        aligns: Vec<Option<&'static str>>,
        rows: Vec<Vec<String>>,
    },
    Html(String),
}

struct Parser<'a> {
    base: Option<&'a str>,
    references: HashMap<String, (String, Option<String>)>,
    ids: HashMap<String, usize>,
}

impl<'a> Parser<'a> {
    fn new(base: Option<&'a str>, references: HashMap<String, (String, Option<String>)>) -> Self {
        Self {
            base,
            references,
            ids: HashMap::new(),
        }
    }

    fn blocks(&mut self, lines: &[String]) -> Vec<Block> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            let trimmed = line.trim();
            if trimmed.is_empty() {
                i += 1;
                continue;
            }
            if let Some((fence_str, fence_ch, info)) = Self::fence(trimmed) {
                let indent = line.chars().take_while(|&c| c == ' ').count();
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() {
                    let inner = &lines[i];
                    let bare = inner.trim();
                    if bare.starts_with(&fence_str) && bare.chars().all(|c| c == fence_ch) {
                        i += 1;
                        break;
                    }
                    let mut chars = inner.chars();
                    let mut dropped = 0;
                    let mut stripped = String::new();
                    while let Some(c) = chars.next() {
                        if dropped < indent && c == ' ' {
                            dropped += 1;
                        } else {
                            stripped.push(c);
                            stripped.extend(chars);
                            break;
                        }
                    }
                    body.push(stripped);
                    i += 1;
                }
                let language = info
                    .split_whitespace()
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                out.push(Block::Code(language, body.join("\n")));
                continue;
            }
            if trimmed.starts_with("<!--") {
                while i < lines.len() && !lines[i].contains("-->") {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if let Some(heading) = Self::atx(trimmed) {
                out.push(heading);
                i += 1;
                continue;
            }
            if Self::is_rule(trimmed) {
                out.push(Block::Rule);
                i += 1;
                continue;
            }
            if trimmed.starts_with('>') {
                let mut inner: Vec<String> = Vec::new();
                while i < lines.len() {
                    let bare = lines[i].trim();
                    if let Some(rest) = bare.strip_prefix('>') {
                        let rest = rest.strip_prefix(' ').unwrap_or(rest);
                        inner.push(rest.to_string());
                    } else if !bare.is_empty()
                        && inner.last().map(|l| !l.trim().is_empty()).unwrap_or(false)
                        && !Self::starts(bare)
                    {
                        inner.push(bare.to_string());
                    } else {
                        break;
                    }
                    i += 1;
                }
                if inner.iter().any(|l| l.contains("llms.txt")) {
                    continue;
                }
                let inner_blocks = self.blocks(&inner);
                out.push(Block::Quote(inner_blocks));
                continue;
            }
            if Self::marker(line).is_some() {
                let (list, next) = self.list(lines, i);
                out.push(list);
                i = next;
                continue;
            }
            if i + 1 < lines.len() && trimmed.contains('|') && Self::delimiters(&lines[i + 1]).is_some() {
                let (table, next) = self.table(lines, i);
                out.push(table);
                i = next;
                continue;
            }
            if Self::is_tag_line(trimmed) {
                let mut tag = trimmed.to_string();
                let mut reach = 0;
                while !tag.contains('>') && i + 1 < lines.len() && reach < 20 {
                    let next = lines[i + 1].trim();
                    if next.is_empty() {
                        break;
                    }
                    tag.push(' ');
                    tag.push_str(next);
                    i += 1;
                    reach += 1;
                }
                out.push(Block::Html(tag));
                i += 1;
                continue;
            }

            // Paragraph / Setext heading
            let mut text = vec![line.trim_start_matches(' ').to_string()];
            i += 1;
            let mut setext: Option<usize> = None;
            while i < lines.len() {
                let next = &lines[i];
                let bare = next.trim();
                if bare.is_empty() {
                    break;
                }
                if !bare.is_empty() && bare.chars().all(|c| c == '=') {
                    setext = Some(1);
                    i += 1;
                    break;
                }
                if bare.chars().count() >= 2 && bare.chars().all(|c| c == '-') {
                    setext = Some(2);
                    i += 1;
                    break;
                }
                if Self::starts(bare) || Self::interrupts(next) {
                    break;
                }
                if bare.contains('|')
                    && i + 1 < lines.len()
                    && Self::delimiters(&lines[i + 1]).is_some()
                {
                    break;
                }
                text.push(next.trim_start_matches(' ').to_string());
                i += 1;
            }
            let joined = text.join("\n");
            if let Some(level) = setext {
                out.push(Block::Heading(level, joined.trim().to_string()));
            } else {
                out.push(Block::Paragraph(joined));
            }
        }
        out
    }

    fn starts(bare: &str) -> bool {
        Self::fence(bare).is_some()
            || Self::atx(bare).is_some()
            || Self::is_rule(bare)
            || bare.starts_with('>')
            || bare.starts_with("<!--")
            || Self::is_tag_line(bare)
    }

    fn interrupts(line: &str) -> bool {
        let Some(item) = Self::marker(line) else {
            return false;
        };
        !item.ordered || item.start == 1
    }

    fn fence(bare: &str) -> Option<(String, char, String)> {
        for mark in ["```", "~~~"] {
            if bare.starts_with(mark) {
                let ch = mark.chars().next().unwrap();
                let run: String = bare.chars().take_while(|&c| c == ch).collect();
                let info = bare[run.len()..].trim().to_string();
                if mark == "```" && info.contains('`') {
                    return None;
                }
                return Some((run, ch, info));
            }
        }
        None
    }

    fn atx(bare: &str) -> Option<Block> {
        let hashes = bare.chars().take_while(|&c| c == '#').count();
        if !(1..=6).contains(&hashes) {
            return None;
        }
        let rest = &bare[hashes..];
        if !rest.is_empty() && !rest.starts_with(' ') {
            return None;
        }
        let mut text = rest.trim().to_string();
        // Strip trailing decorative ` ###` run if standing apart
        let trimmed_end = text.trim_end_matches('#');
        if trimmed_end.len() < text.len()
            && (trimmed_end.is_empty() || trimmed_end.ends_with(char::is_whitespace))
        {
            text = trimmed_end.trim().to_string();
        }
        Some(Block::Heading(hashes, text))
    }

    fn is_rule(bare: &str) -> bool {
        let Some(mark) = bare.chars().next() else {
            return false;
        };
        if mark != '-' && mark != '*' && mark != '_' {
            return false;
        }
        let marks: Vec<char> = bare.chars().filter(|&c| c != ' ').collect();
        marks.len() >= 3 && marks.iter().all(|&c| c == mark)
    }

    fn is_tag_line(bare: &str) -> bool {
        if !bare.starts_with('<') {
            return false;
        }
        let mut chars = bare.chars();
        chars.next(); // '<'
        let Some(second) = chars.next() else {
            return false;
        };
        if chars.next().is_none() && bare.len() <= 2 {
            return false;
        }
        if !(second.is_alphabetic() || second == '/') {
            return false;
        }
        if is_angle_autolink(bare) || is_angle_email(bare) {
            return false;
        }
        bare.ends_with('>') || !bare.contains('>')
    }

    fn marker(line: &str) -> Option<Marker> {
        let indent = line.chars().take_while(|&c| c == ' ').count();
        let rest = &line[indent..];
        let first = rest.chars().next()?;
        let mut width;
        let mut ordered = false;
        let mut start = 1usize;
        let mut sign = first;
        let after_sign;

        if first == '-' || first == '*' || first == '+' {
            width = 1;
            after_sign = &rest[1..];
        } else {
            let digits_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            if !(1..=9).contains(&digits_len) {
                return None;
            }
            let number: usize = rest[..digits_len].parse().ok()?;
            let rem = &rest[digits_len..];
            let delim = rem.chars().next()?;
            if delim != '.' && delim != ')' {
                return None;
            }
            after_sign = &rem[1..];
            ordered = true;
            start = number;
            sign = delim;
            width = digits_len + 1;
        }

        let spaces = after_sign.chars().take_while(|&c| c == ' ').count();
        if spaces == 0 && !after_sign.is_empty() {
            return None;
        }
        let rest_after_spaces_empty = after_sign[spaces..].is_empty();
        width += if spaces > 4 || rest_after_spaces_empty {
            1
        } else {
            spaces
        };
        Some(Marker {
            ordered,
            start,
            sign,
            content: indent + width,
        })
    }

    fn list(&mut self, lines: &[String], begin: usize) -> (Block, usize) {
        let Some(first) = Self::marker(&lines[begin]) else {
            return (Block::Paragraph(lines[begin].clone()), begin + 1);
        };
        let mut items: Vec<Vec<String>> = Vec::new();
        let mut current: Vec<String> = Vec::new();
        let mut content = first.content;
        let mut blank = false;
        let mut loose = false;
        let mut i = begin;

        while i < lines.len() {
            let line = &lines[i];
            let bare = line.trim();
            if bare.is_empty() {
                blank = true;
                current.push(String::new());
                i += 1;
                continue;
            }
            let indent = line.chars().take_while(|&c| c == ' ').count();
            if let Some(item) = Self::marker(line) {
                if item.ordered == first.ordered
                    && item.sign == first.sign
                    && indent < content
                    && !Self::is_rule(bare)
                {
                    if !items.is_empty() || !current.is_empty() {
                        if blank {
                            loose = true;
                        }
                        items.push(current);
                    }
                    let sliced = drop_chars(line, item.content);
                    current = vec![sliced];
                    content = item.content;
                    blank = false;
                    i += 1;
                    continue;
                }
            }
            if indent >= content {
                if blank && current.iter().any(|l| !l.is_empty()) {
                    loose = true;
                }
                current.push(drop_chars(line, content));
                blank = false;
                i += 1;
                continue;
            }
            if !blank && !Self::starts(bare) && Self::marker(line).is_none() {
                current.push(bare.to_string());
                i += 1;
                continue;
            }
            break;
        }
        items.push(current);

        let mut parsed = Vec::with_capacity(items.len());
        let mut checks = Vec::with_capacity(items.len());
        for mut item in items {
            while item.last().map(|s| s.is_empty()) == Some(true) {
                item.pop();
            }
            let mut check = None;
            if let Some(head) = item.first_mut() {
                for (prefix, state) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                    if let Some(rest) = head.strip_prefix(prefix) {
                        check = Some(state);
                        *head = rest.to_string();
                        break;
                    }
                }
            }
            checks.push(check);
            parsed.push(self.blocks(&item));
        }

        (
            Block::List {
                ordered: first.ordered,
                start: first.start,
                items: parsed,
                checks,
                tight: !loose,
            },
            i,
        )
    }

    fn delimiters(line: &str) -> Option<Vec<Option<&'static str>>> {
        if !line.contains('|') {
            return None;
        }
        let cells = Self::row(line);
        if cells.is_empty() {
            return None;
        }
        let mut aligns = Vec::with_capacity(cells.len());
        for cell in cells {
            let bare = cell.trim();
            if !is_delimiter_cell(bare) {
                return None;
            }
            let align = match (bare.starts_with(':'), bare.ends_with(':')) {
                (true, true) => Some("center"),
                (false, true) => Some("right"),
                (true, false) => Some("left"),
                _ => None,
            };
            aligns.push(align);
        }
        Some(aligns)
    }

    fn row(line: &str) -> Vec<String> {
        let mut bare = line.trim();
        if bare.starts_with('|') {
            bare = &bare[1..];
        }
        if bare.ends_with('|') && !bare.ends_with("\\|") {
            bare = &bare[..bare.len() - 1];
        }
        let mut cells = Vec::new();
        let mut cell = String::new();
        let mut code = false;
        let mut escaped = false;
        for ch in bare.chars() {
            if escaped {
                if ch != '|' {
                    cell.push('\\');
                }
                cell.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '`' {
                code = !code;
                cell.push(ch);
            } else if ch == '|' && !code {
                cells.push(cell.trim().to_string());
                cell.clear();
            } else {
                cell.push(ch);
            }
        }
        if escaped {
            cell.push('\\');
        }
        cells.push(cell.trim().to_string());
        cells
    }

    fn table(&self, lines: &[String], begin: usize) -> (Block, usize) {
        let head = Self::row(&lines[begin]);
        let aligns = Self::delimiters(&lines[begin + 1]).unwrap_or_default();
        let mut rows = Vec::new();
        let mut i = begin + 2;
        while i < lines.len() {
            let bare = lines[i].trim();
            if bare.is_empty() || Self::starts(bare) {
                break;
            }
            rows.push(Self::row(bare));
            i += 1;
        }
        (Block::Table { head, aligns, rows }, i)
    }

    fn render_blocks(&mut self, blocks: &[Block], tight: bool) -> String {
        let mut parts = Vec::new();
        for block in blocks {
            let html = self.render_block(block, tight);
            if !html.is_empty() {
                parts.push(html);
            }
        }
        parts.join("\n")
    }

    fn render_block(&mut self, block: &Block, tight: bool) -> String {
        match block {
            Block::Heading(level, text) => {
                let base_id = slug(text);
                let id = if let Some(&seen) = self.ids.get(&base_id) {
                    self.ids.insert(base_id.clone(), seen + 1);
                    format!("{}-{}", base_id, seen)
                } else {
                    self.ids.insert(base_id.clone(), 1);
                    base_id
                };
                let attr = if id.is_empty() {
                    String::new()
                } else {
                    format!(" id=\"{}\"", escape(&id))
                };
                format!("<h{}{}>{}</h{}>", level, attr, self.inline(text), level)
            }
            Block::Paragraph(text) => {
                let html = self.inline(text);
                if html.trim().is_empty() {
                    return String::new();
                }
                if tight {
                    html
                } else {
                    format!("<p>{}</p>", html)
                }
            }
            Block::Code(language, text) => {
                let kind = language
                    .as_ref()
                    .map(|l| format!(" class=\"language-{}\"", escape(l)))
                    .unwrap_or_default();
                format!("<pre><code{}>{}</code></pre>", kind, escape(text))
            }
            Block::Quote(inner) => {
                format!("<blockquote>{}</blockquote>", self.render_blocks(inner, false))
            }
            Block::List {
                ordered,
                start,
                items,
                checks,
                tight,
            } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let from = if *ordered && *start != 1 {
                    format!(" start=\"{}\"", start)
                } else {
                    String::new()
                };
                let mut html = format!("<{}{}>", tag, from);
                for (item, check) in items.iter().zip(checks.iter()) {
                    let box_html = match check {
                        Some(true) => "<input type=\"checkbox\" checked disabled> ",
                        Some(false) => "<input type=\"checkbox\" disabled> ",
                        None => "",
                    };
                    html.push_str("<li>");
                    html.push_str(box_html);
                    html.push_str(&self.render_blocks(item, *tight));
                    html.push_str("</li>");
                }
                html.push_str(&format!("</{}>", tag));
                html
            }
            Block::Rule => "<hr>".to_string(),
            Block::Table { head, aligns, rows } => {
                let render_cells = |values: &[String], tag: &str| -> String {
                    let mut out = String::new();
                    for (idx, align) in aligns.iter().enumerate() {
                        let style = align
                            .map(|a| format!(" style=\"text-align:{}\"", a))
                            .unwrap_or_default();
                        let val = values.get(idx).map(|s| s.as_str()).unwrap_or("");
                        out.push_str(&format!("<{}{}>{}</{}>", tag, style, self.inline(val), tag));
                    }
                    out
                };
                let mut body = String::new();
                for row in rows {
                    body.push_str(&format!("<tr>{}</tr>", render_cells(row, "td")));
                }
                format!(
                    "<div class=\"office-wide\"><table><thead><tr>{}</tr></thead><tbody>{}</tbody></table></div>",
                    render_cells(head, "th"),
                    body
                )
            }
            Block::Html(text) => Inline::tags(text, self.base, true),
        }
    }

    fn inline(&self, text: &str) -> String {
        Inline {
            base: self.base,
            references: &self.references,
        }
        .render(text)
    }
}

struct Marker {
    ordered: bool,
    start: usize,
    sign: char,
    content: usize,
}

fn drop_chars(s: &str, count: usize) -> String {
    s.chars().skip(count).collect()
}

fn is_delimiter_cell(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut inner = s;
    if inner.starts_with(':') {
        inner = &inner[1..];
    }
    if inner.ends_with(':') {
        inner = &inner[..inner.len() - 1];
    }
    !inner.is_empty() && inner.chars().all(|c| c == '-')
}

fn is_angle_autolink(s: &str) -> bool {
    let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) else {
        return false;
    };
    let Some(colon) = inner.find(':') else {
        return false;
    };
    let scheme = &inner[..colon];
    if scheme.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    if !chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
        return false;
    }
    !inner[colon + 1..].chars().any(|c| c.is_whitespace() || c == '<' || c == '>')
}

fn is_angle_email(s: &str) -> bool {
    let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) else {
        return false;
    };
    let Some((left, right)) = inner.split_once('@') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && !left.chars().any(|c| c.is_whitespace() || c == '<' || c == '>')
        && !right
            .chars()
            .any(|c| c.is_whitespace() || c == '<' || c == '>' || c == '@')
}

// MARK: - Inline

struct Inline<'a> {
    base: Option<&'a str>,
    references: &'a HashMap<String, (String, Option<String>)>,
}

impl<'a> Inline<'a> {
    fn render(&self, text: &str) -> String {
        let mut kept: Vec<String> = Vec::new();
        let mut scanner = Scanner::new(text);
        let mut out = String::new();
        let mut pending = String::new();

        macro_rules! keep {
            ($html:expr) => {{
                kept.push($html);
                format!("\u{E000}{}\u{E001}", kept.len() - 1)
            }};
        }

        macro_rules! flush {
            () => {{
                out.push_str(&escape(&pending));
                pending.clear();
            }};
        }

        while let Some(ch) = scanner.peek(0) {
            match ch {
                '\\' => {
                    scanner.advance();
                    if let Some(next) = scanner.peek(0) {
                        if next.is_ascii() && is_ascii_punct_or_symbol(next) {
                            flush!();
                            let esc = escape(&next.to_string());
                            out.push_str(&keep!(esc));
                            scanner.advance();
                        } else if next == '\n' {
                            flush!();
                            out.push_str(&keep!("<br>".to_string()));
                            scanner.advance();
                        } else {
                            pending.push('\\');
                        }
                    } else {
                        pending.push('\\');
                    }
                }
                '`' => {
                    let mark = scanner.run_of('`');
                    if let Some(code) = scanner.until_run(&mark) {
                        let mut code = code.replace('\n', " ");
                        let chars: Vec<char> = code.chars().collect();
                        if chars.len() >= 2
                            && chars[0] == ' '
                            && chars[chars.len() - 1] == ' '
                            && chars.iter().any(|&c| c != ' ')
                        {
                            code = chars[1..chars.len() - 1].iter().collect();
                        }
                        flush!();
                        out.push_str(&keep!(format!("<code>{}</code>", escape(&code))));
                    } else {
                        pending.push_str(&mark);
                    }
                }
                '!' if scanner.peek(1) == Some('[') => {
                    scanner.advance();
                    if let Some((alt, addr, title)) = self.link(&mut scanner) {
                        flush!();
                        if let Some(source) = address(&addr, self.base) {
                            let alt_plain = plain(&alt);
                            let caption = title
                                .map(|t| format!(" title=\"{}\"", escape(&t)))
                                .unwrap_or_default();
                            out.push_str(&keep!(format!(
                                "<img src=\"{}\" alt=\"{}\"{} loading=\"lazy\">",
                                escape(&source),
                                escape(&alt_plain),
                                caption
                            )));
                        }
                    } else {
                        pending.push('!');
                    }
                }
                '[' => {
                    let save = scanner.position;
                    if let Some((label_text, addr, title)) = self.link(&mut scanner) {
                        flush!();
                        let inner = self.render(&label_text);
                        if let Some(target) = address(&addr, self.base) {
                            let caption = title
                                .map(|t| format!(" title=\"{}\"", escape(&t)))
                                .unwrap_or_default();
                            out.push_str(&keep!(format!(
                                "<a href=\"{}\"{}>{}</a>",
                                escape(&target),
                                caption,
                                inner
                            )));
                        } else {
                            out.push_str(&keep!(inner));
                        }
                    } else {
                        scanner.position = save + 1;
                        pending.push('[');
                    }
                }
                '<' => {
                    if let Some((html, len)) = self.angle(scanner.rest()) {
                        flush!();
                        out.push_str(&keep!(html));
                        scanner.position += len;
                    } else {
                        pending.push('<');
                        scanner.advance();
                    }
                }
                'h' | 'w' => {
                    let apart = pending
                        .chars()
                        .last()
                        .map(|c| !c.is_alphanumeric())
                        .unwrap_or(true);
                    let rest = scanner.rest();
                    let looks = starts_with_chars(rest, "http") || starts_with_chars(rest, "www.");
                    if apart && looks {
                        if let Some((html, len)) = bare_url(rest) {
                            flush!();
                            out.push_str(&keep!(html));
                            scanner.position += len;
                        } else {
                            pending.push(ch);
                            scanner.advance();
                        }
                    } else {
                        pending.push(ch);
                        scanner.advance();
                    }
                }
                '\n' => {
                    if pending.ends_with("  ") {
                        while pending.ends_with(' ') {
                            pending.pop();
                        }
                        flush!();
                        out.push_str(&keep!("<br>".to_string()));
                    } else {
                        while pending.ends_with(' ') {
                            pending.pop();
                        }
                        pending.push('\n');
                    }
                    scanner.advance();
                }
                '&' => {
                    if let Some(entity) = scan_entity(scanner.rest()) {
                        flush!();
                        let len = entity.chars().count();
                        out.push_str(&keep!(entity));
                        scanner.position += len;
                    } else {
                        pending.push('&');
                        scanner.advance();
                    }
                }
                _ => {
                    pending.push(ch);
                    scanner.advance();
                }
            }
        }
        flush!();

        out = Self::emphasis(&out);
        for (index, html) in kept.iter().enumerate().rev() {
            let token = format!("\u{E000}{}\u{E001}", index);
            out = out.replace(&token, html);
        }
        out
    }

    fn emphasis(text: &str) -> String {
        if !text.contains('*') && !text.contains('_') && !text.contains('~') {
            return text.to_string();
        }
        let mut s = apply_double_marker(text, '*', false, "strong");
        s = apply_double_marker(&s, '_', true, "strong");
        s = apply_single_marker(&s, '*', false, "em");
        s = apply_single_marker(&s, '_', true, "em");
        s = apply_double_marker(&s, '~', false, "del");
        s
    }

    fn link(&self, scanner: &mut Scanner) -> Option<(String, String, Option<String>)> {
        let save = scanner.position;
        if scanner.peek(0) != Some('[') {
            return None;
        }
        scanner.advance();
        let mut depth = 1usize;
        let mut label_text = String::new();
        while let Some(ch) = scanner.peek(0) {
            scanner.advance();
            if ch == '\\' {
                if let Some(next) = scanner.peek(0) {
                    label_text.push(ch);
                    label_text.push(next);
                    scanner.advance();
                    continue;
                }
            }
            if ch == '`' {
                label_text.push(ch);
                while let Some(inner) = scanner.peek(0) {
                    scanner.advance();
                    label_text.push(inner);
                    if inner == '`' {
                        break;
                    }
                }
                continue;
            }
            if ch == '[' {
                depth += 1;
            } else if ch == ']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            label_text.push(ch);
        }
        if depth != 0 {
            scanner.position = save;
            return None;
        }

        if scanner.peek(0) == Some('(') {
            scanner.advance();
            if let Some((addr, title)) = scanner.destination(Some(')')) {
                return Some((label_text, addr, title));
            }
            scanner.position = save;
            return None;
        }

        let mut name = label_text.clone();
        if scanner.peek(0) == Some('[') {
            let back = scanner.position;
            scanner.advance();
            let mut written = String::new();
            while let Some(ch) = scanner.peek(0) {
                if ch == ']' || ch == '[' {
                    break;
                }
                written.push(ch);
                scanner.advance();
            }
            if scanner.peek(0) == Some(']') {
                scanner.advance();
                if !written.is_empty() {
                    name = written;
                }
            } else {
                scanner.position = back;
            }
        }
        if let Some((addr, title)) = self.references.get(&label(&name)) {
            return Some((label_text, addr.clone(), title.clone()));
        }
        scanner.position = save;
        None
    }

    fn angle(&self, rest: &[char]) -> Option<(String, usize)> {
        let prefix: String = rest.iter().take(2000).collect();

        if let Some((inner, byte_len)) = match_angle_url(&prefix) {
            let char_len = prefix[..byte_len].chars().count();
            return match address(inner, self.base) {
                Some(target) => Some((
                    format!("<a href=\"{}\">{}</a>", escape(&target), escape(inner)),
                    char_len,
                )),
                None => Some((escape(inner), char_len)),
            };
        }

        if let Some((inner, byte_len)) = match_angle_email(&prefix) {
            let char_len = prefix[..byte_len].chars().count();
            return Some((
                format!("<a href=\"mailto:{}\">{}</a>", escape(inner), escape(inner)),
                char_len,
            ));
        }

        if prefix.starts_with("<!--") {
            if let Some(end) = prefix.find("-->") {
                let char_len = prefix[..end + 3].chars().count();
                return Some((String::new(), char_len));
            }
        }

        if let Some(byte_len) = match_html_tag_at_start(&prefix) {
            let tag_str = &prefix[..byte_len];
            let char_len = tag_str.chars().count();
            return Some((Self::tags(tag_str, self.base, false), char_len));
        }

        None
    }

    fn tags(text: &str, base: Option<&str>, standing: bool) -> String {
        let mut out = String::new();
        let mut pos = 0;
        while pos < text.len() {
            if text.as_bytes()[pos] == b'<' {
                if let Some(tag_len) = match_html_tag_at_start(&text[pos..]) {
                    let tag_src = &text[pos..pos + tag_len];
                    out.push_str(&Self::tag(tag_src, base, standing));
                    pos += tag_len;
                    continue;
                }
            }
            let ch = text[pos..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            out.push_str(&escape(&text[pos..pos + ch_len]));
            pos += ch_len;
        }
        out
    }

    fn tag(source: &str, base: Option<&str>, standing: bool) -> String {
        let closing = source.starts_with("</");
        let after = if closing { &source[2..] } else { &source[1..] };
        let name: String = after
            .chars()
            .take_while(|&c| c.is_alphanumeric() || c == '-')
            .collect();
        let lower = name.to_ascii_lowercase();

        if is_callout_tag(&lower) {
            return if closing {
                "</blockquote>".to_string()
            } else {
                "<blockquote>".to_string()
            };
        }

        let attrs = if closing {
            HashMap::new()
        } else {
            Self::attributes(source)
        };

        let is_lowercase_name = name == lower;
        let allowed = if is_lowercase_name {
            kept_attrs(&lower)
        } else {
            None
        };

        let Some(allowed_keys) = allowed else {
            if closing {
                return String::new();
            }
            let Some(title) = attrs.get("title").filter(|t| !t.is_empty()) else {
                return String::new();
            };
            let mut html = format!("<strong>{}</strong>", escape(title));
            if let Some(href) = attrs.get("href").and_then(|h| address(h, base)) {
                html = format!("<a href=\"{}\">{}</a>", escape(&href), html);
            }
            return if standing {
                format!("<p class=\"office-part\">{}</p>", html)
            } else {
                html
            };
        };

        if closing {
            return format!("</{}>", lower);
        }

        let mut sorted_keys: Vec<&str> = allowed_keys.iter().copied().collect();
        sorted_keys.sort_unstable();

        let mut html = format!("<{}", lower);
        for key in sorted_keys {
            let Some(mut value) = attrs.get(key).cloned() else {
                continue;
            };
            if key == "href" || key == "src" {
                match address(&value, base) {
                    Some(target) => value = target,
                    None => {
                        if key == "src" {
                            return String::new();
                        }
                        continue;
                    }
                }
            }
            html.push_str(&format!(" {}=\"{}\"", key, escape(&value)));
        }
        if lower == "img" {
            if !attrs.contains_key("src") {
                return String::new();
            }
            html.push_str(" loading=\"lazy\"");
        }
        html.push('>');
        html
    }

    fn attributes(source: &str) -> HashMap<String, String> {
        let mut found = HashMap::new();
        // Skip `<tagname` or `</tagname`
        let bytes = source.as_bytes();
        let mut i = if source.starts_with("</") { 2 } else { 1 };
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-')
        {
            i += 1;
        }
        while i < source.len() {
            while i < source.len() && source.as_bytes()[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= source.len() || source[i..].starts_with('>') || source[i..].starts_with("/>") {
                break;
            }
            let key_start = i;
            while i < source.len() {
                let c = source[i..].chars().next().unwrap();
                if c.is_whitespace() || matches!(c, '=' | '>' | '/' | '"' | '\'') {
                    break;
                }
                i += c.len_utf8();
            }
            let key = &source[key_start..i];
            if key.is_empty() {
                i += source[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                continue;
            }
            while i < source.len() && source.as_bytes()[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= source.len() || source.as_bytes()[i] != b'=' {
                continue;
            }
            i += 1; // '='
            while i < source.len() && source.as_bytes()[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= source.len() {
                break;
            }
            let first_b = source.as_bytes()[i];
            let val = if first_b == b'"' || first_b == b'\'' {
                let quote = first_b;
                i += 1;
                let v_start = i;
                while i < source.len() && source.as_bytes()[i] != quote {
                    i += 1;
                }
                let v = &source[v_start..i];
                if i < source.len() {
                    i += 1;
                }
                Some(unescape(v))
            } else if first_b == b'{' {
                let mut depth = 0usize;
                while i < source.len() {
                    let b = source.as_bytes()[i];
                    if b == b'{' {
                        depth += 1;
                    } else if b == b'}' {
                        depth = depth.saturating_sub(1);
                        i += 1;
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    i += 1;
                }
                None
            } else {
                let v_start = i;
                while i < source.len() {
                    let c = source[i..].chars().next().unwrap();
                    if c.is_whitespace()
                        || matches!(c, '"' | '\'' | '=' | '<' | '>' | '`' | '{' | '}')
                    {
                        break;
                    }
                    i += c.len_utf8();
                }
                let v = &source[v_start..i];
                if v.is_empty() {
                    None
                } else {
                    Some(unescape(v))
                }
            };
            if let Some(v) = val {
                found.insert(key.to_ascii_lowercase(), v);
            }
        }
        found
    }
}

fn is_callout_tag(lower: &str) -> bool {
    matches!(
        lower,
        "note"
            | "tip"
            | "info"
            | "warning"
            | "caution"
            | "danger"
            | "check"
            | "callout"
            | "aside"
            | "admonition"
    )
}

fn kept_attrs(lower: &str) -> Option<&'static [&'static str]> {
    match lower {
        "a" => Some(&["href", "title"]),
        "img" => Some(&["src", "alt", "title"]),
        "abbr" => Some(&["title"]),
        "th" | "td" => Some(&["align"]),
        "ol" => Some(&["start"]),
        "br" | "em" | "i" | "strong" | "b" | "u" | "s" | "del" | "ins" | "code" | "kbd"
        | "mark" | "sub" | "sup" | "small" | "details" | "summary" | "figure" | "figcaption"
        | "table" | "thead" | "tbody" | "tr" | "blockquote" | "hr" | "p" | "pre" | "ul"
        | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(&[]),
        _ => None,
    }
}

fn is_ascii_punct_or_symbol(ch: char) -> bool {
    ch.is_ascii_punctuation()
}

fn starts_with_chars(rest: &[char], prefix: &str) -> bool {
    let mut it = rest.iter().copied();
    for pc in prefix.chars() {
        if it.next() != Some(pc) {
            return false;
        }
    }
    true
}

fn scan_entity(rest: &[char]) -> Option<String> {
    let window: String = rest.iter().take(12).collect();
    let end = window.find(';')?;
    let entity = &window[..=end];
    let inner = &entity[1..entity.len() - 1];
    if let Some(hex) = inner.strip_prefix("#x").or_else(|| inner.strip_prefix("#X")) {
        if (1..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(entity.to_string());
        }
    } else if let Some(dec) = inner.strip_prefix('#') {
        if (1..=7).contains(&dec.len()) && dec.chars().all(|c| c.is_ascii_digit()) {
            return Some(entity.to_string());
        }
    } else if (2..=11).contains(&inner.len()) {
        let mut chars = inner.chars();
        if chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
            && chars.all(|c| c.is_ascii_alphanumeric())
        {
            return Some(entity.to_string());
        }
    }
    None
}

fn bare_url(rest: &[char]) -> Option<(String, usize)> {
    let s: String = rest.iter().take(2000).collect();
    if !(s.starts_with("http://") || s.starts_with("https://") || s.starts_with("www.")) {
        return None;
    }
    let mut found = String::new();
    for ch in s.chars() {
        if ch.is_whitespace() || ch == '<' || ch == '>' {
            break;
        }
        found.push(ch);
    }
    loop {
        let Some(last) = found.chars().last() else {
            break;
        };
        let trailing_punct = matches!(
            last,
            '.' | ',' | ':' | ';' | '!' | '?' | '"' | '\'' | '*' | '_' | '~'
        );
        let unmatched_paren = last == ')'
            && found.chars().filter(|&c| c == '(').count()
                < found.chars().filter(|&c| c == ')').count();
        if trailing_punct || unmatched_paren {
            found.pop();
        } else {
            break;
        }
    }
    let char_count = found.chars().count();
    if char_count <= 7 {
        return None;
    }
    let raw = if found.starts_with("www.") {
        format!("https://{}", found)
    } else {
        found.clone()
    };
    let target = address(&raw, None)?;
    Some((
        format!("<a href=\"{}\">{}</a>", escape(&target), escape(&found)),
        char_count,
    ))
}

fn match_angle_url(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('<') {
        return None;
    }
    let close = s[1..].find('>')? + 1;
    let inner = &s[1..close];
    let colon = inner.find(':')?;
    let scheme = &inner[..colon];
    if !(2..=32).contains(&scheme.len()) {
        return None;
    }
    let mut chars = scheme.chars();
    if !chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    if inner[colon + 1..]
        .chars()
        .any(|c| c.is_whitespace() || c == '<' || c == '>')
    {
        return None;
    }
    Some((inner, close + 1))
}

fn match_angle_email(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('<') {
        return None;
    }
    let close = s[1..].find('>')? + 1;
    let inner = &s[1..close];
    let (local, domain) = inner.split_once('@')?;
    if local.is_empty()
        || !local.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '.' | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '/' | '=' | '?' | '^' | '_' | '`' | '{' | '|' | '}' | '~' | '-'
                )
        })
    {
        return None;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2
        || labels
            .iter()
            .any(|l| l.is_empty() || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        return None;
    }
    Some((inner, close + 1))
}

fn match_html_tag_at_start(s: &str) -> Option<usize> {
    if !s.starts_with('<') {
        return None;
    }
    let mut i = if s.starts_with("</") { 2 } else { 1 };
    if i >= s.len() {
        return None;
    }
    let first = s[i..].chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    i += first.len_utf8();
    while i < s.len() && (s.as_bytes()[i].is_ascii_alphanumeric() || s.as_bytes()[i] == b'-') {
        i += 1;
    }
    // Parse zero or more attributes separated by whitespace
    loop {
        let ws_start = i;
        while i < s.len() && s.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= s.len() {
            return None;
        }
        if s.as_bytes()[i] == b'>' {
            return Some(i + 1);
        }
        if s[i..].starts_with("/>") {
            return Some(i + 2);
        }
        if i == ws_start {
            return None;
        }
        // Attribute name
        let attr_start = i;
        while i < s.len() {
            let c = s[i..].chars().next()?;
            if c.is_whitespace() || matches!(c, '=' | '>' | '/' | '"' | '\'') {
                break;
            }
            i += c.len_utf8();
        }
        if i == attr_start {
            return None;
        }
        let save = i;
        while i < s.len() && s.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < s.len() && s.as_bytes()[i] == b'=' {
            i += 1;
            while i < s.len() && s.as_bytes()[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= s.len() {
                return None;
            }
            let b = s.as_bytes()[i];
            if b == b'"' || b == b'\'' {
                i += 1;
                while i < s.len() && s.as_bytes()[i] != b {
                    i += 1;
                }
                if i >= s.len() {
                    return None;
                }
                i += 1;
            } else if b == b'{' {
                i += 1;
                while i < s.len() && s.as_bytes()[i] != b'}' {
                    i += 1;
                }
                if i >= s.len() {
                    return None;
                }
                i += 1;
            } else {
                let v_start = i;
                while i < s.len() {
                    let c = s[i..].chars().next()?;
                    if c.is_whitespace() || matches!(c, '"' | '\'' | '=' | '<' | '>' | '`') {
                        break;
                    }
                    i += c.len_utf8();
                }
                if i == v_start {
                    return None;
                }
            }
        } else {
            i = save;
        }
    }
}

fn is_word_char_for_underscore(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn apply_double_marker(input: &str, ch: char, word_boundary: bool, tag: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < n {
        if i + 4 < n
            && chars[i] == ch
            && chars[i + 1] == ch
            && (!word_boundary || i == 0 || !is_word_char_for_underscore(chars[i - 1]))
            && !chars[i + 2].is_whitespace()
        {
            // Find earliest closing `ch ch` where previous char is non-whitespace
            // and trailing char satisfies boundary
            let mut j = i + 3; // content must be at least 1 char (.+?)
            let mut matched_end = None;
            while j + 1 < n {
                if chars[j] == ch
                    && chars[j + 1] == ch
                    && !chars[j - 1].is_whitespace()
                    && (!word_boundary
                        || j + 2 >= n
                        || !is_word_char_for_underscore(chars[j + 2]))
                {
                    matched_end = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = matched_end {
                out.push('<');
                out.push_str(tag);
                out.push('>');
                for &c in &chars[i + 2..end] {
                    out.push(c);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
                i = end + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn apply_single_marker(input: &str, ch: char, word_boundary: bool, tag: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < n {
        if i + 2 < n
            && chars[i] == ch
            && (!word_boundary || i == 0 || !is_word_char_for_underscore(chars[i - 1]))
            && !chars[i + 1].is_whitespace()
            && chars[i + 1] != ch
        {
            let mut j = i + 2;
            let mut matched_end = None;
            while j < n {
                if chars[j] == ch
                    && !chars[j - 1].is_whitespace()
                    && chars[j - 1] != ch
                    && (!word_boundary
                        || j + 1 >= n
                        || !is_word_char_for_underscore(chars[j + 1]))
                {
                    matched_end = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = matched_end {
                out.push('<');
                out.push_str(tag);
                out.push('>');
                for &c in &chars[i + 1..end] {
                    out.push(c);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

struct Scanner {
    characters: Vec<char>,
    position: usize,
}

impl Scanner {
    fn new(text: &str) -> Self {
        Self {
            characters: text.chars().collect(),
            position: 0,
        }
    }

    fn rest(&self) -> &[char] {
        &self.characters[self.position..]
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.characters.get(self.position + ahead).copied()
    }

    fn advance(&mut self) {
        if self.position < self.characters.len() {
            self.position += 1;
        }
    }

    fn run_of(&mut self, ch: char) -> String {
        let mut run = String::new();
        while self.peek(0) == Some(ch) {
            run.push(ch);
            self.advance();
        }
        run
    }

    fn until_run(&mut self, mark: &str) -> Option<String> {
        let save = self.position;
        let mark_len = mark.chars().count();
        let mut text = String::new();
        while let Some(ch) = self.peek(0) {
            if ch == '`' {
                let run = self.run_of('`');
                if run.chars().count() == mark_len {
                    return Some(text);
                }
                text.push_str(&run);
            } else {
                text.push(ch);
                self.advance();
            }
        }
        self.position = save;
        None
    }

    fn destination(&mut self, closing: Option<char>) -> Option<(String, Option<String>)> {
        self.skip_spaces();
        let mut addr = String::new();
        if self.peek(0) == Some('<') {
            self.advance();
            while let Some(ch) = self.peek(0) {
                if ch == '>' || ch == '\n' {
                    break;
                }
                addr.push(ch);
                self.advance();
            }
            if self.peek(0) != Some('>') {
                return None;
            }
            self.advance();
        } else {
            let mut depth = 0usize;
            while let Some(ch) = self.peek(0) {
                if ch.is_whitespace() {
                    break;
                }
                if ch == '(' {
                    depth += 1;
                } else if ch == ')' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                } else if ch == '\\' {
                    if let Some(next) = self.peek(1) {
                        if next.is_ascii_punctuation() {
                            addr.push(next);
                            self.position += 2;
                            continue;
                        }
                    }
                }
                addr.push(ch);
                self.advance();
            }
        }
        self.skip_spaces();
        let mut title = None;
        if let Some(open) = self.peek(0) {
            if open == '"' || open == '\'' || open == '(' {
                let close = if open == '(' { ')' } else { open };
                self.advance();
                let mut text = String::new();
                while let Some(ch) = self.peek(0) {
                    if ch == close {
                        break;
                    }
                    text.push(ch);
                    self.advance();
                }
                if self.peek(0) != Some(close) {
                    return None;
                }
                self.advance();
                title = Some(text);
                self.skip_spaces();
            }
        }
        if let Some(close) = closing {
            if self.peek(0) != Some(close) {
                return None;
            }
            self.advance();
        } else if self.position < self.characters.len() {
            return None;
        }
        Some((addr, title))
    }

    fn skip_spaces(&mut self) {
        while let Some(ch) = self.peek(0) {
            if ch == ' ' || ch == '\n' {
                self.advance();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frontmatter_and_h1_lifting() {
        let md = "---\ntitle: Ignored When H1 Exists\ndate: 2026-01-01\n---\n\n# Real Article Title\n\nHello **world**!";
        let page = parse_markdown_page(md, Some("https://example.com/docs/intro"));
        assert_eq!(page.title.as_deref(), Some("Real Article Title"));
        assert_eq!(page.html, "<p>Hello <strong>world</strong>!</p>");
    }

    #[test]
    fn test_frontmatter_fallback_title_when_no_h1() {
        let md = "---\ntitle: \"Frontmatter Title\"\n---\n\n## Section\n\nBody copy.";
        let page = parse_markdown_page(md, None);
        assert_eq!(page.title.as_deref(), Some("Frontmatter Title"));
        assert!(page.html.contains("<h2 id=\"section\">Section</h2>"));
    }

    #[test]
    fn test_heading_slugs_deduplicated() {
        let md = "## Overview\n\n## Overview\n\n## Overview";
        let page = parse_markdown_page(md, None);
        assert!(page.html.contains("<h2 id=\"overview\">Overview</h2>"));
        assert!(page.html.contains("<h2 id=\"overview-1\">Overview</h2>"));
        assert!(page.html.contains("<h2 id=\"overview-2\">Overview</h2>"));
    }

    #[test]
    fn test_four_space_indent_is_not_code_block_mdx() {
        let md = "    Just indented MDX paragraph.\n\n```rust\nfn main() {}\n```";
        let page = parse_markdown_page(md, None);
        assert_eq!(
            page.html,
            "<p>Just indented MDX paragraph.</p>\n<pre><code class=\"language-rust\">fn main() {}</code></pre>"
        );
    }

    #[test]
    fn test_mdx_callouts_and_components() {
        let md = "<Note>\nImportant **callout**.\n</Note>\n\n<Card title=\"API Guide\" href=\"/api\">\n</Card>";
        let page = parse_markdown_page(md, Some("https://docs.example.com/v1/start"));
        assert!(page.html.contains("<blockquote>\n<p>Important <strong>callout</strong>.</p>\n</blockquote>"));
        assert!(page.html.contains(
            "<p class=\"office-part\"><a href=\"https://docs.example.com/api\"><strong>API Guide</strong></a></p>"
        ));
    }

    #[test]
    fn test_xss_sanitization_and_scheme_allowlist() {
        let md = "Bad [link](javascript:alert(1)) and <script>alert(1)</script> and <a href=\"https://ok.test\" onclick=\"evil()\">ok</a> and <img src=\"javascript:x\" onerror=\"evil()\">.";
        let page = parse_markdown_page(md, Some("https://safe.test/"));
        assert!(!page.html.contains("javascript:"));
        assert!(!page.html.contains("<script>"));
        assert!(!page.html.contains("onclick"));
        assert!(!page.html.contains("onerror"));
        assert!(page.html.contains("<a href=\"https://ok.test\">ok</a>"));
        assert!(page.html.contains("Bad link"));
    }

    #[test]
    fn test_lists_checkboxes_and_tables() {
        let md = "- [ ] Todo item\n- [x] Done item\n\n| Name | Score |\n| :--- | ---: |\n| Alice | 42 |";
        let page = parse_markdown_page(md, None);
        assert!(page.html.contains("<input type=\"checkbox\" disabled> Todo item"));
        assert!(page.html.contains("<input type=\"checkbox\" checked disabled> Done item"));
        assert!(page.html.contains("<div class=\"office-wide\"><table><thead><tr><th style=\"text-align:left\">Name</th><th style=\"text-align:right\">Score</th></tr></thead><tbody><tr><td style=\"text-align:left\">Alice</td><td style=\"text-align:right\">42</td></tr></tbody></table></div>"));
    }

    #[test]
    fn test_reference_links_and_relative_urls() {
        let md = "Read [the docs][docs] or [sub](../guide#top).\n\n[docs]: ./ref/api.md \"API Docs\"";
        let page = parse_markdown_page(md, Some("https://site.dev/v2/intro.md"));
        assert!(page
            .html
            .contains("<a href=\"https://site.dev/v2/ref/api.md\" title=\"API Docs\">the docs</a>"));
        assert!(page
            .html
            .contains("<a href=\"https://site.dev/guide#top\">sub</a>"));
    }

    #[test]
    fn test_llms_txt_blockquote_skipped() {
        let md = "> For agents: see https://example.com/llms.txt for an index.\n\n# Real Heading\n\nContent.";
        let page = parse_markdown_page(md, None);
        assert!(!page.html.contains("llms.txt"));
        assert_eq!(page.title.as_deref(), Some("Real Heading"));
        assert_eq!(page.html, "<p>Content.</p>");
    }

    #[test]
    fn test_emphasis_and_underscore_word_boundaries() {
        let md = "snake_case_var stays _emphasized_ while **bold** and ~~gone~~ work, and `*not italic*`.";
        let page = parse_markdown_page(md, None);
        assert!(page.html.contains("snake_case_var"));
        assert!(page.html.contains("<em>emphasized</em>"));
        assert!(page.html.contains("<strong>bold</strong>"));
        assert!(page.html.contains("<del>gone</del>"));
        assert!(page.html.contains("<code>*not italic*</code>"));
    }
}
