use super::heap::DirEntry;
use super::scanner;

use chrono::{DateTime, Utc};
use std::fs;
use std::io;
use std::path::PathBuf;

pub fn create_insight_entries() -> Vec<DirEntry> {
    let home = match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => h,
        _ => return Vec::new(),
    };

    let mut entries = Vec::new();

    let backup_path = PathBuf::from(&home)
        .join("Library")
        .join("Application Support")
        .join("MobileSync")
        .join("Backup");
    if fs::metadata(&backup_path)
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        entries.push(DirEntry {
            name: "iOS Backups".into(),
            path: backup_path.to_string_lossy().into(),
            is_dir: true,
            size: -1,
            last_access: None,
            is_symlink: false,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        });
    }

    let downloads_path = PathBuf::from(&home).join("Downloads");
    if fs::metadata(&downloads_path)
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        entries.push(DirEntry {
            name: "Old Downloads (90d+)".into(),
            path: downloads_path.to_string_lossy().into(),
            is_dir: true,
            size: -1,
            last_access: None,
            is_symlink: false,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        });
    }

    let cleanable_paths: &[(&str, PathBuf)] = &[
        (
            "System Logs",
            PathBuf::from(&home).join("Library").join("Logs"),
        ),
        (
            "Homebrew Cache",
            PathBuf::from(&home)
                .join("Library")
                .join("Caches")
                .join("Homebrew"),
        ),
        (
            "Xcode DerivedData",
            PathBuf::from(&home)
                .join("Library")
                .join("Developer")
                .join("Xcode")
                .join("DerivedData"),
        ),
        (
            "Xcode Simulators",
            PathBuf::from(&home)
                .join("Library")
                .join("Developer")
                .join("CoreSimulator")
                .join("Devices"),
        ),
        (
            "Xcode Archives",
            PathBuf::from(&home)
                .join("Library")
                .join("Developer")
                .join("Xcode")
                .join("Archives"),
        ),
        (
            "Spotify Cache",
            PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join("Spotify")
                .join("PersistentCache"),
        ),
        (
            "JetBrains Cache",
            PathBuf::from(&home)
                .join("Library")
                .join("Caches")
                .join("JetBrains"),
        ),
        (
            "Docker Data",
            PathBuf::from(&home)
                .join("Library")
                .join("Containers")
                .join("com.docker.docker")
                .join("Data"),
        ),
        (
            "pip Cache",
            PathBuf::from(&home)
                .join("Library")
                .join("Caches")
                .join("pip"),
        ),
        (
            "Gradle Cache",
            PathBuf::from(&home).join(".gradle").join("caches"),
        ),
        (
            "CocoaPods Cache",
            PathBuf::from(&home)
                .join("Library")
                .join("Caches")
                .join("CocoaPods"),
        ),
    ];

    // OrbStack Data — scan Group Containers for dev.orbstack buckets.
    let gc_dir = PathBuf::from(&home)
        .join("Library")
        .join("Group Containers");
    if let Ok(read) = fs::read_dir(&gc_dir) {
        for entry in read.flatten() {
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            if name_s.contains("dev.orbstack")
                && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            {
                let data = gc_dir.join(&*name_s).join("data");
                if data.is_dir() {
                    entries.push(DirEntry {
                        name: "OrbStack Data".into(),
                        path: data.to_string_lossy().into(),
                        is_dir: true,
                        size: -1,
                        last_access: None,
                        is_symlink: false,
                        child_files: 0,
                        child_dirs: 0,
                        child_links: 0,
                        is_bundle_leaf: false,
                        bundle_id: None,
                        bundle_display_name: None,
                    });
                    break;
                }
            }
        }
    }

    for (name, p) in cleanable_paths {
        if fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false) {
            entries.push(DirEntry {
                name: (*name).into(),
                path: p.to_string_lossy().into(),
                is_dir: true,
                size: -1,
                last_access: None,
                is_symlink: false,
                child_files: 0,
                child_dirs: 0,
                child_links: 0,
                is_bundle_leaf: false,
                bundle_id: None,
                bundle_display_name: None,
            });
        }
    }

    entries
}

pub fn measure_insight_size(path: &str) -> Result<i64, String> {
    let home = std::env::var("HOME").unwrap_or_default();

    let downloads = PathBuf::from(&home).join("Downloads");
    if !home.is_empty() && path == downloads.to_string_lossy() {
        return measure_old_downloads(path, 90).map_err(|e| e.to_string());
    }

    scanner::measure_overview_size(path)
}

fn measure_old_downloads(dir: &str, days_old: i32) -> Result<i64, io::Error> {
    let cutoff = Utc::now() - chrono::Duration::days(days_old as i64);
    let mut total: i64 = 0;

    let read = fs::read_dir(dir)?;
    for entry in read.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mtime = match meta.modified() {
            Ok(st) => {
                let d = st.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .unwrap_or_else(Utc::now)
            }
            Err(_) => Utc::now(),
        };

        if mtime >= cutoff {
            continue;
        }

        let sub = PathBuf::from(dir).join(entry.file_name());
        let sub_s = sub.to_string_lossy().to_string();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Ok(s) = get_dir_size_fast(&sub_s) {
                total = total.saturating_add(s);
            }
        } else {
            total = total.saturating_add(meta.len() as i64);
        }
    }

    Ok(total)
}

/// Go `insightIcon`：两图标方案。
///
/// - 📁 用于顶层可浏览目录（Home, User Library, Applications, System Library）
/// - 👀 用于「隐藏空间洞察」：静默累积磁盘用量、值得注意的路径
///
/// 不再使用逐条目独有图标，统一用 👀 表示「关注」，不暗示内容可安全删除。
pub fn insight_icon(entry: &DirEntry) -> &'static str {
    match entry.name.as_str() {
        "Home" | "User Library" | "App Library" | "Applications" | "System Library" => "\u{1F4C1}",
        _ => "\u{1F440}",
    }
}

/// Go `getDirSizeFast`：原生 size-only 并行遍历（V2 替代旧版 `du -sk`，对齐红线 1）。
fn get_dir_size_fast(path: &str) -> Result<i64, io::Error> {
    scanner::measure_dir_size_native(path, "", &[])
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}

// ── tests (mirrors Mole/cmd/analyze/insights_test.go) ──

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // helper: new DirEntry for tests
    fn de(name: &str) -> DirEntry {
        DirEntry {
            name: name.into(),
            path: "/".into(),
            size: 0,
            is_dir: true,
            last_access: None,
            is_symlink: false,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        }
    }

    /// Set file mtime N days ago using macOS `touch -t`.
    #[cfg(target_os = "macos")]
    fn set_mtime_days_ago(path: &std::path::Path, days: u64) {
        let t = chrono::Local::now() - chrono::Duration::days(days as i64);
        let ts = t.format("%Y%m%d%H%M.%S").to_string();
        let status = std::process::Command::new("touch")
            .arg("-t")
            .arg(&ts)
            .arg(path)
            .status()
            .expect("touch -t");
        assert!(status.success(), "touch -t failed: {ts}");
    }

    #[cfg(not(target_os = "macos"))]
    fn set_mtime_days_ago(path: &std::path::Path, days: u64) {
        // fallback: just create the file, test won't filter by mtime
        let _ = (path, days);
    }

    #[test]
    fn create_insight_entries_invariants() {
        let entries = create_insight_entries();
        if entries.is_empty() {
            // Acceptable: HOME may not have any insight dirs.
            return;
        }
        for e in &entries {
            assert!(!e.name.is_empty(), "entry has empty Name");
            assert!(!e.path.is_empty(), "entry has empty Path");
            assert_eq!(e.size, -1, "entry {e:?} should have Size=-1 (pending)");
            assert!(e.is_dir, "entry {e:?} should be a directory");
        }
    }

    #[test]
    fn insight_icon_mapping() {
        // Top-level directory entries → folder icon; everything else → eyes icon.
        let folder_icon = "\u{1F4C1}";
        let eyes_icon = "\u{1F440}";
        let cases: &[(&str, &str)] = &[
            ("Home", folder_icon),
            ("User Library", folder_icon),
            ("App Library", folder_icon),
            ("Applications", folder_icon),
            ("System Library", folder_icon),
            ("iOS Backups", eyes_icon),
            ("Old Downloads (90d+)", eyes_icon),
            ("Homebrew Cache", eyes_icon),
            ("System Logs", eyes_icon),
            ("Docker Data", eyes_icon),
            ("OrbStack Data", eyes_icon),
        ];
        for (name, want) in cases {
            let got = insight_icon(&de(name));
            assert_eq!(got, *want, "insight_icon({name:?})");
        }
    }

    #[test]
    fn insight_icon_non_top_level_entries_get_eyes() {
        // All insight entries get the eyes icon.
        for name in [
            "iOS Backups",
            "Old Downloads (90d+)",
            "Homebrew Cache",
            "pip Cache",
            "CocoaPods Cache",
            "Gradle Cache",
            "Spotify Cache",
            "JetBrains Cache",
            "System Logs",
            "Xcode DerivedData",
            "Xcode Archives",
            "Xcode Simulators",
            "Docker Data",
        ] {
            assert_eq!(
                insight_icon(&de(name)),
                "\u{1F440}",
                "eyes icon for {name:?}"
            );
        }
    }

    #[test]
    fn measure_old_downloads_counts_stale_files() {
        let dir = TempDir::new().expect("tempdir");
        let dir_path = dir.path().to_string_lossy().to_string();

        // old file: mtime 100 days ago
        let old_path = dir.path().join("old.txt");
        std::fs::write(&old_path, b"old content here").expect("write old");
        set_mtime_days_ago(&old_path, 100);

        // new file: current mtime
        let new_path = dir.path().join("new.txt");
        std::fs::write(&new_path, b"new content").expect("write new");

        let size = measure_old_downloads(&dir_path, 90).expect("measure_old_downloads");
        assert!(size > 0, "expected non-zero size for old files, got {size}");
        // "old content here" = 16 bytes, should be well under 1 KiB
        assert!(
            size <= 1024,
            "size {size} seems too large for a 16-byte file"
        );
    }

    #[test]
    fn measure_old_downloads_skips_hidden() {
        let dir = TempDir::new().expect("tempdir");
        let dir_path = dir.path().to_string_lossy().to_string();

        // hidden file, old — should be skipped
        let hidden = dir.path().join(".DS_Store");
        std::fs::write(&hidden, b"hidden").expect("write hidden");
        set_mtime_days_ago(&hidden, 100);

        let size = measure_old_downloads(&dir_path, 90).expect("measure_old_downloads");
        assert_eq!(size, 0, "hidden files should be skipped");
    }

    #[test]
    fn measure_old_downloads_new_file_not_counted() {
        let dir = TempDir::new().expect("tempdir");
        let dir_path = dir.path().to_string_lossy().to_string();

        // only a new file — nothing should be counted
        let new_path = dir.path().join("recent.txt");
        std::fs::write(&new_path, b"recent").expect("write recent");

        let size = measure_old_downloads(&dir_path, 90).expect("measure_old_downloads");
        assert_eq!(size, 0, "file newer than 90 days should not be counted");
    }

    #[test]
    fn measure_insight_size_falls_back_to_overview() {
        // Any non-Downloads path → scanner::measure_overview_size
        let dir = TempDir::new().expect("tempdir");
        let dir_path = dir.path().to_string_lossy().to_string();
        std::fs::write(dir.path().join("test.dat"), &[0u8; 4096]).expect("write test");

        let size = measure_insight_size(&dir_path).expect("measure_insight_size");
        assert!(
            size > 0,
            "expected non-zero size for non-Downloads path, got {size}"
        );
    }
}
