//! Rasterize images/videos into terminal half-block cells (ffmpeg, no extra
//! crates) so the preview pane shows the picture in-place.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct CellImage {
    pub cols: u16,
    pub rows: u16,
    /// Row-major, one cell: upper RGB then lower RGB.
    pub cells: Vec<(u8, u8, u8, u8, u8, u8)>,
}

pub struct MediaState {
    pub path: PathBuf,
    pub image: Option<CellImage>,
    pub fit: (u16, u16),
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

/// Decode `path` into `cols` × `rows` half-block cells (letterboxed).
pub fn rasterize_cells(path: &Path, cols: u16, rows: u16) -> Result<CellImage, String> {
    let cols = cols.max(2);
    let rows = rows.max(1);
    let pw = u32::from(cols);
    let ph = u32::from(rows) * 2;
    match ffmpeg_rgb(path, pw, ph) {
        Ok(rgb) => pack_cells(rgb, cols, rows),
        Err(_) if is_image(path) => {
            let png = sips_png(path)?;
            let rgb = ffmpeg_rgb(&png, pw, ph)?;
            let _ = std::fs::remove_file(&png);
            pack_cells(rgb, cols, rows)
        }
        Err(e) => Err(e),
    }
}

fn sips_png(path: &Path) -> Result<PathBuf, String> {
    let dest = std::env::temp_dir().join(format!("herdr-media-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let status = Command::new("sips")
        .args(["-s", "format", "png", "--out"])
        .arg(&dest)
        .arg(path)
        .status()
        .map_err(|e| format!("sips: {e}"))?;
    if !status.success() {
        return Err("sips could not convert this image".into());
    }
    Ok(dest)
}

fn ffmpeg_rgb(path: &Path, pw: u32, ph: u32) -> Result<Vec<u8>, String> {
    let vf = format!(
        "scale={pw}:{ph}:force_original_aspect_ratio=decrease,pad={pw}:{ph}:(ow-iw)/2:(oh-ih)/2:black"
    );
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if is_video(path) {
        cmd.args(["-ss", "0"]);
    }
    let out = cmd
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("ffmpeg failed {}", err.trim()));
    }
    let want = (pw as usize) * (ph as usize) * 3;
    if out.stdout.len() < want {
        return Err(format!("short frame {} < {want}", out.stdout.len()));
    }
    Ok(out.stdout)
}

fn pack_cells(rgb: Vec<u8>, cols: u16, rows: u16) -> Result<CellImage, String> {
    let cols_us = cols as usize;
    let rows_us = rows as usize;
    let stride = cols_us * 3;
    let mut cells = Vec::with_capacity(cols_us * rows_us);
    for y in 0..rows_us {
        let upper = y * 2 * stride;
        let lower = (y * 2 + 1) * stride;
        for x in 0..cols_us {
            let u = upper + x * 3;
            let l = lower + x * 3;
            cells.push((
                rgb[u],
                rgb[u + 1],
                rgb[u + 2],
                rgb[l],
                rgb[l + 1],
                rgb[l + 2],
            ));
        }
    }
    Ok(CellImage { cols, rows, cells })
}

pub fn open_external(path: &Path) {
    let _ = Command::new("open").arg(path).status();
}

pub fn clear() {}

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
    fn pack_two_by_one() {
        // 2x2 pixels → 2 cols × 1 cell row
        let rgb = vec![
            255, 0, 0, 0, 255, 0, // upper
            0, 0, 255, 255, 255, 255, // lower
        ];
        let img = pack_cells(rgb, 2, 1).unwrap();
        assert_eq!(img.cells[0], (255, 0, 0, 0, 0, 255));
        assert_eq!(img.cells[1], (0, 255, 0, 255, 255, 255));
    }
}
