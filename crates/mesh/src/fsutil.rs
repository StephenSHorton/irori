//! Data-dir file writes the Go way (mesh.md §2.5): `path.tmp` with mode
//! 0600, then renamed over the target.

use std::io::Write as _;
use std::path::Path;

/// Go `os.WriteFile(tmp, raw, 0o600)` then `os.Rename(tmp, path)`.
pub(crate) fn write_replace(path: &Path, raw: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        f.write_all(raw)?;
    }
    std::fs::rename(&tmp, path)
}
