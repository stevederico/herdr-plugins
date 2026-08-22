//! Rasterize images/videos for the preview pane (sips + ffmpeg, no extra
//! crates) and place them with `pane.graphics.set`.

use std::path::Path;
use std::process::Command;

use crate::ipc;

const MAX_EDGE: &str = "1600";

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

/// PNG bytes + pixel size, ready for `pane.graphics.set`.
pub fn rasterize(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let dest = std::env::temp_dir().join(format!("herdr-media-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    if is_video(path) {
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-ss",
                "0",
                "-i",
            ])
            .arg(path)
            .args(["-frames:v", "1", "-vf", &format!("scale='min({MAX_EDGE},iw)':-2")])
            .arg(&dest)
            .status()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        if !status.success() {
            return Err("ffmpeg could not grab a frame".into());
        }
    } else if ext(path) == "png" {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if let Some((w, h)) = png_size(&bytes) {
            return Ok((bytes, w, h));
        }
        return Err("not a valid PNG".into());
    } else {
        let status = Command::new("sips")
            .args(["-s", "format", "png", "-Z", MAX_EDGE, "--out"])
            .arg(&dest)
            .arg(path)
            .status()
            .map_err(|e| format!("sips: {e}"))?;
        if !status.success() {
            return Err("sips could not convert this image".into());
        }
    }
    let bytes = std::fs::read(&dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&dest);
    let (w, h) = png_size(&bytes).ok_or_else(|| "converted file was not PNG".to_string())?;
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

pub fn show(png: &[u8], width: u32, height: u32) -> Result<(), String> {
    let pane_id = std::env::var("HERDR_PANE_ID").map_err(|_| "no pane".to_string())?;
    let _ = ipc::call_text("pane.graphics.clear", serde_json::json!({ "pane_id": pane_id }));
    let resp = ipc::call_text(
        "pane.graphics.set",
        serde_json::json!({
            "pane_id": pane_id,
            "format": "png",
            "image_width": width,
            "image_height": height,
            "data_base64": b64(png),
            "placement": {
                "viewport_col": 1,
                "viewport_row": 1,
                "grid_cols": 0,
                "grid_rows": 0,
            },
        }),
    )
    .map_err(|e| e.to_string())?;
    if resp.contains("feature_disabled") || resp.contains("\"error\"") {
        return Err(resp.trim().to_string());
    }
    Ok(())
}

pub fn clear() {
    let Ok(pane_id) = std::env::var("HERDR_PANE_ID") else { return };
    if pane_id.is_empty() {
        return;
    }
    let _ = ipc::call_text("pane.graphics.clear", serde_json::json!({ "pane_id": pane_id }));
}

pub fn open_external(path: &Path) {
    let _ = Command::new("open").arg(path).status();
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
        assert_eq!(png_size(b"not png"), None);
    }

    #[test]
    fn b64_hello() {
        assert_eq!(b64(b"hello"), "aGVsbG8=");
    }
}
