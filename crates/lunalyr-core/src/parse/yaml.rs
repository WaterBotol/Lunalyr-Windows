//! A small YAML reader for LRCLIB's `lyricsfile` documents.
//!
//! It covers the subset a YAML emitter produces for that schema: block
//! mappings and sequences (including `- key: value` entries and sequences
//! at their parent key's indentation), plain / single-quoted /
//! double-quoted scalars (escapes, multi-line folding), literal and folded
//! block scalars, simple flow collections (`[]`, `{a: 1}`), comments,
//! document markers, and anchors/tags (ignored). Aliases read as null.
//! Malformed input never panics; it yields whatever was understood.
//!
//! Documents arrive from the network (LRCLIB stores any lyricsfile as-is),
//! so the reader is bounded: nesting deeper than [`MAX_DEPTH`] makes the
//! whole document null, as libyaml's nesting limit does for Yams on macOS
//! (every level is a stack frame on a 2 MiB tokio worker), and every line
//! is scanned a bounded number of times (multi-line flow collections too).

/// Deepest nesting accepted (block nodes and flow collections together). A
/// lyricsfile needs about six levels.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Yaml {
    Null,
    /// The scalar text and whether it was quoted / block (never null-like).
    Scalar(String, bool),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub(crate) fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Null, `~`, `null` or an empty plain value.
    pub(crate) fn is_null(&self) -> bool {
        match self {
            Yaml::Null => true,
            Yaml::Scalar(text, false) => matches!(text.as_str(), "" | "~" | "null" | "Null" | "NULL"),
            _ => false,
        }
    }

    /// A number (plain or quoted). Err for non-numeric values.
    pub(crate) fn as_f64(&self) -> Result<Option<f64>, ()> {
        if self.is_null() {
            return Ok(None);
        }
        let Yaml::Scalar(text, _) = self else { return Err(()) };
        let t = text.trim();
        let value = match t {
            ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => f64::INFINITY,
            "-.inf" | "-.Inf" | "-.INF" => f64::NEG_INFINITY,
            ".nan" | ".NaN" | ".NAN" => f64::NAN,
            _ => {
                let lower = t.to_ascii_lowercase();
                // Rust also accepts "inf"/"nan" spellings YAML does not.
                if lower.contains("inf") || lower.contains("nan") {
                    return Err(());
                }
                t.parse::<f64>().map_err(|_| ())?
            }
        };
        Ok(Some(value))
    }

    /// The scalar's text; null-like plain values read as "".
    pub(crate) fn as_text(&self) -> Option<String> {
        match self {
            Yaml::Null => Some(String::new()),
            Yaml::Scalar(text, _) => Some(text.clone()),
            _ => None,
        }
    }
}

pub(crate) fn parse(text: &str) -> Yaml {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
    let mut parser = Parser { lines, pos: 0, depth: 0, too_deep: false };
    parser.skip_preamble();
    let doc = parser.node(-1);
    // Too deep is invalid as a whole (a decode error on macOS), not a partial document.
    if parser.too_deep {
        Yaml::Null
    } else {
        doc
    }
}

struct Parser {
    lines: Vec<String>,
    pos: usize,
    /// Block nodes currently being read (the recursion depth of `node`).
    depth: usize,
    /// Nesting went past [`MAX_DEPTH`]; the rest of the input is skipped.
    too_deep: bool,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_matches([' ', '\t']);
    t.is_empty() || t.starts_with('#')
}

fn is_document_end(line: &str) -> bool {
    line == "---" || line == "..." || line.starts_with("--- ") || line.starts_with("... ")
}

fn is_seq_entry(content: &str) -> bool {
    content == "-" || content.starts_with("- ") || content.starts_with("-\t")
}

/// Removes a ` #` comment from a plain value.
fn strip_comment(text: &str) -> &str {
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &text[..i];
        }
    }
    text
}

/// Parses a quoted scalar that must close on this text; returns the value
/// and the byte length consumed (including quotes).
fn quoted_single_line(text: &str) -> Option<(String, usize)> {
    let quote = text.chars().next()?;
    let (value, close) = read_quoted(quote, &mut text.char_indices().skip(1).peekable())?;
    Some((value, close + 1))
}

/// Decodes a quoted scalar from the characters after its opening `quote`.
/// Returns the value and the position (as `chars` numbers it) of the
/// closing quote; None when it never closes.
fn read_quoted(
    quote: char,
    chars: &mut std::iter::Peekable<impl Iterator<Item = (usize, char)>>,
) -> Option<(String, usize)> {
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        if quote == '\'' {
            if c == '\'' {
                if let Some(&(_, '\'')) = chars.peek() {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                return Some((out, i));
            }
            out.push(c);
        } else {
            if c == '"' {
                return Some((out, i));
            }
            if c == '\\' {
                let (_, e) = chars.next()?;
                push_escape(&mut out, e, chars);
                continue;
            }
            out.push(c);
        }
    }
    None
}

fn push_escape(out: &mut String, e: char, chars: &mut std::iter::Peekable<impl Iterator<Item = (usize, char)>>) {
    let hex = |chars: &mut std::iter::Peekable<_>, n: usize| -> Option<char> {
        let mut value = 0u32;
        for _ in 0..n {
            let (_, c): (usize, char) = chars.next()?;
            value = value * 16 + c.to_digit(16)?;
        }
        char::from_u32(value)
    };
    match e {
        '0' => out.push('\0'),
        'a' => out.push('\u{07}'),
        'b' => out.push('\u{08}'),
        't' | '\t' => out.push('\t'),
        'n' => out.push('\n'),
        'v' => out.push('\u{0B}'),
        'f' => out.push('\u{0C}'),
        'r' => out.push('\r'),
        'e' => out.push('\u{1B}'),
        ' ' => out.push(' '),
        '"' => out.push('"'),
        '/' => out.push('/'),
        '\\' => out.push('\\'),
        'N' => out.push('\u{85}'),
        '_' => out.push('\u{A0}'),
        'L' => out.push('\u{2028}'),
        'P' => out.push('\u{2029}'),
        'x' => out.push(hex(chars, 2).unwrap_or('\u{FFFD}')),
        'u' => out.push(hex(chars, 4).unwrap_or('\u{FFFD}')),
        'U' => out.push(hex(chars, 8).unwrap_or('\u{FFFD}')),
        other => {
            out.push('\\');
            out.push(other);
        }
    }
}

/// Finds `key:` at the start of block content. Returns the key and the
/// byte index just after the colon.
fn mapping_key(content: &str) -> Option<(String, usize)> {
    let first = content.chars().next()?;
    if first == '"' || first == '\'' {
        let (key, used) = quoted_single_line(content)?;
        let rest = &content[used..];
        let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let after = &rest[spaces..];
        if after.starts_with(':') && (after.len() == 1 || after[1..].starts_with([' ', '\t'])) {
            return Some((key, used + spaces + 1));
        }
        return None;
    }
    if matches!(first, '[' | '{' | '#' | '|' | '>' | '*' | '!' | '&' | '%' | '@' | '`') {
        return None;
    }
    let bytes = content.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return None;
        }
        if b == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
            return Some((content[..i].trim_end().to_string(), i + 1));
        }
    }
    None
}

impl Parser {
    fn skip_preamble(&mut self) {
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if line.starts_with('%') || is_blank_or_comment(line) || line == "---" {
                self.pos += 1;
            } else if let Some(rest) = line.strip_prefix("--- ") {
                // `--- value` on the marker line: keep the value.
                let rest = rest.trim_start().to_string();
                self.lines[self.pos] = rest;
                break;
            } else {
                break;
            }
        }
    }

    /// Moves to the next line with content; returns its indentation.
    fn next_significant(&mut self) -> Option<usize> {
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if is_document_end(line) {
                self.pos = self.lines.len();
                return None;
            }
            if is_blank_or_comment(line) {
                self.pos += 1;
                continue;
            }
            return Some(indent_of(line));
        }
        None
    }

    /// A block node whose lines are indented more than `parent`. Every block
    /// recursion (sequence items, mapping values, anchored values) passes
    /// through here, so this is where nesting is capped.
    fn node(&mut self, parent: i64) -> Yaml {
        if self.too_deep || self.depth >= MAX_DEPTH {
            self.too_deep = true;
            self.pos = self.lines.len();
            return Yaml::Null;
        }
        self.depth += 1;
        let value = self.node_inner(parent);
        self.depth -= 1;
        value
    }

    fn node_inner(&mut self, parent: i64) -> Yaml {
        let Some(indent) = self.next_significant() else { return Yaml::Null };
        if indent as i64 <= parent {
            return Yaml::Null;
        }
        // Borrowed, not copied: the line (up to 1 MB) must not be held once
        // per nesting level.
        let content = &self.lines[self.pos][indent..];
        if is_seq_entry(content) {
            return self.sequence(indent);
        }
        if mapping_key(content).is_some() {
            return self.mapping(indent);
        }
        self.inline(parent, indent)
    }

    fn sequence(&mut self, indent: usize) -> Yaml {
        let mut items = Vec::new();
        while let Some(i) = self.next_significant() {
            if i != indent {
                break;
            }
            let content = &self.lines[self.pos][i..];
            if !is_seq_entry(content) {
                break;
            }
            let after = &content[1..];
            let rest = after.trim_start_matches([' ', '\t']);
            if rest.is_empty() || rest.starts_with('#') {
                self.pos += 1;
            } else {
                // `- value`: the value becomes the line, at its own column.
                let column = i + 1 + (after.len() - rest.len());
                let line = format!("{}{}", " ".repeat(column), rest);
                self.lines[self.pos] = line;
            }
            items.push(self.node(indent as i64));
        }
        Yaml::Seq(items)
    }

    fn mapping(&mut self, indent: usize) -> Yaml {
        let mut entries = Vec::new();
        while let Some(i) = self.next_significant() {
            if i != indent {
                break;
            }
            let content = &self.lines[self.pos][i..];
            if is_seq_entry(content) {
                break;
            }
            let Some((key, after)) = mapping_key(content) else { break };
            let rest = content[after..].trim_start_matches([' ', '\t']);
            let value = if rest.is_empty() || rest.starts_with('#') {
                self.pos += 1;
                self.block_value(indent)
            } else {
                let column = i + (content.len() - rest.len());
                self.inline(indent as i64, column)
            };
            entries.push((key, value));
        }
        Yaml::Map(entries)
    }

    /// The value of a `key:` with nothing after the colon.
    fn block_value(&mut self, key_indent: usize) -> Yaml {
        let Some(i) = self.next_significant() else { return Yaml::Null };
        if i > key_indent {
            return self.node(key_indent as i64);
        }
        if i == key_indent && is_seq_entry(&self.lines[self.pos][i..]) {
            return self.sequence(i);
        }
        Yaml::Null
    }

    /// A value starting at `column` of the current line; continuation lines
    /// must be indented more than `parent`.
    fn inline(&mut self, parent: i64, mut column: usize) -> Yaml {
        loop {
            let text = &self.lines[self.pos][column..];
            match text.chars().next() {
                Some('"') | Some('\'') => return self.quoted(column),
                Some('|') | Some('>') => {
                    let header = text.to_string();
                    self.pos += 1;
                    return self.block_scalar(parent, &header);
                }
                Some('[') | Some('{') => return self.flow(column),
                Some('*') => {
                    self.pos += 1;
                    return Yaml::Null;
                }
                Some('&') | Some('!') => {
                    // Anchor or tag: skip the token and read what follows
                    // (a loop, so a line of thousands of tokens cannot recurse).
                    let token_end = text.find([' ', '\t']).unwrap_or(text.len());
                    let rest = text[token_end..].trim_start_matches([' ', '\t']);
                    if rest.is_empty() || rest.starts_with('#') {
                        self.pos += 1;
                        return if parent < 0 { self.node(-1) } else { self.block_value(parent as usize) };
                    }
                    column += text.len() - rest.len();
                }
                _ => return self.plain(parent, column),
            }
        }
    }

    fn plain(&mut self, parent: i64, column: usize) -> Yaml {
        let first = strip_comment(&self.lines[self.pos][column..]).trim_end_matches([' ', '\t']).to_string();
        let mut out = first;
        self.pos += 1;
        let mut breaks = 0;
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if is_document_end(line) {
                break;
            }
            let trimmed = line.trim_matches([' ', '\t']);
            if trimmed.is_empty() {
                breaks += 1;
                self.pos += 1;
                continue;
            }
            if indent_of(line) as i64 <= parent || trimmed.starts_with('#') {
                break;
            }
            let piece = strip_comment(trimmed).trim_end_matches([' ', '\t']);
            if breaks == 0 {
                out.push(' ');
            } else {
                out.push_str(&"\n".repeat(breaks));
            }
            out.push_str(piece);
            breaks = 0;
            self.pos += 1;
        }
        Yaml::Scalar(out, false)
    }

    /// Single- or double-quoted scalar, possibly spanning lines.
    fn quoted(&mut self, column: usize) -> Yaml {
        let quote = self.lines[self.pos][column..].chars().next().unwrap_or('"');
        let mut out = String::new();
        let mut text = self.lines[self.pos][column + 1..].to_string();
        let mut first = true;
        let mut breaks = 0usize;
        let mut escaped_break = false;
        loop {
            let content = if first { text.as_str() } else { text.trim_start_matches([' ', '\t']) };
            if !first && content.is_empty() {
                breaks += 1;
            } else {
                if !first {
                    if escaped_break {
                        // `\` at a line end joins without a space.
                    } else if breaks == 0 {
                        out.push(' ');
                    } else {
                        out.push_str(&"\n".repeat(breaks));
                    }
                }
                breaks = 0;
                escaped_break = false;
                // Length of `out` that trailing-space trimming must keep.
                let mut keep = out.len();
                let mut chars = content.char_indices().peekable();
                let mut closed = false;
                while let Some((_, c)) = chars.next() {
                    if c == quote {
                        if quote == '\'' {
                            if let Some(&(_, '\'')) = chars.peek() {
                                chars.next();
                                out.push('\'');
                                keep = out.len();
                                continue;
                            }
                        }
                        closed = true;
                        break;
                    }
                    if quote == '"' && c == '\\' {
                        match chars.next() {
                            None => {
                                escaped_break = true;
                                break;
                            }
                            Some((_, e)) => {
                                push_escape(&mut out, e, &mut chars);
                                keep = out.len();
                            }
                        }
                        continue;
                    }
                    out.push(c);
                    if c != ' ' && c != '\t' {
                        keep = out.len();
                    }
                }
                if closed {
                    self.pos += 1;
                    return Yaml::Scalar(out, true);
                }
                if !escaped_break {
                    out.truncate(keep);
                }
            }
            first = false;
            self.pos += 1;
            if self.pos >= self.lines.len() {
                // Unterminated: keep what was read.
                return Yaml::Scalar(out, true);
            }
            text = self.lines[self.pos].clone();
        }
    }

    fn block_scalar(&mut self, parent: i64, header: &str) -> Yaml {
        let header = strip_comment(header).trim();
        let folded = header.starts_with('>');
        let mut chomp = 'c';
        let mut explicit: Option<usize> = None;
        for c in header.chars().skip(1) {
            match c {
                '-' => chomp = '-',
                '+' => chomp = '+',
                d if d.is_ascii_digit() && d != '0' => explicit = d.to_digit(10).map(|v| v as usize),
                _ => {}
            }
        }
        let base = if parent < 0 { 0 } else { parent as usize + 1 };
        let mut content_indent = explicit.map(|e| if parent < 0 { e } else { parent as usize + e });
        let mut lines: Vec<String> = Vec::new();
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            let blank = line.trim_matches(' ').is_empty();
            if blank {
                lines.push(String::new());
                self.pos += 1;
                continue;
            }
            let ind = indent_of(line);
            let needed = *content_indent.get_or_insert(ind.max(base));
            if ind < needed || ind < base || is_document_end(line) {
                break;
            }
            lines.push(line[needed..].to_string());
            self.pos += 1;
        }
        let trailing = lines.iter().rev().take_while(|l| l.is_empty()).count();
        let body = &lines[..lines.len() - trailing];
        let mut out = String::new();
        if folded {
            let mut previous_normal = false;
            let mut empties = 0;
            let mut started = false;
            for line in body {
                if line.is_empty() {
                    empties += 1;
                    continue;
                }
                let more = line.starts_with([' ', '\t']);
                if started {
                    if previous_normal && !more {
                        if empties == 0 {
                            out.push(' ');
                        } else {
                            out.push_str(&"\n".repeat(empties));
                        }
                    } else {
                        out.push_str(&"\n".repeat(empties + 1));
                    }
                } else {
                    out.push_str(&"\n".repeat(empties));
                }
                out.push_str(line);
                started = true;
                previous_normal = !more;
                empties = 0;
            }
        } else {
            out = body.join("\n");
        }
        match chomp {
            '-' => {}
            '+' => {
                if !body.is_empty() {
                    out.push('\n');
                }
                out.push_str(&"\n".repeat(trailing));
            }
            _ => {
                if !body.is_empty() {
                    out.push('\n');
                }
            }
        }
        Yaml::Scalar(out, true)
    }

    /// `[ ... ]` / `{ ... }`, joined across lines until balanced. The scan
    /// state carries over from line to line, so each line is scanned once.
    fn flow(&mut self, column: usize) -> Yaml {
        let mut text = self.lines[self.pos][column..].to_string();
        let mut scan = FlowScan::default();
        let mut closed = scan.feed(&text);
        self.pos += 1;
        while !closed && self.pos < self.lines.len() && !is_document_end(&self.lines[self.pos]) {
            let start = text.len();
            text.push(' ');
            text.push_str(self.lines[self.pos].trim());
            closed = scan.feed(&text[start..]);
            self.pos += 1;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut index = 0;
        let mut too_deep = false;
        let value = flow_value(&chars, &mut index, false, self.depth, &mut too_deep);
        if too_deep {
            self.too_deep = true;
            self.pos = self.lines.len();
        }
        value
    }
}

/// Whether a flow collection has closed, fed one line at a time. After
/// each piece it answers what a scan of everything fed so far would: true
/// once the outermost bracket closes or a comment starts outside it.
#[derive(Default)]
struct FlowScan {
    depth: i32,
    quote: Option<char>,
    /// The previous piece ended with a `\` inside double quotes.
    escape: bool,
}

impl FlowScan {
    fn feed(&mut self, piece: &str) -> bool {
        let mut chars = piece.chars().peekable();
        while let Some(c) = chars.next() {
            if self.escape {
                self.escape = false;
                continue;
            }
            match self.quote {
                Some('"') => {
                    if c == '\\' {
                        self.escape = true;
                    } else if c == '"' {
                        self.quote = None;
                    }
                }
                Some(_) => {
                    // `''` is an escaped quote. Every piece after the first
                    // starts with the joining space, so a quote that ends a
                    // piece is never the first half of a `''`.
                    if c == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            self.quote = None;
                        }
                    }
                }
                None => match c {
                    '"' | '\'' => self.quote = Some(c),
                    '[' | '{' => self.depth += 1,
                    ']' | '}' => {
                        self.depth -= 1;
                        if self.depth <= 0 {
                            return true;
                        }
                    }
                    '#' if self.depth == 0 => return true,
                    _ => {}
                },
            }
        }
        self.depth <= 0
    }
}

fn skip_flow_space(chars: &[char], index: &mut usize) {
    while *index < chars.len() && matches!(chars[*index], ' ' | '\t') {
        *index += 1;
    }
}

/// One flow value at nesting level `depth`; past [`MAX_DEPTH`] it sets
/// `too_deep` and consumes the rest of the text.
fn flow_value(chars: &[char], index: &mut usize, in_map: bool, depth: usize, too_deep: &mut bool) -> Yaml {
    skip_flow_space(chars, index);
    let Some(&c) = chars.get(*index) else { return Yaml::Null };
    if depth > MAX_DEPTH {
        *too_deep = true;
        *index = chars.len();
        return Yaml::Null;
    }
    match c {
        '[' => {
            *index += 1;
            let mut items = Vec::new();
            loop {
                skip_flow_space(chars, index);
                match chars.get(*index) {
                    None => break,
                    Some(']') => {
                        *index += 1;
                        break;
                    }
                    Some(',') => {
                        *index += 1;
                    }
                    Some(_) => {
                        let before = *index;
                        items.push(flow_value(chars, index, false, depth + 1, too_deep));
                        if *index == before {
                            *index += 1;
                        }
                    }
                }
            }
            Yaml::Seq(items)
        }
        '{' => {
            *index += 1;
            let mut entries = Vec::new();
            loop {
                skip_flow_space(chars, index);
                match chars.get(*index) {
                    None => break,
                    Some('}') => {
                        *index += 1;
                        break;
                    }
                    Some(',') => {
                        *index += 1;
                    }
                    Some(_) => {
                        let before = *index;
                        let key = flow_value(chars, index, true, depth + 1, too_deep).as_text().unwrap_or_default();
                        skip_flow_space(chars, index);
                        let value = if chars.get(*index) == Some(&':') {
                            *index += 1;
                            skip_flow_space(chars, index);
                            if matches!(chars.get(*index), Some(',') | Some('}') | None) {
                                Yaml::Null
                            } else {
                                flow_value(chars, index, true, depth + 1, too_deep)
                            }
                        } else {
                            Yaml::Null
                        };
                        entries.push((key, value));
                        if *index == before {
                            *index += 1;
                        }
                    }
                }
            }
            Yaml::Map(entries)
        }
        '"' | '\'' => {
            // Decoded in place: copying the rest of the text for every
            // scalar would make a long collection quadratic.
            let after = &chars[*index + 1..];
            match read_quoted(c, &mut after.iter().copied().enumerate().peekable()) {
                Some((value, close)) => {
                    *index += close + 2;
                    Yaml::Scalar(value, true)
                }
                None => {
                    *index = chars.len();
                    Yaml::Scalar(after.iter().collect(), true)
                }
            }
        }
        _ => {
            let start = *index;
            while *index < chars.len() {
                let c = chars[*index];
                if c == ',' || c == ']' || c == '}' {
                    break;
                }
                if c == ':'
                    && in_map
                    && matches!(chars.get(*index + 1), None | Some(' ') | Some('\t') | Some(',') | Some('}'))
                {
                    break;
                }
                if c == '#' && *index > start && matches!(chars[*index - 1], ' ' | '\t') {
                    *index = chars.len();
                    break;
                }
                *index += 1;
            }
            let text: String = chars[start..(*index).min(chars.len())].iter().collect();
            Yaml::Scalar(text.trim().to_string(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Yaml {
        Yaml::Scalar(text.into(), false)
    }
    fn q(text: &str) -> Yaml {
        Yaml::Scalar(text.into(), true)
    }

    #[test]
    fn block_structures() {
        let doc =
            parse("a: 1\nb:\n  c: two words # comment\n  d:\n  - x\n  - y: 3\n    z: 4\ne:\n- p\n-\n  - nested\n");
        assert_eq!(doc.get("a"), Some(&s("1")));
        let b = doc.get("b").unwrap();
        assert_eq!(b.get("c"), Some(&s("two words")));
        match b.get("d").unwrap() {
            Yaml::Seq(items) => {
                assert_eq!(items[0], s("x"));
                assert_eq!(items[1].get("y"), Some(&s("3")));
                assert_eq!(items[1].get("z"), Some(&s("4")));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(doc.get("e"), Some(&Yaml::Seq(vec![s("p"), Yaml::Seq(vec![s("nested")])])));
    }

    #[test]
    fn quoted_scalars_and_escapes() {
        let doc = parse(
            "a: 'it''s # not a comment'\nb: \"tab\\there \\\"q\\\" \\u00e9\\x41 \\U0001F600\"\n\"c d\": \"x: y\"\n",
        );
        assert_eq!(doc.get("a"), Some(&q("it's # not a comment")));
        assert_eq!(doc.get("b"), Some(&q("tab\there \"q\" éA 😀")));
        assert_eq!(doc.get("c d"), Some(&q("x: y")));
    }

    #[test]
    fn multi_line_scalars_fold() {
        let doc = parse(
            "a: first part\n  second part\n\n  new paragraph\nb: \"one\n  two \\\n  three\"\nc: 'x\n\n  y'\nd: done\n",
        );
        assert_eq!(doc.get("a"), Some(&s("first part second part\nnew paragraph")));
        assert_eq!(doc.get("b"), Some(&q("one two three")), "space before an escaped break is kept");
        assert_eq!(doc.get("c"), Some(&q("x\ny")));
        assert_eq!(doc.get("d"), Some(&s("done")));
    }

    #[test]
    fn block_scalars() {
        let doc = parse("a: |\n  line one\n  line two\nb: >-\n  folded\n  text\n\n  para\nc: |+\n  keep\n\nd: x\n");
        assert_eq!(doc.get("a"), Some(&q("line one\nline two\n")));
        assert_eq!(doc.get("b"), Some(&q("folded text\npara")));
        assert_eq!(doc.get("c"), Some(&q("keep\n\n")));
        assert_eq!(doc.get("d"), Some(&s("x")));
    }

    #[test]
    fn flow_collections_and_markers() {
        let doc = parse("%YAML 1.2\n---\nwords: []\nm: {a: 1, b: \"two, three\", c: [x, 'y']}\nn: [\n  1,\n  2\n]\n...\nignored: true\n");
        assert_eq!(doc.get("words"), Some(&Yaml::Seq(vec![])));
        let m = doc.get("m").unwrap();
        assert_eq!(m.get("a"), Some(&s("1")));
        assert_eq!(m.get("b"), Some(&q("two, three")));
        assert_eq!(m.get("c"), Some(&Yaml::Seq(vec![s("x"), q("y")])));
        assert_eq!(doc.get("n"), Some(&Yaml::Seq(vec![s("1"), s("2")])));
        assert!(doc.get("ignored").is_none());
    }

    #[test]
    fn tags_anchors_nulls_and_numbers() {
        let doc = parse("a: !!str 12\nb: &anchor 5\nc: *anchor\nd: ~\ne:\nf: null\ng: -1.5e3\n");
        assert_eq!(doc.get("a"), Some(&s("12")));
        assert_eq!(doc.get("b").unwrap().as_f64(), Ok(Some(5.0)));
        assert!(doc.get("c").unwrap().is_null());
        assert!(doc.get("d").unwrap().is_null());
        assert!(doc.get("e").unwrap().is_null());
        assert!(doc.get("f").unwrap().is_null());
        assert_eq!(doc.get("g").unwrap().as_f64(), Ok(Some(-1500.0)));
        assert_eq!(s("abc").as_f64(), Err(()));
        assert_eq!(s("inf").as_f64(), Err(()));
        assert!(s(".nan").as_f64().unwrap().unwrap().is_nan());
    }

    /// Unwraps `levels` nested one-item sequences.
    fn innermost(mut doc: Yaml, levels: usize) -> Yaml {
        for _ in 0..levels {
            let Yaml::Seq(mut items) = doc else { panic!("expected a sequence") };
            assert_eq!(items.len(), 1);
            doc = items.remove(0);
        }
        doc
    }

    #[test]
    fn nesting_past_the_limit_makes_the_document_null() {
        // Block: the scalar sits one level below the last sequence.
        let block = |levels: usize| "- ".repeat(levels) + "x";
        assert_eq!(innermost(parse(&block(MAX_DEPTH - 1)), MAX_DEPTH - 1), s("x"));
        assert_eq!(parse(&block(MAX_DEPTH)), Yaml::Null);
        // Flow: the innermost (empty) sequence sits at level `levels`.
        let flow = |levels: usize| "[".repeat(levels) + &"]".repeat(levels);
        assert_eq!(innermost(parse(&flow(MAX_DEPTH)), MAX_DEPTH - 1), Yaml::Seq(vec![]));
        assert_eq!(parse(&flow(MAX_DEPTH + 1)), Yaml::Null);
        // Block and flow levels add up; mappings and anchored values count too.
        let mixed = "- ".repeat(MAX_DEPTH / 2) + &"[".repeat(MAX_DEPTH / 2 + 1);
        assert_eq!(parse(&mixed), Yaml::Null);
        let mappings: String = (0..=MAX_DEPTH).map(|i| format!("{}a:\n", " ".repeat(i))).collect();
        assert_eq!(parse(&mappings), Yaml::Null);
        assert_eq!(parse(&("&a\n".repeat(MAX_DEPTH) + "x")), Yaml::Null);
        assert_eq!(parse(&("&a\n".repeat(MAX_DEPTH - 1) + "x")), s("x"));
    }

    #[test]
    fn the_limit_is_on_depth_not_on_size() {
        let doc = "lines:\n".to_string() + &"- a: 1\n  b: [x, {c: 'd'}]\n".repeat(5_000);
        let Some(Yaml::Seq(items)) = parse(&doc).get("lines").cloned() else { panic!("expected a sequence") };
        assert_eq!(items.len(), 5_000);
        assert_eq!(items[4_999].get("b"), Some(&Yaml::Seq(vec![s("x"), Yaml::Map(vec![("c".into(), q("d"))])])));
    }

    #[test]
    fn runs_of_anchors_and_tags_are_skipped_without_recursion() {
        assert_eq!(parse("a: &x !t &y !!str value").get("a"), Some(&s("value")));
        let many = "a: ".to_string() + &"&a !t ".repeat(100_000) + "value";
        assert_eq!(parse(&many).get("a"), Some(&s("value")));
    }

    /// The scan `flow` used before [`FlowScan`]: the whole text, every time.
    fn whole_text_balanced(text: &str) -> bool {
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match quote {
                Some('"') => {
                    if c == '\\' {
                        chars.next();
                    } else if c == '"' {
                        quote = None;
                    }
                }
                Some(_) => {
                    if c == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            quote = None;
                        }
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '[' | '{' => depth += 1,
                    ']' | '}' => {
                        depth -= 1;
                        if depth <= 0 {
                            return true;
                        }
                    }
                    '#' if depth == 0 => return true,
                    _ => {}
                },
            }
        }
        depth <= 0
    }

    #[test]
    fn line_by_line_flow_scan_matches_a_whole_text_scan() {
        let documents = [
            "[a,\nb]",
            "[\"x\\\ny\", z]",
            "[\"x\\\\\n\"]",
            "['ab'\n'c', d]",
            "['it''\n''s']",
            "{a: [1,\n2], b: '#'}\n# after",
            "[a] # comment",
            "[\n\n\n]",
            "[{]\n}]",
            "['open\n]\n]'\n]",
        ];
        for document in documents {
            let lines: Vec<&str> = document.split('\n').collect();
            let mut scan = FlowScan::default();
            let mut text = lines[0].to_string();
            assert_eq!(scan.feed(&text), whole_text_balanced(&text), "{document:?}");
            for line in &lines[1..] {
                if whole_text_balanced(&text) {
                    break;
                }
                let start = text.len();
                text.push(' ');
                text.push_str(line.trim());
                assert_eq!(scan.feed(&text[start..]), whole_text_balanced(&text), "{document:?} at {text:?}");
            }
        }
    }

    #[test]
    fn quoted_flow_scalars_decode_in_place() {
        let doc = parse("a: [\"x\\u00e9\", 'it''s', \"q\\\"\", 'open");
        assert_eq!(doc.get("a"), Some(&Yaml::Seq(vec![q("xé"), q("it's"), q("q\""), q("open")])));
        // A `\x` escape swallows the closing quote: unterminated, kept raw.
        assert_eq!(parse("a: [\"\\x\", b]").get("a"), Some(&Yaml::Seq(vec![q("\\x\", b]")])));
        assert_eq!(parse("a: {'k': \"v\", \"k2\": 'w'}").get("a").unwrap().get("k2"), Some(&q("w")));
    }

    #[test]
    fn long_flow_collections_parse_in_linear_time() {
        // Both took minutes when every line rescanned the whole collection
        // and every quoted scalar copied the rest of the text.
        let started = std::time::Instant::now();
        let unclosed = "lines: [\n".to_string() + &"a,\n".repeat(200_000);
        let Some(Yaml::Seq(items)) = parse(&unclosed).get("lines").cloned() else { panic!("expected a sequence") };
        assert_eq!(items.len(), 200_000);
        let quoted = "lines: [".to_string() + &"'',".repeat(200_000) + "]";
        let Some(Yaml::Seq(items)) = parse(&quoted).get("lines").cloned() else { panic!("expected a sequence") };
        assert_eq!(items.len(), 200_000);
        assert_eq!(items[0], q(""));
        let blank = "lines: [\n".to_string() + &"\n".repeat(500_000);
        assert_eq!(parse(&blank).get("lines"), Some(&Yaml::Seq(vec![])));
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(5), "took {elapsed:?}");
    }

    #[test]
    fn garbage_never_panics() {
        for text in [
            "",
            ":",
            "- - -",
            "a: [1, {b: ",
            "\"unterminated",
            "a:\n\t- x",
            "? complex\n: key",
            "a: |\n",
            "|",
            "- |\n  x",
            "{",
            "a: b: c",
        ] {
            let _ = parse(text);
        }
    }
}
