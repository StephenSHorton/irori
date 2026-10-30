//! The single-instance id (Go `instanceID`, I:gui.go:28-40; architecture.md
//! 4.2). A second launch with the same id hands off to the first, which
//! shows its window; a test copy with its own `IRORI_HOME` gets its own id
//! and runs beside the usual Irori.

use sha2::{Digest, Sha256};

/// Go `singleInstanceID`. Tauri's `identifier` (tauri.conf.json) is the
/// same string, so the release bundle id is unchanged.
pub const SINGLE_INSTANCE_ID: &str = "com.stephenshorton.irori";

/// Go `instanceID`: [`SINGLE_INSTANCE_ID`], plus `.` and the first 6 bytes
/// of sha256(lower(filepath.Clean(IRORI_HOME))) in hex when it is set.
/// `home` is the raw `IRORI_HOME` value; empty counts as unset, as Go's
/// `os.Getenv` does.
pub fn instance_id(home: Option<&str>) -> String {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return SINGLE_INSTANCE_ID.to_string();
    };
    // Go strings.ToLower over UTF-8; the data dir paths are what matters,
    // and Rust's to_lowercase agrees with it outside a few special casings
    // (final sigma) that no path here carries.
    let cleaned = gocompat::filepath::clean(home).to_lowercase();
    let sum = Sha256::digest(cleaned.as_bytes());
    format!("{SINGLE_INSTANCE_ID}.{}", hex::encode(&sum[..6]))
}

/// [`instance_id`] for this process's environment.
pub fn current() -> String {
    instance_id(std::env::var("IRORI_HOME").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_is_the_bundle_id() {
        assert_eq!(instance_id(None), SINGLE_INSTANCE_ID);
        assert_eq!(instance_id(Some("")), SINGLE_INSTANCE_ID);
    }

    #[test]
    fn hash_of_clean_lower_path() {
        let id = instance_id(Some("C:/Scratch/Irori"));
        assert!(id.starts_with("com.stephenshorton.irori."), "{id}");
        assert_eq!(id.len(), SINGLE_INSTANCE_ID.len() + 1 + 12);
        // Case and Clean-equivalent spellings give one id.
        assert_eq!(id, instance_id(Some("c:/scratch/irori/")));
        assert_ne!(id, instance_id(Some("C:/Scratch/Other")));
    }
}
