//! Repo name validation and path confinement.
//!
//! A repo name arrives from an HTTP request, so an attacker controls it.
//! Two independent checks guard every path this module builds.
//! First, `validate_name` accepts a name only when it passes a strict
//! character allowlist. Second, `repo_path` canonicalizes the parent and
//! asserts the result still sits under the configured root. Either check
//! alone stops directory traversal. The pair stops it twice.

use std::path::{Path, PathBuf};

/// The longest repo name the tool accepts.
const MAX_NAME_LEN: usize = 64;

/// Why a repo name failed validation.
#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    LeadingDot,
    GitSuffix,
    DotComponent,
    BadChar(char),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameError::Empty => write!(f, "name is empty"),
            NameError::TooLong => write!(f, "name is longer than {MAX_NAME_LEN} characters"),
            NameError::LeadingDot => write!(f, "name starts with a dot"),
            NameError::GitSuffix => write!(f, "name ends with .git; the tool adds that itself"),
            NameError::DotComponent => write!(f, "name is a dot component"),
            NameError::BadChar(c) => write!(f, "name contains a disallowed character: {c:?}"),
        }
    }
}

impl std::error::Error for NameError {}

/// Accept a repo name only when it passes the allowlist.
///
/// The rules, in order:
/// - The name is not empty.
/// - The name stays within `MAX_NAME_LEN`.
/// - The name is not `.` or `..`.
/// - The name does not start with a dot.
/// - The name does not end with `.git`; the caller appends that.
/// - Every character is ASCII alphanumeric, `.`, `_`, or `-`.
///
/// The allowlist admits only ASCII, so no Unicode look-alike slips through.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
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
/// The function validates the name, canonicalizes `root`, then appends
/// `<name>.git`. It canonicalizes the root rather than the full path
/// because the repo directory may not exist yet, for example on create.
/// The name allowlist already forbids `/`, `.`, and `..`, so the appended
/// component cannot walk out of the canonical root. The function asserts
/// that fact and refuses on any mismatch.
pub fn repo_path(root: &Path, name: &str) -> Result<PathBuf, PathError> {
    validate_name(name).map_err(PathError::Name)?;

    let canonical_root = root.canonicalize().map_err(PathError::RootMissing)?;

    let mut dir_name = String::with_capacity(name.len() + 4);
    dir_name.push_str(name);
    dir_name.push_str(".git");

    let candidate = canonical_root.join(&dir_name);

    // The name allowlist guarantees a single, ordinary path component.
    // Confirm the parent still resolves to the canonical root, so a future
    // change to the allowlist cannot silently open a traversal.
    match candidate.parent() {
        Some(parent) if parent == canonical_root => Ok(candidate),
        _ => Err(PathError::Escaped),
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
    fn rejects_empty() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert_eq!(validate_name(&long), Err(NameError::TooLong));
    }

    #[test]
    fn accepts_max_length() {
        let ok = "a".repeat(MAX_NAME_LEN);
        assert!(validate_name(&ok).is_ok());
    }

    #[test]
    fn rejects_dot_components() {
        assert_eq!(validate_name("."), Err(NameError::DotComponent));
        assert_eq!(validate_name(".."), Err(NameError::DotComponent));
    }

    #[test]
    fn rejects_leading_dot() {
        assert_eq!(validate_name(".hidden"), Err(NameError::LeadingDot));
    }

    #[test]
    fn rejects_git_suffix() {
        assert_eq!(validate_name("repo.git"), Err(NameError::GitSuffix));
    }

    #[test]
    fn rejects_traversal_characters() {
        // A slash never passes the allowlist, so no name can name a subdir.
        // `../etc` trips the leading-dot rule first, which also rejects it.
        assert_eq!(validate_name("../etc"), Err(NameError::LeadingDot));
        assert_eq!(validate_name("a/b"), Err(NameError::BadChar('/')));
        assert_eq!(validate_name("a\\b"), Err(NameError::BadChar('\\')));
        assert_eq!(validate_name("etc/passwd"), Err(NameError::BadChar('/')));
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
