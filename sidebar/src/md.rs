//! Markdown → ratatui lines. GFM-ish + HTML unwrap. No leftover markup.
//! Task items (`- [ ]` / `- [x]`) stay clickable in the preview.

use std::collections::{HashMap, HashSet};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::truncate_to;

const FG: Color = Color::Rgb(0xe6, 0xed, 0xf3);
const MUTED: Color = Color::Rgb(0x8b, 0x94, 0x9e);
const LINK: Color = Color::Rgb(0x58, 0xa6, 0xff);
const HEAD: Color = Color::Rgb(0xff, 0xff, 0xff);
const H3: Color = Color::Rgb(0x79, 0xc0, 0xff);
const CODE_FG: Color = Color::Rgb(0xe3, 0xb3, 0x41);
const CODE_BG: Color = Color::Rgb(0x21, 0x26, 0x2d);
const FENCE_BG: Color = Color::Rgb(0x16, 0x1b, 0x22);
const GREEN: Color = Color::Rgb(0x89, 0xb4, 0x82);
const RULE: Color = Color::Rgb(0x30, 0x36, 0x3d);
const QUOTE: Color = Color::Rgb(0x8b, 0x94, 0x9e);

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
    render_at(src, 80).lines
}

pub fn render_full(src: &str) -> Rendered {
    render_at(src, 80)
}

pub fn render_at(src: &str, width: usize) -> Rendered {
    let width = width.max(16);
    let raw: Vec<&str> = src.lines().collect();
    let refs = collect_refs(&raw);
    let skip_refs: HashSet<usize> = refs.def_lines.iter().copied().collect();
    let mut out = Out {
        lines: Vec::new(),
        tasks: Vec::new(),
        width,
        center: Vec::new(),
        refs: refs.map,
    };
    let mut i = 0;
    // YAML frontmatter
    if raw.first().is_some_and(|l| l.trim() == "---") {
        i = 1;
        while i < raw.len() && raw[i].trim() != "---" {
            i += 1;
        }
        if i < raw.len() {
            i += 1;
        }
    }
    while i < raw.len() {
        if skip_refs.contains(&i) {
            i += 1;
            continue;
        }
        let line = raw[i];
        if let Some((ch, n, lang)) = fence_open(line) {
            i += 1;
            let mut body = Vec::new();
            while i < raw.len() {
                if fence_close(raw[i], ch, n) {
                    i += 1;
                    break;
                }
                body.push(raw[i].to_string());
                i += 1;
            }
            out.fence(lang, &body);
            continue;
        }
        if is_table_row(line) && raw.get(i + 1).is_some_and(|n| is_table_sep(n)) {
            let mut rows = vec![split_cells(line)];
            i += 1; // skip sep
            i += 1;
            while i < raw.len() && is_table_row(raw[i]) {
                rows.push(split_cells(raw[i]));
                i += 1;
            }
            out.table(rows);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("<!--") && !trimmed.contains("-->") {
            i += 1;
            while i < raw.len() && !raw[i].contains("-->") {
                i += 1;
            }
            if i < raw.len() {
                i += 1;
            }
            continue;
        }
        if let Some(kind) = classify_html(line) {
            i += 1;
            match kind {
                HtmlKind::Skip => {}
                HtmlKind::CenterPush(name) => out.center.push(name),
                HtmlKind::Close(name) => {
                    if out.center.last() == Some(&name) {
                        out.center.pop();
                    }
                }
                HtmlKind::Break => out.blank(),
                HtmlKind::Hr => out.hr(false),
                HtmlKind::Image(alt) => out.image(&alt),
                HtmlKind::Heading(level, text) => out.heading(level, &text),
                HtmlKind::HeadingOpen(level) => {
                    let close = format!("</h{level}>");
                    let mut inner = String::new();
                    while i < raw.len() {
                        let t = raw[i].trim();
                        if t.eq_ignore_ascii_case(&close) {
                            i += 1;
                            break;
                        }
                        let piece = inline_html(t).trim().to_string();
                        i += 1;
                        if piece.is_empty() {
                            continue;
                        }
                        if !inner.is_empty() {
                            inner.push(' ');
                        }
                        inner.push_str(&piece);
                    }
                    if !inner.is_empty() {
                        out.heading(level, &inner);
                    }
                }
                HtmlKind::Text(text) => {
                    let mut para = text;
                    while i < raw.len() {
                        if skip_refs.contains(&i) {
                            break;
                        }
                        let t = raw[i].trim();
                        if t == "·" || t == "•" {
                            para.push_str(" · ");
                            i += 1;
                            continue;
                        }
                        if let Some(HtmlKind::Text(more)) = classify_html(raw[i]) {
                            if !para.is_empty() && !para.ends_with(' ') {
                                para.push(' ');
                            }
                            para.push_str(&more);
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    out.paragraph(&para, None);
                }
            }
            continue;
        }
        if let Some((level, text)) = atx_heading(line) {
            out.heading(level, text);
            i += 1;
            continue;
        }
        if let Some(next) = raw.get(i + 1).copied() {
            if let Some(level) = setext_underline(next) {
                if !line.trim().is_empty() && !is_hr(line) {
                    out.heading(level, line.trim());
                    i += 2;
                    continue;
                }
            }
        }
        if is_hr(line) {
            out.hr(false);
            i += 1;
            continue;
        }
        if quote_level(line) > 0 {
            let mut body = Vec::new();
            while i < raw.len() && quote_level(raw[i]) > 0 {
                body.push(strip_quote(raw[i]));
                i += 1;
            }
            out.quote(&body.join(" "));
            continue;
        }
        if let Some(start) = unordered_prefix(line) {
            let indent = leading_ws(line);
            let body = &line[start..];
            if let Some((checked, text)) = parse_task(body) {
                out.task(indent, checked, text, i);
            } else {
                out.bullet(indent, body);
            }
            i += 1;
            continue;
        }
        if let Some((start, n)) = ordered_prefix(line) {
            let indent = leading_ws(line);
            let body = &line[start..];
            if let Some((checked, text)) = parse_task(body) {
                out.task(indent, checked, text, i);
            } else {
                out.ordered(indent, n, body);
            }
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            out.blank();
            i += 1;
            continue;
        }
        // Paragraph: join wrapped source lines.
        let mut para = String::new();
        let mut hard = false;
        while i < raw.len() {
            let l = raw[i];
            if skip_refs.contains(&i)
                || l.trim().is_empty()
                || block_start(l, raw.get(i + 1).copied())
            {
                break;
            }
            if classify_html(l).is_some() {
                break;
            }
            let piece = inline_html(l).trim().to_string();
            if piece.is_empty() {
                i += 1;
                continue;
            }
            let br = l.ends_with("  ") || l.ends_with('\\');
            if !para.is_empty() && !hard {
                para.push(' ');
            } else if hard {
                out.paragraph(&para, None);
                para.clear();
            }
            para.push_str(&piece);
            hard = br;
            i += 1;
        }
        if !para.is_empty() {
            out.paragraph(&para, None);
        }
    }
    if out.lines.is_empty() {
        out.lines.push(Line::raw(""));
    }
    Rendered {
        lines: out.lines,
        tasks: out.tasks,
    }
}

/// Flip `[ ]` ↔ `[x]` on a list task line. `None` if it is not a task item.
pub fn toggle_task_line(line: &str) -> Option<String> {
    let start = unordered_prefix(line).or_else(|| ordered_prefix(line).map(|(s, _)| s))?;
    let (checked, _) = parse_task(&line[start..])?;
    let mut out = line.to_string();
    out.replace_range(start + 1..start + 2, if checked { " " } else { "x" });
    Some(out)
}

struct Refs {
    map: HashMap<String, String>,
    def_lines: Vec<usize>,
}

fn collect_refs(lines: &[&str]) -> Refs {
    let mut map = HashMap::new();
    let mut def_lines = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some((id, url)) = parse_ref_def(line) {
            map.insert(id, url);
            def_lines.push(i);
        }
    }
    Refs { map, def_lines }
}

fn parse_ref_def(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find("]:")?;
    let id = t[1..close].trim();
    if id.is_empty() {
        return None;
    }
    let rest = t[close + 2..].trim();
    let url = rest.split_whitespace().next()?.trim_matches(['<', '>']);
    if url.is_empty() {
        return None;
    }
    Some((id.to_ascii_lowercase(), url.to_string()))
}

fn block_start(line: &str, next: Option<&str>) -> bool {
    fence_open(line).is_some()
        || atx_heading(line).is_some()
        || is_hr(line)
        || quote_level(line) > 0
        || unordered_prefix(line).is_some()
        || ordered_prefix(line).is_some()
        || (is_table_row(line) && next.is_some_and(is_table_sep))
        || next.is_some_and(|n| setext_underline(n).is_some() && !line.trim().is_empty())
}

struct Out {
    lines: Vec<Line<'static>>,
    tasks: Vec<TaskHit>,
    width: usize,
    center: Vec<String>,
    refs: HashMap<String, String>,
}

impl Out {
    fn emit(&mut self, line: Line<'static>) {
        let line = if !self.center.is_empty() {
            center_line(line, self.width)
        } else {
            margin(line)
        };
        self.lines.push(line);
    }

    fn emit_full(&mut self, line: Line<'static>) {
        self.lines.push(line);
    }

    fn blank(&mut self) {
        if self
            .lines
            .last()
            .is_some_and(|l| line_text(l).trim().is_empty())
        {
            return;
        }
        self.lines.push(Line::raw(""));
    }

    fn before_block(&mut self) {
        if !self.lines.is_empty() {
            self.blank();
        }
    }

    fn heading(&mut self, level: usize, text: &str) {
        self.before_block();
        let (fg, heavy) = match level {
            1 => (HEAD, true),
            2 => (HEAD, false),
            3 => (H3, false),
            _ => (H3, false),
        };
        let style = Style::default().fg(fg).add_modifier(Modifier::BOLD);
        self.emit(Line::from(inline(text, style, &self.refs)));
        if level <= 2 {
            let ch = if heavy { "━" } else { "─" };
            self.emit_full(Line::from(Span::styled(
                ch.repeat(self.width),
                Style::default().fg(RULE),
            )));
        }
        self.blank();
    }

    fn paragraph(&mut self, text: &str, base: Option<Style>) {
        let style = base.unwrap_or_else(body_style);
        self.emit(Line::from(inline(text, style, &self.refs)));
    }

    fn image(&mut self, alt: &str) {
        let alt = if alt.trim().is_empty() { "image" } else { alt };
        self.emit(Line::from(Span::styled(
            alt.to_string(),
            Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        )));
    }

    fn hr(&mut self, heavy: bool) {
        self.before_block();
        let ch = if heavy { "━" } else { "─" };
        self.emit_full(Line::from(Span::styled(
            ch.repeat(self.width),
            Style::default().fg(RULE),
        )));
        self.blank();
    }

    fn bullet(&mut self, indent: usize, body: &str) {
        let mut spans = vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(bullet_glyph(indent).to_string(), Style::default().fg(H3)),
        ];
        spans.extend(inline(body, body_style(), &self.refs));
        self.emit(Line::from(spans));
    }

    fn ordered(&mut self, indent: usize, n: u32, body: &str) {
        let mut spans = vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(format!("{n}. "), Style::default().fg(H3)),
        ];
        spans.extend(inline(body, body_style(), &self.refs));
        self.emit(Line::from(spans));
    }

    fn task(&mut self, indent: usize, checked: bool, text: &str, src_line: usize) {
        let boxg = if checked { "☑ " } else { "☐ " };
        let box_style = if checked {
            Style::default().fg(GREEN)
        } else {
            Style::default().fg(H3)
        };
        let text_style = if checked {
            Style::default().fg(MUTED)
        } else {
            body_style()
        };
        let mut spans = vec![Span::raw(" ".repeat(indent)), Span::styled(boxg, box_style)];
        spans.extend(inline(text, text_style, &self.refs));
        self.tasks.push(TaskHit {
            row: self.lines.len(),
            src_line,
        });
        self.emit(Line::from(spans));
    }

    fn quote(&mut self, text: &str) {
        let mut spans = vec![Span::styled("▎ ", Style::default().fg(RULE))];
        spans.extend(inline(
            text,
            Style::default().fg(QUOTE).add_modifier(Modifier::ITALIC),
            &self.refs,
        ));
        self.emit(Line::from(spans));
    }

    fn fence(&mut self, lang: &str, body: &[String]) {
        self.before_block();
        let bg = FENCE_BG;
        if !lang.is_empty() {
            let label = Line::from(vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!(" {lang} "),
                    Style::default()
                        .fg(MUTED)
                        .bg(bg)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]);
            self.emit_full(fill_bg(label, bg, self.width));
        }
        let text = body.join("\n");
        let highlighted = crate::syntax::highlight_lang(lang, &text, body.len().max(1));
        let rows: Vec<Line<'static>> = if let Some(lines) = highlighted {
            if lines.len() >= body.len() {
                lines
            } else {
                // highlighter can drop a trailing empty; pad
                let mut lines = lines;
                while lines.len() < body.len() {
                    lines.push(Line::raw(""));
                }
                lines
            }
        } else {
            body.iter()
                .map(|l| {
                    Line::from(Span::styled(
                        l.clone(),
                        Style::default().fg(Color::Rgb(0x9e, 0xaa, 0xb6)),
                    ))
                })
                .collect()
        };
        for line in rows {
            let mut spans = vec![Span::styled("  ", Style::default().bg(bg))];
            if line.spans.is_empty() {
                spans.push(Span::styled(" ", Style::default().bg(bg)));
            } else {
                for s in line.spans {
                    spans.push(Span::styled(s.content.to_string(), s.style.bg(bg)));
                }
            }
            let mut row = Line::from(spans);
            row.style = Style::default().bg(bg);
            self.emit_full(fill_bg(row, bg, self.width));
        }
        self.blank();
    }

    fn table(&mut self, rows: Vec<Vec<String>>) {
        if rows.is_empty() {
            return;
        }
        self.before_block();
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let mut widths = vec![3usize; cols];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                let w = display_width(&cell_text(cell, &self.refs));
                widths[i] = widths[i].max(w.max(1));
            }
        }
        let gaps = 3 * cols.saturating_sub(1);
        let budget = self.width.saturating_sub(1).max(cols * 3 + gaps);
        let mut total = widths.iter().sum::<usize>() + gaps;
        while total > budget {
            let Some((i, _)) = widths.iter().enumerate().max_by_key(|(_, w)| *w) else {
                break;
            };
            if widths[i] <= 3 {
                break;
            }
            widths[i] -= 1;
            total -= 1;
        }
        for (r, row) in rows.iter().enumerate() {
            let mut spans = Vec::new();
            for (c, width) in widths.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::raw("   "));
                }
                let raw_cell = row.get(c).map(String::as_str).unwrap_or("");
                let style = if r == 0 {
                    Style::default().fg(HEAD).add_modifier(Modifier::BOLD)
                } else {
                    body_style()
                };
                let cell_spans = inline(raw_cell, style, &self.refs);
                let text = cell_spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>();
                let shown = if display_width(&text) > *width {
                    truncate_to(text, *width)
                } else {
                    let pad = width.saturating_sub(display_width(&text));
                    format!("{text}{}", " ".repeat(pad))
                };
                spans.push(Span::styled(shown, style));
            }
            self.emit(Line::from(spans));
            if r == 0 {
                let mut rule = String::new();
                for (c, width) in widths.iter().enumerate() {
                    if c > 0 {
                        rule.push_str("   ");
                    }
                    rule.push_str(&"─".repeat(*width));
                }
                self.emit(Line::from(Span::styled(rule, Style::default().fg(RULE))));
            }
        }
        self.blank();
    }
}

fn body_style() -> Style {
    Style::default().fg(FG)
}

fn margin(line: Line<'static>) -> Line<'static> {
    if line.spans.is_empty() {
        return line;
    }
    let style = line.style;
    let mut spans = vec![Span::raw(" ")];
    spans.extend(line.spans);
    let mut out = Line::from(spans);
    out.style = style;
    out
}

fn center_line(line: Line<'static>, width: usize) -> Line<'static> {
    let w = line.width();
    if w >= width {
        return margin(line);
    }
    let pad = (width - w) / 2;
    let style = line.style;
    let mut spans = vec![Span::raw(" ".repeat(pad.max(1)))];
    spans.extend(line.spans);
    let mut out = Line::from(spans);
    out.style = style;
    out
}

fn fill_bg(mut line: Line<'static>, bg: Color, width: usize) -> Line<'static> {
    let pad = width.saturating_sub(line.width());
    if pad > 0 {
        line.spans
            .push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    }
    line.style = line.style.bg(bg);
    line
}

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn display_width(s: &str) -> usize {
    Span::raw(s).width()
}

fn cell_text(cell: &str, refs: &HashMap<String, String>) -> String {
    inline(cell, Style::default(), refs)
        .into_iter()
        .map(|s| s.content.to_string())
        .collect()
}

fn bullet_glyph(indent: usize) -> &'static str {
    match indent / 2 {
        0 => "• ",
        1 => "◦ ",
        _ => "▪ ",
    }
}

fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_end();
    let bytes = t.as_bytes();
    let mut n = 0;
    while n < bytes.len() && n < 6 && bytes[n] == b'#' {
        n += 1;
    }
    if n == 0 || bytes.get(n).copied() != Some(b' ') {
        return None;
    }
    let body = t[n..].trim().trim_end_matches('#').trim();
    Some((n, body))
}

fn setext_underline(line: &str) -> Option<usize> {
    let t = line.trim();
    if t.len() < 2 {
        return None;
    }
    if t.chars().all(|c| c == '=') {
        return Some(1);
    }
    if t.chars().all(|c| c == '-') && t.len() >= 3 && !t.contains('|') {
        return Some(2);
    }
    None
}

fn is_hr(line: &str) -> bool {
    let t = line.trim();
    let chars: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 3 {
        return false;
    }
    let first = chars.chars().next().unwrap_or(' ');
    (first == '-' || first == '*' || first == '_') && chars.chars().all(|c| c == first)
}

fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let indent = leading_ws(line);
    if indent > 3 {
        return None;
    }
    let t = line[indent..].trim_end();
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = t.chars().take_while(|&c| c == ch).count();
    if n < 3 {
        return None;
    }
    let rest = t[n..].trim();
    if ch == '`' && rest.contains('`') {
        return None;
    }
    Some((ch, n, rest))
}

fn fence_close(line: &str, ch: char, n: usize) -> bool {
    let indent = leading_ws(line);
    if indent > 3 {
        return false;
    }
    let t = line[indent..].trim_end();
    let got = t.chars().take_while(|&c| c == ch).count();
    got >= n && t.chars().nth(got).is_none()
}

fn quote_level(line: &str) -> usize {
    let mut n = 0;
    let mut rest = line.trim_start();
    while let Some(r) = rest.strip_prefix('>') {
        n += 1;
        rest = r.strip_prefix(' ').unwrap_or(r);
    }
    n
}

fn strip_quote(line: &str) -> String {
    let mut rest = line.trim_start();
    while let Some(r) = rest.strip_prefix('>') {
        rest = r.strip_prefix(' ').unwrap_or(r);
    }
    rest.to_string()
}

fn leading_ws(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

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

fn ordered_prefix(line: &str) -> Option<(usize, u32)> {
    let i = leading_ws(line);
    let rest = &line[i..];
    let bytes = rest.as_bytes();
    let mut j = 0;
    let mut n = 0u32;
    while j < bytes.len() && bytes[j].is_ascii_digit() && j < 9 {
        n = n
            .saturating_mul(10)
            .saturating_add((bytes[j] - b'0') as u32);
        j += 1;
    }
    if j == 0 {
        return None;
    }
    if !matches!(bytes.get(j), Some(b'.' | b')')) {
        return None;
    }
    if bytes.get(j + 1).copied() != Some(b' ') {
        return None;
    }
    Some((i + j + 2, n))
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

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_sep(line: &str) -> bool {
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim().trim_matches(':').trim();
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
}

fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn inline(src: &str, base: Style, refs: &HashMap<String, String>) -> Vec<Span<'static>> {
    let chars: Vec<char> = src.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut buf = String::new();
    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>, style: Style| {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    };
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            flush(&mut buf, &mut spans, base);
            let n = run_len(&chars, i, '`');
            if let Some(end) = find_run(&chars, i + n, '`', n) {
                let code: String = chars[i + n..end].iter().collect();
                spans.push(Span::styled(
                    format!(" {code} "),
                    Style::default().fg(CODE_FG).bg(CODE_BG),
                ));
                i = end + n;
                continue;
            }
        }
        if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            if let Some(end) = find_delim(&chars, i + 2, &['~', '~']) {
                flush(&mut buf, &mut spans, base);
                let inner: String = chars[i + 2..end].iter().collect();
                spans.push(Span::styled(
                    inner,
                    base.add_modifier(Modifier::CROSSED_OUT).fg(MUTED),
                ));
                i = end + 2;
                continue;
            }
        }
        if let Some((delim, style)) = emphasis_open(&chars, i, base) {
            if let Some(end) = find_delim(&chars, i + delim.len(), delim) {
                if delim == &['*'] || delim == &['_'] {
                    if delim == &['_'] && !underscore_ok(&chars, i, end) {
                        buf.push(chars[i]);
                        i += 1;
                        continue;
                    }
                }
                flush(&mut buf, &mut spans, base);
                let inner: String = chars[i + delim.len()..end].iter().collect();
                spans.extend(inline(&inner, style, refs));
                i = end + delim.len();
                continue;
            }
        }
        if chars[i] == '!' {
            if let Some((label, dest, skip)) = parse_link(&chars[i + 1..], refs) {
                flush(&mut buf, &mut spans, base);
                let alt = if label.is_empty() { dest } else { label };
                let alt = if alt.is_empty() { "image".into() } else { alt };
                spans.push(Span::styled(
                    alt,
                    Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
                ));
                i += 1 + skip;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((label, _dest, skip)) = parse_link(&chars[i..], refs) {
                flush(&mut buf, &mut spans, base);
                let link_style = Style::default().fg(LINK).add_modifier(Modifier::UNDERLINED);
                spans.extend(inline(&label, link_style, refs));
                i += skip;
                continue;
            }
        }
        if chars[i] == '<' {
            if let Some((label, skip)) = parse_autolink(&chars[i..]) {
                flush(&mut buf, &mut spans, base);
                spans.push(Span::styled(
                    label,
                    Style::default().fg(LINK).add_modifier(Modifier::UNDERLINED),
                ));
                i += skip;
                continue;
            }
        }
        if let Some(skip) = url_start(&chars, i) {
            flush(&mut buf, &mut spans, base);
            let url: String = chars[i..i + skip].iter().collect();
            spans.push(Span::styled(
                url,
                Style::default().fg(LINK).add_modifier(Modifier::UNDERLINED),
            ));
            i += skip;
            continue;
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

fn emphasis_open(chars: &[char], i: usize, base: Style) -> Option<(&'static [char], Style)> {
    let rest = chars.len() - i;
    if rest >= 3 && chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*' {
        return Some((
            &['*', '*', '*'],
            base.add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ));
    }
    if rest >= 3 && chars[i] == '_' && chars[i + 1] == '_' && chars[i + 2] == '_' {
        return Some((
            &['_', '_', '_'],
            base.add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ));
    }
    if rest >= 2 && chars[i] == '*' && chars[i + 1] == '*' {
        return Some((&['*', '*'], base.add_modifier(Modifier::BOLD)));
    }
    if rest >= 2 && chars[i] == '_' && chars[i + 1] == '_' {
        return Some((&['_', '_'], base.add_modifier(Modifier::BOLD)));
    }
    if chars[i] == '*' && (i + 1 >= chars.len() || chars[i + 1] != ' ') {
        return Some((&['*'], base.add_modifier(Modifier::ITALIC)));
    }
    if chars[i] == '_' && (i + 1 >= chars.len() || chars[i + 1] != ' ') {
        return Some((&['_'], base.add_modifier(Modifier::ITALIC)));
    }
    None
}

fn underscore_ok(chars: &[char], start: usize, end: usize) -> bool {
    let prev_ok = start == 0 || !chars[start - 1].is_alphanumeric();
    let next_ok =
        end + 1 >= chars.len() || !chars.get(end + 1).is_some_and(|c| c.is_alphanumeric());
    prev_ok && next_ok
}

fn run_len(chars: &[char], i: usize, ch: char) -> usize {
    chars[i..].iter().take_while(|&&c| c == ch).count()
}

fn find_run(chars: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i + n <= chars.len() {
        if run_len(chars, i, ch) == n {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_delim(chars: &[char], from: usize, delim: &[char]) -> Option<usize> {
    let mut i = from;
    while i + delim.len() <= chars.len() {
        if chars[i..].starts_with(delim) && i > from.saturating_sub(0) && i >= from {
            if i == from {
                i += 1;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link(chars: &[char], refs: &HashMap<String, String>) -> Option<(String, String, usize)> {
    if chars.first() != Some(&'[') {
        return None;
    }
    let mut i = 1;
    let mut label = String::new();
    let mut depth = 1;
    while i < chars.len() {
        if chars[i] == '[' {
            depth += 1;
            label.push('[');
        } else if chars[i] == ']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            label.push(']');
        } else {
            label.push(chars[i]);
        }
        i += 1;
    }
    if i >= chars.len() || chars[i] != ']' {
        return None;
    }
    i += 1;
    if i < chars.len() && chars[i] == '(' {
        i += 1;
        let mut dest = String::new();
        while i < chars.len() && chars[i] != ')' && !chars[i].is_whitespace() {
            dest.push(chars[i]);
            i += 1;
        }
        while i < chars.len() && chars[i] != ')' {
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        return Some((label, dest, i + 1));
    }
    if i < chars.len() && chars[i] == '[' {
        i += 1;
        let mut id = String::new();
        while i < chars.len() && chars[i] != ']' {
            id.push(chars[i]);
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        let key = if id.is_empty() {
            label.to_ascii_lowercase()
        } else {
            id.to_ascii_lowercase()
        };
        let dest = refs.get(&key).cloned().unwrap_or_default();
        return Some((label, dest, i + 1));
    }
    let dest = refs.get(&label.to_ascii_lowercase()).cloned()?;
    Some((label, dest, i))
}

fn parse_autolink(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'<') {
        return None;
    }
    let mut i = 1;
    let mut url = String::new();
    while i < chars.len() && chars[i] != '>' {
        if chars[i].is_whitespace() {
            return None;
        }
        url.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        return None;
    }
    Some((url, i + 1))
}

fn url_start(chars: &[char], i: usize) -> Option<usize> {
    let rest: String = chars[i..].iter().take(8).collect();
    let prefix = if rest.starts_with("https://") {
        8
    } else if rest.starts_with("http://") {
        7
    } else {
        return None;
    };
    if i > 0 && matches!(chars[i - 1], '<' | '(' | '[') {
        return None;
    }
    let mut n = prefix;
    while i + n < chars.len() {
        let c = chars[i + n];
        if c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '<' | '`') {
            break;
        }
        n += 1;
    }
    while n > prefix && matches!(chars[i + n - 1], '.' | ',' | ';' | ':' | '!') {
        n -= 1;
    }
    (n > prefix).then_some(n)
}

// --- HTML ---------------------------------------------------------------

enum HtmlKind {
    Skip,
    CenterPush(String),
    Close(String),
    Break,
    Hr,
    Image(String),
    Heading(usize, String),
    HeadingOpen(usize),
    Text(String),
}

enum Tag {
    Comment,
    Open {
        name: String,
        attrs: String,
        self_close: bool,
    },
    Close {
        name: String,
    },
}

fn classify_html(line: &str) -> Option<HtmlKind> {
    let t = line.trim();
    if t.is_empty() || !t.starts_with('<') {
        return None;
    }
    if t.starts_with("<!--") {
        return Some(HtmlKind::Skip);
    }
    let (tag, n) = parse_tag(t)?;
    let rest = t[n..].trim();
    match tag {
        Tag::Comment => Some(HtmlKind::Skip),
        Tag::Close { name } => Some(HtmlKind::Close(name)),
        Tag::Open {
            name,
            attrs,
            self_close,
        } => {
            if name == "br" {
                return Some(HtmlKind::Break);
            }
            if name == "hr" {
                return Some(HtmlKind::Hr);
            }
            if name == "img" {
                return Some(HtmlKind::Image(img_alt(&attrs)));
            }
            let center = has_center(&attrs) || name == "center";
            if rest.is_empty() {
                if self_close {
                    return if is_html_name(&name) {
                        Some(HtmlKind::Skip)
                    } else {
                        None
                    };
                }
                if let Some(level) = heading_tag(&name) {
                    return Some(HtmlKind::HeadingOpen(level));
                }
                if center {
                    return Some(HtmlKind::CenterPush(name));
                }
                if is_html_name(&name) {
                    return Some(HtmlKind::Skip);
                }
                return None;
            }
            let (inner, end) = find_close_tag(rest, &name)?;
            if !rest[end..].trim().is_empty() {
                return None;
            }
            let inner = inline_html(&inner);
            if let Some(level) = heading_tag(&name) {
                return Some(HtmlKind::Heading(level, inner));
            }
            if name == "img" {
                return Some(HtmlKind::Image(if inner.is_empty() {
                    img_alt(&attrs)
                } else {
                    inner
                }));
            }
            if inner.trim().is_empty() {
                return Some(if center {
                    HtmlKind::CenterPush(name)
                } else {
                    HtmlKind::Skip
                });
            }
            Some(HtmlKind::Text(inner))
        }
    }
}

fn is_html_name(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr"
            | "b"
            | "blockquote"
            | "br"
            | "center"
            | "code"
            | "del"
            | "div"
            | "em"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "i"
            | "img"
            | "kbd"
            | "li"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "s"
            | "section"
            | "small"
            | "span"
            | "strike"
            | "strong"
            | "sub"
            | "sup"
            | "table"
            | "tbody"
            | "td"
            | "th"
            | "thead"
            | "tr"
            | "u"
            | "ul"
    )
}

fn heading_tag(name: &str) -> Option<usize> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn has_center(attrs: &str) -> bool {
    let a = attrs.to_ascii_lowercase();
    a.contains("align=\"center\"")
        || a.contains("align='center'")
        || a.contains("align=center")
        || a.contains("text-align:center")
        || a.contains("text-align: center")
}

fn img_alt(attrs: &str) -> String {
    attr(attrs, "alt")
        .filter(|s| !s.is_empty())
        .or_else(|| attr(attrs, "src").map(basename))
        .unwrap_or_else(|| "image".into())
}

fn basename(path: String) -> String {
    path.rsplit('/').next().unwrap_or(&path).to_string()
}

fn attr(attrs: &str, key: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let key_l = key.to_ascii_lowercase();
    let mut search = 0;
    while let Some(pos) = lower[search..].find(&key_l) {
        let at = search + pos;
        let after = at + key_l.len();
        let rest = attrs[after..].trim_start();
        if !rest.starts_with('=') {
            search = after;
            continue;
        }
        let rest = rest[1..].trim_start();
        let (q, rest) = match rest.chars().next() {
            Some(c @ ('"' | '\'')) => (c, &rest[c.len_utf8()..]),
            _ => {
                let v: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '>')
                    .collect();
                return Some(decode_entities(&v));
            }
        };
        let end = rest.find(q)?;
        return Some(decode_entities(&rest[..end]));
    }
    None
}

fn parse_tag(s: &str) -> Option<(Tag, usize)> {
    if !s.starts_with('<') {
        return None;
    }
    if s.starts_with("<!--") {
        let end = s.find("-->").map(|i| i + 3).unwrap_or(s.len());
        return Some((Tag::Comment, end));
    }
    let bytes = s.as_bytes();
    let mut i = 1;
    let close = if bytes.get(i) == Some(&b'/') {
        i += 1;
        true
    } else {
        false
    };
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = s[name_start..i].to_ascii_lowercase();
    let attrs_start = i;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            let attrs = s[attrs_start..i].trim();
            let self_close = attrs.ends_with('/')
                || matches!(
                    name.as_str(),
                    "br" | "img" | "hr" | "input" | "meta" | "link" | "source"
                );
            let attrs = attrs.trim_end_matches('/').trim().to_string();
            i += 1;
            let tag = if close {
                Tag::Close { name }
            } else {
                Tag::Open {
                    name,
                    attrs,
                    self_close,
                }
            };
            return Some((tag, i));
        }
        i += 1;
    }
    None
}

fn find_close_tag(s: &str, name: &str) -> Option<(String, usize)> {
    let lower = s.to_ascii_lowercase();
    let open_pat = format!("<{name}");
    let close_pat = format!("</{name}>");
    let mut depth = 1;
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with(&close_pat) {
            depth -= 1;
            if depth == 0 {
                return Some((s[..i].to_string(), i + close_pat.len()));
            }
            i += close_pat.len();
            continue;
        }
        if lower[i..].starts_with(&open_pat) {
            let after = i + open_pat.len();
            let next = lower.as_bytes().get(after).copied();
            if next.is_none_or(|b| b == b' ' || b == b'>' || b == b'/' || b == b'\n') {
                depth += 1;
            }
        }
        i += 1;
    }
    None
}

fn inline_html(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with('`') {
            let n = s[i..].bytes().take_while(|&b| b == b'`').count();
            out.push_str(&s[i..i + n]);
            i += n;
            if let Some(rel) = s[i..].find(&"`".repeat(n)) {
                out.push_str(&s[i..i + rel + n]);
                i += rel + n;
            }
            continue;
        }
        if s[i..].starts_with("<!--") {
            i = s[i..].find("-->").map(|n| i + n + 3).unwrap_or(s.len());
            continue;
        }
        if s[i..].starts_with('<') {
            if let Some((tag, n)) = parse_tag(&s[i..]) {
                match tag {
                    Tag::Comment => {
                        i += n;
                        continue;
                    }
                    Tag::Close { name } => {
                        if is_html_name(&name) {
                            i += n;
                            continue;
                        }
                    }
                    Tag::Open {
                        name,
                        attrs,
                        self_close,
                    } => {
                        if is_html_name(&name) {
                            i += n;
                            if self_close {
                                out.push_str(&void_html(&name, &attrs));
                                continue;
                            }
                            if let Some((inner, end)) = find_close_tag(&s[i..], &name) {
                                i += end;
                                out.push_str(&wrap_html(&name, &attrs, &inner));
                            }
                            continue;
                        }
                    }
                }
            }
        }
        if s[i..].starts_with('&') {
            if let Some((decoded, n)) = entity(&s[i..]) {
                out.push_str(&decoded);
                i += n;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn void_html(name: &str, attrs: &str) -> String {
    match name {
        "br" => "\n".into(),
        "img" => format!("![{}]()", img_alt(attrs)),
        _ => String::new(),
    }
}

fn wrap_html(name: &str, attrs: &str, inner: &str) -> String {
    let inner = inline_html(inner);
    match name {
        "a" => {
            let href = attr(attrs, "href").unwrap_or_default();
            if inner.is_empty() {
                inner
            } else {
                format!("[{inner}]({href})")
            }
        }
        "strong" | "b" => format!("**{inner}**"),
        "em" | "i" => format!("*{inner}*"),
        "code" => format!("`{inner}`"),
        "s" | "del" | "strike" => format!("~~{inner}~~"),
        _ => inner,
    }
}

fn entity(s: &str) -> Option<(String, usize)> {
    let end = s.find(';')?;
    if end == 0 || end > 10 {
        return None;
    }
    let body = &s[1..end];
    let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
    } else if let Some(num) = body.strip_prefix('#') {
        num.parse::<u32>().ok().and_then(char::from_u32)
    } else {
        match body {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "mdash" => Some('-'),
            "ndash" => Some('-'),
            _ => None,
        }
    }?;
    Some((ch.to_string(), end + 1))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with('&') {
            if let Some((d, n)) = entity(&s[i..]) {
                out.push_str(&d);
                i += n;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'static>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn heading_and_list() {
        let lines = render("# Hi\n\n- one\n");
        assert!(lines[0].spans.iter().any(|s| s.content == "Hi"));
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("one")))
        );
    }

    #[test]
    fn task_items_render_and_map_source() {
        let md = render_full("- [ ] open\n- [x] done\n  - [ ] nested\n");
        assert_eq!(md.tasks.len(), 3);
        assert_eq!(
            md.tasks[0],
            TaskHit {
                row: 0,
                src_line: 0
            }
        );
        assert_eq!(md.tasks[2].src_line, 2);
        let joined = plain(&md.lines);
        assert!(joined.contains('☐'), "{joined}");
        assert!(joined.contains('☑'), "{joined}");
        assert!(joined.contains("nested"), "{joined}");
    }

    #[test]
    fn tasks_inside_fences_are_not_clickable() {
        let md = render_full("```\n- [ ] no\n```\n- [ ] yes\n");
        assert_eq!(md.tasks.len(), 1);
        assert_eq!(md.tasks[0].src_line, 3);
        assert!(!md.tasks.iter().any(|t| t.src_line == 1));
    }

    #[test]
    fn toggle_task_line_flips_marker() {
        assert_eq!(toggle_task_line("- [ ] a").as_deref(), Some("- [x] a"));
        assert_eq!(toggle_task_line("- [x] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_task_line("* [X] a").as_deref(), Some("* [ ] a"));
        assert_eq!(
            toggle_task_line("  - [ ] nested").as_deref(),
            Some("  - [x] nested")
        );
        assert_eq!(toggle_task_line("- not a task"), None);
        assert_eq!(toggle_task_line("- [link](url)"), None);
    }

    #[test]
    fn inline_markup_does_not_leak() {
        let text = plain(&render(
            "*italic* **bold** ~~strike~~ `code` [hi](http://x) ![alt](pic.png)\n",
        ));
        assert!(!text.contains('*'), "{text}");
        assert!(!text.contains("~~"), "{text}");
        assert!(!text.contains('`'), "{text}");
        assert!(!text.contains("]("), "{text}");
        assert!(!text.contains("!["), "{text}");
        assert!(text.contains("italic"), "{text}");
        assert!(text.contains("bold"), "{text}");
        assert!(text.contains("hi"), "{text}");
        assert!(text.contains("alt"), "{text}");
        assert!(!text.contains("http://x"), "{text}");
        assert!(!text.contains("pic.png"), "{text}");
    }

    #[test]
    fn html_and_tables_look_rendered() {
        let src = r#"<div align="center">
<h1>Title</h1>
<img alt="hero shot" src="a.png">
</div>
<br />
## Hello **World**

| A | B |
|---|---|
| 1 | 2 |

1. first
- second
"#;
        let text = plain(&render(src));
        for needle in ["<div", "<br", "<h1", "<img", "</", "|---", "**", "![", "]("] {
            assert!(!text.contains(needle), "leaked {needle:?}\n{text}");
        }
        assert!(text.contains("Title"), "{text}");
        assert!(text.contains("Hello"), "{text}");
        assert!(text.contains("World"), "{text}");
        assert!(text.contains("hero shot"), "{text}");
        assert!(text.contains("first"), "{text}");
        assert!(text.contains('•'), "{text}");
        let nav = r#"<a href="https://herdr.dev"><strong>herdr</strong></a>
    ·
    <a href="https://github.com/x"><strong>GitHub</strong></a>
"#;
        let nav_text = plain(&render(nav));
        assert!(nav_text.contains("herdr · GitHub"), "{nav_text}");
        let after = plain(&render(
            "<p align=\"center\">\n<img alt=\"pic\" src=\"x.png\">\n</p>\n\n## Keys\n\nHello\n",
        ));
        let keys = after.lines().find(|l| l.contains("Keys")).unwrap_or("");
        assert!(
            keys.trim_start().starts_with("Keys") && keys.len() - keys.trim_start().len() <= 2,
            "Keys still centered after </p>\n{after}"
        );
    }

    #[test]
    fn paragraphs_join_wrapped_source() {
        let text = plain(&render("foo\nbar\n\nbaz\n"));
        assert!(text.contains("foo bar"), "{text}");
        assert!(text.contains("baz"), "{text}");
    }

    #[test]
    fn readme_has_no_raw_markup() {
        let src = include_str!("../../README.md");
        let text = plain(&render(src));
        for needle in [
            "<div",
            "<br",
            "<h1",
            "<h3",
            "<img",
            "<p ",
            "</div",
            "</p>",
            "|-----",
            "**Explorer**",
            "![",
            "](http",
        ] {
            assert!(!text.contains(needle), "leaked {needle:?}");
        }
        assert!(text.contains("herdr-plugins"), "{text}");
        assert!(text.contains("Quick Start"), "{text}");
        assert!(text.contains("herdr-sidebar"), "{text}");
        assert!(
            text.contains("herdr-sidebar-preview-<pane>.ctl"),
            "code span ate <pane>\n{text}"
        );
    }
}
