//! Clipboard write access for images. iced only offers text on the
//! clipboard, so images go through the platform's usual paths: wl-copy /
//! xclip on Linux, osascript on macOS, and PowerShell (.NET) on Windows.

use anyhow::{bail, Context, Result};

/// MIME type of the smallest image formats we can place on the clipboard.
/// PNG, JPEG and GIF can all be sent as-is; anything else (WebP, BMP) is
/// rejected — mail uses these three almost exclusively.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, 0x50, 0x4E, 0x47, ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x47, 0x49, 0x46, ..] => Some("image/gif"),
        _ => None,
    }
}

/// Places an image on the clipboard. `bytes` keeps its original encoding.
pub async fn copy_image(bytes: &[u8]) -> Result<()> {
    let Some(mime) = image_mime(bytes) else {
        bail!("Unsupported image format (PNG, JPEG or GIF only).");
    };
    #[cfg(target_os = "linux")]
    return copy_linux(bytes, mime).await;
    #[cfg(target_os = "macos")]
    return copy_macos(bytes).await;
    #[cfg(target_os = "windows")]
    return copy_windows(bytes).await;
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = mime;
        bail!("Copying images is not supported on this platform.")
    }
}

/// Feed bytes to a command's stdin (the clipboard tools read from there).
async fn pipe_stdin(tool: &str, args: &[&str], bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(tool)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not start {tool}"))?;
    let mut stdin = child.stdin.take().context("{tool} stdin closed")?;
    stdin.write_all(bytes).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let err = child.wait().await?;
    if err.success() {
        Ok(())
    } else {
        bail!("{tool} exited with {}", err.code().unwrap_or_default());
    }
}

async fn copy_linux(bytes: &[u8], mime: &str) -> Result<()> {
    // wl-copy auto-detects the type; xclip needs it spelled out.
    if pipe_stdin("wl-copy", &[], bytes).await.is_ok() {
        return Ok(());
    }
    pipe_stdin("xclip", &["-selection", "clipboard", "-t", mime], bytes).await
}

#[cfg(target_os = "macos")]
async fn copy_macos(bytes: &[u8]) -> Result<()> {
    let path = temp_png(bytes)?;
    let f = tokio::process::Command::new("osascript")
        .args(["-e", &format!("set the clipboard to (read (POSIX file \"{path}\") as «class PNGf»)")] )
        .spawn()?
        .wait()
        .await?;
    cleanup(&path);
    if f.success() { Ok(()) } else { bail!("osascript could not set the clipboard") }
}

#[cfg(target_os = "windows")]
async fn copy_windows(bytes: &[u8]) -> Result<()> {
    let path = temp_png(bytes)?;
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; \
         $i=[System.Drawing.Image]::FromFile('{}'); [System.Windows.Forms.Clipboard]::SetImage($i)",
        path.display()
    );
    let f = tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .spawn()?
        .wait()
        .await?;
    cleanup(&path);
    if f.success() { Ok(()) } else { bail!("could not set the clipboard") }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn temp_png(bytes: &[u8]) -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("mailix-clip-{}.img", std::process::id()));
    tokio::fs::write(&path, bytes).await?;
    Ok(path)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_image_formats() {
        assert_eq!(image_mime(b"\x89PNG\r"), Some("image/png"));
        assert_eq!(image_mime(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(image_mime(b"GIF89a...."), Some("image/gif"));
        assert_eq!(image_mime(b"RIFF....WEBP"), None, "webp is not clippable");
        assert_eq!(image_mime(b""), None);
    }
}
