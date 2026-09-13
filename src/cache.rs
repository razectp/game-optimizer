//! Safe cleanup of standard Windows user temp folders.
//!
//! Only user Temp / INetCache style locations are eligible. Game shader
//! caches, Prefetch, Windows Update, and system roots are never touched.

use std::fs;
use std::path::{Path, PathBuf};

/// Outcome of one cache-cleanup pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheCleanReport {
    /// Bytes successfully deleted.
    pub bytes_freed: u64,
    /// Files deleted.
    pub files_removed: u64,
    /// Locked or denied files left in place.
    pub files_skipped: u64,
    /// I/O errors while walking.
    pub errors: u64,
}

impl CacheCleanReport {
    /// Merge another pass into this report.
    pub fn merge(&mut self, other: Self) {
        self.bytes_freed += other.bytes_freed;
        self.files_removed += other.files_removed;
        self.files_skipped += other.files_skipped;
        self.errors += other.errors;
    }

    /// Short Portuguese summary for the GUI.
    pub fn friendly_summary(&self) -> String {
        if self.files_removed == 0 {
            return "Nada para limpar na pasta Temp.".to_string();
        }
        format!(
            "Limpeza concluída: {} liberados ({} arquivo{}).",
            format_bytes(self.bytes_freed),
            self.files_removed,
            if self.files_removed == 1 { "" } else { "s" }
        )
    }
}

/// Human-readable byte size (KB/MB/GB).
pub fn format_bytes(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let value = bytes as f64;
    if value < K {
        format!("{bytes} B")
    } else if value < K * K {
        format!("{:.1} KB", value / K)
    } else if value < K * K * K {
        format!("{:.1} MB", value / (K * K))
    } else {
        format!("{:.1} GB", value / (K * K * K))
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// True when a path looks like a game shader cache or OS-critical location.
pub fn is_denied_cache_path(path: &Path) -> bool {
    let s = normalize_path(path);
    const DENY: &[&str] = &[
        "\\windows\\prefetch",
        "\\windows\\softwaredistribution",
        "\\windows\\winsxs",
        "\\windows\\system32",
        "\\windows\\syswow64",
        "\\shadercache",
        "\\d3dscache",
        "\\d3dcachedshaders",
        "\\dxcache",
        "\\glcache",
        "steamapps\\shadercache",
        "\\program files\\",
        "\\program files (x86)\\",
    ];
    DENY.iter().any(|marker| s.contains(marker))
}

/// True when `path` is a standard user temp/cache root we are willing to clean.
pub fn is_allowed_cache_path(path: &Path) -> bool {
    if is_denied_cache_path(path) {
        return false;
    }
    let s = normalize_path(path);
    if s.len() < 6 || s.ends_with(":\\") || s == "\\" || s == "/" {
        return false;
    }

    if let Ok(temp) = std::env::temp_dir().canonicalize() {
        if let Ok(canonical) = path.canonicalize() {
            if canonical == temp || canonical.starts_with(&temp) {
                return true;
            }
        }
    }

    s.contains("\\appdata\\local\\temp")
        || s.contains("\\appdata\\local\\microsoft\\windows\\inetcache")
        || s.contains("\\temporary internet files")
}

/// Roots cleaned by the GUI action (existing directories only).
pub fn user_cache_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let temp = std::env::temp_dir();
    roots.push(temp.clone());

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let local_temp = PathBuf::from(&local).join("Temp");
        if local_temp != temp {
            roots.push(local_temp);
        }
        roots.push(
            PathBuf::from(&local)
                .join("Microsoft")
                .join("Windows")
                .join("INetCache"),
        );
    }

    roots
        .into_iter()
        .filter(|path| path.is_dir() && is_allowed_cache_path(path))
        .collect()
}

/// Delete files under an allowed cache directory.
///
/// Never follows symlinks or Windows directory junctions.
pub fn clean_directory(root: &Path) -> CacheCleanReport {
    let mut report = CacheCleanReport::default();
    if !root.is_dir() || !is_allowed_cache_path(root) {
        report.errors += 1;
        return report;
    }
    walk_and_delete(root, root, 0, &mut report);
    report
}

fn is_link_or_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn stays_inside_root(root: &Path, path: &Path) -> bool {
    let Ok(root_canon) = root.canonicalize() else {
        return false;
    };
    match path.canonicalize() {
        Ok(canon) => canon.starts_with(&root_canon),
        Err(_) => {
            // Locked/broken entries: keep walking only if the lexical path stays
            // under the original root (junctions are skipped via reparse checks).
            path.starts_with(root)
        }
    }
}

/// Clean every allowlisted user cache root.
pub fn clean_user_caches() -> CacheCleanReport {
    let mut total = CacheCleanReport::default();
    for root in user_cache_roots() {
        total.merge(clean_directory(&root));
    }
    total
}

fn walk_and_delete(root: &Path, dir: &Path, depth: u32, report: &mut CacheCleanReport) {
    if depth > 8 {
        report.files_skipped += 1;
        return;
    }
    if is_denied_cache_path(dir) {
        report.files_skipped += 1;
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            report.errors += 1;
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => {
                report.files_skipped += 1;
                continue;
            }
        };
        if is_link_or_reparse(&meta) {
            report.files_skipped += 1;
            continue;
        }
        if !stays_inside_root(root, &path) {
            report.files_skipped += 1;
            continue;
        }
        if meta.is_dir() {
            walk_and_delete(root, &path, depth + 1, report);
            let _ = fs::remove_dir(&path);
        } else if meta.is_file() {
            let len = meta.len();
            match fs::remove_file(&path) {
                Ok(()) => {
                    report.files_removed += 1;
                    report.bytes_freed += len;
                }
                Err(_) => report.files_skipped += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn denies_shader_and_system_locations() {
        assert!(is_denied_cache_path(Path::new(
            r"C:\Windows\Prefetch\FOO.pf"
        )));
        assert!(is_denied_cache_path(Path::new(
            r"C:\Steam\steamapps\shadercache\123"
        )));
        assert!(is_denied_cache_path(Path::new(
            r"C:\Users\me\AppData\Local\D3DSCache"
        )));
        assert!(is_denied_cache_path(Path::new(
            r"C:\Users\me\AppData\Local\NVIDIA\DXCache"
        )));
        assert!(!is_denied_cache_path(Path::new(
            r"C:\Users\me\AppData\Local\Temp"
        )));
    }

    #[test]
    fn rejects_drive_roots() {
        assert!(!is_allowed_cache_path(Path::new(r"C:\")));
        assert!(!is_allowed_cache_path(Path::new("/")));
    }

    #[test]
    fn format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn friendly_summary_when_empty() {
        assert_eq!(
            CacheCleanReport::default().friendly_summary(),
            "Nada para limpar na pasta Temp."
        );
    }

    #[test]
    fn cleans_sandbox_under_temp_and_skips_denied_names() {
        let sandbox = std::env::temp_dir().join(format!(
            "go-cache-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(sandbox.join("nested")).unwrap();
        fs::write(sandbox.join("a.tmp"), b"hello-cache").unwrap();
        fs::write(sandbox.join("nested").join("b.tmp"), b"more").unwrap();

        assert!(is_allowed_cache_path(&sandbox));
        let report = clean_directory(&sandbox);
        assert!(report.files_removed >= 2, "{report:?}");
        assert!(!sandbox.join("a.tmp").exists());

        let _ = fs::remove_dir_all(&sandbox);

        #[cfg(unix)]
        {
            let sandbox = std::env::temp_dir().join(format!(
                "go-cache-link-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&sandbox).unwrap();
            let outside = std::env::temp_dir().join(format!(
                "go-cache-outside-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::write(&outside, b"keep-me").unwrap();
            let _ = std::os::unix::fs::symlink(&outside, sandbox.join("escape.tmp"));
            let report = clean_directory(&sandbox);
            assert!(
                outside.exists(),
                "cleanup must not follow symlinks: {report:?}"
            );
            let _ = fs::remove_file(&outside);
            let _ = fs::remove_dir_all(&sandbox);
        }

        let denied = PathBuf::from(r"C:\Windows\System32");
        let blocked = clean_directory(&denied);
        assert_eq!(blocked.files_removed, 0);
        assert!(blocked.errors >= 1 || blocked.files_skipped >= 1);
    }
}
