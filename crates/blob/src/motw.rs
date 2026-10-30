//! Go `MarkDownloaded` (`motw_*.go`, capture.md §4): marks a file written
//! out of the store as from another computer.

use std::io;

/// Windows: a `Zone.Identifier` stream with the Internet zone (Mark of the
/// Web), so SmartScreen, Office Protected View and the like treat the file
/// as a download.
#[cfg(windows)]
pub fn mark_downloaded(path: &std::path::Path) -> io::Result<()> {
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    std::fs::write(ads, b"[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// macOS: the quarantine attribute a browser download gets (flags 0081:
/// downloaded, not yet approved), so Gatekeeper checks it before it runs.
/// Not yet run on a Mac.
#[cfg(target_os = "macos")]
pub fn mark_downloaded(path: &std::path::Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let v = format!("0081;{now:08x};Irori;");
    let p = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let name = c"com.apple.quarantine";
    // SAFETY: p and name are NUL-terminated; v outlives the call.
    let rc = unsafe { libc::setxattr(p.as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0, 0) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Elsewhere there is no download mark.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn mark_downloaded(_path: &std::path::Path) -> io::Result<()> {
    Ok(())
}
