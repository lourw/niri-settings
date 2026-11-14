//! A small, lossless KDL v1 parser/editor tailored to niri configs.
//!
//! niri still uses KDL v1 syntax (`true`, `r#"..."#`), and users' config files
//! carry lots of comments and hand alignment. Rather than round-tripping
//! through a DOM (which would reformat everything), this module parses the
//! text into nodes that remember their byte spans, and edits are applied as
//! targeted text splices. Everything outside the edited span is untouched.

use std::fmt::Write as _;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Serialize as KDL v1.
    pub fn to_kdl(&self) -> String {
        match self {
            Value::String(s) => quote(s),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
        }
    }
}

pub fn format_float(f: f64) -> String {
    // Round away binary noise from sliders, but always keep a decimal point:
    // niri's decoder rejects integers where it expects floats.
    let mut s = format!("{:.4}", f);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    if s == "-0.0" {
        s = "0.0".into();
    }
    s
}

pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Serialize a node name / property key, quoting only when needed.
pub fn ident(s: &str) -> String {
    let plain = !s.is_empty()
        && !s.chars().next().unwrap().is_ascii_digit()
        && !matches!(s, "true" | "false" | "null")
        && !(s.starts_with('-') && s[1..].starts_with(|c: char| c.is_ascii_digit()))
        && !(s.starts_with('r') && s[1..].starts_with(['"', '#']))
        && s.chars().all(|c| !is_special(c) && !c.is_whitespace());
    if plain { s.to_string() } else { quote(s) }
}

fn is_special(c: char) -> bool {
    matches!(c, '\\' | '/' | '(' | ')' | '{' | '}' | '<' | '>' | ';' | '[' | ']' | '=' | ',' | '"')
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: Option<String>,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// Start of the node (type annotation or name).
    pub start: usize,
    /// End of the node's last significant token (excludes `;` and comments).
    pub end: usize,
    /// Start of the line if the node is the first thing on its line, else `start`.
    pub line_start: usize,
    pub entries: Vec<Entry>,
    pub children: Option<Children>,
}

#[derive(Debug, Clone)]
pub struct Children {
    /// Byte offset of `{`.
    pub open: usize,
    /// Byte offset of `}`.
    pub close: usize,
    pub nodes: Vec<Node>,
}

impl Node {
    pub fn args(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().filter(|e| e.key.is_none()).map(|e| &e.value)
    }
    pub fn arg(&self, i: usize) -> Option<&Value> {
        self.args().nth(i)
    }
    pub fn prop(&self, key: &str) -> Option<&Value> {
        // Last one wins, as in KDL.
        self.entries
            .iter()
            .rev()
            .find(|e| e.key.as_deref() == Some(key))
            .map(|e| &e.value)
    }
    pub fn children(&self) -> &[Node] {
        self.children.as_ref().map(|c| c.nodes.as_slice()).unwrap_or(&[])
    }
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub text: String,
    pub nodes: Vec<Node>,
}

impl Document {
    pub fn parse(text: impl Into<String>) -> Result<Self, ParseError> {
        let text = text.into();
        let nodes = Parser { s: &text, b: text.as_bytes(), i: 0 }.parse_document()?;
        Ok(Document { text, nodes })
    }

    /// Find the last node matching `path` (niri lets later definitions win).
    pub fn find(&self, path: &[&str]) -> Option<&Node> {
        find_in(&self.nodes, path)
    }

    pub fn find_all<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Node> {
        self.nodes.iter().filter(move |n| n.name == name)
    }

    /// Replace a byte range and re-parse.
    pub fn splice(&mut self, range: Range<usize>, with: &str) -> Result<(), ParseError> {
        let mut text = self.text.clone();
        text.replace_range(range, with);
        *self = Document::parse(text)?;
        Ok(())
    }


    /// Range covering the node for removal: the whole line(s) if the node is
    /// alone on its line(s), otherwise just the node and its terminator.
    pub fn removal_range(&self, node: &Node) -> Range<usize> {
        let b = self.text.as_bytes();
        let mut end = node.end;
        while end < b.len() && matches!(b[end], b' ' | b'\t') {
            end += 1;
        }
        if end < b.len() && b[end] == b';' {
            end += 1;
        }
        let mut probe = end;
        while probe < b.len() && matches!(b[probe], b' ' | b'\t') {
            probe += 1;
        }
        // A trailing `// comment` describes this node; remove it along with it.
        if b[probe..].starts_with(b"//") {
            while probe < b.len() && b[probe] != b'\n' {
                probe += 1;
            }
            if probe > 0 && b[probe - 1] == b'\r' {
                probe -= 1;
            }
        }
        let alone_start = node.line_start != node.start || at_line_start(b, node.start);
        if alone_start && (probe >= b.len() || b[probe] == b'\n' || b[probe..].starts_with(b"\r\n")) {
            let line_end = if probe >= b.len() {
                probe
            } else if b[probe] == b'\n' {
                probe + 1
            } else {
                probe + 2
            };
            return node.line_start..line_end;
        }
        node.start..end
    }

    /// Insert a new child node (given as KDL text, single or multi-line, without
    /// indentation) at the end of `parent`'s children block, creating the block
    /// if needed. Returns Err if the result fails to parse.
    pub fn append_child(&mut self, parent_path: &[&str], child_text: &str) -> Result<(), ParseError> {
        let parent = self.find(parent_path).ok_or_else(|| ParseError {
            offset: 0,
            message: format!("node {:?} not found", parent_path),
        })?;
        let (range, text) = append_child_edit(&self.text, parent, child_text);
        self.splice(range, &text)
    }

    /// Insert a top-level node on the line after `node`, at the same indent.
    pub fn insert_after(&mut self, node: &Node, node_text: &str) -> Result<(), ParseError> {
        let b = self.text.as_bytes();
        let mut end = node.end;
        while end < b.len() && b[end] != b'\n' {
            end += 1;
        }
        let indent = line_indent(&self.text, node.start);
        let body = indent_block(node_text, &indent);
        let mut text = self.text.clone();
        text.insert_str(end, &format!("\n{body}"));
        *self = Document::parse(text)?;
        Ok(())
    }

    /// Append a top-level node at the end of the document.
    pub fn append_top(&mut self, node_text: &str) -> Result<(), ParseError> {
        let mut text = self.text.clone();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(node_text);
        text.push('\n');
        *self = Document::parse(text)?;
        Ok(())
    }
}

/// Separates a node name from its first argument in a keyed path segment,
/// e.g. `output "eDP-1"` is addressed as `keyed("output", "eDP-1")`.
pub const KEY_SEP: char = '\u{1f}';

/// Path segment matching a node by name and first string argument.
pub fn keyed(name: &str, arg: &str) -> String {
    format!("{name}{KEY_SEP}{arg}")
}

/// Does `n` match the path segment `seg` (plain name or `keyed`)?
pub fn seg_matches(n: &Node, seg: &str) -> bool {
    match seg.split_once(KEY_SEP) {
        None => n.name == seg,
        Some((name, arg)) => {
            n.name == name && n.arg(0).and_then(Value::as_str).is_some_and(|a| a.eq_ignore_ascii_case(arg))
        }
    }
}

/// KDL header for a path segment: `name` or `name "arg"`.
pub fn seg_header(seg: &str) -> String {
    match seg.split_once(KEY_SEP) {
        None => ident(seg),
        Some((name, arg)) => format!("{} {}", ident(name), quote(arg)),
    }
}

/// String literal for a regex: a raw string when escapes would get in the way.
pub fn regex_lit(s: &str) -> String {
    if !s.contains('\\') && !s.contains('"') {
        return quote(s);
    }
    let mut hashes = String::from("#");
    while s.contains(&format!("\"{hashes}")) {
        hashes.push('#');
    }
    format!("r{hashes}\"{s}\"{hashes}")
}

/// Rebuild a node's text with its properties changed: `Some` sets (in place,
/// or appended), `None` removes. Arguments and the children block are kept.
/// Returns None if the node would end up with no entries and no children.
pub fn with_props(text: &str, node: &Node, props: &[(&str, Option<Value>)]) -> Option<String> {
    let mut entries: Vec<Entry> = node.entries.clone();
    for (k, v) in props {
        match v {
            Some(v) => match entries.iter_mut().rev().find(|e| e.key.as_deref() == Some(*k)) {
                Some(e) => e.value = v.clone(),
                None => entries.push(Entry { key: Some(k.to_string()), value: v.clone() }),
            },
            None => entries.retain(|e| e.key.as_deref() != Some(*k)),
        }
    }
    let children = node.children.as_ref().map(|c| &text[c.open..=c.close]);
    if entries.is_empty() && children.is_none() {
        return None;
    }
    let mut out = ident(&node.name);
    for e in &entries {
        out.push(' ');
        if let Some(k) = &e.key {
            out.push_str(&ident(k));
            out.push('=');
        }
        out.push_str(&e.value.to_kdl());
    }
    if let Some(c) = children {
        out.push(' ');
        out.push_str(c);
    }
    Some(out)
}

fn find_in<'a>(nodes: &'a [Node], path: &[&str]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    for n in nodes.iter().rev() {
        if seg_matches(n, first) {
            if rest.is_empty() {
                return Some(n);
            }
            if let Some(found) = find_in(n.children(), rest) {
                return Some(found);
            }
        }
    }
    None
}

fn at_line_start(b: &[u8], mut i: usize) -> bool {
    while i > 0 && matches!(b[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    i == 0 || b[i - 1] == b'\n'
}

pub fn line_indent(text: &str, pos: usize) -> String {
    let b = text.as_bytes();
    let mut ls = pos;
    while ls > 0 && b[ls - 1] != b'\n' {
        ls -= 1;
    }
    let mut e = ls;
    while e < b.len() && matches!(b[e], b' ' | b'\t') {
        e += 1;
    }
    text[ls..e].to_string()
}

fn indent_block(text: &str, indent: &str) -> String {
    text.lines()
        .map(|l| if l.is_empty() { String::new() } else { format!("{indent}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compute the splice that appends `child_text` inside `parent`.
pub fn append_child_edit(text: &str, parent: &Node, child_text: &str) -> (Range<usize>, String) {
    let parent_indent = line_indent(text, parent.start);
    let b = text.as_bytes();
    match &parent.children {
        None => {
            let unit = guess_indent_unit(text);
            let body = indent_block(child_text, &format!("{parent_indent}{unit}"));
            (parent.end..parent.end, format!(" {{\n{body}\n{parent_indent}}}"))
        }
        Some(ch) => {
            // Child indentation: copy from the first child that starts its own line.
            let child_indent = ch
                .nodes
                .iter()
                .find(|n| n.line_start != n.start || at_line_start(b, n.start))
                .map(|n| line_indent(text, n.start))
                .unwrap_or_else(|| format!("{parent_indent}{}", guess_indent_unit(text)));
            let body = indent_block(child_text, &child_indent);
            if at_line_start(b, ch.close) {
                // `}` sits on its own line: insert a new line right above it,
                // after any trailing blank lines/comments of the block.
                let mut ls = ch.close;
                while ls > 0 && b[ls - 1] != b'\n' {
                    ls -= 1;
                }
                (ls..ls, format!("{body}\n"))
            } else {
                // Inline block like `struts {}` or `{ foo; }`.
                let inner = text[ch.open + 1..ch.close].trim();
                let mut new_inner = String::new();
                if !inner.is_empty() {
                    new_inner.push_str(&indent_block(inner, &child_indent));
                    new_inner.push('\n');
                }
                new_inner.push_str(&body);
                (ch.open + 1..ch.close, format!("\n{new_inner}\n{parent_indent}"))
            }
        }
    }
}

fn guess_indent_unit(text: &str) -> String {
    for line in text.lines() {
        if line.starts_with('\t') {
            return "\t".into();
        }
    }
    "    ".into()
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { offset: self.i, message: msg.into() })
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn starts(&self, s: &str) -> bool {
        self.b[self.i..].starts_with(s.as_bytes())
    }

    fn parse_document(&mut self) -> Result<Vec<Node>, ParseError> {
        // Skip BOM.
        if self.starts("\u{feff}") {
            self.i += 3;
        }
        let nodes = self.parse_nodes(false)?;
        Ok(nodes)
    }

    /// Skip a `/* ... */` block comment (nested).
    fn skip_block_comment(&mut self) -> Result<(), ParseError> {
        let start = self.i;
        self.i += 2;
        let mut depth = 1;
        while depth > 0 {
            if self.i >= self.b.len() {
                self.i = start;
                return self.err("unterminated block comment");
            }
            if self.starts("/*") {
                depth += 1;
                self.i += 2;
            } else if self.starts("*/") {
                depth -= 1;
                self.i += 2;
            } else {
                self.i += 1;
            }
        }
        Ok(())
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == b'\n' {
                break;
            }
            self.i += 1;
        }
    }

    /// Skip whitespace, newlines, comments and `;` between nodes.
    fn skip_between_nodes(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b';') => self.i += 1,
                Some(b'/') if self.starts("//") => self.skip_line_comment(),
                Some(b'/') if self.starts("/*") => self.skip_block_comment()?,
                Some(b'\\') => self.skip_line_continuation()?,
                _ if self.starts("\u{feff}") => self.i += 3,
                _ => return Ok(()),
            }
        }
    }

    /// Inline whitespace inside a node: spaces, block comments, escaped newlines.
    fn skip_node_space(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t') => self.i += 1,
                Some(b'/') if self.starts("/*") => self.skip_block_comment()?,
                Some(b'\\') => self.skip_line_continuation()?,
                _ => return Ok(()),
            }
        }
    }

    fn skip_line_continuation(&mut self) -> Result<(), ParseError> {
        self.i += 1;
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.i += 1;
        }
        if self.starts("//") {
            self.skip_line_comment();
        }
        if self.peek() == Some(b'\r') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'\n') => {
                self.i += 1;
                Ok(())
            }
            None => Ok(()),
            _ => self.err("expected newline after `\\`"),
        }
    }

    fn parse_nodes(&mut self, in_block: bool) -> Result<Vec<Node>, ParseError> {
        let mut nodes = Vec::new();
        loop {
            self.skip_between_nodes()?;
            match self.peek() {
                None => {
                    if in_block {
                        return self.err("unexpected end of file, expected `}`");
                    }
                    return Ok(nodes);
                }
                Some(b'}') => {
                    if in_block {
                        return Ok(nodes);
                    }
                    return self.err("unexpected `}`");
                }
                _ => {}
            }
            let slashdash = if self.starts("/-") {
                self.i += 2;
                self.skip_between_nodes()?;
                true
            } else {
                false
            };
            let node = self.parse_node()?;
            if !slashdash {
                nodes.push(node);
            }
        }
    }

    fn parse_type_annotation(&mut self) -> Result<(), ParseError> {
        if self.peek() == Some(b'(') {
            self.i += 1;
            self.parse_string_or_ident()?;
            if self.peek() != Some(b')') {
                return self.err("expected `)`");
            }
            self.i += 1;
        }
        Ok(())
    }

    fn parse_node(&mut self) -> Result<Node, ParseError> {
        let start = self.i;
        self.parse_type_annotation()?;
        let name = match self.parse_string_or_ident()? {
            Value::String(s) => s,
            _ => return self.err("expected node name"),
        };
        let mut end = self.i;
        let mut entries = Vec::new();
        let mut children = None;
        loop {
            self.skip_node_space()?;
            match self.peek() {
                None | Some(b'\n' | b'\r' | b';' | b'}') => break,
                Some(b'/') if self.starts("//") => break,
                Some(b'{') => {
                    let open = self.i;
                    self.i += 1;
                    let nodes = self.parse_nodes(true)?;
                    let close = self.i;
                    self.i += 1;
                    end = self.i;
                    children = Some(Children { open, close, nodes });
                }
                Some(b'/') if self.starts("/-") => {
                    self.i += 2;
                    self.skip_node_space()?;
                    if self.peek() == Some(b'{') {
                        self.i += 1;
                        self.parse_nodes(true)?;
                        self.i += 1;
                    } else {
                        self.parse_entry()?;
                    }
                    end = self.i;
                }
                Some(_) => {
                    let e = self.parse_entry()?;
                    entries.push(e);
                    end = self.i;
                }
            }
        }
        let line_start = {
            let mut ls = start;
            while ls > 0 && matches!(self.b[ls - 1], b' ' | b'\t') {
                ls -= 1;
            }
            if ls == 0 || self.b[ls - 1] == b'\n' { ls } else { start }
        };
        Ok(Node { name, start, end, line_start, entries, children })
    }

    fn parse_entry(&mut self) -> Result<Entry, ParseError> {
        self.parse_type_annotation()?;
        let first_start = self.i;
        let first = self.parse_value()?;
        if self.peek() == Some(b'=') {
            let key = match first {
                Value::String(s) => s,
                _ => {
                    self.i = first_start;
                    return self.err("property key must be a string or identifier");
                }
            };
            self.i += 1;
            self.parse_type_annotation()?;
            let value = self.parse_value()?;
            return Ok(Entry { key: Some(key), value });
        }
        Ok(Entry { key: None, value: first })
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'"') => self.parse_quoted().map(Value::String),
            Some(b'r') if matches!(self.b.get(self.i + 1), Some(b'"' | b'#')) => {
                self.parse_raw().map(Value::String)
            }
            Some(c) if c.is_ascii_digit() => self.parse_number(),
            Some(b'-' | b'+') if self.b.get(self.i + 1).is_some_and(|c| c.is_ascii_digit()) => {
                self.parse_number()
            }
            Some(_) => {
                let id = self.parse_ident()?;
                Ok(match id.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    "null" => Value::Null,
                    _ => Value::String(id),
                })
            }
            None => self.err("unexpected end of file"),
        }
    }

    fn parse_string_or_ident(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'"') => self.parse_quoted().map(Value::String),
            Some(b'r') if matches!(self.b.get(self.i + 1), Some(b'"' | b'#')) => {
                self.parse_raw().map(Value::String)
            }
            _ => self.parse_ident().map(Value::String),
        }
    }

    fn parse_ident(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || (c < 0x80 && is_special(c as char)) {
                break;
            }
            self.i += 1;
        }
        if start == self.i {
            return self.err(format!(
                "unexpected character `{}`",
                self.s[self.i..].chars().next().unwrap_or(' ')
            ));
        }
        Ok(self.s[start..self.i].to_string())
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'+' | b'-') {
                self.i += 1;
            } else {
                break;
            }
        }
        let raw: String = self.s[start..self.i].chars().filter(|&c| c != '_').collect();
        let (neg, body) = match raw.as_bytes()[0] {
            b'-' => (true, &raw[1..]),
            b'+' => (false, &raw[1..]),
            _ => (false, &raw[..]),
        };
        let radix = if body.starts_with("0x") {
            Some(16)
        } else if body.starts_with("0o") {
            Some(8)
        } else if body.starts_with("0b") {
            Some(2)
        } else {
            None
        };
        if let Some(r) = radix {
            return match i64::from_str_radix(&body[2..], r) {
                Ok(v) => Ok(Value::Int(if neg { -v } else { v })),
                Err(_) => {
                    self.i = start;
                    self.err("invalid number")
                }
            };
        }
        if let Ok(v) = raw.trim_start_matches('+').parse::<i64>() {
            return Ok(Value::Int(v));
        }
        match raw.trim_start_matches('+').parse::<f64>() {
            Ok(v) => Ok(Value::Float(v)),
            Err(_) => {
                self.i = start;
                self.err("invalid number")
            }
        }
    }

    fn parse_quoted(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.s[self.i..].chars().next() else {
                self.i = start;
                return self.err("unterminated string");
            };
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(e) = self.s[self.i..].chars().next() else {
                        return self.err("unterminated escape");
                    };
                    self.i += e.len_utf8();
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        '"' => out.push('"'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => {
                            if self.peek() != Some(b'{') {
                                return self.err("expected `{` in unicode escape");
                            }
                            let close = self.s[self.i..].find('}').map(|p| self.i + p);
                            let Some(close) = close else {
                                return self.err("unterminated unicode escape");
                            };
                            let hex = &self.s[self.i + 1..close];
                            let ch = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                            let Some(ch) = ch else {
                                return self.err("invalid unicode escape");
                            };
                            out.push(ch);
                            self.i = close + 1;
                        }
                        _ => return self.err(format!("invalid escape `\\{e}`")),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn parse_raw(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        self.i += 1; // r
        let mut hashes = 0;
        while self.peek() == Some(b'#') {
            hashes += 1;
            self.i += 1;
        }
        if self.peek() != Some(b'"') {
            self.i = start;
            return self.err("expected `\"` in raw string");
        }
        self.i += 1;
        let terminator = format!("\"{}", "#".repeat(hashes));
        match self.s[self.i..].find(&terminator) {
            Some(p) => {
                let content = self.s[self.i..self.i + p].to_string();
                self.i += p + terminator.len();
                Ok(content)
            }
            None => {
                self.i = start;
                self.err("unterminated raw string")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"// header comment
input {
    keyboard {
        xkb {
            //layout "us"
        }
        numlock // Enable numlock
    }

    touchpad {
        tap // tap
        natural-scroll
    }
    /- mouse {
        accel-profile "flat"
    }
}
binds {
    Mod+Q                               { close-window; }
    XF86AudioMute   allow-when-locked=true { spawn-sh "noctalia msg volume-mute"; }
    Mod+Minus { set-column-width "-10%"; }
}
window-rule {
    match app-id="steam" title=r#"^notificationtoasts_\d+_desktop$"#
    clip-to-geometry true
    struts {}
}
"##;

    #[test]
    fn parses_sample() {
        let d = Document::parse(SAMPLE).unwrap();
        assert_eq!(d.nodes.len(), 3);
        let input = &d.nodes[0];
        assert_eq!(input.children().len(), 2, "slashdashed mouse is skipped");
        assert!(d.find(&["input", "touchpad", "tap"]).is_some());
        assert!(d.find(&["input", "mouse"]).is_none());
        let mute = d.find(&["binds", "XF86AudioMute"]).unwrap();
        assert_eq!(mute.prop("allow-when-locked"), Some(&Value::Bool(true)));
        let action = &mute.children()[0];
        assert_eq!(action.name, "spawn-sh");
        assert_eq!(action.arg(0).unwrap().as_str(), Some("noctalia msg volume-mute"));
        let m = d.find(&["window-rule", "match"]).unwrap();
        assert_eq!(m.prop("title").unwrap().as_str(), Some(r"^notificationtoasts_\d+_desktop$"));
        assert_eq!(
            d.find(&["binds", "Mod+Minus"]).unwrap().children()[0].arg(0),
            Some(&Value::String("-10%".into()))
        );
    }

    #[test]
    fn removal_whole_line_keeps_rest() {
        let mut d = Document::parse(SAMPLE).unwrap();
        let tap = d.find(&["input", "touchpad", "tap"]).unwrap().clone();
        let r = d.removal_range(&tap);
        d.splice(r, "").unwrap();
        assert!(d.find(&["input", "touchpad", "tap"]).is_none());
        assert!(d.text.contains("    touchpad {\n        natural-scroll\n"));
    }

    #[test]
    fn append_child_matches_indent() {
        let mut d = Document::parse(SAMPLE).unwrap();
        d.append_child(&["input", "touchpad"], "dwt").unwrap();
        assert!(d.text.contains("        natural-scroll\n        dwt\n    }"), "{}", d.text);
        d.append_child(&["input", "keyboard", "xkb"], "layout \"de\"").unwrap();
        assert!(d.text.contains("            //layout \"us\"\n            layout \"de\"\n        }"), "{}", d.text);
        d.append_child(&["window-rule", "struts"], "left 10").unwrap();
        assert!(d.text.contains("    struts {\n        left 10\n    }"), "{}", d.text);
    }

    #[test]
    fn append_child_creates_block() {
        let mut d = Document::parse("input {\n    touchpad\n}\n").unwrap();
        d.append_child(&["input", "touchpad"], "tap").unwrap();
        assert_eq!(d.text, "input {\n    touchpad {\n        tap\n    }\n}\n");
    }

    #[test]
    fn floats() {
        assert_eq!(format_float(0.0), "0.0");
        assert_eq!(format_float(-0.30000000004), "-0.3");
        assert_eq!(format_float(1.0), "1.0");
        assert_eq!(format_float(1.25), "1.25");
    }

    #[test]
    fn idents() {
        assert_eq!(ident("Mod+Shift+Q"), "Mod+Shift+Q");
        assert_eq!(ident("hello world"), "\"hello world\"");
        assert_eq!(ident("1"), "\"1\"");
    }

    #[test]
    fn real_config_files_parse() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let dir = std::path::Path::new(&home).join(".config/niri");
        let Ok(rd) = std::fs::read_dir(dir.join("cfg")) else { return };
        for e in rd.flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap();
            Document::parse(text).unwrap_or_else(|err| panic!("{:?}: {err}", e.path()));
        }
    }
}
