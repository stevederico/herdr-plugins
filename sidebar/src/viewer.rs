//! The preview pane: file contents AND git diffs, opened beside the sidebar
//! (the tree stays visible, like VS Code's editor area). The sidebar keeps
//! ONE viewer pane per tab and steers it through a small CONTROL FILE: each
//! click writes a request there; the running viewer polls it and reloads in
//! place, so repeated clicks never churn panes. Diff requests re-run git
//! every couple of seconds, so the diff live-updates while you edit.
//! `q`/Esc (or clicking the ✕ header) closes the pane itself.
//!
//! The tail of this module is the CLIENT side — the request format plus the
//! ensure-a-viewer-pane logic both sidebar views share.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ansi;
use crate::icons::{IconTheme, icon};
use crate::ipc;

/// Metadata source/token that marks the viewer pane, so the sidebar can find
/// and reuse it (distinct from the sidebar's own identity tokens).
pub const METADATA_SOURCE: &str = "herdr-sidebar-preview";

/// How often the control file is re-checked while idle.
const POLL: Duration = Duration::from_millis(250);

/// Preview size guards: don't slurp huge files into a pane.
const MAX_BYTES: usize = 1024 * 1024;
const MAX_LINES: usize = 5000;

/// The control file the sidebar writes requests into, unique per sidebar
/// pane (tab) so tabs don't steer each other's viewers.
pub fn control_path(sidebar_pane_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-sidebar-preview-{}.ctl",
        sidebar_pane_id.replace(':', "_")
    ))
}

/// What the sidebar asked the viewer to show.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Request {
    File(PathBuf),
    Diff {
        root: PathBuf,
        rel: String,
        /// "staged" | "worktree" | "untracked" — which diff to run.
        kind: String,
    },
    /// `git show <spec>` — a commit, stash, tag, or branch tip, optionally
    /// narrowed to one file.
    Show {
        root: PathBuf,
        spec: String,
        path: Option<String>,
    },
}

/// Control-file payload for a file preview.
pub fn file_request(path: &Path) -> String {
    format!("file\t{}", path.display())
}

/// Control-file payload for a git diff (`kind`: staged | worktree | untracked).
pub fn diff_request(root: &Path, rel: &str, kind: &str) -> String {
    format!("diff\t{}\t{rel}\t{kind}", root.display())
}

/// Control-file payload for `git show <spec>` (commit hash, stash@{n}, tag…),
/// optionally narrowed to one file.
pub fn show_request(root: &Path, spec: &str, path: Option<&str>) -> String {
    format!("show\t{}\t{spec}\t{}", root.display(), path.unwrap_or(""))
}

fn parse_request(raw: &str) -> Option<Request> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.split('\t');
    match parts.next() {
        Some("diff") => {
            let root = PathBuf::from(parts.next()?);
            let rel = parts.next()?.to_string();
            let kind = parts.next().unwrap_or("worktree").to_string();
            Some(Request::Diff { root, rel, kind })
        }
        Some("show") => {
            let root = PathBuf::from(parts.next()?);
            let spec = parts.next()?.to_string();
            let path = parts.next().filter(|p| !p.is_empty()).map(str::to_string);
            Some(Request::Show { root, spec, path })
        }
        Some("file") => Some(Request::File(PathBuf::from(parts.next()?))),
        // Legacy: a bare path.
        _ => Some(Request::File(PathBuf::from(raw))),
    }
}

struct Doc {
    name: String,
    context: String,
    /// Display lines for read-only content (diffs / errors / binary).
    lines: Vec<Line<'static>>,
    /// Editable raw lines for file previews (None = read-only).
    buffer: Option<Vec<String>>,
    path: Option<PathBuf>,
    cursor_row: usize,
    /// Byte index into the current line (clamped on draw).
    cursor_col: usize,
    dirty: bool,
    notice: String,
    /// File previews get a line-number gutter; diffs carry their own +/-.
    numbered: bool,
    scroll: usize,
    media: Option<crate::media::MediaState>,
}

impl Doc {
    fn empty_wait() -> Self {
        Self {
            name: "(nothing to show)".into(),
            context: String::new(),
            lines: vec![Line::raw("(waiting for a click in the sidebar)")],
            buffer: None,
            path: None,
            cursor_row: 0,
            cursor_col: 0,
            dirty: false,
            notice: String::new(),
            numbered: false,
            scroll: 0,
            media: None,
        }
    }

    fn line_count(&self) -> usize {
        if let Some(buf) = &self.buffer {
            buf.len().max(1)
        } else {
            self.lines.len().max(1)
        }
    }

    fn editable(&self) -> bool {
        self.buffer.is_some() && self.path.is_some()
    }

    fn clamp_cursor(&mut self) {
        let Some(buf) = &self.buffer else { return };
        if buf.is_empty() {
            self.cursor_row = 0;
            self.cursor_col = 0;
            return;
        }
        self.cursor_row = self.cursor_row.min(buf.len() - 1);
        let len = buf[self.cursor_row].len();
        self.cursor_col = self.cursor_col.min(len);
    }

    fn ensure_visible(&mut self, page: usize) {
        let page = page.max(1);
        if self.cursor_row < self.scroll {
            self.scroll = self.cursor_row;
        } else if self.cursor_row >= self.scroll + page {
            self.scroll = self.cursor_row + 1 - page;
        }
    }

    fn insert_char(&mut self, c: char) {
        let Some(buf) = &mut self.buffer else { return };
        self.cursor_row = self.cursor_row.min(buf.len().saturating_sub(1));
        if buf.is_empty() {
            buf.push(String::new());
        }
        let line = &mut buf[self.cursor_row];
        self.cursor_col = self.cursor_col.min(line.len());
        line.insert(self.cursor_col, c);
        self.cursor_col += c.len_utf8();
        self.dirty = true;
        self.notice.clear();
    }

    fn insert_newline(&mut self) {
        let Some(buf) = &mut self.buffer else { return };
        if buf.is_empty() {
            buf.push(String::new());
        }
        self.cursor_row = self.cursor_row.min(buf.len() - 1);
        let line = &mut buf[self.cursor_row];
        self.cursor_col = self.cursor_col.min(line.len());
        let rest = line[self.cursor_col..].to_string();
        line.truncate(self.cursor_col);
        buf.insert(self.cursor_row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.dirty = true;
        self.notice.clear();
    }

    fn backspace(&mut self) {
        let Some(buf) = &mut self.buffer else { return };
        if buf.is_empty() {
            return;
        }
        self.cursor_row = self.cursor_row.min(buf.len() - 1);
        if self.cursor_col > 0 {
            let line = &mut buf[self.cursor_row];
            // delete previous UTF-8 char
            let mut i = self.cursor_col;
            while i > 0 && !line.is_char_boundary(i - 1) {
                i -= 1;
            }
            i -= 1;
            line.remove(i);
            self.cursor_col = i;
            self.dirty = true;
            self.notice.clear();
            return;
        }
        if self.cursor_row == 0 {
            return;
        }
        let cur = buf.remove(self.cursor_row);
        self.cursor_row -= 1;
        self.cursor_col = buf[self.cursor_row].len();
        buf[self.cursor_row].push_str(&cur);
        self.dirty = true;
        self.notice.clear();
    }

    /// Option/Alt+Delete (macOS word-delete): remove the word left of the cursor.
    fn backspace_word(&mut self) {
        let Some(buf) = &mut self.buffer else { return };
        if buf.is_empty() {
            return;
        }
        self.cursor_row = self.cursor_row.min(buf.len() - 1);
        if self.cursor_col == 0 {
            // Same as plain backspace at column 0: join with previous line.
            self.backspace();
            return;
        }
        let line = &buf[self.cursor_row];
        let before = &line[..self.cursor_col];
        // Skip trailing whitespace, then skip the preceding word (non-whitespace).
        let trimmed = before.trim_end();
        let word_start = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| {
                // i is the last whitespace char; word starts after it.
                let mut j = i + 1;
                while j < trimmed.len() && !trimmed.is_char_boundary(j) {
                    j += 1;
                }
                j
            })
            .unwrap_or(0);
        let line = &mut buf[self.cursor_row];
        line.replace_range(word_start..self.cursor_col, "");
        self.cursor_col = word_start;
        self.dirty = true;
        self.notice.clear();
    }

    /// Fn+Delete / Forward delete: remove the char under the cursor.
    fn delete_forward(&mut self) {
        let Some(buf) = &mut self.buffer else { return };
        if buf.is_empty() {
            return;
        }
        self.cursor_row = self.cursor_row.min(buf.len() - 1);
        let line_len = buf[self.cursor_row].len();
        if self.cursor_col < line_len {
            let line = &mut buf[self.cursor_row];
            let mut end = self.cursor_col + 1;
            while end < line.len() && !line.is_char_boundary(end) {
                end += 1;
            }
            line.replace_range(self.cursor_col..end, "");
            self.dirty = true;
            self.notice.clear();
            return;
        }
        // At end of line: join with next line.
        if self.cursor_row + 1 < buf.len() {
            let next = buf.remove(self.cursor_row + 1);
            buf[self.cursor_row].push_str(&next);
            self.dirty = true;
            self.notice.clear();
        }
    }

    fn move_left(&mut self) {
        self.clamp_cursor();
        if self.cursor_col > 0 {
            let Some(buf) = &self.buffer else { return };
            let line = &buf[self.cursor_row];
            let mut i = self.cursor_col;
            while i > 0 && !line.is_char_boundary(i - 1) {
                i -= 1;
            }
            self.cursor_col = i - 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            if let Some(buf) = &self.buffer {
                self.cursor_col = buf[self.cursor_row].len();
            }
        }
    }

    fn move_right(&mut self) {
        self.clamp_cursor();
        let Some(buf) = &self.buffer else { return };
        let line_len = buf[self.cursor_row].len();
        if self.cursor_col < line_len {
            let line = &buf[self.cursor_row];
            let mut i = self.cursor_col + 1;
            while i < line.len() && !line.is_char_boundary(i) {
                i += 1;
            }
            self.cursor_col = i;
        } else if self.cursor_row + 1 < buf.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.clamp_cursor();
        }
    }

    fn move_down(&mut self) {
        let Some(buf) = &self.buffer else { return };
        if self.cursor_row + 1 < buf.len() {
            self.cursor_row += 1;
            self.clamp_cursor();
        }
    }

    fn save(&mut self) {
        let (Some(path), Some(buf)) = (&self.path, &self.buffer) else {
            self.notice = "not editable".into();
            return;
        };
        let mut body = buf.join("\n");
        // Preserve trailing newline if the original had content.
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        match std::fs::write(path, body) {
            Ok(()) => {
                self.dirty = false;
                self.notice = "saved".into();
            }
            Err(e) => self.notice = format!("save failed: {e}"),
        }
    }
}

fn load(request: &Request) -> Doc {
    match request {
        Request::File(path) if crate::media::is_media(path) => load_media(path),
        Request::File(path) => {
            crate::media::clear();
            load_file(path)
        }
        Request::Diff { root, rel, kind } => {
            crate::media::clear();
            load_diff(root, rel, kind)
        }
        Request::Show { root, spec, path } => {
            crate::media::clear();
            load_show(root, spec, path.as_deref())
        }
    }
}

fn load_media(target: &Path) -> Doc {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.display().to_string());
    let kind = if crate::media::is_video(target) { "video" } else { "image" };
    Doc {
        name,
        context: format!("{kind} — {}", target.display()),
        lines: vec![Line::raw("decoding…")],
        buffer: None,
        path: Some(target.to_path_buf()),
        cursor_row: 0,
        cursor_col: 0,
        dirty: false,
        notice: String::new(),
        numbered: false,
        scroll: 0,
        media: Some(crate::media::MediaState {
            path: target.to_path_buf(),
            image: None,
            fit: (0, 0),
        }),
    }
}

/// `git show` with stat + patch, colored — what a click on a commit, stash,
/// tag, or branch line renders. Immutable content: no refresh loop needed.
fn load_show(root: &Path, spec: &str, path: Option<&str>) -> Doc {
    let mut args: Vec<String> = vec![
        "-c".into(),
        "color.ui=always".into(),
        "show".into(),
        "--color=always".into(),
        "--stat".into(),
        "--patch".into(),
        "--no-ext-diff".into(),
        spec.to_string(),
    ];
    if let Some(p) = path {
        args.push("--".into());
        args.push(p.replace('/', std::path::MAIN_SEPARATOR_STR));
    }
    let output = std::process::Command::new("git").args(&args).current_dir(root).output();
    let lines = match output {
        Err(e) => vec![Line::raw(format!("(git failed: {e})"))],
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.trim().is_empty() {
                let err = String::from_utf8_lossy(&out.stderr);
                if err.trim().is_empty() {
                    vec![Line::raw("(nothing to show)")]
                } else {
                    vec![Line::raw(format!("({})", err.trim()))]
                }
            } else {
                ansi::to_lines(&text)
            }
        }
    };
    Doc {
        name: spec.to_string(),
        context: format!("git show {spec} — {}", root.display()),
        lines,
        buffer: None,
        path: None,
        cursor_row: 0,
        cursor_col: 0,
        dirty: false,
        notice: String::new(),
        numbered: false,
        scroll: 0,
        media: None,
    }
}

fn load_file(target: &Path) -> Doc {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.display().to_string());
    match std::fs::read(target) {
        Err(e) => Doc {
            name,
            context: target.display().to_string(),
            lines: vec![Line::raw(format!("(unreadable: {e})"))],
            buffer: None,
            path: None,
            cursor_row: 0,
            cursor_col: 0,
            dirty: false,
            notice: String::new(),
            numbered: true,
            scroll: 0,
            media: None,
        },
        Ok(bytes) => {
            let head = &bytes[..bytes.len().min(8192)];
            if head.contains(&0) {
                return Doc {
                    name,
                    context: target.display().to_string(),
                    lines: vec![Line::raw(format!("(binary file — {} bytes)", bytes.len()))],
                    buffer: None,
                    path: None,
                    cursor_row: 0,
                    cursor_col: 0,
                    dirty: false,
                    notice: String::new(),
                    numbered: true,
                    scroll: 0,
                    media: None,
                };
            }
            let truncated = bytes.len() > MAX_BYTES;
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BYTES)]);
            // Cap edit buffer size so the pane stays responsive.
            let mut buffer: Vec<String> = text.lines().take(MAX_LINES).map(str::to_string).collect();
            if buffer.is_empty() {
                buffer.push(String::new());
            }
            let mut notice = String::new();
            if truncated || text.lines().count() > MAX_LINES {
                notice = "truncated — edit head only".into();
            }
            Doc {
                name,
                context: target.display().to_string(),
                lines: Vec::new(),
                buffer: Some(buffer),
                path: Some(target.to_path_buf()),
                cursor_row: 0,
                cursor_col: 0,
                dirty: false,
                notice,
                numbered: true,
                scroll: 0,
                media: None,
            }
        }
    }
}

fn load_diff(root: &Path, rel: &str, kind: &str) -> Doc {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    // Plain (uncolored) diff: crate::diffview parses it and renders the
    // VS Code look — dual gutters, tinted rows, syntax-highlighted code.
    let mut args: Vec<String> = vec!["diff".into(), "--no-ext-diff".into()];
    match kind {
        "staged" => args.push("--cached".into()),
        // An untracked file has no diff; --no-index against the null device
        // renders it as one big addition, like VS Code does.
        "untracked" => {
            args.push("--no-index".into());
            args.push(if cfg!(windows) { "NUL".into() } else { "/dev/null".into() });
        }
        _ => {}
    }
    args.push("--".into());
    args.push(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(root)
        .output();
    let lines = match output {
        Err(e) => vec![Line::raw(format!("(git failed: {e})"))],
        Ok(out) => {
            // --no-index exits 1 when the files differ; that's not an error.
            let text = String::from_utf8_lossy(&out.stdout);
            if text.trim().is_empty() {
                let err = String::from_utf8_lossy(&out.stderr);
                if err.trim().is_empty() {
                    vec![Line::raw("(no changes)")]
                } else {
                    vec![Line::raw(format!("({})", err.trim()))]
                }
            } else {
                crate::diffview::render(rel, &text)
            }
        }
    };
    let what = match kind {
        "staged" => "staged",
        "untracked" => "untracked",
        _ => "working tree",
    };
    Doc {
        name: name.clone(),
        context: format!("{} — {what} diff", root.join(rel).display()),
        lines,
        buffer: None,
        path: None,
        cursor_row: 0,
        cursor_col: 0,
        dirty: false,
        notice: String::new(),
        numbered: false,
        scroll: 0,
        media: None,
    }
}

fn read_control(control: &Path) -> Option<Request> {
    let mut buf = String::new();
    std::fs::File::open(control).ok()?.read_to_string(&mut buf).ok()?;
    parse_request(&buf)
}

/// Tag our pane (heartbeat-stamped, see launch::HEARTBEAT_STALE_SECS) and
/// title it with the shown document's name.
fn report_identity(doc_name: &str) {
    let Ok(pane_id) = std::env::var("HERDR_PANE_ID") else { return };
    if pane_id.is_empty() {
        return;
    }
    let _ = ipc::call_text(
        "pane.report_metadata",
        serde_json::json!({
            "pane_id": pane_id,
            "source": METADATA_SOURCE,
            "tokens": { METADATA_SOURCE: crate::state::unix_now().to_string() },
        }),
    );
    let _ = ipc::call_text(
        "pane.rename",
        serde_json::json!({ "pane_id": pane_id, "label": doc_name }),
    );
}

/// Close our own pane (ends this process with it), handing focus back to
/// the sidebar that spawned us — its pane id is baked into the control-file
/// name, so a full-screen (zoomed) preview drops the user exactly where
/// they were.
fn close_own_pane(control: &Path) {
    crate::media::clear();
    if let Some(owner) = owner_pane_id(control) {
        restore_parked(&owner);
        let _ = ipc::call_text("pane.focus", serde_json::json!({ "pane_id": owner }));
    }
    if let Ok(pane_id) = std::env::var("HERDR_PANE_ID")
        && !pane_id.is_empty()
    {
        let _ = ipc::call_text("pane.close", serde_json::json!({ "pane_id": pane_id }));
    }
}

/// The sidebar pane that owns this viewer, recovered from the control-file
/// name (`herdr-sidebar-preview-<id with ':' as '_'>.ctl`).
fn owner_pane_id(control: &Path) -> Option<String> {
    let stem = control.file_stem()?.to_str()?;
    let id = stem.strip_prefix("herdr-sidebar-preview-")?.replace('_', ":");
    (!id.is_empty()).then_some(id)
}

/// The viewer's event loop; returns when the user closes it.
pub fn run(control: &Path) -> std::io::Result<()> {
    let theme = IconTheme::resolve(
        std::env::var("HERDR_SIDEBAR_ICONS")
            .or_else(|_| std::env::var("HERDR_AA_FILETREE_ICONS"))
            .ok()
            .as_deref(),
        crate::state::load_state().icons,
    );
    let mut current = read_control(control);
    let mut doc = current.as_ref().map(load).unwrap_or_else(Doc::empty_wait);
    report_identity(&doc.name);

    // Blank the primary screen so pane handoffs never flash the shell.
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
        crossterm::cursor::MoveTo(0, 0),
    );
    crossterm::style::force_color_output(true); // TUI colors ≠ pipeable output
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    let mut page: usize = 20;
    let mut beat: u64 = 0;
    let result = loop {
        let draw = terminal.draw(|frame| page = draw_doc(frame, &mut doc, theme));
        if let Err(e) = draw {
            break Err(e);
        }
        let max = doc.line_count().saturating_sub(1);
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    match key.code {
                        KeyCode::Esc => {
                            close_own_pane(control);
                            break Ok(());
                        }
                        // Ctrl+S save (editable files).
                        KeyCode::Char('s') if ctrl && doc.editable() => doc.save(),
                        // Ctrl+W = word delete (when Option isn't passed through by the terminal).
                        KeyCode::Char('w') if ctrl && doc.editable() => {
                            doc.backspace_word();
                            doc.ensure_visible(page);
                        }
                        // q closes read-only previews; in edit mode type q as text (Esc closes).
                        KeyCode::Char('q') if !doc.editable() => {
                            close_own_pane(control);
                            break Ok(());
                        }
                        KeyCode::Char('o') if !doc.editable() => {
                            if let Some(path) = &doc.path {
                                crate::media::open_external(path);
                            }
                        }
                        KeyCode::Up => {
                            if doc.editable() {
                                doc.move_up();
                                doc.ensure_visible(page);
                            } else {
                                doc.scroll = doc.scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            if doc.editable() {
                                doc.move_down();
                                doc.ensure_visible(page);
                            } else {
                                doc.scroll = (doc.scroll + 1).min(max);
                            }
                        }
                        KeyCode::Left if doc.editable() => doc.move_left(),
                        KeyCode::Right if doc.editable() => doc.move_right(),
                        KeyCode::PageUp => {
                            doc.scroll = doc.scroll.saturating_sub(page);
                            if doc.editable() {
                                doc.cursor_row = doc.cursor_row.saturating_sub(page);
                                doc.clamp_cursor();
                            }
                        }
                        KeyCode::PageDown => {
                            doc.scroll = (doc.scroll + page).min(max);
                            if doc.editable() {
                                doc.cursor_row = (doc.cursor_row + page).min(max);
                                doc.clamp_cursor();
                            }
                        }
                        KeyCode::Home if doc.editable() => {
                            doc.cursor_col = 0;
                        }
                        KeyCode::End if doc.editable() => {
                            if let Some(buf) = &doc.buffer {
                                doc.cursor_col = buf.get(doc.cursor_row).map(|l| l.len()).unwrap_or(0);
                            }
                        }
                        KeyCode::Home => doc.scroll = 0,
                        KeyCode::End => doc.scroll = max,
                        KeyCode::Enter if doc.editable() => {
                            doc.insert_newline();
                            doc.ensure_visible(page);
                        }
                        KeyCode::Backspace if doc.editable() => {
                            // Option+Delete on macOS arrives as Alt+Backspace (word delete).
                            if key.modifiers.contains(KeyModifiers::ALT)
                                || key.modifiers.contains(KeyModifiers::META)
                            {
                                doc.backspace_word();
                            } else {
                                doc.backspace();
                            }
                            doc.ensure_visible(page);
                        }
                        KeyCode::Delete if doc.editable() => {
                            if key.modifiers.contains(KeyModifiers::ALT)
                                || key.modifiers.contains(KeyModifiers::META)
                            {
                                // Option+Fn+Delete: delete word forward (simple: word left if at boundary).
                                doc.backspace_word();
                            } else {
                                doc.delete_forward();
                            }
                            doc.ensure_visible(page);
                        }
                        KeyCode::Char(c) if doc.editable() && !ctrl => {
                            // j/k scroll only in read-only mode; in edit mode they type.
                            doc.insert_char(c);
                            doc.ensure_visible(page);
                        }
                        KeyCode::Char('k') if !doc.editable() => {
                            doc.scroll = doc.scroll.saturating_sub(1);
                        }
                        KeyCode::Char('j') if !doc.editable() => {
                            doc.scroll = (doc.scroll + 1).min(max);
                        }
                        KeyCode::Char('g') if !doc.editable() => doc.scroll = 0,
                        KeyCode::Char('G') if !doc.editable() => doc.scroll = max,
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => doc.scroll = doc.scroll.saturating_sub(3),
                    MouseEventKind::ScrollDown => {
                        doc.scroll = (doc.scroll + 3).min(max);
                    }
                    MouseEventKind::Down(MouseButton::Left) if mouse.row == 0 => {
                        close_own_pane(control);
                        break Ok(());
                    }
                    MouseEventKind::Down(MouseButton::Left) if doc.editable() && mouse.row >= 1 => {
                        // Click to place cursor (row under header).
                        let row = usize::from(mouse.row.saturating_sub(1)) + doc.scroll;
                        if let Some(buf) = &doc.buffer {
                            doc.cursor_row = row.min(buf.len().saturating_sub(1));
                            let number_width = buf.len().to_string().len() + 1;
                            let col = usize::from(mouse.column).saturating_sub(number_width);
                            doc.cursor_col = col;
                            doc.clamp_cursor();
                        }
                    }
                    _ => {}
                },
                _ => {} // resize etc: redraw
            }
        } else {
            // Idle: heartbeat, follow the control file, and live-refresh diffs.
            beat += 1;
            if beat.is_multiple_of(20) {
                report_identity(&doc.name);
            }
            let target = read_control(control);
            if target != current {
                // Don't clobber unsaved edits when the sidebar re-sends the same file.
                let same_file = matches!(
                    (&current, &target),
                    (Some(Request::File(a)), Some(Request::File(b))) if a == b
                );
                if same_file && doc.dirty {
                    // keep buffer
                } else {
                    current = target;
                    if let Some(request) = &current {
                        doc = load(request);
                        report_identity(&doc.name);
                    }
                }
            } else if beat.is_multiple_of(8)
                && !doc.dirty
                && let Some(request @ Request::Diff { .. }) = &current
            {
                let keep = doc.scroll;
                doc = load(request);
                doc.scroll = keep.min(doc.line_count().saturating_sub(1));
            }
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

/// Header (✕ close + name + context), body, hint footer. Returns the page
/// stride for PageUp/Down.
fn draw_doc(frame: &mut Frame, doc: &mut Doc, theme: IconTheme) -> usize {
    let area = frame.area();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    let total = doc.line_count();
    doc.scroll = doc
        .scroll
        .min(total.saturating_sub(usize::from(body.height).max(1)));

    let file_icon = icon(theme, &doc.name, false, false);
    let icon_style = match file_icon.rgb {
        Some((r, g, b)) => Style::default().fg(Color::Rgb(r, g, b)),
        None => Style::default(),
    };
    let dirty_mark = if doc.dirty { " ●" } else { "" };
    let left = vec![
        Span::styled(" ✕ ", Style::default().bold().fg(Color::LightBlue)),
        Span::styled(format!("{} ", file_icon.glyph), icon_style),
        Span::styled(format!("{}{dirty_mark}", doc.name), Style::default().bold()),
    ];
    let used: usize = left.iter().map(Span::width).sum();
    let avail = usize::from(area.width).saturating_sub(used + 2);
    let shown = if doc.context.chars().count() > avail {
        let tail: String = doc
            .context
            .chars()
            .skip(doc.context.chars().count().saturating_sub(avail.saturating_sub(1)))
            .collect();
        format!("…{tail}")
    } else {
        doc.context.clone()
    };
    let mut spans = left;
    spans.push(Span::styled(format!("  {shown}"), Style::default().dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), header);

    if let Some(media) = doc.media.as_mut() {
        let fit = (body.width, body.height);
        if media.fit != fit && fit.0 >= 2 && fit.1 >= 1 {
            match crate::media::rasterize_cells(&media.path, fit.0, fit.1) {
                Ok(img) => {
                    media.image = Some(img);
                    doc.notice.clear();
                }
                Err(e) => {
                    doc.lines = vec![
                        Line::raw(format!("({e})")),
                        Line::raw("press o to open in Preview"),
                    ];
                    media.image = None;
                    doc.notice = e;
                }
            }
            media.fit = fit;
        }
        if let Some(img) = &media.image {
            draw_cell_image(frame, body, img);
            frame.render_widget(
                Paragraph::new(Line::from(" o open original  esc close".dim())),
                footer,
            );
            return usize::from(body.height).max(1);
        }
    }

    let number_width = total.to_string().len();
    let text: Vec<Line> = if let Some(buf) = &doc.buffer {
        buf.iter()
            .enumerate()
            .skip(doc.scroll)
            .take(usize::from(body.height))
            .map(|(n, line)| {
                let mut spans =
                    vec![Span::styled(format!("{:>number_width$} ", n + 1), Style::default().dim())];
                spans.push(Span::raw(line.clone()));
                Line::from(spans)
            })
            .collect()
    } else {
        doc.lines
            .iter()
            .enumerate()
            .skip(doc.scroll)
            .take(usize::from(body.height))
            .map(|(n, line)| {
                if doc.numbered {
                    let mut spans = vec![Span::styled(
                        format!("{:>number_width$} ", n + 1),
                        Style::default().dim(),
                    )];
                    spans.extend(line.spans.iter().cloned());
                    Line::from(spans)
                } else {
                    let mut line = line.clone();
                    // Tinted diff rows fill the full row, like an editor.
                    if line.style.bg.is_some() {
                        let pad = usize::from(body.width).saturating_sub(line.width());
                        if pad > 0 {
                            line.spans.push(Span::raw(" ".repeat(pad)));
                        }
                    }
                    line
                }
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(text), body);

    // Place the hardware cursor on the edit position.
    if doc.editable() {
        doc.clamp_cursor();
        if doc.cursor_row >= doc.scroll
            && doc.cursor_row < doc.scroll + usize::from(body.height)
        {
            let y = body.y + (doc.cursor_row - doc.scroll) as u16;
            let col_chars = doc
                .buffer
                .as_ref()
                .and_then(|b| b.get(doc.cursor_row))
                .map(|l| {
                    let end = doc.cursor_col.min(l.len());
                    l[..end].chars().count()
                })
                .unwrap_or(0);
            let x = body.x + (number_width as u16 + 1) + col_chars as u16;
            if x < body.x + body.width {
                frame.set_cursor_position((x, y));
            }
        }
    }

    let hint = if doc.editable() {
        let status = if doc.notice.is_empty() {
            if doc.dirty {
                "modified"
            } else {
                "edit"
            }
        } else {
            doc.notice.as_str()
        };
        format!(" type to edit  ^s save  esc close  · {status}")
    } else {
        if doc.path.as_deref().is_some_and(crate::media::is_media) {
            " o open in Preview  esc close".into()
        } else {
            " ↑↓ scroll  ⇞⇟ page  g G ends  q close".into()
        }
    };
    frame.render_widget(Paragraph::new(Line::from(hint.dim())), footer);
    usize::from(body.height).saturating_sub(1).max(1)
}

fn draw_cell_image(frame: &mut Frame, area: Rect, img: &crate::media::CellImage) {
    let rows = usize::from(img.rows.min(area.height));
    let cols = usize::from(img.cols.min(area.width));
    let mut lines = Vec::with_capacity(rows);
    for y in 0..rows {
        let mut spans = Vec::with_capacity(cols);
        for x in 0..cols {
            let i = y * usize::from(img.cols) + x;
            let (ur, ug, ub, lr, lg, lb) = img.cells[i];
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(ur, ug, ub))
                    .bg(Color::Rgb(lr, lg, lb)),
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

// ---------------------------------------------------------------------------
// Client side: how the sidebar views open things in the viewer pane.
// ---------------------------------------------------------------------------

/// Write `payload` to the caller's control file and make sure a live viewer
/// pane exists on the far right of the tab. Errors are human-readable notices.
pub fn open_in_pane(my_pane_id: &str, spawn_cwd: &Path, payload: &str) -> Result<(), String> {
    let control = control_path(my_pane_id);
    std::fs::write(&control, payload).map_err(|e| format!("preview failed: {e}"))?;
    let full = crate::state::load_state().preview_full;
    // Files are editable — focus the viewer so typing works. Diffs keep the tree focused.
    let focus_viewer = payload.starts_with("file\t");

    // Measure the sidebar's width share FIRST: closing a stale viewer below
    // leaves the sidebar momentarily alone at full width, which reads as a
    // meaningless ~1.0 (that ordering is how the half-width sidebar bug
    // happened — the old clamp turned the degenerate 1.0 into 0.5).
    let mut pre_park_frac = owner_frac(my_pane_id);

    // A live viewer in this tab follows the control file by itself; a DEAD
    // one (stale heartbeat) is closed and replaced.
    if let Ok(json) = ipc::call_text("pane.list", serde_json::json!({})) {
        match viewer_pane_in_tab(&json, my_pane_id) {
            Some((id, false)) => {
                if full {
                    // Covers toggling the setting on while a preview is
                    // already open beside the sidebar.
                    park_others(my_pane_id);
                    enforce_owner_width(my_pane_id, pre_park_frac.unwrap_or(0.3));
                }
                if focus_viewer {
                    let _ = ipc::call_text("pane.focus", serde_json::json!({ "pane_id": id }));
                }
                return Ok(());
            }
            Some((id, true)) => {
                let _ = ipc::call_text("pane.close", serde_json::json!({ "pane_id": id }));
            }
            None => {
                // The viewer died with panes parked (redeploy, tab surgery):
                // bring them home before opening fresh — and re-measure,
                // since the restore just rebuilt the layout.
                restore_parked(my_pane_id);
                pre_park_frac = owner_frac(my_pane_id).or(pre_park_frac);
            }
        }
    }
    if full {
        park_others(my_pane_id);
    }
    spawn_viewer_pane(my_pane_id, spawn_cwd, &control, pre_park_frac, focus_viewer)?;
    if full {
        enforce_owner_width(my_pane_id, pre_park_frac.unwrap_or(0.3));
    }
    Ok(())
}

/// The owner pane's share of its tab width right now — `None` when the
/// geometry carries no signal (a full-width sidebar in a stacked or
/// single-pane layout would read as ~1.0 and, clamped, once produced a
/// half-tab sidebar — user-reported).
fn owner_frac(owner: &str) -> Option<f64> {
    let layout = ipc::call_text("pane.layout", serde_json::json!({ "pane_id": owner })).ok()?;
    let msg = serde_json::from_str::<LayoutMsg2>(&layout).ok()?;
    let body = msg.result.layout;
    body.panes
        .iter()
        .find(|p| p.pane_id.as_deref() == Some(owner))
        .and_then(|p| p.rect)
        .map(|r| r.width as f64 / body.area.width.max(1) as f64)
        .filter(|f| (0.02..0.55).contains(f))
        .map(|f| f.max(0.1))
}

/// Pin the sidebar's width after the preview opens: in full mode the tab is
/// sidebar | viewer, so the root split ratio IS the sidebar's share. Makes
/// the width deterministic no matter which spawn path ran.
fn enforce_owner_width(owner: &str, frac: f64) {
    let _ = ipc::call_text(
        "layout.set_split_ratio",
        serde_json::json!({ "pane_id": owner, "path": [], "ratio": frac }),
    );
}

/// Close this tab's viewer pane if one is open (Esc from the sidebar),
/// bringing any parked panes home first.
pub fn close_in_tab(my_pane_id: &str) {
    restore_parked(my_pane_id);
    let Ok(json) = ipc::call_text("pane.list", serde_json::json!({})) else {
        return;
    };
    if let Some((id, _)) = viewer_pane_in_tab(&json, my_pane_id) {
        let _ = ipc::call_text("pane.close", serde_json::json!({ "pane_id": id }));
    }
}

/// The viewer pane in the same tab, by metadata token, plus whether its
/// heartbeat says it is DEAD (`(pane_id, stale)`).
fn viewer_pane_in_tab(pane_list_json: &str, my_pane_id: &str) -> Option<(String, bool)> {
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
        tab_id: Option<String>,
        label: Option<String>,
        #[serde(default)]
        tokens: serde_json::Map<String, serde_json::Value>,
    }
    let msg: Msg = serde_json::from_str(pane_list_json.trim_start_matches('\u{feff}')).ok()?;
    let panes = &msg.result.panes;
    let my_tab = panes
        .iter()
        .find(|p| p.pane_id.as_deref() == Some(my_pane_id))?
        .tab_id
        .clone()?;
    // Token match finds a live viewer; a "Preview"-labeled pane WITHOUT the
    // token is a resumed corpse (labels survive server restarts, tokens
    // don't) — report it too, with a missing token, so the stale check
    // below flags it and the caller closes it instead of spawning a twin.
    let viewer = panes
        .iter()
        .filter(|p| p.tab_id.as_deref() == Some(my_tab.as_str()))
        .find(|p| {
            p.tokens.contains_key(METADATA_SOURCE) || p.label.as_deref() == Some("Preview")
        })?;
    let id = viewer.pane_id.clone()?;
    let now = crate::state::unix_now();
    let stale = viewer
        .tokens
        .get(METADATA_SOURCE)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|ts| now.saturating_sub(ts) > crate::launch::HEARTBEAT_STALE_SECS)
        .unwrap_or(true);
    Some((id, stale))
}

/// Split a pane to the caller's right and run `command` in it.
/// `focus_new`: true focuses the new pane (editing); false keeps the sidebar.
fn spawn_command_pane(
    my_pane_id: &str,
    spawn_cwd: &Path,
    command: &str,
    label: &str,
    pre_park_frac: Option<f64>,
    focus_new: bool,
) -> Result<(), String> {
    let layout = ipc::call_text("pane.layout", serde_json::json!({ "pane_id": my_pane_id })).ok();
    // Far-right preview: explorer | grok | preview. Split the rightmost
    // pane (not our immediate neighbor — that sandwiched preview between
    // the tree and the agent).
    let rightmost = layout.as_deref().and_then(|json| rightmost_pane(json, my_pane_id));
    // Splitting ourselves (no other panes — e.g. everything else just parked,
    // leaving us momentarily full-width): keep the width the sidebar had
    // BEFORE the park, not a ballooned 30-50%.
    let own_frac = pre_park_frac.unwrap_or(0.3);
    let (target, ratio) = match &rightmost {
        Some(id) => (id.clone(), 0.5),
        None => (my_pane_id.to_string(), own_frac),
    };
    let response = ipc::call_text(
        "pane.split",
        serde_json::json!({
            "target_pane_id": target,
            "direction": "right",
            "ratio": ratio,
            "focus": false,
            "cwd": spawn_cwd.display().to_string(),
            "env": crate::state::spawn_env(),
        }),
    );
    let new_pane = response
        .ok()
        .and_then(|r| crate::launch::split_pane_id(&r))
        .ok_or_else(|| "preview pane failed to open".to_string())?;
    let _ = ipc::call_text(
        "pane.send_input",
        serde_json::json!({ "pane_id": new_pane, "text": command, "keys": ["Enter"] }),
    );
    let _ = ipc::call_text(
        "pane.rename",
        serde_json::json!({ "pane_id": new_pane, "label": label }),
    );
    // Stamp as preview-slot so Esc/close and reuse logic still find this pane.
    let _ = ipc::call_text(
        "pane.report_metadata",
        serde_json::json!({
            "pane_id": new_pane,
            "source": METADATA_SOURCE,
            "tokens": { METADATA_SOURCE: crate::state::unix_now().to_string() },
        }),
    );
    let focus_id = if focus_new {
        new_pane.as_str()
    } else {
        my_pane_id
    };
    let _ = ipc::call_text("pane.focus", serde_json::json!({ "pane_id": focus_id }));
    Ok(())
}

/// Split a viewer pane on the far right of the tab so the layout reads
/// explorer | rest | preview (not explorer | preview | rest).
fn spawn_viewer_pane(
    my_pane_id: &str,
    spawn_cwd: &Path,
    control: &Path,
    pre_park_frac: Option<f64>,
    focus_viewer: bool,
) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "herdr-sidebar".to_string());
    #[cfg(windows)]
    let command = format!("& \"{exe}\" --preview \"{}\"", control.display());
    #[cfg(not(windows))]
    let command = format!("exec \"{exe}\" --preview \"{}\"", control.display());
    spawn_command_pane(
        my_pane_id,
        spawn_cwd,
        &command,
        "Preview",
        pre_park_frac,
        focus_viewer,
    )
}


// ---------------------------------------------------------------------------
// Full-size mode: park the tab's other panes while a preview is open.
// ---------------------------------------------------------------------------

/// Park plan for `owner`'s tab, recorded beside the control file so either
/// process (sidebar or viewer) can restore.
fn park_path(owner: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("herdr-sidebar-preview-{}.park.json", owner.replace(':', "_")))
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
struct RectJ {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParkPlan {
    /// The tab the panes came from (and go back to).
    tab: String,
    /// Sidebar's share of the tab width at park time, for the re-split.
    owner_ratio: f64,
    /// Parked panes with their ORIGINAL rects, reading order.
    panes: Vec<(String, RectJ)>,
}

#[derive(serde::Deserialize)]
struct LayoutMsg2 {
    result: LayoutRes2,
}
#[derive(serde::Deserialize)]
struct LayoutRes2 {
    layout: LayoutBody2,
}
#[derive(serde::Deserialize)]
struct LayoutBody2 {
    area: RectJ,
    panes: Vec<LayoutPane2>,
}
#[derive(serde::Deserialize)]
struct LayoutPane2 {
    pane_id: Option<String>,
    rect: Option<RectJ>,
}

/// Pane ids in `owner`'s tab that are OURS (sidebar views / preview) and so
/// never get parked, from a `pane.list` response.
fn our_panes_in_tab(pane_list_json: &str, owner: &str) -> Vec<String> {
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
        tab_id: Option<String>,
        label: Option<String>,
        #[serde(default)]
        tokens: serde_json::Map<String, serde_json::Value>,
    }
    let Ok(msg) = serde_json::from_str::<Msg>(pane_list_json.trim_start_matches('\u{feff}'))
    else {
        return vec![owner.to_string()];
    };
    let tab = msg
        .result
        .panes
        .iter()
        .find(|p| p.pane_id.as_deref() == Some(owner))
        .and_then(|p| p.tab_id.clone());
    msg.result
        .panes
        .iter()
        .filter(|p| p.tab_id == tab)
        .filter(|p| {
            p.pane_id.as_deref() == Some(owner)
                || matches!(
                    p.label.as_deref(),
                    Some("Sidebar" | "Explorer" | "Source Control" | "Preview")
                )
                || p.tokens.keys().any(|k| k.starts_with("herdr-sidebar"))
        })
        .filter_map(|p| p.pane_id.clone())
        .collect()
}

/// Move every non-ours pane of `owner`'s tab into a background tab and
/// record how to put them back. No-op when nothing to park or a plan
/// already exists.
fn park_others(owner: &str) {
    if park_path(owner).exists() {
        return;
    }
    let Ok(list) = ipc::call_text("pane.list", serde_json::json!({})) else { return };
    let ours = our_panes_in_tab(&list, owner);
    let tab = crate::launch::tab_of(&list, owner);
    if tab.is_empty() {
        return;
    }
    let Ok(layout) = ipc::call_text("pane.layout", serde_json::json!({ "pane_id": owner }))
    else {
        return;
    };
    let Ok(msg) = serde_json::from_str::<LayoutMsg2>(&layout) else { return };
    let body = msg.result.layout;
    let owner_ratio = body
        .panes
        .iter()
        .find(|p| p.pane_id.as_deref() == Some(owner))
        .and_then(|p| p.rect)
        .map(|r| r.width as f64 / body.area.width.max(1) as f64)
        .filter(|f| (0.02..0.55).contains(f))
        .map(|f| f.max(0.1))
        .unwrap_or(0.3);
    let mut others: Vec<(String, RectJ)> = body
        .panes
        .into_iter()
        .filter_map(|p| Some((p.pane_id?, p.rect?)))
        .filter(|(id, _)| !ours.contains(id))
        .collect();
    if others.is_empty() {
        return;
    }
    others.sort_by_key(|(_, r)| (r.y, r.x));

    // First pane opens the park tab; the rest pile in (their layout there
    // is irrelevant — the tab is never focused).
    let mut park_tab = String::new();
    for (i, (id, _)) in others.iter().enumerate() {
        let dest = if i == 0 {
            serde_json::json!({ "type": "new_tab", "label": "· preview" })
        } else {
            serde_json::json!({ "type": "tab", "tab_id": park_tab, "split": "right" })
        };
        let _ = ipc::call_text(
            "pane.move",
            serde_json::json!({ "pane_id": id, "destination": dest, "focus": false }),
        );
        if i == 0 {
            if let Ok(list2) = ipc::call_text("pane.list", serde_json::json!({})) {
                park_tab = crate::launch::tab_of(&list2, id);
            }
            if park_tab.is_empty() || park_tab == tab {
                return; // move didn't take; don't strand a half-plan
            }
        }
    }
    let plan = ParkPlan { tab, owner_ratio, panes: others };
    if let Ok(json) = serde_json::to_string(&plan) {
        let _ = std::fs::write(park_path(owner), json);
    }
}

/// Bring parked panes home, rebuilding their grid from the recorded rects
/// (each pane re-splits the recorded left/top neighbor at the recorded
/// proportions). Returns whether a plan existed.
pub fn restore_parked(owner: &str) -> bool {
    let path = park_path(owner);
    let Ok(json) = std::fs::read_to_string(&path) else { return false };
    let _ = std::fs::remove_file(&path);
    let Ok(plan) = serde_json::from_str::<ParkPlan>(&json) else { return false };

    let viewer = ipc::call_text("pane.list", serde_json::json!({}))
        .ok()
        .and_then(|l| viewer_pane_in_tab(&l, owner).map(|(id, _)| id));

    let tree = build_tree(&plan.panes);
    // The tree's representative (top-left-most) pane comes home first,
    // splitting whatever holds the region: the preview (which closes right
    // after, handing everything over) or the sidebar itself.
    let (anchor, ratio) = match &viewer {
        Some(v) => (v.clone(), 0.1),
        None => (owner.to_string(), plan.owner_ratio),
    };
    move_into(&plan.tab, rep(&tree), &anchor, "right", ratio);
    replay(&plan.tab, &tree);
    // And the sidebar's own width, which the park inflated.
    let _ = ipc::call_text(
        "layout.set_split_ratio",
        serde_json::json!({ "pane_id": owner, "path": [], "ratio": plan.owner_ratio }),
    );
    true
}

/// A recovered split tree: exactly the rects the panes had at park time.
enum Node {
    Leaf(String),
    Split {
        dir: &'static str,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// The subtree's representative: its top-left-most pane, which stands in
/// for the whole region until the subtree's own splits are replayed.
fn rep(node: &Node) -> &str {
    match node {
        Node::Leaf(id) => id,
        Node::Split { first, .. } => rep(first),
    }
}

/// Rebuild the split tree from pane rects by guillotine recovery: find a
/// full-height (or full-width) cut line that cleanly partitions the panes,
/// recurse on both sides. Binary-split layouts always admit one; if none is
/// found (foreign layout), fall back to a degenerate right-stack.
fn build_tree(panes: &[(String, RectJ)]) -> Node {
    if panes.len() == 1 {
        return Node::Leaf(panes[0].0.clone());
    }
    let min_x = panes.iter().map(|(_, r)| r.x).min().unwrap_or(0);
    let max_x = panes.iter().map(|(_, r)| r.x + r.width).max().unwrap_or(0);
    let min_y = panes.iter().map(|(_, r)| r.y).min().unwrap_or(0);
    let max_y = panes.iter().map(|(_, r)| r.y + r.height).max().unwrap_or(0);

    // Vertical cut candidates: every pane's left edge strictly inside.
    for (_, r) in panes {
        let cut = r.x;
        if cut <= min_x || cut >= max_x {
            continue;
        }
        if panes.iter().all(|(_, q)| q.x + q.width <= cut || q.x >= cut) {
            let (a, b): (Vec<_>, Vec<_>) =
                panes.iter().cloned().partition(|(_, q)| q.x + q.width <= cut);
            if !a.is_empty() && !b.is_empty() {
                return Node::Split {
                    dir: "right",
                    ratio: (cut - min_x) as f64 / (max_x - min_x).max(1) as f64,
                    first: Box::new(build_tree(&a)),
                    second: Box::new(build_tree(&b)),
                };
            }
        }
    }
    for (_, r) in panes {
        let cut = r.y;
        if cut <= min_y || cut >= max_y {
            continue;
        }
        if panes.iter().all(|(_, q)| q.y + q.height <= cut || q.y >= cut) {
            let (a, b): (Vec<_>, Vec<_>) =
                panes.iter().cloned().partition(|(_, q)| q.y + q.height <= cut);
            if !a.is_empty() && !b.is_empty() {
                return Node::Split {
                    dir: "down",
                    ratio: (cut - min_y) as f64 / (max_y - min_y).max(1) as f64,
                    first: Box::new(build_tree(&a)),
                    second: Box::new(build_tree(&b)),
                };
            }
        }
    }
    // No clean cut (shouldn't happen for herdr layouts): stack them.
    let rest = build_tree(&panes[1..]);
    Node::Split {
        dir: "right",
        ratio: 0.5,
        first: Box::new(Node::Leaf(panes[0].0.clone())),
        second: Box::new(rest),
    }
}

/// Pre-order replay: at each split, the region is currently held entirely
/// by rep(first); moving rep(second) in with the recorded direction/ratio
/// carves the region correctly before either side's inner splits run.
fn replay(tab: &str, node: &Node) {
    let Node::Split { dir, ratio, first, second } = node else { return };
    move_into(tab, rep(second), rep(first), dir, *ratio);
    replay(tab, first);
    replay(tab, second);
}

fn move_into(tab: &str, pane: &str, target: &str, split: &str, ratio: f64) {
    let _ = ipc::call_text(
        "pane.move",
        serde_json::json!({
            "pane_id": pane,
            "destination": {
                "type": "tab",
                "tab_id": tab,
                "split": split,
                "target_pane_id": target,
                "ratio": ratio.clamp(0.1, 0.9),
            },
            "focus": false,
        }),
    );
}

/// The rightmost pane in this layout, not `except`. Used to dock preview
/// on the far right (explorer | grok | preview).
fn rightmost_pane(layout_json: &str, except: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Msg {
        result: Res,
    }
    #[derive(serde::Deserialize)]
    struct Res {
        layout: L,
    }
    #[derive(serde::Deserialize)]
    struct L {
        #[serde(default)]
        panes: Vec<P>,
    }
    #[derive(serde::Deserialize)]
    struct P {
        pane_id: Option<String>,
        rect: Option<R>,
    }
    #[derive(serde::Deserialize)]
    struct R {
        x: i64,
        y: i64,
        width: i64,
        height: i64,
    }
    let msg: Msg = serde_json::from_str(layout_json.trim_start_matches('\u{feff}')).ok()?;
    msg.result
        .layout
        .panes
        .iter()
        .filter(|p| p.pane_id.as_deref() != Some(except))
        .filter_map(|p| Some((p.pane_id.clone()?, p.rect.as_ref()?)))
        .max_by_key(|(_, r)| (r.x + r.width, r.height, -r.y))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_roundtrip() {
        let f = file_request(Path::new("C:/x/y.rs"));
        assert_eq!(parse_request(&f), Some(Request::File(PathBuf::from("C:/x/y.rs"))));
        let s = show_request(Path::new("C:/repo"), "stash@{1}", None);
        assert_eq!(
            parse_request(&s),
            Some(Request::Show {
                root: PathBuf::from("C:/repo"),
                spec: "stash@{1}".into(),
                path: None,
            })
        );
        let s = show_request(Path::new("C:/repo"), "a1b2c3d", Some("src/a.rs"));
        assert_eq!(
            parse_request(&s),
            Some(Request::Show {
                root: PathBuf::from("C:/repo"),
                spec: "a1b2c3d".into(),
                path: Some("src/a.rs".into()),
            })
        );
        let d = diff_request(Path::new("C:/repo"), "src/a.rs", "staged");
        assert_eq!(
            parse_request(&d),
            Some(Request::Diff {
                root: PathBuf::from("C:/repo"),
                rel: "src/a.rs".into(),
                kind: "staged".into()
            })
        );
        // Legacy bare path still works.
        assert_eq!(
            parse_request("C:/plain.txt"),
            Some(Request::File(PathBuf::from("C:/plain.txt")))
        );
        assert_eq!(parse_request("  "), None);
    }

    #[test]
    fn viewer_lookup_reports_staleness() {
        let now = crate::state::unix_now();
        let json = format!(
            r#"{{"result":{{"panes":[
                {{"pane_id":"w1:p1","tab_id":"w1:t1"}},
                {{"pane_id":"w1:p2","tab_id":"w1:t1","tokens":{{"herdr-sidebar-preview":"{}"}}}}
            ]}}}}"#,
            now - 2
        );
        assert_eq!(viewer_pane_in_tab(&json, "w1:p1"), Some(("w1:p2".into(), false)));
        let stale = format!(
            r#"{{"result":{{"panes":[
                {{"pane_id":"w1:p1","tab_id":"w1:t1"}},
                {{"pane_id":"w1:p2","tab_id":"w1:t1","tokens":{{"herdr-sidebar-preview":"{}"}}}}
            ]}}}}"#,
            now - 999
        );
        assert_eq!(viewer_pane_in_tab(&stale, "w1:p1"), Some(("w1:p2".into(), true)));
    }

    #[test]
    fn rightmost_pane_skips_explorer_and_picks_far_edge() {
        let json = r#"{"result":{"layout":{"panes":[
            {"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":20,"height":40}},
            {"pane_id":"w1:p2","rect":{"x":20,"y":0,"width":50,"height":40}},
            {"pane_id":"w1:p3","rect":{"x":70,"y":0,"width":30,"height":40}}
        ]}}}"#;
        assert_eq!(rightmost_pane(json, "w1:p1").as_deref(), Some("w1:p3"));
        assert_eq!(rightmost_pane(json, "w1:p3").as_deref(), Some("w1:p2"));
        assert_eq!(rightmost_pane(r#"{"result":{"layout":{"panes":[]}}}"#, "w1:p1"), None);
    }
}
