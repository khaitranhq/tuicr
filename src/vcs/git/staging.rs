use git2::Repository;
use std::path::Path;

use crate::error::{Result, TuicrError};

pub fn stage_file(repo: &Repository, path: &Path) -> Result<()> {
    let mut index = repo.index()?;
    index.add_path(path)?;
    index.write()?;
    Ok(())
}

pub fn unstage_file(repo: &Repository, path: &Path) -> Result<()> {
    let mut index = repo.index()?;
    index.remove_path(path)?;
    // If file is tracked (exists in HEAD tree), restore the HEAD version
    // to the index so it matches `git restore --staged -- path`.
    // For untracked files or unborn HEAD, remove_path alone correctly
    // removes them from the index.
    if let Ok(head) = repo.head() {
        if let Ok(head_tree) = head.peel_to_tree() {
            if let Ok(tree_entry) = head_tree.get_path(path) {
                let blob = repo.find_blob(tree_entry.id())?;
                let path_str = path.to_str().ok_or_else(|| {
                    TuicrError::VcsCommand(format!("Non-UTF-8 path: {}", path.display()))
                })?;
                let entry = git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: tree_entry.filemode() as u32,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id: tree_entry.id(),
                    flags: 0,
                    flags_extended: 0,
                    path: path_str.as_bytes().to_vec(),
                };
                index.add_frombuffer(&entry, blob.content())?;
            }
        }
    }
    index.write()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stage_file_adds_to_index() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello\n").unwrap();

        stage_file(&repo, Path::new("test.txt")).unwrap();

        let index = repo.index().unwrap();
        assert!(index.get_path(Path::new("test.txt"), 0).is_some());
    }

    #[test]
    fn unstage_file_removes_from_index_for_new_file() {
        // For an untracked file that was staged, unstage removes it entirely
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let file_path = temp_dir.path().join("new.txt");
        fs::write(&file_path, "content\n").unwrap();

        stage_file(&repo, Path::new("new.txt")).unwrap();
        let index = repo.index().unwrap();
        assert!(index.get_path(Path::new("new.txt"), 0).is_some());
        drop(index);

        unstage_file(&repo, Path::new("new.txt")).unwrap();

        let index = repo.index().unwrap();
        assert!(index.get_path(Path::new("new.txt"), 0).is_none());
    }

    #[test]
    fn unstage_file_resets_tracked_file_to_head() {
        // For a tracked file with staged changes, unstage restores HEAD content
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        let file_path = temp_dir.path().join("tracked.txt");
        fs::write(&file_path, "original\n").unwrap();

        // Set up git config for committing
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "test").unwrap();
        cfg.set_str("user.email", "test@test").unwrap();
        drop(cfg);

        // First commit the file
        let sig = repo.signature().unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        drop(index);

        // Store the original blob OID for later comparison
        let head_tree = repo.head().unwrap().peel_to_tree().unwrap();
        let head_blob_id = head_tree.get_path(Path::new("tracked.txt")).unwrap().id();

        // Modify and stage the new version
        fs::write(&file_path, "modified\n").unwrap();
        stage_file(&repo, Path::new("tracked.txt")).unwrap();
        let index = repo.index().unwrap();
        let staged_entry = index.get_path(Path::new("tracked.txt"), 0).unwrap();
        assert_ne!(staged_entry.id, head_blob_id);
        drop(index);

        // Unstage — should reset to HEAD content (original blob)
        unstage_file(&repo, Path::new("tracked.txt")).unwrap();

        let index = repo.index().unwrap();
        let entry = index.get_path(Path::new("tracked.txt"), 0).unwrap();
        assert_eq!(
            entry.id, head_blob_id,
            "index entry blob id should match HEAD blob"
        );
        // Working tree content unchanged
        let wt_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(wt_content, "modified\n");
    }
}
