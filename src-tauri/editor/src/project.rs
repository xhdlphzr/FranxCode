// Copyright (C) 2026 xhdlphzr

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.

// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Project management: file trees, opening/saving files, and Git integration.

use crate::edit::{Range, TextEdit};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A node in the file tree (file or directory).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
    pub size: Option<u64>,
    pub extension: Option<String>,
}

impl FileNode {
    /// Creates a directory node.
    ///
    /// # Arguments
    /// * `name` - The directory name.
    /// * `path` - The full filesystem path.
    ///
    /// # Returns
    /// A `FileNode` representing a directory.
    pub fn dir(name: &str, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            path,
            is_dir: true,
            children: vec![],
            size: None,
            extension: None,
        }
    }

    /// Creates a file node.
    ///
    /// # Arguments
    /// * `name` - The file name.
    /// * `path` - The full filesystem path.
    ///
    /// # Returns
    /// A `FileNode` representing a file.
    pub fn file(name: &str, path: PathBuf) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.into());
        Self {
            name: name.into(),
            path,
            is_dir: false,
            children: vec![],
            size: None,
            extension: ext,
        }
    }

    /// Adds a child node.
    ///
    /// # Arguments
    /// * `c` - The child node to add.
    pub fn add_child(&mut self, c: FileNode) {
        self.children.push(c);
    }

    /// Sorts children recursively: directories first, then files, alphabetically (case-insensitive).
    pub fn sort_children(&mut self) {
        self.children.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        for c in &mut self.children {
            c.sort_children();
        }
    }
}

/// A Git status entry for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEntry {
    pub path: String,
    pub status: String,
    pub old_path: Option<String>,
}

/// Git status for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub entries: Vec<StatusEntry>,
    pub is_clean: bool,
}

/// Manages project state: current file, dirty flag, file operations, Git commands.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectManager {
    pub current_file: Option<PathBuf>,
    pub is_dirty: bool,
}

impl ProjectManager {
    /// Creates a new empty project manager.
    ///
    /// # Returns
    /// A new `ProjectManager` instance.
    pub fn new() -> Self {
        Self {
            current_file: None,
            is_dirty: false,
        }
    }

    /// Opens a file and returns its content as a `TextEdit`.
    ///
    /// # Arguments
    /// * `path` - The file path to open.
    ///
    /// # Returns
    /// A `TextEdit` containing the file content, or an error.
    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<TextEdit> {
        let c = std::fs::read_to_string(path)?;
        self.current_file = Some(path.to_path_buf());
        self.is_dirty = false;
        Ok(TextEdit::from_str(&c))
    }

    /// Opens a file lossily (replaces invalid UTF‑8 sequences).
    ///
    /// # Arguments
    /// * `path` - The file path to open.
    ///
    /// # Returns
    /// A `TextEdit` containing the file content (lossy conversion), or an error.
    pub fn open_file_lossy(&mut self, path: &Path) -> anyhow::Result<TextEdit> {
        let b = std::fs::read(path)?;
        let c = String::from_utf8(b.clone())
            .unwrap_or_else(|_| String::from_utf8_lossy(&b).into_owned());
        self.current_file = Some(path.to_path_buf());
        self.is_dirty = false;
        Ok(TextEdit::from_str(&c))
    }

    /// Saves the current document to the currently open file.
    ///
    /// # Arguments
    /// * `t` - The `TextEdit` to save.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if no file is open.
    pub fn save_file(&mut self, t: &TextEdit) -> anyhow::Result<()> {
        let p = self
            .current_file
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No file open"))?;
        std::fs::write(p, t.get_text(&Range::new(0, t.len())))?;
        self.is_dirty = false;
        Ok(())
    }

    /// Saves the current document to a specified path (Save As).
    ///
    /// # Arguments
    /// * `t` - The `TextEdit` to save.
    /// * `p` - The target path.
    ///
    /// # Returns
    /// `Ok(())` on success.
    pub fn save_file_as(&mut self, t: &TextEdit, p: &Path) -> anyhow::Result<()> {
        std::fs::write(p, t.get_text(&Range::new(0, t.len())))?;
        self.current_file = Some(p.to_path_buf());
        self.is_dirty = false;
        Ok(())
    }

    /// Marks the current document as dirty (unsaved changes).
    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    /// Marks the current document as clean (saved).
    pub fn mark_clean(&mut self) {
        self.is_dirty = false;
    }

    /// Recursively builds a file tree for the given root directory (no filtering).
    ///
    /// # Arguments
    /// * `root` - The root directory path.
    ///
    /// # Returns
    /// A vector of top‑level `FileNode`s (the children of the root).
    pub fn get_project_tree(&self, root: &Path) -> anyhow::Result<Vec<FileNode>> {
        let mut rn = FileNode::dir(
            root.file_name().and_then(|n| n.to_str()).unwrap_or("root"),
            root.to_path_buf(),
        );

        // Walk the entire directory tree without any filtering
        for e in walkdir::WalkDir::new(root).max_depth(10).into_iter() {
            let e = e?;
            if e.path() == root {
                continue;
            }
            let r = e.path().strip_prefix(root)?;
            self.ins(
                &mut rn,
                r,
                e.path(),
                e.file_type().is_dir(),
                e.metadata().ok(),
            );
        }
        rn.sort_children();
        Ok(rn.children)
    }

    /// Internal helper to insert a node into the tree recursively.
    fn ins(
        &self,
        n: &mut FileNode,
        rel: &Path,
        full: &Path,
        is_dir: bool,
        meta: Option<std::fs::Metadata>,
    ) {
        let c: Vec<&str> = rel.iter().map(|c| c.to_str().unwrap_or("")).collect();
        if c.is_empty() {
            return;
        }
        let nm = c[0];
        if c.len() == 1 {
            if is_dir {
                n.add_child(FileNode::dir(nm, full.to_path_buf()));
            } else {
                let mut f = FileNode::file(nm, full.to_path_buf());
                if let Some(ref m) = meta {
                    f.size = Some(m.len());
                }
                n.add_child(f);
            }
        } else {
            let rem: PathBuf = c[1..].iter().collect();
            if let Some(ch) = n.children.iter_mut().find(|x| x.name == nm && x.is_dir) {
                self.ins(ch, &rem, full, is_dir, meta);
            } else {
                let mut d = FileNode::dir(nm, full.to_path_buf());
                self.ins(&mut d, &rem, full, is_dir, meta);
                n.add_child(d);
            }
        }
    }

    /// Retrieves Git status for the repository containing `repo_path`.
    ///
    /// # Arguments
    /// * `repo_path` - Path to the Git repository root (or any path inside it).
    ///
    /// # Returns
    /// A `GitStatus` structure, or an error if not a Git repository.
    pub fn git_status(&self, repo_path: &Path) -> anyhow::Result<GitStatus> {
        let repo = git2::Repository::open(repo_path)?;
        let branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(|s| s.to_string()))
            .unwrap_or_else(|| "HEAD".into());
        let mut entries = Vec::new();
        let mut is_clean = true;
        for e in repo.statuses(None)?.iter() {
            is_clean = false;
            let st = e.status();
            let status = if st.is_index_new() || st.is_wt_new() {
                "new"
            } else if st.is_index_deleted() || st.is_wt_deleted() {
                "deleted"
            } else if st.is_index_renamed() || st.is_wt_renamed() {
                "renamed"
            } else if st.is_conflicted() {
                "conflicted"
            } else if st.is_ignored() {
                "ignored"
            } else {
                "modified"
            };
            let old = if st.is_index_renamed() || st.is_wt_renamed() {
                e.head_to_index()
                    .and_then(|d| d.old_file().path().map(|s| s.display().to_string()))
            } else {
                None
            };
            entries.push(StatusEntry {
                path: e.path().map_or(String::new(), |s| s.to_string()),
                status: status.into(),
                old_path: old,
            });
        }
        Ok(GitStatus {
            branch,
            entries,
            is_clean,
        })
    }

    /// Creates a commit with specified files (or all changes if files list is empty).
    ///
    /// # Arguments
    /// * `repo_path` - Path to the Git repository.
    /// * `message` - The commit message.
    /// * `files` - A slice of file paths (relative to repository root) to commit.
    ///              If empty, stages all changes (equivalent to `git add -A`).
    ///
    /// # Returns
    /// `Ok(())` on success.
    pub fn git_commit(
        &self,
        repo_path: &Path,
        message: &str,
        files: &[&Path],
    ) -> anyhow::Result<()> {
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        if files.is_empty() {
            index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        } else {
            for path in files {
                index.add_path(path)?;
            }
        }
        index.write()?;
        let tid = index.write_tree()?;
        let tree = repo.find_tree(tid)?;
        let sig = git2::Signature::now("FranxCode", "franxcode@editor.local")?;
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
        Ok(())
    }

    /// Returns the diff of a specific file (working directory vs index).
    ///
    /// # Arguments
    /// * `repo_path` - Path to the Git repository.
    /// * `file_path` - The file path (relative to repository root).
    ///
    /// # Returns
    /// A string containing the unified diff.
    pub fn git_diff(&self, repo_path: &Path, file_path: &Path) -> anyhow::Result<String> {
        let repo = git2::Repository::open(repo_path)?;
        let mut opts = git2::DiffOptions::new();
        if let Some(s) = file_path.to_str() {
            opts.pathspec(s);
        }
        let diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
        let mut out = String::new();
        diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
            out.push(line.origin());
            out.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            true
        })?;
        Ok(out)
    }

    /// Returns the diff of all changes (working directory vs index).
    ///
    /// # Arguments
    /// * `repo_path` - Path to the Git repository.
    ///
    /// # Returns
    /// A string containing the unified diff for all changes.
    pub fn git_diff_all(&self, repo_path: &Path) -> anyhow::Result<String> {
        let repo = git2::Repository::open(repo_path)?;
        let mut opts = git2::DiffOptions::new();
        opts.include_untracked(true);
        let diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
        let mut out = String::new();
        diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
            out.push(line.origin());
            out.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            true
        })?;
        Ok(out)
    }

    /// Checks whether a path is inside a Git repository (ascends parents).
    ///
    /// # Arguments
    /// * `path` - The path to check.
    ///
    /// # Returns
    /// `true` if `.git` directory exists in `path` or any parent.
    pub fn is_git_repo(path: &Path) -> bool {
        let mut cur = Some(path.to_path_buf());
        while let Some(p) = cur {
            if p.join(".git").is_dir() {
                return true;
            }
            cur = p.parent().map(|p| p.to_path_buf());
        }
        false
    }

    /// Finds the Git repository root for the given path.
    ///
    /// # Arguments
    /// * `path` - The path to start from.
    ///
    /// # Returns
    /// `Some(PathBuf)` of the repository root, or `None` if not found.
    pub fn find_git_root(path: &Path) -> Option<PathBuf> {
        let mut cur = Some(path.to_path_buf());
        while let Some(p) = cur {
            if p.join(".git").is_dir() {
                return Some(p);
            }
            cur = p.parent().map(|p| p.to_path_buf());
        }
        None
    }
}

impl Default for ProjectManager {
    /// Returns a default project manager using `new()`.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_fn() {
        let f = FileNode::file("m.rs", PathBuf::from("/t/m.rs"));
        assert_eq!(f.name, "m.rs");
    }

    #[test]
    fn test_pm() {
        let pm = ProjectManager::new();
        assert!(pm.current_file.is_none());
    }

    #[test]
    fn test_open_save_file() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test.txt");
        let mut pm = ProjectManager::new();

        assert!(pm.open_file(&file_path).is_err());

        let original_content = "Hello, world!";
        let mut f = File::create(&file_path)?;
        f.write_all(original_content.as_bytes())?;
        f.sync_all()?;

        let text_edit = pm.open_file(&file_path)?;
        assert_eq!(text_edit.full_text(), original_content);
        assert_eq!(pm.current_file, Some(file_path.clone()));
        assert!(!pm.is_dirty);

        let new_content = "New content";
        let mut text_edit = text_edit;
        text_edit.replace(&Range::new(0, text_edit.len()), new_content);
        pm.mark_dirty();
        pm.save_file(&text_edit)?;
        assert!(!pm.is_dirty);
        let saved = fs::read_to_string(&file_path)?;
        assert_eq!(saved, new_content);

        let new_path = dir.path().join("new.txt");
        pm.save_file_as(&text_edit, &new_path)?;
        assert_eq!(pm.current_file, Some(new_path.clone()));
        let saved_as = fs::read_to_string(&new_path)?;
        assert_eq!(saved_as, new_content);

        Ok(())
    }

    #[test]
    fn test_get_project_tree() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir(root.join("subdir"))?;
        File::create(root.join("file1.txt"))?;
        File::create(root.join("subdir/file2.rs"))?;

        let pm = ProjectManager::new();
        let nodes = pm.get_project_tree(root)?;
        assert_eq!(nodes.len(), 2);
        let subdir = nodes.iter().find(|n| n.name == "subdir").unwrap();
        assert!(subdir.is_dir);
        assert_eq!(subdir.children.len(), 1);
        assert_eq!(subdir.children[0].name, "file2.rs");
        assert!(!subdir.children[0].is_dir);
        Ok(())
    }

    #[test]
    fn test_git_integration() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo_path = dir.path();
        let repo = git2::Repository::init(repo_path)?;
        let file_path = repo_path.join("test.txt");
        let mut f = File::create(&file_path)?;
        f.write_all(b"initial")?;
        f.sync_all()?;
        let mut index = repo.index()?;
        index.add_path(Path::new("test.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])?;

        let pm = ProjectManager::new();
        let status = pm.git_status(repo_path)?;
        assert!(status.is_clean);

        fs::write(&file_path, "modified")?;
        let status2 = pm.git_status(repo_path)?;
        assert!(!status2.is_clean);
        assert!(status2
            .entries
            .iter()
            .any(|e| e.path == "test.txt" && e.status == "modified"));

        pm.git_commit(repo_path, "second commit", &[Path::new("test.txt")])?;
        let status3 = pm.git_status(repo_path)?;
        assert!(status3.is_clean);

        fs::write(&file_path, "another change")?;
        let diff = pm.git_diff(repo_path, Path::new("test.txt"))?;
        assert!(!diff.is_empty());
        let diff_all = pm.git_diff_all(repo_path)?;
        assert!(!diff_all.is_empty());

        Ok(())
    }

    #[test]
    fn test_git_repo_detection() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let sub = root.join("sub");
        fs::create_dir(&sub)?;
        assert!(!ProjectManager::is_git_repo(root));
        git2::Repository::init(root)?;
        assert!(ProjectManager::is_git_repo(root));
        assert!(ProjectManager::is_git_repo(&sub));
        assert_eq!(
            ProjectManager::find_git_root(&sub),
            Some(root.to_path_buf())
        );
        Ok(())
    }
}
