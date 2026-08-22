//! Tiny markdown → ratatui lines. Headings, lists, fences, inline ** * `.
//! Task items (`- [ ]` / `- [x]`) are clickable in the preview.

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

/// A rendered task-list row the preview can click to toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskHit {
    pub row: usize,
    pub src_line: usize,
}

pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub tasks: Vec<TaskHit>,
}

pub fn render(src: &str) -> Vec<Line<'static>> {
    render_full(src).lines
}

pub fn render_full(src: &str) -> Rendered {
    let mut out = Vec::new();
    let mut tasks = Vec::new();
    let mut fence: Option<String> = None;
    for (src_line, raw) in src.lines().enumerate() {
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
        if let Some(start) = unordered_prefix(line) {
            let indent = leading_ws(line);
            let body = &line[start..];
            if let Some((checked, text)) = parse_task(body) {
                let boxg = if checked { "☑ " } else { "☐ " };
                let box_style = if checked {
                    Style::default().fg(Color::Rgb(0x89, 0xb4, 0x82))
                } else {
                    Style::default().fg(Color::LightBlue)
                };
                let text_style = if checked {
                    Style::default().fg(Color::Rgb(0x9e, 0xaa, 0xb6))
                } else {
                    Style::default()
                };
                let mut spans = vec![
                    Span::raw(" ".repeat(indent)),
                    Span::styled(boxg, box_style),
                ];
                spans.extend(inline(text, text_style));
                tasks.push(TaskHit { row: out.len(), src_line });
                out.push(Line::from(spans));
                continue;
            }
            let mut spans = vec![
                Span::raw(" ".repeat(indent)),
                Span::styled("• ", Style::default().fg(Color::LightBlue)),
            ];
            spans.extend(inline(body, Style::default()));
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
    Rendered { lines: out, tasks }
}

/// Flip `[ ]` ↔ `[x]` on a list task line. `None` if it is not a task item.
pub fn toggle_task_line(line: &str) -> Option<String> {
    let start = unordered_prefix(line)?;
    let (checked, _) = parse_task(&line[start..])?;
    let mut out = line.to_string();
    out.replace_range(start + 1..start + 2, if checked { " " } else { "x" });
    Some(out)
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

fn leading_ws(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ' || *b == b'\t').count()
}

/// Byte index of the list body after `- ` / `* ` / `+ `, allowing indent.
fn unordered_prefix(line: &str) -> Option<usize> {
    let i = leading_ws(line);
    let rest = &line[i..];
    for p in ["- ", "* ", "+ "] {
        if rest.starts_with(p) {
            return Some(i + p.len());
        }
    }
    None
}

fn parse_task(body: &str) -> Option<(bool, &str)> {
    let b = body.as_bytes();
    if b.len() < 3 || b[0] != b'[' || b[2] != b']' {
        return None;
    }
    let checked = match b[1] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    let rest = &body[3..];
    Some((checked, rest.strip_prefix(' ').unwrap_or(rest)))
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

    #[test]
    fn task_items_render_and_map_source() {
        let md = render_full("- [ ] open\n- [x] done\n  - [ ] nested\n");
        assert_eq!(md.tasks.len(), 3);
        assert_eq!(md.tasks[0], TaskHit { row: 0, src_line: 0 });
        assert_eq!(md.tasks[2].src_line, 2);
        let joined: String = md.lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.as_ref())).collect();
        assert!(joined.contains('☐'), "{joined}");
        assert!(joined.contains('☑'), "{joined}");
        assert!(joined.contains("nested"), "{joined}");
    }

    #[test]
    fn tasks_inside_fences_are_not_clickable() {
        let md = render_full("```\n- [ ] no\n```\n- [ ] yes\n");
        assert_eq!(md.tasks, vec![TaskHit { row: 3, src_line: 3 }]);
    }

    #[test]
    fn toggle_task_line_flips_marker() {
        assert_eq!(toggle_task_line("- [ ] a").as_deref(), Some("- [x] a"));
        assert_eq!(toggle_task_line("- [x] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_task_line("* [X] a").as_deref(), Some("* [ ] a"));
        assert_eq!(toggle_task_line("  - [ ] nested").as_deref(), Some("  - [x] nested"));
        assert_eq!(toggle_task_line("- not a task"), None);
        assert_eq!(toggle_task_line("- [link](url)"), None);
    }
}
