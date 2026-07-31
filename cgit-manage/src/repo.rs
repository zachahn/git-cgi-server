//! Repo name validation and path confinement.
//!
//! A repo name arrives from an HTTP request, so an attacker controls it.
//! A name may carry slashes, so `work/site` names a repo in a subfolder.
//! Three independent checks guard every path this module builds.
//! First, `validate_name` splits the name on slashes and accepts each
//! component only when it passes a strict character allowlist. Second,
//! `repo_path` canonicalizes the root and asserts the built path still sits
//! under it. Third, it canonicalizes the deepest part of the path that exists
//! on disk, so a symlinked folder cannot point the tool out of the root.
//! Each check alone stops directory traversal. The three stop it three times.

use std::path::{Path, PathBuf};

/// The longest single path component the tool accepts.
const MAX_NAME_LEN: usize = 64;

/// The most components a name may carry, the repo itself included.
const MAX_DEPTH: usize = 8;

/// Why a repo name failed validation.
#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    EmptyComponent,
    TooLong,
    TooDeep,
    LeadingDot,
    GitSuffix,
    DotComponent,
    BadChar(char),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameError::Empty => write!(f, "name is empty"),
            NameError::EmptyComponent => write!(f, "name has an empty folder component"),
            NameError::TooLong => {
                write!(f, "a name component is longer than {MAX_NAME_LEN} characters")
            }
            NameError::TooDeep => write!(f, "name nests deeper than {MAX_DEPTH} folders"),
            NameError::LeadingDot => write!(f, "a name component starts with a dot"),
            NameError::GitSuffix => write!(f, "name ends with .git; the tool adds that itself"),
            NameError::DotComponent => write!(f, "a name component is a dot component"),
            NameError::BadChar(c) => write!(f, "name contains a disallowed character: {c:?}"),
        }
    }
}

impl std::error::Error for NameError {}

/// Accept a repo name only when every component passes the allowlist.
///
/// A name is one or more components joined by `/`, so `work/site` names the
/// repo `site` inside the folder `work`. The rules, in order:
/// - The name is not empty and holds at most `MAX_DEPTH` components.
/// - No component is empty, so no leading, trailing, or doubled slash passes.
/// - Every component stays within `MAX_NAME_LEN`.
/// - No component is `.` or `..`, and none starts with a dot.
/// - No component ends with `.git`; the caller appends that to the last one.
/// - Every character is ASCII alphanumeric, `.`, `_`, or `-`.
///
/// The allowlist admits only ASCII, so no Unicode look-alike slips through.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }

    let components: Vec<&str> = name.split('/').collect();
    if components.len() > MAX_DEPTH {
        return Err(NameError::TooDeep);
    }

    for component in components {
        validate_component(component)?;
    }
    Ok(())
}

/// Apply the allowlist to one path component.
fn validate_component(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::EmptyComponent);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong);
    }
    if name == "." || name == ".." {
        return Err(NameError::DotComponent);
    }
    if name.starts_with('.') {
        return Err(NameError::LeadingDot);
    }
    if name.ends_with(".git") {
        return Err(NameError::GitSuffix);
    }
    for c in name.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-';
        if !ok {
            return Err(NameError::BadChar(c));
        }
    }
    Ok(())
}

/// Why the tool refused to build a repo path.
#[derive(Debug)]
pub enum PathError {
    Name(NameError),
    /// The configured root does not exist or does not canonicalize.
    RootMissing(std::io::Error),
    /// The built path escaped the root.
    Escaped,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Name(e) => write!(f, "invalid name: {e}"),
            PathError::RootMissing(e) => write!(f, "git root is unusable: {e}"),
            PathError::Escaped => write!(f, "path escaped the git root"),
        }
    }
}

impl std::error::Error for PathError {}

/// Build the on-disk path for a repo, confined to `root`.
///
/// The function validates the name, canonicalizes `root`, then appends one
/// directory per name component and `.git` to the last. `work/site` becomes
/// `<root>/work/site.git`. It canonicalizes the root rather than the full
/// path because the repo directory and its folders may not exist yet, for
/// example on create. The allowlist forbids `.` and `..` in every component,
/// so no component can walk out of the canonical root. The function asserts
/// that fact and refuses on any mismatch.
pub fn repo_path(root: &Path, name: &str) -> Result<PathBuf, PathError> {
    validate_name(name).map_err(PathError::Name)?;

    let canonical_root = root.canonicalize().map_err(PathError::RootMissing)?;

    let components: Vec<&str> = name.split('/').collect();
    let mut candidate = canonical_root.clone();
    for (i, component) in components.iter().enumerate() {
        if i + 1 == components.len() {
            candidate.push(format!("{component}.git"));
        } else {
            candidate.push(component);
        }
    }

    // The allowlist guarantees ordinary path components. Confirm the built
    // path still lies under the canonical root and adds exactly the depth the
    // name asked for, so a future change to the allowlist cannot silently
    // open a traversal.
    let depth = candidate
        .strip_prefix(&canonical_root)
        .map(|rest| rest.components().count())
        .unwrap_or(0);
    if depth != components.len() {
        return Err(PathError::Escaped);
    }

    assert_no_symlink_escape(&canonical_root, &candidate)?;
    Ok(candidate)
}

/// Refuse a path that reaches outside the root through a symlink.
///
/// The component checks above work on the text of the name, so they cannot
/// see a folder that is a symlink to somewhere else. This walks up from the
/// candidate to the first path that exists on disk, resolves it, and requires
/// the result to sit under the root. The root itself exists, so the walk
/// always terminates.
fn assert_no_symlink_escape(canonical_root: &Path, candidate: &Path) -> Result<(), PathError> {
    let mut probe = candidate;
    loop {
        if let Ok(real) = probe.canonicalize() {
            return if real.starts_with(canonical_root) {
                Ok(())
            } else {
                Err(PathError::Escaped)
            };
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return Err(PathError::Escaped),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn accepts_ordinary_names() {
        for name in ["repo", "my-repo", "my_repo", "repo.2", "a", "R2D2"] {
            assert!(validate_name(name).is_ok(), "should accept {name:?}");
        }
    }

    #[test]
    fn accepts_nested_names() {
        for name in ["work/site", "a/b/c", "team-1/proj_2/repo.3"] {
            assert!(validate_name(name).is_ok(), "should accept {name:?}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
    }

    #[test]
    fn rejects_empty_components() {
        assert_eq!(validate_name("/repo"), Err(NameError::EmptyComponent));
        assert_eq!(validate_name("repo/"), Err(NameError::EmptyComponent));
        assert_eq!(validate_name("a//b"), Err(NameError::EmptyComponent));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert_eq!(validate_name(&long), Err(NameError::TooLong));
        assert_eq!(validate_name(&format!("work/{long}")), Err(NameError::TooLong));
    }

    #[test]
    fn accepts_max_length() {
        let ok = "a".repeat(MAX_NAME_LEN);
        assert!(validate_name(&ok).is_ok());
    }

    #[test]
    fn rejects_too_deep() {
        let deep = vec!["a"; MAX_DEPTH + 1].join("/");
        assert_eq!(validate_name(&deep), Err(NameError::TooDeep));

        let at_limit = vec!["a"; MAX_DEPTH].join("/");
        assert!(validate_name(&at_limit).is_ok());
    }

    #[test]
    fn rejects_dot_components() {
        assert_eq!(validate_name("."), Err(NameError::DotComponent));
        assert_eq!(validate_name(".."), Err(NameError::DotComponent));
        assert_eq!(validate_name("work/../etc"), Err(NameError::DotComponent));
    }

    #[test]
    fn rejects_leading_dot() {
        assert_eq!(validate_name(".hidden"), Err(NameError::LeadingDot));
        assert_eq!(validate_name("work/.hidden"), Err(NameError::LeadingDot));
    }

    #[test]
    fn rejects_git_suffix() {
        assert_eq!(validate_name("repo.git"), Err(NameError::GitSuffix));
        // A folder named `x.git` would nest a repo inside a repo directory.
        assert_eq!(validate_name("x.git/repo"), Err(NameError::GitSuffix));
    }

    #[test]
    fn rejects_traversal_characters() {
        // `..` is a component of its own here, so the dot-component rule
        // rejects it before the leading-dot rule sees it.
        assert_eq!(validate_name("../etc"), Err(NameError::DotComponent));
        assert_eq!(validate_name("..etc"), Err(NameError::LeadingDot));
        assert_eq!(validate_name("a\\b"), Err(NameError::BadChar('\\')));
        assert_eq!(validate_name("etc/../../passwd"), Err(NameError::DotComponent));
    }

    #[test]
    fn rejects_shell_metacharacters() {
        for bad in ["a;b", "a b", "a$b", "a|b", "a&b", "a`b", "a\0b"] {
            assert!(
                matches!(validate_name(bad), Err(NameError::BadChar(_))),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_non_ascii_lookalikes() {
        // A Cyrillic 'а' looks like ASCII 'a' but must not pass.
        assert!(matches!(validate_name("repo\u{0430}"), Err(NameError::BadChar(_))));
    }

    #[test]
    fn repo_path_stays_under_root() {
        let root = std::env::temp_dir().join(format!("cgit-manage-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let canonical = root.canonicalize().unwrap();

        let path = repo_path(&root, "project").unwrap();
        assert!(path.starts_with(&canonical), "{path:?} not under {canonical:?}");
        assert_eq!(path.file_name().unwrap(), "project.git");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn repo_path_nests_one_directory_per_component() {
        let d = crate::tmpdir::TempDir::new("path-nested");
        let canonical = d.path().canonicalize().unwrap();

        let path = repo_path(d.path(), "work/team/site").unwrap();

        assert_eq!(path, canonical.join("work").join("team").join("site.git"));
    }

    #[test]
    #[cfg(unix)]
    fn repo_path_refuses_a_folder_that_symlinks_out_of_the_root() {
        let d = crate::tmpdir::TempDir::new("path-symlink");
        let outside = crate::tmpdir::TempDir::new("path-symlink-target");
        std::os::unix::fs::symlink(outside.path(), d.join("escape")).unwrap();

        assert!(matches!(
            repo_path(d.path(), "escape/repo"),
            Err(PathError::Escaped)
        ));
    }

    #[test]
    fn repo_path_rejects_bad_name() {
        let root = std::env::temp_dir();
        assert!(matches!(
            repo_path(&root, "../evil"),
            Err(PathError::Name(_))
        ));
    }

    #[test]
    fn repo_path_reports_missing_root() {
        let root = Path::new("/no/such/root/for/cgit/manage");
        assert!(matches!(
            repo_path(root, "project"),
            Err(PathError::RootMissing(_))
        ));
    }
}
