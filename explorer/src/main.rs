//! Thin file tree for herdr. Navigate dirs; Enter opens files in $EDITOR.
//! yagni: no git badges / preview / fuzzy — add when the tree feels right.

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde_json::Value;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

struct App {
    root: PathBuf,
    cwd: PathBuf,
    entries: Vec<Entry>,
    state: ListState,
    status: String,
}

impl App {
    fn new(root: PathBuf) -> Self {
        let mut app = Self {
            cwd: root.clone(),
            root,
            entries: Vec::new(),
            state: ListState::default(),
            status: String::new(),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.entries.clear();
        if self.cwd != self.root {
            self.entries.push(Entry {
                name: "..".into(),
                path: self.cwd.parent().unwrap_or(&self.cwd).to_path_buf(),
                is_dir: true,
            });
        }
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(rd) = fs::read_dir(&self.cwd) {
            for e in rd.flatten() {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = path.is_dir();
                let entry = Entry {
                    name,
                    path,
                    is_dir,
                };
                if is_dir {
                    dirs.push(entry);
                } else {
                    files.push(entry);
                }
            }
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.entries.extend(dirs);
        self.entries.extend(files);
        if self.entries.is_empty() {
            self.state.select(None);
        } else {
            let i = self.state.selected().unwrap_or(0).min(self.entries.len() - 1);
            self.state.select(Some(i));
        }
        self.status.clear();
    }

    fn move_sel(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len) as usize;
        self.state.select(Some(next));
    }

    fn open_selected(&mut self) {
        let Some(i) = self.state.selected() else {
            return;
        };
        let Some(entry) = self.entries.get(i) else {
            return;
        };
        if entry.is_dir {
            self.cwd = entry.path.clone();
            self.state.select(Some(0));
            self.reload();
            return;
        }
        let path = entry.path.clone();
        self.open_file(&path);
    }

    fn open_file(&mut self, path: &Path) {
        let editor = env::var("VISUAL")
            .or_else(|_| env::var("EDITOR"))
            .unwrap_or_else(|_| "nvim".into());
        // Leave alt screen so the editor owns the pane cleanly.
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} {}",
                editor,
                shell_escape(&path.display().to_string())
            ))
            .status();
        let _ = enable_raw_mode();
        let _ = execute!(io::stdout(), EnterAlternateScreen);
        match status {
            Ok(s) if s.success() => self.status = format!("edited {}", path.display()),
            Ok(s) => self.status = format!("editor exit {}", s.code().unwrap_or(-1)),
            Err(e) => self.status = format!("editor failed: {e}"),
        }
        self.reload();
    }

    fn go_up(&mut self) {
        if self.cwd == self.root {
            return;
        }
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.state.select(Some(0));
            self.reload();
        }
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn workspace_root() -> PathBuf {
    if let Ok(raw) = env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        if let Ok(v) = serde_json::from_str::<Value>(&raw) {
            for key in ["workspace_cwd", "focused_pane_cwd", "cwd"] {
                if let Some(p) = v.get(key).and_then(|x| x.as_str()) {
                    let pb = PathBuf::from(p);
                    if pb.is_dir() {
                        return pb;
                    }
                }
            }
        }
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() -> io::Result<()> {
    let root = workspace_root();
    let mut app = App::new(root);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| {
            let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());
            let items: Vec<ListItem> = app
                .entries
                .iter()
                .map(|e| {
                    let icon = if e.is_dir { "▸ " } else { "  " };
                    let style = if e.is_dir {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    ListItem::new(Line::from(Span::styled(
                        format!("{icon}{}", e.name),
                        style,
                    )))
                })
                .collect();

            let title = format!(" {} ", app.cwd.display());
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .border_style(Style::default().fg(Color::DarkGray)),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Indexed(236))
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› ");
            f.render_stateful_widget(list, chunks[0], &mut app.state);

            let help = if app.status.is_empty() {
                "j/k move  h up  l/↵ open  e edit  q quit".into()
            } else {
                app.status.clone()
            };
            f.render_widget(
                Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
            (KeyCode::Char('j') | KeyCode::Down, _) => app.move_sel(1),
            (KeyCode::Char('k') | KeyCode::Up, _) => app.move_sel(-1),
            (KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace, _) => app.go_up(),
            (KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter, _) => app.open_selected(),
            (KeyCode::Char('e'), _) => {
                if let Some(i) = app.state.selected() {
                    if let Some(e) = app.entries.get(i) {
                        if !e.is_dir {
                            let p = e.path.clone();
                            app.open_file(&p);
                        }
                    }
                }
            }
            (KeyCode::Char('r'), _) => app.reload(),
            _ => {}
        }
    }
    Ok(())
}
