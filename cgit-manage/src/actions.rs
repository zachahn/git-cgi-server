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
/// cgit with `scan-path` lists the repo on its next request. The description
/// and owner files give cgit the text it shows on the index and summary.
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

    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(&path)
        .output()
        .map_err(|e| ActionError::Git(format!("could not run git: {e}")))?;

    if !output.status.success() {
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
pub fn delete(root: &Path, name: &str, actor: Option<&str>) -> Result<(), ActionError> {
    let path = repo::repo_path(root, name)?;
    if !path.exists() {
        return Err(ActionError::Existence(format!("repo {name:?} does not exist")));
    }

    std::fs::remove_dir_all(&path).map_err(ActionError::Io)?;
    log("delete", name, actor);
    Ok(())
}

/// Move a repo directory from one validated name to another.
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

    std::fs::rename(&from, &to).map_err(ActionError::Io)?;
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
