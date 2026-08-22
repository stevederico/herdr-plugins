//! Show images/videos in the preview pane as real pixels (Kitty graphics),
//! not half-block ASCII. Herdr has no webview; this is the in-pane path.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct MediaState {
    pub path: PathBuf,
    pub png: Option<(Vec<u8>, u32, u32)>,
    pub painted: Option<(u16, u16)>,
}

pub fn is_image(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "heic" | "heif"
    )
}

pub fn is_video(path: &Path) -> bool {
    matches!(ext(path).as_str(), "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v")
}

pub fn is_media(path: &Path) -> bool {
    is_image(path) || is_video(path)
}

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// PNG bytes + pixel size.
pub fn rasterize_png(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    if ext(path) == "png" {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let (w, h) = png_size(&bytes).ok_or_else(|| "not a valid PNG".to_string())?;
        return Ok((bytes, w, h));
    }
    let dest = std::env::temp_dir().join(format!("herdr-media-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let ok = if is_video(path) {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "0", "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf", "scale='min(1600,iw)':-2"])
            .arg(&dest)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        Command::new("sips")
            .args(["-s", "format", "png", "-Z", "1600", "--out"])
            .arg(&dest)
            .arg(path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !ok {
        return Err("could not convert this file to PNG".into());
    }
    let bytes = std::fs::read(&dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&dest);
    let (w, h) = png_size(&bytes).ok_or_else(|| "not a valid PNG".to_string())?;
    Ok((bytes, w, h))
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// Paint header + real image + footer. Skips ratatui so it cannot cover pixels.
pub fn paint(state: &mut MediaState, title: &str) -> io::Result<()> {
    let (cols, rows) = crossterm::terminal::size()?;
    let body_rows = rows.saturating_sub(2).max(1);
    let body_cols = cols.max(2);
    if state.png.is_none() {
        match rasterize_png(&state.path) {
            Ok(png) => state.png = Some(png),
            Err(e) => {
                write_fallback(title, &e)?;
                return Ok(());
            }
        }
    }
    if state.painted == Some((body_cols, body_rows)) {
        return Ok(());
    }
    let Some((png, _, _)) = &state.png else { return Ok(()) };
    let mut out = io::stdout();
    // Clear + header
    write!(out, "\x1b[2J\x1b[H\x1b[1;36m ✕ \x1b[0;1m{title}\x1b[0m\r\n")?;
    emit_kitty(&mut out, png, body_cols, body_rows)?;
    write!(
        out,
        "\x1b[{rows};1H\x1b[0;2m o open original  esc close\x1b[0m"
    )?;
    out.flush()?;
    state.painted = Some((body_cols, body_rows));
    Ok(())
}

fn write_fallback(title: &str, err: &str) -> io::Result<()> {
    let mut out = io::stdout();
    write!(
        out,
        "\x1b[2J\x1b[H\x1b[1m{title}\x1b[0m\r\n({err})\r\npress o to open\n"
    )?;
    out.flush()
}

fn emit_kitty(out: &mut impl Write, png: &[u8], cols: u16, rows: u16) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=A\x1b\\")?;
    let payload = b64(png);
    let bytes = payload.as_bytes();
    let mut first = true;
    let mut rest = bytes;
    while !rest.is_empty() {
        let (chunk, tail) = rest.split_at(rest.len().min(4096));
        rest = tail;
        let more = u8::from(!rest.is_empty());
        if first {
            write!(out, "\x1b_Ga=T,f=100,c={cols},r={rows},C=1,q=2,m={more};")?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

pub fn open_external(path: &Path) {
    let _ = Command::new("open").arg(path).status();
}

pub fn clear() {
    let _ = write!(io::stdout(), "\x1b_Ga=d,d=A\x1b\\");
    let _ = io::stdout().flush();
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk.get(1).copied().unwrap_or(0));
        let c = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (a << 16) | (b << 8) | c;
        out.push(T[(n >> 18) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_exts() {
        assert!(is_image(Path::new("a.PNG")));
        assert!(is_video(Path::new("clip.mov")));
        assert!(!is_media(Path::new("readme.md")));
    }

    #[test]
    fn png_header() {
        let mut b = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        b.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&b), Some((640, 480)));
    }

    #[test]
    fn b64_hello() {
        assert_eq!(b64(b"hello"), "aGVsbG8=");
    }
}
