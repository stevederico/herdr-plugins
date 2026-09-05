//! Shared UI vocabulary for both sidebar views: keycap footer hints, the
//! activity-bar / settings / suggest glyphs, hit-test helpers, and the
//! pane-list parsing both views use to find their sibling panes. One home —
//! these used to be copy-mirrored between two crates.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::icons::IconTheme;
use crate::state::View;

/// Keycap chip colors for the footer hints — a subtle "keyboard key" look
/// instead of a wall of dim text.
pub const KEYCAP_BG: Color = Color::Rgb(0x32, 0x36, 0x3d);
pub const KEYCAP_FG: Color = Color::Rgb(0xc9, 0xce, 0xd6);

/// Rendered width of one `key label` hint: keycap padding + gap + label.
fn hint_width(key: &str, label: &str) -> usize {
    Span::raw(key).width() + 2 + 1 + Span::raw(label).width()
}

/// Pack hotkey hints into as many footer lines as they need at `width`
/// (max 4), instead of clipping — each as a keycap chip plus a dim label.
/// `reserve` columns stay free on the LAST line (for a corner button).
pub fn wrap_hints(
    hints: &[(&'static str, &'static str)],
    width: u16,
    reserve: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width.max(8));
    let reserve = usize::from(reserve);
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used: usize = 1;
    for (key, label) in hints {
        let w = hint_width(key, label);
        let empty = lines.last().is_some_and(Vec::is_empty);
        if !empty && used + 2 + w > width.saturating_sub(reserve) {
            lines.push(Vec::new());
            used = 1;
        }
        let line = lines.last_mut().unwrap();
        line.push(Span::raw(if line.is_empty() { " " } else { "  " }));
        line.push(Span::styled(
            format!(" {key} "),
            Style::default().bg(KEYCAP_BG).fg(KEYCAP_FG),
        ));
        line.push(Span::styled(format!(" {label}"), Style::default().dim()));
        used += if line.len() == 3 { w } else { 2 + w };
    }
    lines.into_iter().map(Line::from).collect()
}

/// A subtle right-edge scrollbar when the list overflows its viewport.
/// Purely an indicator: the wheel scrolls, the bar just shows where.
pub fn draw_scrollbar(frame: &mut Frame, area: Rect, total: usize, viewport: usize, pos: usize) {
    if total <= viewport || area.width == 0 || area.height == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(viewport)).position(pos);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("┃")
            .track_style(Style::default().dim())
            .thumb_style(Style::default()),
        area,
        &mut state,
    );
}

/// Rows the unified activity bar occupies (icon row + half-block caps).
pub const ACTIVITY_BAR_ROWS: u16 = 3;

/// True when a click at pane-local (column, row) lands on the `«` collapse
/// button: the 3-cell region at the right end of the footer line, mirroring
/// herdr's own sidebar collapse control. `bottom_inset` skips a docked
/// activity bar sitting below that footer.
pub fn hits_collapse_button(
    column: u16,
    row: u16,
    pane_width: u16,
    pane_height: u16,
    bottom_inset: u16,
) -> bool {
    row == pane_height.saturating_sub(1 + bottom_inset)
        && column >= pane_width.saturating_sub(4)
}

/// Settings popover: sits above `anchor` (the ⚙), right-aligned to it,
/// clamped inside `area`. Shrinks rather than covering the button.
pub fn popover_above(area: Rect, anchor: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width).max(1);
    let bottom = if anchor.height > 0 {
        anchor.y
    } else {
        area.y + area.height
    };
    let avail = bottom.saturating_sub(area.y).max(1);
    let height = height.min(avail).min(area.height).max(1);
    let y = bottom.saturating_sub(height);
    let right = if anchor.width > 0 {
        (anchor.x + anchor.width).min(area.x + area.width)
    } else {
        area.x + area.width
    };
    let x = right.saturating_sub(width).max(area.x);
    Rect::new(x, y, width, height)
}

/// Theme-matched activity-bar icons: (explorer, source control). Both FA
/// glyphs render two cells wide in the non-Mono Nerd Font — chips reserve
/// the second cell (see the activity-bar renderer).
pub fn activity_icons(theme: IconTheme) -> (&'static str, &'static str) {
    match theme {
        IconTheme::Material => ("\u{f07b}", "\u{f126}"),
        IconTheme::Emoji => ("📁", "🔀"),
    }
}

/// Theme-matched ⚙ settings glyph.
pub fn gear_icon(theme: IconTheme) -> &'static str {
    match theme {
        IconTheme::Material => "\u{f013}",
        IconTheme::Emoji => "⚙",
    }
}

/// Monochrome outline sparkles for the suggest button: MDI "creation"
/// (the classic three-sparkle ✨ silhouette) with a text-presentation
/// fallback for the emoji theme.
pub fn sparkle_icon(theme: IconTheme) -> &'static str {
    match theme {
        IconTheme::Material => "\u{f0674}",
        IconTheme::Emoji => "✧",
    }
}

/// Theme-matched branch glyph for repo rows.
pub fn branch_icon(theme: IconTheme) -> &'static str {
    match theme {
        IconTheme::Material => "\u{e725}",
        IconTheme::Emoji => "⎇",
    }
}

/// One VS Code-style title-bar action button (the hover row at the top-right
/// of a panel's title bar).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitleAction {
    GoUp,
    NewFile,
    NewFolder,
    Refresh,
    CollapseAll,
}

/// How long the title-bar action buttons stay visible after the last mouse
/// event. Terminals emit no "mouse left the pane" event, so hover can only be
/// approximated: any mouse activity over the pane shows the buttons, and they
/// fade after this linger (any further motion re-shows them instantly).
pub const TITLE_ACTIONS_LINGER: std::time::Duration = std::time::Duration::from_secs(3);

/// The hover approximation described on [`TITLE_ACTIONS_LINGER`].
pub fn title_actions_visible(last_mouse: Option<std::time::Instant>) -> bool {
    last_mouse.is_some_and(|at| at.elapsed() < TITLE_ACTIONS_LINGER)
}

/// Theme-matched glyph for a title-bar action: VS Code's own codicons in the
/// material theme (the Nerd Font ships the cod- set), VS16-free fallbacks
/// otherwise.
pub fn title_action_icon(theme: IconTheme, action: TitleAction) -> &'static str {
    match (theme, action) {
        (IconTheme::Material, TitleAction::GoUp) => "\u{eaa1}", //  cod-arrow_up
        (IconTheme::Material, TitleAction::NewFile) => "\u{ea7f}", //  cod-new_file
        (IconTheme::Material, TitleAction::NewFolder) => "\u{ea80}", //  cod-new_folder
        (IconTheme::Material, TitleAction::Refresh) => "\u{eb37}", //  cod-refresh
        (IconTheme::Material, TitleAction::CollapseAll) => "\u{eac5}", //  cod-collapse_all
        (IconTheme::Emoji, TitleAction::GoUp) => "↑",
        (IconTheme::Emoji, TitleAction::NewFile) => "📄",
        (IconTheme::Emoji, TitleAction::NewFolder) => "📁",
        (IconTheme::Emoji, TitleAction::Refresh) => "⟳",
        (IconTheme::Emoji, TitleAction::CollapseAll) => "⊟",
    }
}

/// A button's rendered chip: one space each side, NO extra slack cell. The
/// Mono Nerd Font build renders codicons in a single cell, so a trailing
/// slack cell (as the activity bar uses) pushes the glyph's center left of
/// the chip's — its right edge lands mid-chip (user-reported). In the
/// non-Mono build the glyph just overflows into its own trailing space, which
/// is how the tree's file icons already render.
fn title_action_chip(theme: IconTheme, action: TitleAction) -> String {
    format!(" {} ", title_action_icon(theme, action))
}

/// Total rendered width of the button row, for right-aligning it.
pub fn title_actions_width(theme: IconTheme, actions: &[TitleAction]) -> u16 {
    actions
        .iter()
        .map(|&a| Span::raw(title_action_chip(theme, a)).width() as u16)
        .sum()
}

/// Build the title-bar buttons as spans (left edge at `x` on row `y`) plus
/// their click zones for hit-testing. The chip under `hover` renders with a
/// keycap background so the mouse target is visible before the click.
pub fn title_action_spans(
    theme: IconTheme,
    actions: &[TitleAction],
    x: u16,
    y: u16,
    hover: Option<(u16, u16)>,
) -> (Vec<Span<'static>>, Vec<(Rect, TitleAction)>) {
    let mut spans = Vec::new();
    let mut zones = Vec::new();
    let mut cx = x;
    for &action in actions {
        let chip = title_action_chip(theme, action);
        let w = Span::raw(chip.as_str()).width() as u16;
        let rect = Rect::new(cx, y, w, 1);
        let hovered = hover.is_some_and(|(hx, hy)| hits(rect, hx, hy));
        // GoUp is navigation, not a utility chip — dim made the arrow vanish.
        let style = if action == TitleAction::GoUp {
            let s = Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD);
            if hovered { s.bg(KEYCAP_BG) } else { s }
        } else if hovered {
            Style::default().bg(KEYCAP_BG)
        } else {
            Style::default().dim()
        };
        spans.push(Span::styled(chip, style));
        zones.push((rect, action));
        cx += w;
    }
    (spans, zones)
}

pub fn within(x: u16, (start, end): (u16, u16)) -> bool {
    (start..end).contains(&x)
}

pub fn hits(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// Cut `s` down to at most `max` display columns, ending in `…` when trimmed.
/// Empty when even the ellipsis wouldn't fit.
/// Word-wrap `s` to `width` display columns. Returns byte ranges into `s`.
/// One range even when `s` is empty. Breaks on spaces; overlong tokens hard-break.
pub fn wrap_cols(s: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    if s.is_empty() {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut col = 0;
        let mut end = start;
        let mut last_space: Option<usize> = None;
        for (off, c) in s[start..].char_indices() {
            let i = start + off;
            let cw = Span::raw(c.to_string()).width().max(1);
            if col + cw > width && end > start {
                break;
            }
            if c == ' ' {
                last_space = Some(i);
            }
            col += cw;
            end = i + c.len_utf8();
        }
        if end < s.len()
            && let Some(sp) = last_space
            && sp > start
            && sp < end
        {
            end = sp;
        }
        if end == start {
            let c = s[start..].chars().next();
            end = start + c.map(char::len_utf8).unwrap_or(1);
        }
        out.push((start, end));
        start = end;
        if start < s.len() && s.as_bytes()[start] == b' ' {
            start += 1;
        }
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    out
}

/// Slice a styled line to the byte range `[start, end)` of its concatenated text.
pub fn slice_line(line: &Line<'static>, start: usize, end: usize) -> Line<'static> {
    if start >= end {
        return Line::styled(String::new(), line.style);
    }
    let mut off = 0;
    let mut spans = Vec::new();
    for sp in &line.spans {
        let len = sp.content.len();
        let a = off.max(start);
        let b = (off + len).min(end);
        if a < b && a >= off && b <= off + len {
            let rel_a = a - off;
            let rel_b = b - off;
            if rel_a < rel_b && rel_a <= sp.content.len() && rel_b <= sp.content.len() {
                spans.push(Span::styled(sp.content[rel_a..rel_b].to_string(), sp.style));
            }
        }
        off += len;
    }
    let mut out = Line::from(spans);
    out.style = line.style;
    out
}

/// Word-wrap one styled line to `width` columns.
pub fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    wrap_cols(&text, width)
        .into_iter()
        .map(|(a, b)| slice_line(line, a, b))
        .collect()
}

/// Word-wrap a uniform-style footer message to the pane width: one leading
/// space per line, `reserve` columns kept clear of the right edge (the «
/// button zone). Unbreakable words longer than a line hard-break. Never
/// returns an empty vec — footers size themselves from `.len()`.
pub fn wrap_footer_message(text: &str, width: u16, reserve: u16) -> Vec<String> {
    let max = usize::from(width.max(12))
        .saturating_sub(usize::from(reserve))
        .max(6);
    let fits = |s: &str, extra: usize| Span::raw(s).width() + extra <= max;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::from(" ");
    for word in text.split_whitespace() {
        let sep = usize::from(cur.len() > 1);
        if !fits(&cur, sep + Span::raw(word).width()) && cur.len() > 1 {
            lines.push(std::mem::replace(&mut cur, String::from(" ")));
        }
        if cur.len() > 1 {
            cur.push(' ');
        }
        if fits(&cur, Span::raw(word).width()) {
            cur.push_str(word);
        } else {
            for c in word.chars() {
                if !fits(&cur, 2) {
                    lines.push(std::mem::replace(&mut cur, String::from(" ")));
                }
                cur.push(c);
            }
        }
    }
    if cur.len() > 1 || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Keep the TAIL of a typed input so the cursor end stays visible when the
/// text outgrows the prompt line (shell-style).
pub fn input_tail(input: &str, max: usize) -> String {
    if Span::raw(input).width() <= max {
        return input.to_string();
    }
    let mut out = String::new();
    for c in input.chars().rev() {
        let mut candidate = c.to_string();
        candidate.push_str(&out);
        if Span::raw(candidate.as_str()).width() + 1 > max {
            break;
        }
        out = candidate;
    }
    format!("…{out}")
}

pub fn truncate_to(s: String, max: usize) -> String {
    if Span::raw(s.as_str()).width() <= max {
        return s;
    }
    if max < 2 {
        return String::new();
    }
    let mut out = String::new();
    for c in s.chars() {
        let mut candidate = out.clone();
        candidate.push(c);
        if Span::raw(candidate.as_str()).width() + 1 > max {
            break;
        }
        out = candidate;
    }
    out.push('…');
    out
}

/// Pane ids in the same tab as `my_pane_id` that belong to the `other` view
/// (matched by its standalone label or its metadata token), from a
/// `pane.list` response.
pub fn sibling_panes_of(pane_list_json: &str, my_pane_id: &str, other: View) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Msg {
        result: Res,
    }
    #[derive(serde::Deserialize)]
    struct Res {
        #[serde(default)]
        panes: Vec<Pane>,
    }
    #[derive(serde::Deserialize)]
    struct Pane {
        pane_id: Option<String>,
        label: Option<String>,
        tab_id: Option<String>,
        #[serde(default)]
        tokens: serde_json::Map<String, serde_json::Value>,
    }
    let Ok(msg) = serde_json::from_str::<Msg>(pane_list_json.trim_start_matches('\u{feff}'))
    else {
        return Vec::new();
    };
    let panes = &msg.result.panes;
    let Some(my_tab) = panes
        .iter()
        .find(|p| p.pane_id.as_deref() == Some(my_pane_id))
        .and_then(|p| p.tab_id.clone())
    else {
        return Vec::new();
    };
    panes
        .iter()
        .filter(|p| p.tab_id.as_deref() == Some(my_tab.as_str()))
        .filter(|p| p.pane_id.as_deref() != Some(my_pane_id))
        .filter(|p| {
            p.label.as_deref() == Some(other.label()) || p.tokens.contains_key(other.plugin_id())
        })
        .filter_map(|p| p.pane_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_cols_breaks_on_spaces_and_hard_cuts() {
        assert_eq!(wrap_cols("", 8), vec![(0, 0)]);
        assert_eq!(wrap_cols("hi", 8), vec![(0, 2)]);
        assert_eq!(wrap_cols("hello world", 8), vec![(0, 5), (6, 11)]);
        assert_eq!(wrap_cols("abcdefghij", 4), vec![(0, 4), (4, 8), (8, 10)]);
        let parts: Vec<&str> = wrap_cols("hello world", 8)
            .into_iter()
            .map(|(a, b)| &"hello world"[a..b])
            .collect();
        assert_eq!(parts, ["hello", "world"]);
    }

    #[test]
    fn messages_wrap_to_narrow_panes() {
        // The reported clip: a delete confirm in a ~26-col pane.
        let lines = wrap_footer_message("Delete 'test' permanently? (y/N)", 26, 4);
        assert!(lines.len() > 1, "must wrap, not clip: {lines:?}");
        for l in &lines {
            assert!(l.starts_with(' '), "leading space kept: {l:?}");
            assert!(
                ratatui::text::Span::raw(l.as_str()).width() <= 22,
                "line within width-reserve: {l:?}"
            );
        }
        assert_eq!(
            lines.join("").split_whitespace().collect::<Vec<_>>().join(" "),
            "Delete 'test' permanently? (y/N)",
            "no words lost"
        );
        // Wide panes stay single-line.
        assert_eq!(wrap_footer_message("Delete 'test' permanently? (y/N)", 80, 4).len(), 1);
        // An unbreakable long word hard-breaks instead of overflowing.
        let long = wrap_footer_message("Deleted averyveryverylongfilename.extension", 20, 4);
        assert!(long.len() > 1);
        // Empty input still yields one (empty) line — footers size from len().
        assert_eq!(wrap_footer_message("", 30, 4).len(), 1);
    }

    #[test]
    fn input_tail_keeps_cursor_end_visible() {
        assert_eq!(input_tail("short", 10), "short");
        let tail = input_tail("a-very-long-typed-filename.txt", 12);
        assert!(tail.starts_with('…'), "{tail:?}");
        assert!(tail.ends_with(".txt"), "{tail:?}");
        assert!(ratatui::text::Span::raw(tail.as_str()).width() <= 12);
    }

    #[test]
    fn hints_wrap_instead_of_clipping() {
        let hints = [("⏎", "stage"), ("a", "all"), ("u", "none"), ("q", "quit")];
        let wide = wrap_hints(&hints, 80, 0);
        assert_eq!(wide.len(), 1);
        let narrow = wrap_hints(&hints, 14, 0);
        assert!(narrow.len() >= 2, "narrow pane stacks hints");
        for line in &narrow {
            assert!(line.width() <= 14, "no line exceeds the pane width");
        }
    }

    #[test]
    fn hints_have_no_line_cap() {
        let hints = [
            ("a", "aaa"),
            ("b", "bbb"),
            ("c", "ccc"),
            ("d", "ddd"),
            ("e", "eee"),
            ("f", "fff"),
            ("g", "ggg"),
            ("h", "hhh"),
        ];
        // Every chip lands on its own line rather than overflowing the
        // width — there is no line cap to clip against anymore.
        let lines = wrap_hints(&hints, 10, 0);
        assert_eq!(lines.len(), hints.len());
        for line in &lines {
            assert!(line.width() <= 10, "no line exceeds the pane width");
        }
    }

    #[test]
    fn truncation_keeps_width_budget() {
        assert_eq!(truncate_to("short".into(), 10), "short");
        let cut = truncate_to("averylongdirectoryname".into(), 8);
        assert!(cut.ends_with('…'));
        assert!(Span::raw(cut.as_str()).width() <= 8);
        assert_eq!(truncate_to("abc".into(), 1), "");
    }

    #[test]
    fn settings_popover_sits_above_and_right_aligns_to_gear() {
        let area = Rect::new(0, 0, 32, 50);
        let gear = Rect::new(28, 47, 4, 1);
        let popup = popover_above(area, gear, 30, 20);
        assert_eq!(popup.x, 2, "right-aligned to the gear");
        assert_eq!(popup.y + popup.height, gear.y, "bottom edge meets the gear");
        assert_eq!(popup.width, 30);
        assert_eq!(popup.height, 20);
        let tight = popover_above(area, Rect::new(28, 5, 4, 1), 30, 20);
        assert_eq!(tight.y, 0);
        assert_eq!(tight.height, 5, "shrinks instead of covering the gear");
    }

    #[test]
    fn go_up_uses_cod_arrow_up_not_cod_add() {
        assert_eq!(title_action_icon(IconTheme::Material, TitleAction::GoUp), "\u{eaa1}");
        assert_ne!(title_action_icon(IconTheme::Material, TitleAction::GoUp), "\u{ea60}");
    }

    #[test]
    fn title_action_zones_are_contiguous_and_match_width() {
        for theme in [IconTheme::Material, IconTheme::Emoji] {
            let actions = [
                TitleAction::NewFile,
                TitleAction::NewFolder,
                TitleAction::Refresh,
                TitleAction::CollapseAll,
            ];
            let (spans, zones) = title_action_spans(theme, &actions, 10, 0, None);
            assert_eq!(spans.len(), 4);
            let mut x = 10;
            for (rect, _) in &zones {
                assert_eq!(rect.x, x, "chips tile left to right with no gaps");
                x += rect.width;
            }
            let total: u16 = zones.iter().map(|(r, _)| r.width).sum();
            assert_eq!(total, title_actions_width(theme, &actions));
            // A click inside the second chip maps to New Folder.
            let (rect, action) = zones[1];
            assert!(hits(rect, rect.x, 0));
            assert_eq!(action, TitleAction::NewFolder);
        }
    }

    #[test]
    fn title_actions_hide_without_recent_mouse() {
        assert!(!title_actions_visible(None));
        assert!(title_actions_visible(Some(std::time::Instant::now())));
        let old = std::time::Instant::now() - TITLE_ACTIONS_LINGER - std::time::Duration::from_secs(1);
        assert!(!title_actions_visible(Some(old)));
    }

    #[test]
    fn sibling_lookup_matches_label_or_token() {
        let json = r#"{"result":{"panes":[
            {"pane_id":"w1:p1","tab_id":"w1:t1","label":"Sidebar"},
            {"pane_id":"w1:p2","tab_id":"w1:t1","label":"Source Control"},
            {"pane_id":"w1:p3","tab_id":"w1:t1","tokens":{"herdr-sidebar-git":{}}},
            {"pane_id":"w1:p9","tab_id":"w1:t2","label":"Source Control"}
        ]}}"#;
        let found = sibling_panes_of(json, "w1:p1", View::SourceControl);
        assert_eq!(found, ["w1:p2", "w1:p3"], "same tab only, label or token");
    }
}
