//! Copy text to attached herdr *clients* via OSC 52.
//!
//! The herdr **server** on this Mac owns pbcopy. Select-to-copy therefore lands
//! on the Studio pasteboard. An `ssh -t herdr` client has a real TTY that
//! reaches the laptop terminal — write OSC 52 there and the laptop clipboard
//! updates. Native pbcopy still runs so a local Studio session keeps working.

use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const BRIDGE_FLAG: &str = "--clipboard-bridge";

pub fn osc52_payload(text: &str) -> Vec<u8> {
    let encoded = base64_encode(text.as_bytes());
    let mut out = Vec::with_capacity(encoded.len() + 8);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(encoded.as_bytes());
    out.push(0x07);
    out
}

/// Client ttys from `ps -axo tty=,args=` (not the server, not sidebar).
pub fn herdr_client_ttys_from_ps(ps: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in ps.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((tty, args)) = split_tty_args(line) else {
            continue;
        };
        if tty == "??" || tty.is_empty() || !is_herdr_client(args) {
            continue;
        }
        out.push(PathBuf::from(format!("/dev/{tty}")));
    }
    out
}

pub fn forward_to_clients(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let payload = osc52_payload(text);
    let ttys = herdr_client_ttys();
    let mut n = 0;
    for tty in ttys {
        if write_all(&tty, &payload).is_ok() {
            n += 1;
        }
    }
    n
}

/// Start the log/file watcher once (ensure hook fires constantly).
pub fn spawn_bridge() {
    if cfg!(not(unix)) {
        return;
    }
    if bridge_alive() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let child = Command::new(exe)
        .arg(BRIDGE_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(child) = child {
        let _ = std::fs::write(pid_path(), child.id().to_string());
    }
}

pub fn run_bridge() {
    let last_copy = grok_last_copy_path();
    let server_log = herdr_server_log_path();
    let mut seen_mtime: Option<SystemTime> = mtime(&last_copy);
    let mut log_pos = file_len(&server_log);
    let mut last_sent = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(400));
        if let Some(text) = changed_file_text(&last_copy, &mut seen_mtime) {
            send(&text, &mut last_sent);
        }
        if let Some(chunk) = new_log_bytes(&server_log, &mut log_pos) {
            if chunk.contains("copied selection to clipboard")
                || chunk.contains("copied double-clicked token to clipboard")
            {
                std::thread::sleep(Duration::from_millis(50));
                if let Some(text) = pbpaste() {
                    send(&text, &mut last_sent);
                }
            }
        }
    }
}

fn send(text: &str, last_sent: &mut String) {
    let text = text.trim_end_matches(['\n', '\r']);
    if text.is_empty() || text == last_sent {
        return;
    }
    if forward_to_clients(text) > 0 {
        last_sent.clear();
        last_sent.push_str(text);
    }
}

fn herdr_client_ttys() -> Vec<PathBuf> {
    let output = Command::new("ps").args(["-axo", "tty=,args="]).output();
    match output {
        Ok(out) => herdr_client_ttys_from_ps(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

fn split_tty_args(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let tty = parts.next()?.trim();
    let args = parts.next()?.trim();
    Some((tty, args))
}

fn is_herdr_client(args: &str) -> bool {
    let prog = args.split_whitespace().next().unwrap_or("");
    let name = Path::new(prog).file_name().and_then(|s| s.to_str()).unwrap_or(prog);
    name == "herdr" && !args.split_whitespace().any(|a| a == "server")
}

fn write_all(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

fn grok_last_copy_path() -> PathBuf {
    home().join(".grok").join("last-copy.txt")
}

fn herdr_server_log_path() -> PathBuf {
    home().join(".config").join("herdr").join("herdr-server.log")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn pid_path() -> PathBuf {
    std::env::temp_dir().join("herdr-clipboard-bridge.pid")
}

fn bridge_alive() -> bool {
    let Ok(raw) = std::fs::read_to_string(pid_path()) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return false;
    };
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn changed_file_text(path: &Path, seen: &mut Option<SystemTime>) -> Option<String> {
    let now = mtime(path)?;
    if *seen == Some(now) {
        return None;
    }
    *seen = Some(now);
    std::fs::read_to_string(path).ok()
}

fn new_log_bytes(path: &Path, pos: &mut u64) -> Option<String> {
    let len = file_len(path);
    if len < *pos {
        *pos = 0;
    }
    if len == *pos {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(io::SeekFrom::Start(*pos)).ok()?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;
    *pos = len;
    Some(buf)
}

fn pbpaste() -> Option<String> {
    let out = Command::new("pbpaste").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied();
        let b2 = data.get(i + 2).copied();
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4)) as usize] as char);
        match (b1, b2) {
            (Some(b1), Some(b2)) => {
                out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
                out.push(T[(b2 & 0x3f) as usize] as char);
            }
            (Some(b1), None) => {
                out.push(T[((b1 & 0x0f) << 2) as usize] as char);
                out.push('=');
            }
            (None, _) => {
                out.push('=');
                out.push('=');
            }
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_hello() {
        assert_eq!(osc52_payload("hello"), b"\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn client_ttys_skip_server_and_sidebar() {
        let ps = "\
ttys002 herdr
??       /usr/local/bin/herdr server
ttys000  /tmp/herdr-sidebar
ttys003 /usr/local/bin/herdr
ttys004 herdr-sidebar-ensure
";
        let ttys = herdr_client_ttys_from_ps(ps);
        assert_eq!(
            ttys,
            vec![PathBuf::from("/dev/ttys002"), PathBuf::from("/dev/ttys003")]
        );
    }
}
