use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::syntax;

const WIDTH: usize = 100;
const TAB_SPACES: usize = 4;

#[derive(Clone, Debug)]
struct FormatterConfig {
    max_width: usize,
    tab_spaces: usize,
    hard_tabs: bool,
    newline_style: NewlineStyle,
    rustfmt_config_dir: Option<PathBuf>,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            max_width: WIDTH,
            tab_spaces: TAB_SPACES,
            hard_tabs: false,
            newline_style: NewlineStyle::Auto,
            rustfmt_config_dir: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NewlineStyle {
    Auto,
    Native,
    Unix,
    Windows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
}

impl NewlineStyle {
    fn resolve(self, source: &str) -> LineEnding {
        match self {
            Self::Auto => detect_line_ending(source).unwrap_or(LineEnding::Lf),
            Self::Native => LineEnding::native(),
            Self::Unix => LineEnding::Lf,
            Self::Windows => LineEnding::CrLf,
        }
    }
}

impl LineEnding {
    fn native() -> Self {
        if cfg!(windows) { Self::CrLf } else { Self::Lf }
    }

    fn apply(self, source: &str) -> String {
        let source = normalize_newlines(source);
        match self {
            Self::Lf => source,
            Self::CrLf => source.replace('\n', "\r\n"),
        }
    }
}

thread_local! {
    static USE_RUSTFMT: Cell<bool> = const { Cell::new(true) };
    static FORMATTER_CONFIG: RefCell<FormatterConfig> = RefCell::new(FormatterConfig::default());
}

pub fn ensure_rustfmt() -> Result<(), String> {
    match Command::new("rustfmt")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err("`rustfmt --version` failed; install the rustfmt component".into()),
        Err(error) => Err(format!(
            "failed to start rustfmt: {error}; install the rustfmt component"
        )),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Token(String),
    Comment(String),
    Group {
        open: char,
        close: char,
        nodes: Vec<Node>,
    },
}

impl Node {
    fn token(&self) -> Option<&str> {
        match self {
            Self::Token(token) => Some(token),
            _ => None,
        }
    }

    fn group(&self, open: char) -> Option<&[Node]> {
        match self {
            Self::Group {
                open: actual,
                nodes,
                ..
            } if *actual == open => Some(nodes),
            _ => None,
        }
    }
}

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    use_rustfmt: bool,
) -> Result<String, String> {
    USE_RUSTFMT.set(use_rustfmt);
    let config = if use_rustfmt {
        rustfmt_config(path)?
    } else {
        FormatterConfig::default()
    };
    let line_ending = config.newline_style.resolve(source);
    FORMATTER_CONFIG.set(config);
    let source = normalize_newlines(source);
    let source = if use_rustfmt {
        rustfmt_source(&source, path)?
    } else {
        source
    };
    format_layouts_in_source(&source, path).map(|formatted| line_ending.apply(&formatted))
}

fn rustfmt_config(path: Option<&Path>) -> Result<FormatterConfig, String> {
    let current_dir = std::env::current_dir()
        .map_err(|error| format!("failed to determine current directory: {error}"))?;
    let probe = match path {
        Some(path) if path.is_absolute() => path.to_owned(),
        Some(path) => current_dir.join(path),
        None => current_dir.join("__nestix_fmt_stdin.rs"),
    };
    let search_dir = probe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| current_dir.clone());
    let config_dir = rustfmt_config_directory(&search_dir);
    let output = Command::new("rustfmt")
        .args(["--print-config", "current"])
        .arg(&probe)
        .output()
        .map_err(|error| format!("failed to read rustfmt configuration: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{}: failed to read rustfmt configuration: {}",
            path.map_or_else(|| "<stdin>".into(), |path| path.display().to_string()),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|error| format!("rustfmt configuration was not valid UTF-8: {error}"))?;
    Ok(FormatterConfig {
        max_width: config_usize(&output, "max_width")?,
        tab_spaces: config_usize(&output, "tab_spaces")?.max(1),
        hard_tabs: config_bool(&output, "hard_tabs")?,
        newline_style: config_newline_style(&output)?,
        rustfmt_config_dir: config_dir,
    })
}

fn rustfmt_config_directory(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|directory| {
        ["rustfmt.toml", ".rustfmt.toml"]
            .into_iter()
            .any(|name| directory.join(name).is_file())
            .then(|| directory.to_owned())
    })
}

fn config_value<'a>(config: &'a str, name: &str) -> Result<&'a str, String> {
    config
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name} = ")))
        .ok_or_else(|| format!("rustfmt did not report `{name}` in its configuration"))
}

fn config_usize(config: &str, name: &str) -> Result<usize, String> {
    config_value(config, name)?
        .parse()
        .map_err(|error| format!("invalid rustfmt `{name}` value: {error}"))
}

fn config_bool(config: &str, name: &str) -> Result<bool, String> {
    config_value(config, name)?
        .parse()
        .map_err(|error| format!("invalid rustfmt `{name}` value: {error}"))
}

fn config_newline_style(config: &str) -> Result<NewlineStyle, String> {
    match config_value(config, "newline_style")?.trim_matches('"') {
        "Auto" => Ok(NewlineStyle::Auto),
        "Native" => Ok(NewlineStyle::Native),
        "Unix" => Ok(NewlineStyle::Unix),
        "Windows" => Ok(NewlineStyle::Windows),
        value => Err(format!("unknown rustfmt `newline_style` value `{value}`")),
    }
}

fn detect_line_ending(source: &str) -> Option<LineEnding> {
    source.match_indices('\n').next().map(|(index, _)| {
        if index > 0 && source.as_bytes()[index - 1] == b'\r' {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    })
}

fn normalize_newlines(source: &str) -> String {
    source.replace("\r\n", "\n")
}

fn max_width() -> usize {
    FORMATTER_CONFIG.with(|config| config.borrow().max_width)
}

fn tab_spaces() -> usize {
    FORMATTER_CONFIG.with(|config| config.borrow().tab_spaces)
}

fn rustfmt_config_dir() -> Option<PathBuf> {
    FORMATTER_CONFIG.with(|config| config.borrow().rustfmt_config_dir.clone())
}

fn format_layouts_in_source(source: &str, path: Option<&Path>) -> Result<String, String> {
    let invocations =
        find_layouts(source).map_err(|error| diagnostic(path, source, error.0, &error.1))?;
    if invocations.is_empty() {
        return Ok(source.to_owned());
    }

    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for invocation in invocations {
        output.push_str(&source[cursor..invocation.body_start]);
        let body = &source[invocation.body_start..invocation.body_end];
        let nested = format_layouts_in_source(body, path)?;
        syntax::validate(&nested).map_err(|error| {
            let location = error.span().start();
            let relative = offset_at(&nested, location.line, location.column);
            diagnostic(
                path,
                source,
                invocation.body_start + relative,
                &format!("invalid layout syntax: {error}"),
            )
        })?;
        let nodes = lex_nodes(&nested)
            .map_err(|error| diagnostic(path, source, invocation.body_start + error.0, &error.1))?;
        let base = line_indent(source, invocation.open_offset);
        let formatted = format_layout(&nodes, base + tab_spaces())
            .map_err(|error| diagnostic(path, source, invocation.body_start, &error))?;
        syntax::validate(&formatted).map_err(|error| {
            diagnostic(
                path,
                source,
                invocation.body_start,
                &format!("formatter produced invalid layout syntax: {error}"),
            )
        })?;
        if nodes.is_empty() {
            // An empty layout has no useful internal shape to preserve.
        } else if !body.contains('\n')
            && is_single_element(&nodes)
            && !formatted.contains('\n')
            && base + formatted.trim().len() + "layout! {  }".len() <= max_width()
        {
            output.push(' ');
            output.push_str(formatted.trim());
            output.push(' ');
        } else {
            output.push('\n');
            output.push_str(&formatted);
            output.push('\n');
            output.push_str(&spaces(base));
        }
        cursor = invocation.body_end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

fn is_single_element(nodes: &[Node]) -> bool {
    let mut cursor = 0;
    let yielded = take(nodes, &mut cursor, "yield");
    if matches!(nodes.get(cursor), Some(Node::Comment(_))) {
        return false;
    }
    if nodes.get(cursor).and_then(Node::token) == Some("$")
        || (!yielded && matches!(nodes.get(cursor).and_then(Node::token), Some("if" | "for")))
    {
        return false;
    }
    if format_element(nodes, &mut cursor, 0, yielded).is_err() {
        return false;
    }
    while take(nodes, &mut cursor, ",") {}
    cursor == nodes.len()
}

fn rustfmt_source(source: &str, path: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("rustfmt");
    command.args([
        "--emit",
        "stdout",
        "--edition",
        "2024",
        "--config",
        "skip_children=true,newline_style=Unix",
    ]);
    if let Some(config_dir) = rustfmt_config_dir() {
        command.arg("--config-path").arg(config_dir);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start rustfmt: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "failed to open rustfmt stdin".to_owned())?
        .write_all(source.as_bytes())
        .map_err(|error| format!("failed to send source to rustfmt: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for rustfmt: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{}: rustfmt failed: {}",
            path.map_or_else(|| "<stdin>".into(), |path| path.display().to_string()),
            stderr.trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("rustfmt returned invalid UTF-8: {error}"))
}

#[derive(Clone, Copy)]
struct Invocation {
    open_offset: usize,
    body_start: usize,
    body_end: usize,
}

fn find_layouts(source: &str) -> Result<Vec<Invocation>, (usize, String)> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = skip_literal_or_comment(source, index) {
            index = end;
            continue;
        }
        if is_ident_start(bytes[index]) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            let name = &source[start..index];
            if matches!(name, "quote" | "quote_spanned") {
                let mut look = skip_space(source, index);
                if bytes.get(look) == Some(&b'!') {
                    look = skip_space(source, look + 1);
                    if let Some(&open) = bytes.get(look)
                        && let Some(close) = (match open {
                            b'{' => Some(b'}'),
                            b'(' => Some(b')'),
                            b'[' => Some(b']'),
                            _ => None,
                        })
                    {
                        index = matching_delimiter(source, look, open, close)? + 1;
                    }
                }
                continue;
            }
            if name != "layout" {
                continue;
            }
            let mut look = skip_space(source, index);
            if bytes.get(look) != Some(&b'!') {
                continue;
            }
            look = skip_space(source, look + 1);
            let Some(&open_byte) = bytes.get(look) else {
                continue;
            };
            let close = match open_byte {
                b'{' => b'}',
                b'(' => b')',
                b'[' => b']',
                _ => continue,
            };
            let body_end = matching_delimiter(source, look, open_byte, close)?;
            found.push(Invocation {
                open_offset: look,
                body_start: look + 1,
                body_end,
            });
            index = body_end + 1;
        } else {
            index += char_len(source, index);
        }
    }
    Ok(found)
}

fn matching_delimiter(
    source: &str,
    open_at: usize,
    open: u8,
    close: u8,
) -> Result<usize, (usize, String)> {
    let bytes = source.as_bytes();
    let mut stack = vec![close];
    let mut index = open_at + 1;
    while index < bytes.len() {
        if let Some(end) = skip_literal_or_comment(source, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'{' => stack.push(b'}'),
            b'(' => stack.push(b')'),
            b'[' => stack.push(b']'),
            byte if Some(&byte) == stack.last() => {
                stack.pop();
                if stack.is_empty() {
                    return Ok(index);
                }
            }
            byte if byte == b'}' || byte == b')' || byte == b']' => {
                return Err((index, "mismatched delimiter in layout macro".into()));
            }
            _ => {}
        }
        index += char_len(source, index);
    }
    Err((
        open_at,
        format!("unclosed `{}` delimiter in layout macro", open as char),
    ))
}

fn skip_space(source: &str, mut index: usize) -> usize {
    while source
        .as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_whitespace)
    {
        index += 1;
    }
    index
}

fn skip_literal_or_comment(source: &str, index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(index..index + 2) == Some(b"//") {
        return Some(
            source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset),
        );
    }
    if bytes.get(index..index + 2) == Some(b"/*") {
        let mut depth = 1usize;
        let mut cursor = index + 2;
        while cursor < bytes.len() {
            if bytes.get(cursor..cursor + 2) == Some(b"/*") {
                depth += 1;
                cursor += 2;
            } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                depth -= 1;
                cursor += 2;
                if depth == 0 {
                    return Some(cursor);
                }
            } else {
                cursor += char_len(source, cursor);
            }
        }
        return Some(source.len());
    }

    let mut prefix = index;
    if bytes.get(prefix) == Some(&b'b') || bytes.get(prefix) == Some(&b'c') {
        prefix += 1;
    }
    if bytes.get(prefix) == Some(&b'r') {
        let mut cursor = prefix + 1;
        while bytes.get(cursor) == Some(&b'#') {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'"') {
            let hashes = cursor - prefix - 1;
            cursor += 1;
            while cursor < bytes.len() {
                if bytes[cursor] == b'"'
                    && bytes.get(cursor + 1..cursor + 1 + hashes) == Some(&vec![b'#'; hashes][..])
                {
                    return Some(cursor + 1 + hashes);
                }
                cursor += char_len(source, cursor);
            }
            return Some(source.len());
        }
    }
    if bytes.get(prefix) == Some(&b'"') {
        return Some(skip_quoted(source, prefix, b'"'));
    }
    if bytes.get(prefix) == Some(&b'\'') && looks_like_char(source, prefix) {
        return Some(skip_quoted(source, prefix, b'\''));
    }
    None
}

fn looks_like_char(source: &str, start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    if bytes.get(index) == Some(&b'\\') {
        index += 2;
    } else if index < bytes.len() {
        index += char_len(source, index);
    }
    bytes.get(index) == Some(&b'\'')
}

fn skip_quoted(source: &str, start: usize, quote: u8) -> usize {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += char_len(source, index);
        }
    }
    bytes.len()
}

fn char_len(source: &str, index: usize) -> usize {
    source[index..].chars().next().map_or(1, char::len_utf8)
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic() || byte >= 0x80
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

fn line_indent(source: &str, offset: usize) -> usize {
    let start = source[..offset].rfind('\n').map_or(0, |line| line + 1);
    source[start..offset]
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'\t')
        .fold(0, |column, byte| {
            if byte == b'\t' {
                column + tab_spaces() - column % tab_spaces()
            } else {
                column + 1
            }
        })
}

fn diagnostic(path: Option<&Path>, source: &str, offset: usize, message: &str) -> String {
    let offset = offset.min(source.len());
    let line = source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let line_start = source[..offset].rfind('\n').map_or(0, |at| at + 1);
    let column = source[line_start..offset].chars().count() + 1;
    format!(
        "{}:{line}:{column}: {message}",
        path.map_or_else(|| "<stdin>".into(), |path| path.display().to_string())
    )
}

fn offset_at(source: &str, line: usize, column: usize) -> usize {
    let line_start = source
        .match_indices('\n')
        .nth(line.saturating_sub(2))
        .map_or(0, |(offset, _)| offset + 1);
    line_start
        + source[line_start..]
            .char_indices()
            .nth(column)
            .map_or_else(|| source[line_start..].len(), |(offset, _)| offset)
}

fn lex_nodes(source: &str) -> Result<Vec<Node>, (usize, String)> {
    let mut index = 0;
    lex_until(source, &mut index, None)
}

fn lex_until(
    source: &str,
    index: &mut usize,
    expected: Option<char>,
) -> Result<Vec<Node>, (usize, String)> {
    let bytes = source.as_bytes();
    let mut nodes = Vec::new();
    while *index < bytes.len() {
        if bytes[*index].is_ascii_whitespace() {
            *index += 1;
            continue;
        }
        if bytes.get(*index..*index + 2) == Some(b"//") {
            let end = source[*index..]
                .find('\n')
                .map_or(source.len(), |offset| *index + offset);
            nodes.push(Node::Comment(source[*index..end].trim_end().to_owned()));
            *index = end;
            continue;
        }
        if bytes.get(*index..*index + 2) == Some(b"/*") {
            let end = skip_literal_or_comment(source, *index).unwrap_or(source.len());
            nodes.push(Node::Comment(source[*index..end].to_owned()));
            *index = end;
            continue;
        }
        let current = bytes[*index] as char;
        if Some(current) == expected {
            *index += 1;
            return Ok(nodes);
        }
        if matches!(current, '}' | ')' | ']') {
            return Err((*index, format!("unexpected `{current}`")));
        }
        if let Some(close) = match current {
            '{' => Some('}'),
            '(' => Some(')'),
            '[' => Some(']'),
            _ => None,
        } {
            *index += 1;
            let inner = lex_until(source, index, Some(close))?;
            nodes.push(Node::Group {
                open: current,
                close,
                nodes: inner,
            });
            continue;
        }
        if let Some(end) = skip_literal_or_comment(source, *index) {
            nodes.push(Node::Token(source[*index..end].to_owned()));
            *index = end;
            continue;
        }
        if is_ident_start(bytes[*index]) || bytes[*index].is_ascii_digit() {
            let start = *index;
            *index += char_len(source, *index);
            while *index < bytes.len()
                && (is_ident_continue(bytes[*index])
                    || (bytes[*index] == b'.'
                        && bytes.get(*index + 1).is_some_and(u8::is_ascii_digit)))
            {
                *index += char_len(source, *index);
            }
            nodes.push(Node::Token(source[start..*index].to_owned()));
            continue;
        }
        let start = *index;
        let remaining = &source[*index..];
        let punctuation = [
            "<<=", ">>=", "..=", "::", "==", "!=", "<=", ">=", "&&", "||", "=>", "->", "<-", "+=",
            "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "..",
        ]
        .into_iter()
        .find(|punctuation| remaining.starts_with(punctuation))
        .map_or_else(|| char_len(source, *index), str::len);
        *index += punctuation;
        nodes.push(Node::Token(source[start..*index].to_owned()));
    }
    if let Some(close) = expected {
        Err((source.len(), format!("expected `{close}`")))
    } else {
        Ok(nodes)
    }
}

fn format_layout(nodes: &[Node], indent: usize) -> Result<String, String> {
    let mut cursor = 0;
    let mut lines = Vec::new();
    while cursor < nodes.len() {
        if nodes[cursor].token() == Some(",") {
            cursor += 1;
            continue;
        }
        while let Some(Node::Comment(comment)) = nodes.get(cursor) {
            push_comment(&mut lines, comment, indent);
            cursor += 1;
        }
        if cursor == nodes.len() {
            break;
        }
        lines.push(format_item(nodes, &mut cursor, indent)?);
    }
    Ok(lines.join("\n"))
}

fn format_item(nodes: &[Node], cursor: &mut usize, indent: usize) -> Result<String, String> {
    let start = *cursor;
    let yielded = take(nodes, cursor, "yield");
    if nodes.get(*cursor).and_then(Node::token) == Some("$") {
        *cursor += 1;
        let group = nodes.get(*cursor).ok_or("expected expression after `$`")?;
        let Some(inner) = group.group('(') else {
            return Err("expected parenthesized expression after `$`".into());
        };
        *cursor += 1;
        let expression = format_generic(inner, indent + 2);
        return Ok(format!(
            "{}{}$({expression})",
            spaces(indent),
            if yielded { "yield " } else { "" }
        ));
    }
    if !yielded && nodes.get(*cursor).and_then(Node::token) == Some("if") {
        return format_if(nodes, cursor, indent);
    }
    if !yielded && nodes.get(*cursor).and_then(Node::token) == Some("for") {
        return format_for(nodes, cursor, indent);
    }
    format_element(nodes, cursor, indent, yielded).map_err(|error| {
        format!(
            "could not format layout item beginning with `{}`: {error}",
            inline(&nodes[start..nodes.len().min(start + 4)])
        )
    })
}

fn format_if(nodes: &[Node], cursor: &mut usize, indent: usize) -> Result<String, String> {
    *cursor += 1;
    let branch = nodes[*cursor..]
        .iter()
        .position(|node| node.group('{').is_some())
        .map(|offset| *cursor + offset)
        .ok_or("expected block after `if` condition")?;
    let condition_nodes = &nodes[*cursor..branch];
    let condition_inline = inline(condition_nodes);
    let condition = if USE_RUSTFMT.get() && indent + 3 + condition_inline.len() > max_width() {
        format_rust_expression(&condition_inline, indent + 3, indent)
            .unwrap_or_else(|| format_generic(condition_nodes, indent + 3))
    } else {
        format_generic(condition_nodes, indent + 3)
    };
    let then = nodes[branch].group('{').unwrap();
    *cursor = branch + 1;
    let mut output = format!(
        "{}if {condition} {{\n{}\n{}}}",
        spaces(indent),
        format_layout(then, indent + tab_spaces())?,
        spaces(indent)
    );
    if take(nodes, cursor, "else") {
        if nodes.get(*cursor).and_then(Node::token) == Some("if") {
            let nested = format_if(nodes, cursor, indent)?;
            let nested = nested.strip_prefix(&spaces(indent)).unwrap_or(&nested);
            output.push_str(" else ");
            output.push_str(nested);
        } else {
            let Some(body) = nodes.get(*cursor).and_then(|node| node.group('{')) else {
                return Err("expected block after `else`".into());
            };
            *cursor += 1;
            output.push_str(&format!(
                " else {{\n{}\n{}}}",
                format_layout(body, indent + tab_spaces())?,
                spaces(indent)
            ));
        }
    }
    Ok(output)
}

fn format_for(nodes: &[Node], cursor: &mut usize, indent: usize) -> Result<String, String> {
    *cursor += 1;
    let body_at = nodes[*cursor..]
        .iter()
        .position(|node| node.group('{').is_some())
        .map(|offset| *cursor + offset)
        .ok_or("expected block after `for` header")?;
    let header_nodes = &nodes[*cursor..body_at];
    let header = format_for_header(header_nodes, indent + tab_spaces());
    let body = nodes[body_at].group('{').unwrap();
    *cursor = body_at + 1;
    Ok(format!(
        "{}for {header} {{\n{}\n{}}}",
        spaces(indent),
        format_layout(body, indent + tab_spaces())?,
        spaces(indent)
    ))
}

fn format_for_header(nodes: &[Node], indent: usize) -> String {
    let Some(in_at) = nodes.iter().position(|node| node.token() == Some("in")) else {
        return format_generic(nodes, indent);
    };
    let where_at = nodes.iter().position(|node| node.token() == Some("where"));
    let data_end = where_at.unwrap_or(nodes.len());
    let binding = inline(&nodes[..in_at]);
    let data = format_generic(&nodes[in_at + 1..data_end], indent);
    let mut header = format!("{binding} in {data}");
    if let Some(where_at) = where_at {
        let key_nodes = &nodes[where_at + 1..];
        if let Some(equals) = key_nodes.iter().position(|node| node.token() == Some("=")) {
            let key_name = inline(&key_nodes[..equals]);
            let key = format_generic(&key_nodes[equals + 1..], indent);
            header.push_str(&format!(" where {key_name} = {key}"));
        } else {
            header.push_str(" where ");
            header.push_str(&format_generic(key_nodes, indent));
        }
    }
    header
}

fn format_element(
    nodes: &[Node],
    cursor: &mut usize,
    indent: usize,
    yielded: bool,
) -> Result<String, String> {
    let mut prefix = String::new();
    if yielded {
        prefix.push_str("yield ");
    }
    if nodes.get(*cursor + 1).and_then(Node::token) == Some("@") {
        let binding = nodes[*cursor].token().ok_or("expected binding name")?;
        prefix.push_str(binding);
        prefix.push_str(" @ ");
        *cursor += 2;
    }
    let type_start = *cursor;
    consume_type(nodes, cursor)?;
    if type_start == *cursor {
        return Err("expected component type".into());
    }
    prefix.push_str(&inline(&nodes[type_start..*cursor]));

    let mut interstitial_comments = Vec::new();
    collect_postfix_comments(nodes, cursor, &mut interstitial_comments);

    if take(nodes, cursor, "$") {
        let Some(props) = nodes.get(*cursor).and_then(|node| node.group('(')) else {
            return Err("expected direct props after `$`".into());
        };
        prefix.push('$');
        prefix.push_str(&format_parens(props, indent, prefix.len()));
        *cursor += 1;
    } else if let Some(props) = nodes.get(*cursor).and_then(|node| node.group('(')) {
        prefix.push_str(&format_parens(props, indent, prefix.len()));
        *cursor += 1;
    }
    collect_postfix_comments(nodes, cursor, &mut interstitial_comments);

    if let Some(captures) = nodes.get(*cursor).and_then(|node| node.group('[')) {
        prefix.push(' ');
        prefix.push('[');
        prefix.push_str(&format_generic(captures, indent + prefix.len() + 1));
        prefix.push(']');
        *cursor += 1;
    }
    collect_postfix_comments(nodes, cursor, &mut interstitial_comments);

    if take(nodes, cursor, "|") {
        let args_start = *cursor;
        while *cursor < nodes.len() && nodes[*cursor].token() != Some("|") {
            *cursor += 1;
        }
        if *cursor == nodes.len() {
            return Err("unclosed child closure arguments".into());
        }
        prefix.push(' ');
        prefix.push('|');
        prefix.push_str(&format_generic(
            &nodes[args_start..*cursor],
            indent + prefix.len(),
        ));
        prefix.push('|');
        *cursor += 1;
    }
    collect_postfix_comments(nodes, cursor, &mut interstitial_comments);

    let mut output = String::new();
    for comment in interstitial_comments {
        for line in comment.lines() {
            output.push_str(&spaces(indent));
            output.push_str(line.trim());
            output.push('\n');
        }
    }
    output.push_str(&format!("{}{}", spaces(indent), prefix));
    if let Some(children) = nodes.get(*cursor).and_then(|node| node.group('{')) {
        *cursor += 1;
        if children.is_empty() {
            output.push_str(" {}");
        } else {
            output.push_str(" {\n");
            output.push_str(&format_layout(children, indent + tab_spaces())?);
            output.push('\n');
            output.push_str(&spaces(indent));
            output.push('}');
        }
    }
    Ok(output)
}

fn collect_postfix_comments<'a>(
    nodes: &'a [Node],
    cursor: &mut usize,
    comments: &mut Vec<&'a str>,
) {
    let start = *cursor;
    while matches!(nodes.get(*cursor), Some(Node::Comment(_))) {
        *cursor += 1;
    }
    let is_postfix = nodes.get(*cursor).is_some_and(|node| {
        matches!(node.token(), Some("$") | Some("|"))
            || matches!(
                node,
                Node::Group {
                    open: '(' | '[' | '{',
                    ..
                }
            )
    });
    if is_postfix {
        comments.extend(nodes[start..*cursor].iter().filter_map(|node| match node {
            Node::Comment(comment) => Some(comment.as_str()),
            _ => None,
        }));
    } else {
        *cursor = start;
    }
}

fn consume_type(nodes: &[Node], cursor: &mut usize) -> Result<(), String> {
    let Some(first) = nodes.get(*cursor) else {
        return Ok(());
    };
    if matches!(first, Node::Group { open: '(', .. }) {
        *cursor += 1;
        return Ok(());
    }
    if first.token().is_none() {
        return Ok(());
    }
    if matches!(first.token(), Some("&") | Some("*")) {
        *cursor += 1;
        if nodes
            .get(*cursor)
            .and_then(Node::token)
            .is_some_and(|token| token.starts_with('\'') || matches!(token, "mut" | "const"))
        {
            *cursor += 1;
        }
        return consume_type(nodes, cursor);
    }
    if first.token() == Some("::") {
        *cursor += 1;
        if nodes.get(*cursor).and_then(Node::token).is_none() {
            return Err("expected path segment after leading `::`".into());
        }
    }
    *cursor += 1;
    let mut angles = 0isize;
    loop {
        let token = nodes.get(*cursor).and_then(Node::token);
        match token {
            Some("<") => {
                angles += 1;
                *cursor += 1;
            }
            Some(">") if angles > 0 => {
                angles -= 1;
                *cursor += 1;
            }
            Some(_) if angles > 0 => *cursor += 1,
            None if angles > 0 => return Err("unclosed generic arguments in component type".into()),
            Some("::") => {
                *cursor += 1;
                if nodes.get(*cursor).is_none() {
                    return Err("expected path segment after `::`".into());
                }
                *cursor += 1;
            }
            _ => break,
        }
    }
    Ok(())
}

fn format_parens(nodes: &[Node], indent: usize, prefix_len: usize) -> String {
    if nodes.is_empty() {
        return "()".into();
    }
    let mut compact = inline(nodes);
    if nodes.last().and_then(Node::token) == Some(",") {
        compact = compact
            .trim_end()
            .strip_suffix(',')
            .unwrap_or(compact.trim_end())
            .trim_end()
            .to_owned();
    }
    let has_comments = contains_comments(nodes);
    let parts = split_commas(nodes);
    if !has_comments
        && !has_statement_block(nodes)
        && indent + prefix_len + compact.len() + 2 <= max_width()
    {
        return format!("({compact})");
    }
    let child_indent = indent + tab_spaces();
    let mut output = String::from("(\n");
    for part in parts {
        if part.is_empty() {
            continue;
        }
        output.push_str(&spaces(child_indent));
        output.push_str(&format_generic(part, child_indent));
        output.push_str(",\n");
    }
    output.push_str(&spaces(indent));
    output.push(')');
    output
}

fn format_generic(nodes: &[Node], indent: usize) -> String {
    let compact = inline(nodes);
    if USE_RUSTFMT.get()
        && !contains_comments(nodes)
        && !has_statement_block(nodes)
        && !has_nested_layout(nodes)
        && indent + compact.len() <= max_width()
    {
        return compact;
    }
    if !contains_comments(nodes)
        && !has_nested_layout(nodes)
        && let Some(formatted) = format_rust_expression(&compact, indent, indent)
        && fits_width(&formatted, indent)
    {
        return formatted;
    }
    if USE_RUSTFMT.get()
        && let Some(formatted) = format_dsl_assignment(nodes, indent)
    {
        return formatted;
    }
    let comma_parts = split_commas(nodes);
    if comma_parts.len() > 1 {
        return comma_parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(|part| format_generic(part, indent))
            .collect::<Vec<_>>()
            .join(&format!(",\n{}", spaces(indent)));
    }
    let mut output = String::new();
    let mut closure_pipe = false;
    let mut index = 0;
    while index < nodes.len() {
        if nodes[index].token() == Some("format")
            && nodes.get(index + 1).and_then(Node::token) == Some("!")
            && let Some(arguments) = nodes.get(index + 2).and_then(|node| node.group('('))
        {
            let current_column = if let Some(line) = output.rsplit('\n').next() {
                if output.contains('\n') {
                    line.len()
                } else {
                    indent + line.len()
                }
            } else {
                indent
            };
            output.push_str(&format_format_macro(arguments, current_column));
            index += 3;
            continue;
        }
        if nodes[index].token() == Some("layout")
            && nodes.get(index + 1).and_then(Node::token) == Some("!")
            && let Some(Node::Group {
                open,
                close,
                nodes: body,
            }) = nodes.get(index + 2)
        {
            append_token(&mut output, "layout");
            append_token(&mut output, "!");
            if *open == '{' {
                output.push(' ');
            }
            output.push(*open);
            output.push('\n');
            output.push_str(
                &format_layout(body, indent + tab_spaces()).unwrap_or_else(|_| {
                    format!("{}{}", spaces(indent + tab_spaces()), inline(body))
                }),
            );
            output.push('\n');
            output.push_str(&spaces(indent));
            output.push(*close);
            index += 3;
            continue;
        }
        let node = &nodes[index];
        match node {
            Node::Comment(comment) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(comment);
                if index + 1 < nodes.len() {
                    output.push('\n');
                    output.push_str(&spaces(indent));
                }
            }
            Node::Group { open, close, nodes } => {
                if *open == '{' && (has_top_level_statement(nodes) || has_statement_block(nodes)) {
                    while output.ends_with(' ') {
                        output.pop();
                    }
                    if !output.is_empty() {
                        output.push(' ');
                    }
                    output.push_str(&format_rust_block(nodes, indent));
                } else {
                    let formatted = format_generic(nodes, indent);
                    output.push(*open);
                    output.push_str(&formatted);
                    output.push(*close);
                }
            }
            Node::Token(token) if token == "|" => append_pipe(&mut output, &mut closure_pipe),
            Node::Token(token) => append_token(&mut output, token),
        }
        index += 1;
    }
    output
}

fn format_format_macro(nodes: &[Node], indent: usize) -> String {
    let child_indent = indent + tab_spaces();
    let parts = split_commas(nodes);
    if parts.len() <= 1 && indent + inline(nodes).len() + 9 <= max_width() {
        return format!("format!({})", inline(nodes));
    }
    let mut output = String::from("format!(\n");
    for part in parts.into_iter().filter(|part| !part.is_empty()) {
        output.push_str(&spaces(child_indent));
        output.push_str(&format_generic(part, child_indent));
        output.push_str(",\n");
    }
    output.push_str(&spaces(indent));
    output.push(')');
    output
}

fn format_dsl_assignment(nodes: &[Node], indent: usize) -> Option<String> {
    let separator = nodes
        .iter()
        .position(|node| matches!(node.token(), Some("=") | Some(":")))?;
    if separator == 0 || separator + 1 == nodes.len() {
        return None;
    }
    let left = &nodes[..=separator];
    let is_dsl_prefix = left.first().and_then(Node::token) == Some(".")
        || left.iter().any(|node| node.token() == Some(":"));
    if !is_dsl_prefix {
        return None;
    }
    let left = inline(left);
    let left = left.trim_end();
    let expression_nodes = &nodes[separator + 1..];
    let expression = inline(expression_nodes);
    if let Some(formatted) = format_rust_expression(&expression, indent + left.len() + 1, indent)
        && fits_width(&format!("{left} {formatted}"), indent)
    {
        return Some(format!("{left} {formatted}"));
    }

    let [
        Node::Token(macro_name),
        Node::Token(bang),
        Node::Group {
            open: '(',
            close: ')',
            nodes: arguments,
        },
    ] = expression_nodes
    else {
        return None;
    };
    if bang != "!" {
        return None;
    }
    let child_indent = indent + tab_spaces();
    let mut arguments = if macro_name == "callback" {
        format_callback_arguments(arguments, child_indent)
            .unwrap_or_else(|| format_generic(arguments, child_indent))
    } else {
        format_generic(arguments, child_indent)
    };
    if macro_name == "callback" {
        arguments = arguments.replace(" | {", "| {");
    }
    Some(format!(
        "{left} {macro_name}!(\n{}{}\n{})",
        spaces(child_indent),
        arguments,
        spaces(indent)
    ))
}

fn format_callback_arguments(nodes: &[Node], indent: usize) -> Option<String> {
    let captures = nodes.first()?.group('[')?;
    let Node::Group {
        open: '{',
        nodes: body,
        ..
    } = nodes.last()?
    else {
        return None;
    };
    let closure = inline(&nodes[1..nodes.len() - 1]);
    let compact_captures = inline(captures);
    let mut output = if indent + compact_captures.len() + closure.len() + 5 <= max_width() {
        format!("[{compact_captures}]")
    } else {
        let child_indent = indent + tab_spaces();
        let mut captures_output = String::from("[\n");
        for capture in split_commas(captures)
            .into_iter()
            .filter(|capture| !capture.is_empty())
        {
            captures_output.push_str(&spaces(child_indent));
            captures_output.push_str(&inline(capture));
            captures_output.push_str(",\n");
        }
        captures_output.push_str(&spaces(indent));
        captures_output.push(']');
        captures_output
    };
    output.push(' ');
    output.push_str(closure.trim());
    output.push(' ');
    output.push_str(&format_rust_block(body, indent));
    Some(output)
}

fn fits_width(text: &str, first_indent: usize) -> bool {
    text.lines().enumerate().all(|(index, line)| {
        let initial_column = if index == 0 { first_indent } else { 0 };
        display_column(line, initial_column) <= max_width()
    })
}

fn display_column(text: &str, initial_column: usize) -> usize {
    text.bytes().fold(initial_column, |column, byte| {
        if byte == b'\t' {
            column + tab_spaces() - column % tab_spaces()
        } else {
            column + 1
        }
    })
}

fn format_rust_expression(
    expression: &str,
    start_column: usize,
    continuation_indent: usize,
) -> Option<String> {
    syn::parse_str::<syn::Expr>(expression).ok()?;
    let marker = "    let __nestix_value = ";
    let first_line_width = max_width()
        .saturating_add(marker.len())
        .saturating_sub(start_column);
    // Continuation lines are indented four columns inside the synthetic
    // function, then moved to the real continuation indent.
    let continuation_width = max_width()
        .saturating_sub(continuation_indent)
        .saturating_add(4);
    let rustfmt_width = first_line_width.min(continuation_width).max(20);
    let wrapper = format!("fn __nestix_fmt() {{\n    let __nestix_value = {expression};\n}}\n");
    let mut command = Command::new("rustfmt");
    command.args([
        "--emit",
        "stdout",
        "--edition",
        "2024",
        "--config",
        &format!("max_width={rustfmt_width},hard_tabs=false,tab_spaces=4,newline_style=Unix"),
    ]);
    if let Some(config_dir) = rustfmt_config_dir() {
        command.arg("--config-path").arg(config_dir);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(wrapper.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let start = output.find(marker)? + marker.len();
    let end = output.rfind(";\n}")?;
    let fragment = &output[start..end];
    let continuation = spaces(continuation_indent);
    let mut lines = fragment.lines();
    let mut formatted = lines.next()?.to_owned();
    for line in lines {
        formatted.push('\n');
        formatted.push_str(&continuation);
        formatted.push_str(line.strip_prefix("    ").unwrap_or(line));
    }
    Some(formatted)
}

fn format_rust_statement(statement: &str, indent: usize) -> Option<String> {
    let rustfmt_width = max_width().saturating_sub(indent).saturating_add(4).max(20);
    let wrapper = format!("fn __nestix_fmt() {{\n    {statement};\n}}\n");
    let mut command = Command::new("rustfmt");
    command.args([
        "--emit",
        "stdout",
        "--edition",
        "2024",
        "--config",
        &format!("max_width={rustfmt_width},hard_tabs=false,tab_spaces=4,newline_style=Unix"),
    ]);
    if let Some(config_dir) = rustfmt_config_dir() {
        command.arg("--config-path").arg(config_dir);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(wrapper.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let fragment = output
        .strip_prefix("fn __nestix_fmt() {\n")?
        .strip_suffix("\n}\n")?
        .strip_prefix("    ")?
        .strip_suffix(';')?;
    let mut lines = fragment.lines();
    let mut formatted = lines.next()?.to_owned();
    for line in lines {
        formatted.push('\n');
        formatted.push_str(&spaces(indent));
        formatted.push_str(line.strip_prefix("    ").unwrap_or(line));
    }
    Some(formatted)
}

fn inline(nodes: &[Node]) -> String {
    let mut output = String::new();
    let mut closure_pipe = false;
    for node in nodes {
        match node {
            Node::Token(token) if token == "|" => {
                append_pipe(&mut output, &mut closure_pipe);
            }
            Node::Token(token) => append_token(&mut output, token),
            Node::Comment(comment) => {
                if !output.is_empty() && !output.ends_with(' ') {
                    output.push(' ');
                }
                output.push_str(comment);
            }
            Node::Group { open, close, nodes } => {
                if needs_space_before_group(&output) && *open == '{' {
                    output.push(' ');
                }
                output.push(*open);
                if *open == '{' && !nodes.is_empty() {
                    output.push(' ');
                }
                output.push_str(&inline(nodes));
                if *open == '{' && !nodes.is_empty() {
                    output.push(' ');
                }
                output.push(*close);
            }
        }
    }
    output
}

fn append_pipe(output: &mut String, closure_pipe: &mut bool) {
    if *closure_pipe {
        while output.ends_with(' ') {
            output.pop();
        }
        output.push('|');
        output.push(' ');
    } else {
        if !output.is_empty() && !output.ends_with([' ', '(', '[', '{']) {
            output.push(' ');
        }
        output.push('|');
    }
    *closure_pipe = !*closure_pipe;
}

fn append_token(output: &mut String, token: &str) {
    let previous = output.chars().last();
    let no_space_before = matches!(token, "," | ";" | "." | "?" | ":" | "::" | "!");
    let no_space_after_previous = matches!(
        previous,
        None | Some(' ')
            | Some('\n')
            | Some('(')
            | Some('[')
            | Some('.')
            | Some(':')
            | Some('!')
            | Some('#')
            | Some('$')
    );
    let operator = matches!(
        token,
        "=" | "+"
            | "-"
            | "/"
            | "%"
            | "=="
            | "!="
            | "<="
            | ">="
            | "&&"
            | "||"
            | "=>"
            | "->"
            | "@"
            | "<-"
            | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "&="
            | "|="
            | "^="
            | "<<="
            | ">>="
            | "|"
    );
    let word = token.as_bytes().first().is_some_and(|byte| {
        is_ident_start(*byte) || byte.is_ascii_digit() || *byte == b'\'' || *byte == b'"'
    });
    let follows_value = previous
        .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\'' | '"' | ')' | ']' | '}'));
    if !no_space_before && !no_space_after_previous && (operator || (word && follows_value)) {
        output.push(' ');
    }
    if operator {
        while output.ends_with(' ') {
            output.pop();
        }
        if !output.is_empty() && !output.ends_with('\n') {
            output.push(' ');
        }
    }
    output.push_str(token);
    if matches!(token, "," | ";" | ":") || operator {
        output.push(' ');
    }
}

fn has_statement_block(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Group {
            open: '{', nodes, ..
        } => has_top_level_statement(nodes),
        Node::Group { nodes, .. } => has_statement_block(nodes),
        _ => false,
    })
}

fn has_nested_layout(nodes: &[Node]) -> bool {
    nodes.windows(3).any(|window| {
        window[0].token() == Some("layout")
            && window[1].token() == Some("!")
            && matches!(window[2], Node::Group { .. })
    }) || nodes.iter().any(|node| match node {
        Node::Group { nodes, .. } => has_nested_layout(nodes),
        _ => false,
    })
}

fn has_top_level_statement(nodes: &[Node]) -> bool {
    contains_comments(nodes) || nodes.iter().any(|node| node.token() == Some(";"))
}

fn format_rust_block(nodes: &[Node], indent: usize) -> String {
    let child_indent = indent + tab_spaces();
    let mut output = String::from("{\n");
    let mut start = 0;
    for (index, node) in nodes.iter().enumerate() {
        if node.token() == Some(";") {
            if start < index {
                let statement = inline(&nodes[start..index]);
                let formatted = format_rust_statement(&statement, child_indent)
                    .filter(|formatted| fits_width(formatted, child_indent))
                    .unwrap_or_else(|| format_generic(&nodes[start..index], child_indent));
                output.push_str(&spaces(child_indent));
                output.push_str(&formatted);
                output.push(';');
                output.push('\n');
            }
            start = index + 1;
        }
    }
    if start < nodes.len() {
        output.push_str(&spaces(child_indent));
        output.push_str(&format_generic(&nodes[start..], child_indent));
        output.push('\n');
    }
    output.push_str(&spaces(indent));
    output.push('}');
    output
}

fn needs_space_before_group(output: &str) -> bool {
    output
        .chars()
        .last()
        .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ')' | ']'))
}

fn contains_comments(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Comment(_) => true,
        Node::Group { nodes, .. } => contains_comments(nodes),
        Node::Token(_) => false,
    })
}

fn split_commas(nodes: &[Node]) -> Vec<&[Node]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, node) in nodes.iter().enumerate() {
        if node.token() == Some(",") {
            parts.push(&nodes[start..index]);
            start = index + 1;
        }
    }
    if start < nodes.len() {
        parts.push(&nodes[start..]);
    }
    parts
}

fn push_comment(lines: &mut Vec<String>, comment: &str, indent: usize) {
    for line in comment.lines() {
        lines.push(format!("{}{}", spaces(indent), line.trim()));
    }
}

fn take(nodes: &[Node], cursor: &mut usize, expected: &str) -> bool {
    if nodes.get(*cursor).and_then(Node::token) == Some(expected) {
        *cursor += 1;
        true
    } else {
        false
    }
}

fn spaces(count: usize) -> String {
    FORMATTER_CONFIG.with(|config| {
        let config = config.borrow();
        if !config.hard_tabs {
            return " ".repeat(count);
        }
        let tabs = count / config.tab_spaces;
        let spaces = count % config.tab_spaces;
        format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces))
    })
}

#[cfg(test)]
mod tests {
    use super::{WIDTH, format_source};

    fn format_dsl(source: &str) -> Result<String, String> {
        format_source(source, None, false)
    }

    fn format_default(source: &str) -> Result<String, String> {
        format_source(source, None, true)
    }

    #[test]
    fn formats_nested_layout() {
        let input = "fn view(){layout! {Root{Button(.title=\"Go\"){Text(\"Hi\")}if ready.get(){Ok}else{No}}}}";
        let expected = r#"fn view(){layout! {
    Root {
        Button(.title = "Go") {
            Text("Hi")
        }
        if ready.get() {
            Ok
        } else {
            No
        }
    }
}}"#;
        assert_eq!(format_dsl(input).unwrap(), expected);
    }

    #[test]
    fn keeps_empty_children_on_one_line() {
        let input = "layout! { Root { Div{} Button(.disabled = true) { } } }";
        let expected =
            "layout! {\n    Root {\n        Div {}\n        Button(.disabled = true) {}\n    }\n}";
        let formatted = format_dsl(input).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn removes_a_trailing_comma_from_inline_props() {
        let input = r#"layout! { TabViewItem(.id = "counter", .title = "Counter", ) { Counter } }"#;
        let formatted = format_dsl(input).unwrap();
        assert!(formatted.contains(r#"TabViewItem(.id = "counter", .title = "Counter") {"#));
        assert!(!formatted.contains(r#".title = "Counter",) {"#));
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn retains_trailing_commas_in_multiline_props() {
        let input = r#"layout! { Widget(.first_property_with_a_long_name = first_value_with_a_long_name, .second_property_with_a_long_name = second_value_with_a_long_name,) }"#;
        let formatted = format_dsl(input).unwrap();
        assert!(
            formatted.contains(
                "    .second_property_with_a_long_name = second_value_with_a_long_name,\n"
            )
        );
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn collapses_an_empty_layout_block() {
        for input in ["layout! {}", "layout! { }", "layout! {\n\n}"] {
            assert_eq!(format_dsl(input).unwrap(), "layout! {}");
        }
    }

    #[test]
    fn keeps_a_short_single_element_inline_when_already_inline() {
        let input = "let view = layout! { ExampleApp };";
        assert_eq!(format_dsl(input).unwrap(), input);
    }

    #[test]
    fn keeps_a_short_single_element_multiline_when_already_multiline() {
        let input = "let view = layout! {\n    ExampleApp\n};";
        let formatted = format_dsl(input).unwrap();
        assert_eq!(formatted, input);
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn leaves_non_layout_source_untouched() {
        let input = "fn  odd_spacing( ) { let text = r#\"layout! { Nope }\"#; }\n";
        assert_eq!(format_dsl(input).unwrap(), input);
    }

    #[test]
    fn rustfmt_formats_complete_source_before_the_layout() {
        let input = "fn  view( ){let value=1+2;layout! {Root}}";
        let formatted = format_default(input).unwrap();
        assert!(formatted.starts_with("fn view() {\n    let value = 1 + 2;"));
        assert!(formatted.contains("layout! { Root }"));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn rustfmt_preserves_a_multiline_single_element_layout() {
        let input = "fn view() {\n    layout! {\n        ExampleApp\n    }\n}\n";
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("layout! {\n        ExampleApp\n    }"));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_comments() {
        let input = "layout! { Root { // greeting\n Text(\"hello\") /* tail */ Button } }";
        let formatted = format_dsl(input).unwrap();
        assert!(formatted.contains("// greeting"));
        assert!(formatted.contains("/* tail */"));
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_comment_between_an_element_and_children() {
        let input = "layout! { Root /* children follow */ { Text(\"hi\") } }";
        let formatted = format_dsl(input).unwrap();
        assert!(formatted.contains("/* children follow */"));
        assert!(formatted.contains("Root {"));
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_qualified_and_alternate_delimiters() {
        let input = "let a=nestix::layout!(ui::Root<Model>{ui::Child});";
        let formatted = format_dsl(input).unwrap();
        assert_eq!(
            formatted,
            "let a=nestix::layout!(\n    ui::Root<Model> {\n        ui::Child\n    }\n);"
        );
    }

    #[test]
    fn formats_nested_layout_macros() {
        let input = "layout! { $(layout! { Root{Child} }) }";
        let formatted = format_dsl(input).unwrap();
        assert!(formatted.contains("$(layout! {\n"));
        assert!(formatted.contains("Root {\n"));
        assert!(formatted.contains("Child\n"));
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn ignores_layout_tokens_inside_quote() {
        let input = "quote! { layout! { #children } }";
        assert_eq!(format_dsl(input).unwrap(), input);
    }

    #[test]
    fn rustfmt_wraps_long_prop_expressions() {
        let input = r#"layout! { Widget(.value = build_a_value_with_a_very_long_name(first_argument_with_a_long_name, second_argument_with_a_long_name, third_argument_with_a_long_name)) }"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("build_a_value_with_a_very_long_name(\n"));
        assert!(formatted.contains("first_argument_with_a_long_name,"));
        assert!(formatted.lines().all(|line| line.len() <= WIDTH));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn rustfmt_formats_blocks_inside_prop_macros() {
        let input = r#"layout! { Button(.on_click = callback!([state] || { state.update(|value| *value += 1); notify_every_interested_observer_with_a_long_name(first_observer_with_a_long_name, second_observer_with_a_long_name); })) }"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("callback!(\n"));
        assert!(formatted.contains("state.update(|value| *value += 1);"));
        assert!(formatted.contains("notify_every_interested_observer_with_a_long_name(\n"));
        assert!(formatted.lines().all(|line| line.len() <= WIDTH));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn wraps_a_callback_with_dsl_capture_syntax_at_its_actual_indent() {
        let input = r#"fn view() {
    layout! {
        Div {
            if editing {
                Input(
                    .value = value.clone(),
                    .on_value_change = callback!([key, props.set_content] |value: String| { set_content(&key.get(), value); }),
                )
            }
        }
    }
}"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains(".on_value_change = callback!(\n"));
        assert!(
            formatted
                .contains("                        [key, props.set_content] |value: String| {")
        );
        assert!(formatted.lines().all(|line| line.len() <= WIDTH));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn wraps_a_callback_block_with_a_nested_if_expression() {
        let input = r#"fn view() {
    layout! {
        Button(
            .title = "Show context menu",
            .on_click = callback!([context_menu, status] || { if let Err(error) = context_menu.show(ContextMenuPosition::Cursor) { status.set(format!("Could not show menu: {error}")); } }),
        )
    }
}"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("[context_menu, status] || {\n"));
        assert!(
            formatted
                .contains("if let Err(error) = context_menu.show(ContextMenuPosition::Cursor) {")
        );
        assert!(formatted.lines().all(|line| line.len() <= WIDTH));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn wraps_long_statements_and_format_arguments_inside_callbacks() {
        let input = r#"fn view() {
    layout! {
        Root {
            Section {
                Form {
                    Actions {
                        Button(
                            .on_click = callback!([name, newsletter, notifications, density, country, volume, status] || { let country = country.get().unwrap_or_else(|| "not selected".to_string()); status.set(format!("Saved: name={:?}, newsletter={}, notifications={}, density={}, country={}, volume={:.0}", name.get(), newsletter.get(), notifications.get(), density.get(), country, volume.get())); }),
                        )
                    }
                }
            }
        }
    }
}"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("let country"));
        assert!(formatted.contains("format!(\n"));
        assert!(!formatted.contains("concat!(\n"));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }

    #[test]
    fn rustfmt_wraps_long_conditions_and_loop_expressions() {
        let input = r#"layout! { if first_condition_with_a_long_name() && second_condition_with_a_long_name() && third_condition_with_a_long_name() { Ready } for item in load_every_item_from_a_very_long_source_name(first_source_with_a_long_name, second_source_with_a_long_name, third_source_with_a_long_name) where key = |item| item.identifier_with_a_long_name().clone() { Row(.item = item) } }"#;
        let formatted = format_default(input).unwrap();
        assert!(formatted.contains("\n        && second_condition"));
        assert!(formatted.contains("load_every_item_from_a_very_long_source_name(\n"));
        assert!(formatted.lines().all(|line| line.len() <= WIDTH));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }
}
