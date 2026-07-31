//! The four repo operations.
//!
//! Each operation confines its work to `GIT_ROOT` through `repo::repo_path`.
//! Create runs `git` with an argument list, so no shell parses the name.
//! The other three call the standard library filesystem functions, so no
//! shell runs at all. Every operation logs its outcome to standard error,
//! which the web server captures.

use crate::repo::{self, PathError};
use std::path::Path;
use std::process::Command;

/// Why an operation failed.
#[derive(Debug)]
pub enum ActionError {
    /// The name or path check refused the request.
    Path(PathError),
    /// A repo that had to exist did not, or one that had to be new existed.
    Existence(String),
    /// A filesystem call failed.
    Io(std::io::Error),
    /// `git` exited non-zero or did not run.
    Git(String),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::Path(e) => write!(f, "{e}"),
            ActionError::Existence(m) => write!(f, "{m}"),
            ActionError::Io(e) => write!(f, "filesystem error: {e}"),
            ActionError::Git(m) => write!(f, "git error: {m}"),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<PathError> for ActionError {
    fn from(e: PathError) -> Self {
        ActionError::Path(e)
    }
}

/// Create a bare repo, then seed its description and owner.
///
/// A name with slashes puts the repo in a subfolder, so the function makes
/// any missing folder on the way down. cgit with `scan-path` scans folders
/// recursively, so it lists the repo under that path on its next request.
/// The description and owner files give cgit the text it shows on the index
/// and summary.
pub fn create(
    root: &Path,
    name: &str,
    description: &str,
    owner: Option<&str>,
) -> Result<(), ActionError> {
    let path = repo::repo_path(root, name)?;
    if path.exists() {
        return Err(ActionError::Existence(format!("repo {name:?} already exists")));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ActionError::Io)?;
    }

    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(&path)
        .output()
        .map_err(|e| {
            prune_empty_folders(root, &path);
            ActionError::Git(format!("could not run git: {e}"))
        })?;

    if !output.status.success() {
        prune_empty_folders(root, &path);
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(ActionError::Git(msg.trim().to_string()));
    }

    if !description.is_empty() {
        write_description(root, name, description)?;
    }
    if let Some(owner) = owner {
        std::fs::write(path.join("owner"), format!("{owner}\n")).map_err(ActionError::Io)?;
    }

    log("create", name, owner);
    Ok(())
}

/// Remove a repo directory and everything under it.
///
/// Deleting the last repo in a subfolder leaves the folder behind, and cgit
/// shows an empty folder as nothing at all. The function removes each folder
/// the repo leaves empty, up to but not including the root.
pub fn delete(root: &Path, name: &str, actor: Option<&str>) -> Result<(), ActionError> {
    let path = repo::repo_path(root, name)?;
    if !path.exists() {
        return Err(ActionError::Existence(format!("repo {name:?} does not exist")));
    }

    std::fs::remove_dir_all(&path).map_err(ActionError::Io)?;
    prune_empty_folders(root, &path);
    log("delete", name, actor);
    Ok(())
}

/// Move a repo directory from one validated name to another.
///
/// The new name may name a different subfolder, so the move doubles as a way
/// to file a repo away. The function makes the folders the new name needs and
/// removes the ones the old name leaves empty.
pub fn rename(
    root: &Path,
    old: &str,
    new: &str,
    actor: Option<&str>,
) -> Result<(), ActionError> {
    let from = repo::repo_path(root, old)?;
    let to = repo::repo_path(root, new)?;

    if !from.exists() {
        return Err(ActionError::Existence(format!("repo {old:?} does not exist")));
    }
    if to.exists() {
        return Err(ActionError::Existence(format!("repo {new:?} already exists")));
    }

    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(ActionError::Io)?;
    }

    std::fs::rename(&from, &to).map_err(|e| {
        prune_empty_folders(root, &to);
        ActionError::Io(e)
    })?;
    prune_empty_folders(root, &from);
    log("rename", &format!("{old} -> {new}"), actor);
    Ok(())
}

/// Write a repo's `description` file, the text cgit shows.
pub fn set_description(
    root: &Path,
    name: &str,
    description: &str,
    actor: Option<&str>,
) -> Result<(), ActionError> {
    let path = repo::repo_path(root, name)?;
    if !path.exists() {
        return Err(ActionError::Existence(format!("repo {name:?} does not exist")));
    }
    write_description(root, name, description)?;
    log("set-description", name, actor);
    Ok(())
}

/// Write the `description` file with a trailing newline, as git expects.
fn write_description(root: &Path, name: &str, description: &str) -> Result<(), ActionError> {
    let path = repo::repo_path(root, name)?;
    let mut text = description.trim_end().to_string();
    text.push('\n');
    std::fs::write(path.join("description"), text).map_err(ActionError::Io)?;
    Ok(())
}

/// Remove the folders above `path` that now hold nothing.
///
/// The walk climbs from the repo's parent toward the root and stops at the
/// first folder that still holds something, because `remove_dir` refuses a
/// non-empty directory. It never removes the root itself and never touches a
/// path outside it. A failure leaves an empty folder behind, which costs
/// nothing, so the function reports no error.
fn prune_empty_folders(root: &Path, path: &Path) {
    let canonical_root = match root.canonicalize() {
        Ok(r) => r,
        Err(_) => return,
    };

    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == canonical_root || !dir.starts_with(&canonical_root) {
            return;
        }
        if std::fs::remove_dir(dir).is_err() {
            return;
        }
        current = dir.parent();
    }
}

/// Record one mutating action on standard error for the web server log.
fn log(action: &str, target: &str, actor: Option<&str>) {
    let who = actor.unwrap_or("-");
    eprintln!("cgit-manage: user={who} action={action} target={target:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmpdir::TempDir;

    /// `create` runs `git init`, so these tests need git on PATH.
    fn root(label: &str) -> TempDir {
        TempDir::new(label)
    }

    #[test]
    fn create_makes_a_bare_repo() {
        let d = root("create");
        create(d.path(), "demo", "", None).unwrap();

        assert!(d.exists("demo.git"), "the repo directory exists");
        assert!(d.exists("demo.git/HEAD"), "git initialized it");
        assert!(d.exists("demo.git/objects"), "it has an object store");
        assert!(!d.exists("demo.git/.git"), "it is bare, with no working tree");
    }

    #[test]
    fn create_writes_the_description_and_the_owner() {
        let d = root("create-meta");
        create(d.path(), "demo", "A test repo", Some("alice")).unwrap();

        assert_eq!(d.read("demo.git/description"), "A test repo\n");
        assert_eq!(d.read("demo.git/owner"), "alice\n");
    }

    #[test]
    fn create_without_an_owner_writes_no_owner_file() {
        let d = root("create-anon");
        create(d.path(), "demo", "", None).unwrap();

        assert!(!d.exists("demo.git/owner"));
    }

    #[test]
    fn create_makes_the_folders_a_nested_name_needs() {
        let d = root("create-nested");
        create(d.path(), "work/team/site", "", None).unwrap();

        assert!(d.exists("work/team/site.git/HEAD"), "the repo sits in the folders");
    }

    #[test]
    fn create_puts_a_second_repo_in_an_existing_folder() {
        let d = root("create-nested-sibling");
        create(d.path(), "work/one", "", None).unwrap();
        create(d.path(), "work/two", "", None).unwrap();

        assert!(d.exists("work/one.git/HEAD"));
        assert!(d.exists("work/two.git/HEAD"));
    }

    #[test]
    fn create_refuses_a_name_already_taken() {
        let d = root("create-dup");
        create(d.path(), "demo", "", None).unwrap();

        let err = create(d.path(), "demo", "", None).unwrap_err();
        assert!(matches!(err, ActionError::Existence(_)), "got {err:?}");
    }

    #[test]
    fn create_refuses_a_traversing_name_and_writes_nothing() {
        let d = root("create-traverse");
        let outside = d.path().parent().unwrap().join("escape.git");

        let err = create(d.path(), "../escape", "", None).unwrap_err();

        assert!(matches!(err, ActionError::Path(_)), "got {err:?}");
        assert!(!outside.exists(), "nothing was written outside the root");
    }

    #[test]
    fn create_refuses_a_root_that_does_not_exist() {
        let d = root("create-noroot");
        let missing = d.join("no-such-dir");

        let err = create(&missing, "demo", "", None).unwrap_err();
        assert!(
            matches!(err, ActionError::Path(PathError::RootMissing(_))),
            "got {err:?}"
        );
    }

    #[test]
    fn delete_removes_the_repo_and_its_contents() {
        let d = root("delete");
        create(d.path(), "demo", "", None).unwrap();
        d.write("demo.git/marker", "x");

        delete(d.path(), "demo", None).unwrap();

        assert!(!d.exists("demo.git"));
    }

    #[test]
    fn delete_removes_the_folders_it_empties() {
        let d = root("delete-nested");
        create(d.path(), "work/team/site", "", None).unwrap();

        delete(d.path(), "work/team/site", None).unwrap();

        assert!(!d.exists("work"), "the empty folders went with the repo");
    }

    #[test]
    fn delete_keeps_a_folder_that_still_holds_a_repo() {
        let d = root("delete-nested-sibling");
        create(d.path(), "work/one", "", None).unwrap();
        create(d.path(), "work/two", "", None).unwrap();

        delete(d.path(), "work/one", None).unwrap();

        assert!(d.exists("work/two.git/HEAD"), "the sibling survives");
    }

    #[test]
    fn delete_refuses_a_missing_repo() {
        let d = root("delete-missing");
        let err = delete(d.path(), "demo", None).unwrap_err();
        assert!(matches!(err, ActionError::Existence(_)), "got {err:?}");
    }

    #[test]
    fn delete_refuses_a_traversing_name() {
        let d = root("delete-traverse");
        let err = delete(d.path(), "../..", None).unwrap_err();
        assert!(matches!(err, ActionError::Path(_)), "got {err:?}");
    }

    #[test]
    fn rename_moves_the_repo_and_keeps_its_contents() {
        let d = root("rename");
        create(d.path(), "demo", "A test repo", None).unwrap();
        d.write("demo.git/marker", "still here");

        rename(d.path(), "demo", "archive", None).unwrap();

        assert!(!d.exists("demo.git"), "the old name is gone");
        assert_eq!(d.read("archive.git/marker"), "still here");
        assert_eq!(d.read("archive.git/description"), "A test repo\n");
    }

    #[test]
    fn rename_moves_a_repo_into_a_folder() {
        let d = root("rename-into-folder");
        create(d.path(), "demo", "", None).unwrap();
        d.write("demo.git/marker", "still here");

        rename(d.path(), "demo", "archive/2024/demo", None).unwrap();

        assert!(!d.exists("demo.git"), "the old name is gone");
        assert_eq!(d.read("archive/2024/demo.git/marker"), "still here");
    }

    #[test]
    fn rename_moves_a_repo_out_of_a_folder_and_prunes_it() {
        let d = root("rename-out-of-folder");
        create(d.path(), "work/site", "", None).unwrap();

        rename(d.path(), "work/site", "site", None).unwrap();

        assert!(d.exists("site.git/HEAD"));
        assert!(!d.exists("work"), "the emptied folder went with the move");
    }

    #[test]
    fn rename_refuses_a_target_that_exists() {
        let d = root("rename-taken");
        create(d.path(), "demo", "", None).unwrap();
        create(d.path(), "archive", "", None).unwrap();

        let err = rename(d.path(), "demo", "archive", None).unwrap_err();

        assert!(matches!(err, ActionError::Existence(_)), "got {err:?}");
        assert!(d.exists("demo.git"), "the source survives a refused rename");
    }

    #[test]
    fn rename_refuses_a_missing_source() {
        let d = root("rename-missing");
        let err = rename(d.path(), "demo", "archive", None).unwrap_err();
        assert!(matches!(err, ActionError::Existence(_)), "got {err:?}");
    }

    #[test]
    fn rename_refuses_an_invalid_new_name() {
        let d = root("rename-invalid");
        create(d.path(), "demo", "", None).unwrap();

        let err = rename(d.path(), "demo", "../escape", None).unwrap_err();

        assert!(matches!(err, ActionError::Path(_)), "got {err:?}");
        assert!(d.exists("demo.git"), "the source survives a refused rename");
    }

    #[test]
    fn set_description_overwrites_the_previous_text() {
        let d = root("describe");
        create(d.path(), "demo", "first", None).unwrap();

        set_description(d.path(), "demo", "second", None).unwrap();

        assert_eq!(d.read("demo.git/description"), "second\n");
    }

    #[test]
    fn set_description_ends_the_file_with_one_newline() {
        let d = root("describe-newline");
        create(d.path(), "demo", "", None).unwrap();

        set_description(d.path(), "demo", "text with trailing space   \n\n", None).unwrap();

        assert_eq!(d.read("demo.git/description"), "text with trailing space\n");
    }

    #[test]
    fn set_description_keeps_inner_spacing() {
        let d = root("describe-inner");
        create(d.path(), "demo", "", None).unwrap();

        set_description(d.path(), "demo", "two  spaces inside", None).unwrap();

        assert_eq!(d.read("demo.git/description"), "two  spaces inside\n");
    }

    #[test]
    fn set_description_refuses_a_missing_repo() {
        let d = root("describe-missing");
        let err = set_description(d.path(), "demo", "text", None).unwrap_err();
        assert!(matches!(err, ActionError::Existence(_)), "got {err:?}");
    }
}
