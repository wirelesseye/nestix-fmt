use crate::layout_format::{
    Node, format_generic, inline, is_ident_start, max_width, spaces, split_commas, tab_spaces,
};

pub(crate) fn format(nodes: &[Node], indent: usize, computed: bool) -> Result<String, String> {
    let mut cursor = 0;
    let mut chunks = Vec::new();
    if computed && let Some(captures) = nodes.first().and_then(|node| node.group('[')) {
        chunks.push(format!(
            "{}[{}]",
            spaces(indent),
            style_selector_inline(captures)
        ));
        cursor = 1;
    }

    while cursor < nodes.len() {
        let mut comments = Vec::new();
        while let Some(Node::Comment(comment)) = nodes.get(cursor) {
            comments.push(format!("{}{}", spaces(indent), comment));
            cursor += 1;
        }
        if cursor == nodes.len() {
            if !comments.is_empty() {
                chunks.push(comments.join("\n"));
            }
            break;
        }

        let item = if nodes[cursor].token() == Some("$") {
            format_style_insertion(nodes, &mut cursor, indent)?
        } else {
            format_style_rule(nodes, &mut cursor, indent)?
        };
        comments.push(item);
        chunks.push(comments.join("\n"));
    }
    Ok(chunks.join("\n\n"))
}

fn format_style_insertion(
    nodes: &[Node],
    cursor: &mut usize,
    indent: usize,
) -> Result<String, String> {
    *cursor += 1;
    let expression = nodes
        .get(*cursor)
        .and_then(|node| node.group('('))
        .ok_or("expected parenthesized expression after `$` in style sheet")?;
    *cursor += 1;
    Ok(format!(
        "{}$({})",
        spaces(indent),
        format_generic(expression, indent + 2)
    ))
}

fn format_style_rule(nodes: &[Node], cursor: &mut usize, indent: usize) -> Result<String, String> {
    let body_at = nodes[*cursor..]
        .iter()
        .position(|node| node.group('{').is_some())
        .map(|offset| *cursor + offset)
        .ok_or("expected block after style selector")?;
    let selector = format_style_selector(&nodes[*cursor..body_at], indent);
    let body = nodes[body_at].group('{').unwrap();
    *cursor = body_at + 1;
    if body.is_empty() {
        return Ok(format!("{}{selector} {{}}", spaces(indent)));
    }
    Ok(format!(
        "{}{selector} {{\n{}\n{}}}",
        spaces(indent),
        format_style_rule_body(body, indent + tab_spaces())?,
        spaces(indent)
    ))
}

fn format_style_rule_body(nodes: &[Node], indent: usize) -> Result<String, String> {
    let mut cursor = 0;
    let mut output = String::new();
    let mut previous_was_rule = false;
    while cursor < nodes.len() {
        let mut comments = Vec::new();
        while let Some(Node::Comment(comment)) = nodes.get(cursor) {
            comments.push(format!("{}{}", spaces(indent), comment));
            cursor += 1;
        }
        if cursor == nodes.len() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&comments.join("\n"));
            break;
        }

        let is_declaration = is_style_declaration_start(&nodes[cursor..]);
        if !output.is_empty() {
            output.push('\n');
            if previous_was_rule || !is_declaration {
                output.push('\n');
            }
        }
        if !comments.is_empty() {
            output.push_str(&comments.join("\n"));
            output.push('\n');
        }
        if is_declaration {
            output.push_str(&format_style_declaration(nodes, &mut cursor, indent)?);
        } else {
            output.push_str(&format_style_rule(nodes, &mut cursor, indent)?);
        }
        previous_was_rule = !is_declaration;
    }
    Ok(output)
}

fn is_style_declaration_start(nodes: &[Node]) -> bool {
    match nodes.first().and_then(Node::token) {
        Some("-") => nodes.get(1).and_then(Node::token) == Some("-"),
        Some(token) => token
            .as_bytes()
            .first()
            .is_some_and(|byte| is_ident_start(*byte)),
        None => false,
    }
}

fn format_style_declaration(
    nodes: &[Node],
    cursor: &mut usize,
    indent: usize,
) -> Result<String, String> {
    let start = *cursor;
    let semicolon = nodes[start..]
        .iter()
        .position(|node| node.token() == Some(";"))
        .map(|offset| start + offset)
        .ok_or("expected `;` after style property value")?;
    let colon = nodes[start..semicolon]
        .iter()
        .position(|node| node.token() == Some(":"))
        .map(|offset| start + offset)
        .ok_or("expected `:` after style property name")?;
    let name = style_inline(&nodes[start..colon]);
    let value_nodes = &nodes[colon + 1..semicolon];
    let value = if value_nodes.first().and_then(Node::token) == Some("$")
        && value_nodes.len() == 2
        && let Some(expression) = value_nodes[1].group('(')
    {
        let compact = inline(expression);
        let expression = if !contains_statement_block(expression)
            && indent + name.len() + compact.len() + 5 <= max_width()
        {
            compact
        } else {
            format_generic(expression, indent + name.len() + 4)
        };
        format!("$({expression})")
    } else {
        style_inline(value_nodes)
    };
    *cursor = semicolon + 1;
    Ok(format!("{}{name}: {value};", spaces(indent)))
}

fn format_style_selector(nodes: &[Node], indent: usize) -> String {
    let selector = style_selector_inline(nodes);
    if indent + selector.len() <= max_width() {
        return selector;
    }
    let parts = split_commas(nodes);
    if parts.len() == 1 {
        return selector;
    }
    let continuation_indent = spaces(indent);
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            format!(
                "{}{}{}",
                if index == 0 { "" } else { &continuation_indent },
                style_selector_inline(part),
                if index + 1 < parts.len() { "," } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn style_selector_inline(nodes: &[Node]) -> String {
    let mut output = String::new();
    for node in nodes {
        match node {
            Node::Token(token) if token == "," => {
                while output.ends_with(' ') {
                    output.pop();
                }
                output.push_str(", ");
            }
            Node::Token(token) if matches!(token.as_str(), ">" | ">>" | "+" | "~") => {
                if !output.is_empty() && !output.ends_with(' ') {
                    output.push(' ');
                }
                output.push_str(token);
                output.push(' ');
            }
            Node::Token(token) => output.push_str(token),
            Node::Group {
                open, close, nodes, ..
            } => {
                output.push(*open);
                output.push_str(&style_selector_inline(nodes));
                output.push(*close);
            }
            Node::Comment(comment) => {
                if !output.ends_with(' ') {
                    output.push(' ');
                }
                output.push_str(comment);
                output.push(' ');
            }
        }
    }
    output.trim_end().to_owned()
}

fn style_inline(nodes: &[Node]) -> String {
    let mut output = String::new();
    let mut previous_word = false;
    for node in nodes {
        if node.token() == Some(",") {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push_str(", ");
            previous_word = false;
            continue;
        }
        let (text, word) = match node {
            Node::Token(token) => (format_style_token(token), style_token_is_word(token)),
            Node::Group {
                open, close, nodes, ..
            } => (format!("{open}{}{close}", style_inline(nodes)), true),
            Node::Comment(comment) => (comment.clone(), true),
        };
        if previous_word && word {
            output.push(' ');
        }
        output.push_str(&text);
        previous_word = word;
    }
    output.trim_end().to_owned()
}

fn contains_statement_block(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Group {
            open: '{', nodes, ..
        } => nodes.iter().any(|node| node.token() == Some(";")),
        Node::Group { nodes, .. } => contains_statement_block(nodes),
        Node::Token(_) | Node::Comment(_) => false,
    })
}

fn style_token_is_word(token: &str) -> bool {
    token
        .as_bytes()
        .first()
        .is_some_and(|byte| is_ident_start(*byte) || byte.is_ascii_digit() || *byte == b'\"')
}

fn format_style_token(token: &str) -> String {
    for unit in ["px", "em"] {
        if let Some(number) = token.strip_suffix(unit)
            && !number.is_empty()
            && number
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'_'))
        {
            return format!("{number} {unit}");
        }
    }
    token.to_owned()
}

#[cfg(test)]
mod tests {
    use crate::layout_format::format_source;

    fn format_dsl(source: &str) -> Result<String, String> {
        format_source(source, None, false)
    }

    fn format_default(source: &str) -> Result<String, String> {
        format_source(source, None, true)
    }

    #[test]
    fn formats_style_rules_declarations_and_insertions() {
        let input = r#"let styles=style!{ // shared
$(base).app,.panel{padding:2 em;--label:$(format!("panel-{}",id));>.child{font_weight:semi-bold;}&:not(.disabled){bg_color:#EEF2F8;}}};"#;
        let expected = r#"let styles=style!{
    // shared
    $(base)

    .app, .panel {
        padding: 2 em;
        --label: $(format!("panel-{}", id));

        > .child {
            font_weight: semi-bold;
        }

        &:not(.disabled) {
            bg_color: #EEF2F8;
        }
    }
};"#;
        let formatted = format_dsl(input).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_computed_style_with_and_without_captures() {
        let captured = "computed_style!([color,count].counter{bg_color:$(color.get());gap:10px;})";
        let expected = "computed_style!(\n    [color, count]\n\n    .counter {\n        bg_color: $(color.get());\n        gap: 10 px;\n    }\n)";
        let formatted = format_dsl(captured).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);

        let uncaptured = "computed_style!{.counter{bg_color:white;}}";
        let expected = "computed_style!{\n    .counter {\n        bg_color: white;\n    }\n}";
        assert_eq!(format_dsl(uncaptured).unwrap(), expected);
    }

    #[test]
    fn formats_qualified_style_macros_and_ignores_them_inside_quote() {
        let input = "let a=nestix_native::style!(.a{gap:1px;}); let b=quote!{computed_style!{.b{gap:2px;}}};";
        let expected = "let a=nestix_native::style!(\n    .a {\n        gap: 1 px;\n    }\n); let b=quote!{computed_style!{.b{gap:2px;}}};";
        assert_eq!(format_dsl(input).unwrap(), expected);
    }

    #[test]
    fn separates_numbers_from_style_units() {
        let input =
            "style!{.panel{width:1px;margin_left:-2px;transition:width 420ms ease;height:3;}}";
        let expected = "style!{\n    .panel {\n        width: 1 px;\n        margin_left: -2 px;\n        transition: width 420ms ease;\n        height: 3;\n    }\n}";
        let formatted = format_dsl(input).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_dsl(&formatted).unwrap(), formatted);
    }

    #[test]
    fn reports_invalid_style_syntax() {
        let error = format_dsl("style! { .panel { gap: 10 px } }").unwrap_err();
        assert!(error.contains("invalid style syntax"));
        assert!(error.contains("expected `;`"));
    }

    #[test]
    fn formats_rust_blocks_and_comma_separated_style_values() {
        let input = r#"fn styles(){let sheet=style!{.panel{transition:width 420ms ease_in_out,height 420ms ease_in_out;margin:$({calls+=1;WithAuto::from(8.0)});}};}"#;
        let formatted = format_default(input).unwrap();
        assert!(
            formatted.contains("transition: width 420ms ease_in_out, height 420ms ease_in_out;")
        );
        assert!(formatted.contains("margin: $({\n"));
        assert!(formatted.contains("calls += 1;"));
        assert_eq!(format_default(&formatted).unwrap(), formatted);
    }
}
