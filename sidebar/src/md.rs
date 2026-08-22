//! Tiny markdown → ratatui lines. Headings, lists, fences, inline ** * `.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn is_markdown(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "markdown")
    )
}

pub fn render(src: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut fence: Option<String> = None;
    for raw in src.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("```") {
            if fence.is_some() {
                fence = None;
                out.push(Line::raw(""));
            } else {
                fence = Some(rest.trim().to_string());
                let label = if rest.trim().is_empty() {
                    "code".to_string()
                } else {
                    rest.trim().to_string()
                };
                out.push(Line::from(Span::styled(
                    format!(" {label} "),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }
        if fence.is_some() {
            out.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(Color::Rgb(0x9e, 0xaa, 0xb6)),
            )));
            continue;
        }
        if let Some(n) = heading_level(line) {
            let text = line[n..].trim().to_string();
            let style = Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD);
            out.push(Line::from(Span::styled(text, style)));
            continue;
        }
        if line.starts_with("---") || line.starts_with("***") {
            out.push(Line::from(Span::styled(
                "────────",
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("> ") {
            let mut spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
            spans.extend(inline(rest, Style::default().fg(Color::Rgb(0x9e, 0xaa, 0xb6))));
            out.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = unordered(line) {
            let mut spans = vec![Span::styled("• ", Style::default().fg(Color::LightBlue))];
            spans.extend(inline(rest, Style::default()));
            out.push(Line::from(spans));
            continue;
        }
        if line.trim().is_empty() {
            out.push(Line::raw(""));
            continue;
        }
        out.push(Line::from(inline(line, Style::default())));
    }
    if out.is_empty() {
        out.push(Line::raw(""));
    }
    out
}

fn heading_level(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut n = 0;
    while n < bytes.len() && n < 6 && bytes[n] == b'#' {
        n += 1;
    }
    if n > 0 && bytes.get(n).copied() == Some(b' ') {
        Some(n)
    } else {
        None
    }
}

fn unordered(line: &str) -> Option<&str> {
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(p) {
            return Some(rest);
        }
    }
    None
}

fn inline(src: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut buf = String::new();
    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>, style: Style| {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    };
    while i < chars.len() {
        if chars[i] == '`' {
            flush(&mut buf, &mut spans, base);
            i += 1;
            let mut code = String::new();
            while i < chars.len() && chars[i] != '`' {
                code.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            spans.push(Span::styled(
                format!(" {code} "),
                Style::default()
                    .fg(Color::Rgb(0xe3, 0xb3, 0x41))
                    .bg(Color::Rgb(0x21, 0x26, 0x2d)),
            ));
            continue;
        }
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            flush(&mut buf, &mut spans, base);
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            }
            spans.push(Span::styled(
                inner,
                base.add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        if chars[i] == '[' {
            if let Some((label, skip)) = parse_link(&chars[i..]) {
                flush(&mut buf, &mut spans, base);
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                i += skip;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut buf, &mut spans, base);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn parse_link(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'[') {
        return None;
    }
    let mut i = 1;
    let mut label = String::new();
    while i < chars.len() && chars[i] != ']' {
        label.push(chars[i]);
        i += 1;
    }
    if i + 1 >= chars.len() || chars[i] != ']' || chars[i + 1] != '(' {
        return None;
    }
    i += 2;
    while i < chars.len() && chars[i] != ')' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    Some((label, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_and_list() {
        let lines = render("# Hi\n\n- one\n");
        assert!(lines[0].spans.iter().any(|s| s.content == "Hi"));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("one"))));
    }
}
