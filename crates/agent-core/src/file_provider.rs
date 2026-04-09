//! FileProvider trait: abstracts file read/write for agents.
//!
//! Implementations:
//! - DiskFileProvider: reads/writes from filesystem (current behavior)
//! - VirtFileProvider: reads/writes from virt-git VirtWorkspace (zero-disk)

use std::path::{Path, PathBuf};

/// Abstraction over file operations. Agents use this instead of std::fs.
pub trait FileProvider: Send + Sync {
    fn read_file(&self, path: &str) -> Result<String, String>;
    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String>;
    fn file_exists(&self, path: &str) -> bool;
    fn list_files(&self) -> Vec<String>;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Filesystem-backed file provider (current behavior).
pub struct DiskFileProvider {
    root: PathBuf,
}

impl DiskFileProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl FileProvider for DiskFileProvider {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn read_file(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(self.root.join(path))
            .map_err(|e| format!("read {path}: {e}"))
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        std::fs::write(&full, content).map_err(|e| format!("write {path}: {e}"))
    }

    fn file_exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }

    fn list_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        walk(&self.root, &self.root, &mut files);
        files.sort();
        files
    }
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" { continue; }
            walk(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().to_string());
        }
    }
}

/// VirtWorkspace-backed file provider (zero-disk).
pub struct VirtFileProvider {
    pub workspace: virt_git::VirtWorkspace,
    pub store: virt_git::MemoryBlobStore,
}

impl VirtFileProvider {
    pub fn new() -> Self {
        Self {
            workspace: virt_git::VirtWorkspace::new(),
            store: virt_git::MemoryBlobStore::new(),
        }
    }

    /// Load a file into the workspace (sets both base and working tree).
    pub fn load_file(&mut self, path: &str, content: &str) {
        self.workspace.load_file(&mut self.store, path, content);
    }

    /// Check if there are uncommitted changes.
    pub fn has_changes(&self) -> bool {
        self.workspace.has_changes()
    }

    /// Get diff text.
    pub fn diff_text(&self) -> String {
        self.workspace.diff_text(&self.store)
    }

    /// Get modified file contents (path → new content).
    pub fn modified_files(&self) -> Vec<(String, String)> {
        let diffs = self.workspace.diff(&self.store);
        diffs.iter().filter_map(|d| {
            if d.kind == virt_git::DiffKind::Deleted { return None; }
            let content = self.workspace.read_file(&self.store, &d.path)?;
            Some((d.path.clone(), content))
        }).collect()
    }
}

impl Default for VirtFileProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FileProvider for VirtFileProvider {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn read_file(&self, path: &str) -> Result<String, String> {
        self.workspace.read_file(&self.store, path)
            .ok_or_else(|| format!("file not found: {path}"))
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        self.workspace.write_file(&mut self.store, path, content);
        Ok(())
    }

    fn file_exists(&self, path: &str) -> bool {
        self.workspace.file_exists(path)
    }

    fn list_files(&self) -> Vec<String> {
        self.workspace.list_files().iter().map(|s| s.to_string()).collect()
    }
}
