use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicI64, Ordering}, Mutex, OnceLock};

use crate::safety::normalize_path;

/// In-memory Virtual File System representing files and directory hierarchy.
#[derive(Debug, Default, Clone)]
pub struct InMemoryVfs {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
}

impl InMemoryVfs {
    pub fn new() -> Self {
        let mut vfs = Self {
            files: BTreeMap::new(),
            dirs: BTreeSet::new(),
        };
        vfs.dirs.insert(PathBuf::from("/"));
        vfs.dirs.insert(PathBuf::from("."));
        vfs
    }

    /// Write file data at the given path, creating necessary ancestor directories.
    pub fn write(&mut self, path: &Path, data: impl AsRef<[u8]>) -> Result<(), String> {
        let norm = normalize_path(path);
        if let Some(parent) = norm.parent() {
            self.create_dir_all(parent)?;
        }
        self.files.insert(norm, data.as_ref().to_vec());
        Ok(())
    }

    /// Read raw file bytes.
    pub fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        let norm = normalize_path(path);
        self.files
            .get(&norm)
            .cloned()
            .ok_or_else(|| format!("File not found: '{}'", norm.display()))
    }

    /// Read file as UTF-8 string.
    pub fn read_to_string(&self, path: &Path) -> Result<String, String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| format!("File '{}' is not valid UTF-8: {e}", path.display()))
    }

    /// Create all directories along the path.
    pub fn create_dir_all(&mut self, path: &Path) -> Result<(), String> {
        let norm = normalize_path(path);
        let mut curr = PathBuf::new();
        for component in norm.components() {
            curr.push(component);
            self.dirs.insert(curr.clone());
        }
        self.dirs.insert(norm);
        Ok(())
    }

    /// Check if path exists as a file or directory.
    pub fn exists(&self, path: &Path) -> bool {
        let norm = normalize_path(path);
        self.files.contains_key(&norm) || self.dirs.contains(&norm)
    }

    /// Check if path is a regular file.
    pub fn is_file(&self, path: &Path) -> bool {
        let norm = normalize_path(path);
        self.files.contains_key(&norm)
    }

    /// Check if path is a directory.
    pub fn is_dir(&self, path: &Path) -> bool {
        let norm = normalize_path(path);
        self.dirs.contains(&norm)
    }

    /// Remove a file.
    pub fn remove_file(&mut self, path: &Path) -> Result<(), String> {
        let norm = normalize_path(path);
        if self.files.remove(&norm).is_some() {
            Ok(())
        } else {
            Err(format!("File not found to remove: '{}'", norm.display()))
        }
    }

    /// Remove a directory and all nested files and subdirectories.
    pub fn remove_dir_all(&mut self, path: &Path) -> Result<(), String> {
        let norm = normalize_path(path);
        let prefix = norm.clone();

        self.files.retain(|p, _| !p.starts_with(&prefix));
        self.dirs.retain(|p| !p.starts_with(&prefix));
        Ok(())
    }

    /// Read entries within a directory (returns child paths).
    pub fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, String> {
        let norm = normalize_path(path);
        if !self.is_dir(&norm) {
            return Err(format!("Directory not found: '{}'", norm.display()));
        }

        let mut entries = BTreeSet::new();

        // Check child files
        for f in self.files.keys() {
            if let Some(parent) = f.parent() {
                if parent == norm && f != &norm {
                    entries.insert(f.clone());
                }
            }
        }

        // Check child directories
        for d in &self.dirs {
            if let Some(parent) = d.parent() {
                if parent == norm && d != &norm {
                    entries.insert(d.clone());
                }
            }
        }

        Ok(entries.into_iter().collect())
    }

    /// Recursively list all files under the given directory.
    pub fn walk_dir(&self, path: &Path) -> Vec<PathBuf> {
        let norm = normalize_path(path);
        self.files
            .keys()
            .filter(|p| p.starts_with(&norm) && *p != &norm)
            .cloned()
            .collect()
    }

    /// Clear all files and directories.
    pub fn clear(&mut self) {
        self.files.clear();
        self.dirs.clear();
        self.dirs.insert(PathBuf::from("/"));
        self.dirs.insert(PathBuf::from("."));
    }
}

static GLOBAL_VFS: OnceLock<Mutex<InMemoryVfs>> = OnceLock::new();

fn get_vfs() -> &'static Mutex<InMemoryVfs> {
    GLOBAL_VFS.get_or_init(|| Mutex::new(InMemoryVfs::new()))
}

/// Global VFS write function
pub fn write(path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> Result<(), String> {
    let mut vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.write(path.as_ref(), data)
}

/// Global VFS read function
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, String> {
    let vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.read(path.as_ref())
}

/// Global VFS read_to_string function
pub fn read_to_string(path: impl AsRef<Path>) -> Result<String, String> {
    let vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.read_to_string(path.as_ref())
}

/// Global VFS create_dir_all function
pub fn create_dir_all(path: impl AsRef<Path>) -> Result<(), String> {
    let mut vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.create_dir_all(path.as_ref())
}

/// Global VFS exists function
pub fn exists(path: impl AsRef<Path>) -> bool {
    if let Ok(vfs) = get_vfs().lock() {
        vfs.exists(path.as_ref())
    } else {
        false
    }
}

/// Global VFS is_file function
pub fn is_file(path: impl AsRef<Path>) -> bool {
    if let Ok(vfs) = get_vfs().lock() {
        vfs.is_file(path.as_ref())
    } else {
        false
    }
}

/// Global VFS is_dir function
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    if let Ok(vfs) = get_vfs().lock() {
        vfs.is_dir(path.as_ref())
    } else {
        false
    }
}

/// Global VFS remove_file function
pub fn remove_file(path: impl AsRef<Path>) -> Result<(), String> {
    let mut vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.remove_file(path.as_ref())
}

/// Global VFS remove_dir_all function
pub fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), String> {
    let mut vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.remove_dir_all(path.as_ref())
}

/// Global VFS read_dir function
pub fn read_dir(path: impl AsRef<Path>) -> Result<Vec<PathBuf>, String> {
    let vfs = get_vfs().lock().map_err(|e| format!("VFS lock error: {e}"))?;
    vfs.read_dir(path.as_ref())
}

/// Global VFS walk_dir function
pub fn walk_dir(path: impl AsRef<Path>) -> Vec<PathBuf> {
    if let Ok(vfs) = get_vfs().lock() {
        vfs.walk_dir(path.as_ref())
    } else {
        Vec::new()
    }
}

/// Clear global VFS storage (used in tests)
pub fn clear() {
    if let Ok(mut vfs) = get_vfs().lock() {
        vfs.clear();
    }
}

// ---------------------------------------------------------------------------
// Pure Timestamp & Date Generator (no WASI clock syscalls)
// ---------------------------------------------------------------------------

static TIMESTAMP_COUNTER: AtomicI64 = AtomicI64::new(1700000000);

/// Returns a monotonic timestamp suitable for commit and tag metadata without WASI clocks.
pub fn current_timestamp() -> i64 {
    TIMESTAMP_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Format a unix timestamp into an ISO 8601 / RFC 3339 UTC date string.
pub fn format_timestamp(timestamp: i64) -> String {
    // Days since unix epoch
    let seconds_in_day = 86400i64;
    let mut remaining_secs = timestamp % seconds_in_day;
    let mut days = timestamp / seconds_in_day;

    if remaining_secs < 0 {
        remaining_secs += seconds_in_day;
        days -= 1;
    }

    let hours = remaining_secs / 3600;
    let mins = (remaining_secs % 3600) / 60;
    let secs = remaining_secs % 60;

    // Convert days to Y-M-D (proleptic Gregorian algorithm)
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hours, mins, secs)
}
