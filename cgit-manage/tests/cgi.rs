//! End-to-end tests that run the compiled binary the way Apache runs it:
//! CGI variables in the environment, the form body on standard input, and the
//! response on standard output.
//!
//! The helpers below stand in for a web server. They are a few lines of
//! `std::process`, so the crate still needs no dependencies. `cargo test`
//! builds the binary first and hands its path to `CARGO_BIN_EXE_cgit-manage`.

#[path = "../src/tmpdir.rs"]
mod tmpdir;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tmpdir::TempDir;

/// One parsed CGI response.
struct Response {
    status: u16,
    headers: String,
    body: String,
    stderr: String,
}

impl Response {
    fn assert_has(&self, needle: &str) {
        assert!(
            self.body.contains(needle),
            "response body lacks {needle:?}\n--- body ---\n{}",
            self.body
        );
    }

    fn assert_lacks(&self, needle: &str) {
        assert!(
            !self.body.contains(needle),
            "response body contains {needle:?}\n--- body ---\n{}",
            self.body
        );
    }
}

/// Run the binary once with the CGI environment a web server would set.
///
/// `root` becomes `GIT_ROOT`. `None` leaves the variable unset, the way a
/// misconfigured server would.
fn call(
    root: Option<&Path>,
    method: &str,
    path_info: &str,
    body: &str,
    remote_user: Option<&str>,
) -> Response {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cgit-manage"));

    // Start from an empty environment so a stray CGI variable in the test
    // runner's own environment cannot change the result. git needs PATH and
    // HOME, so pass those through.
    cmd.env_clear();
    for key in ["PATH", "HOME"] {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
    if let Some(root) = root {
        cmd.env("GIT_ROOT", root);
    }
    if let Some(user) = remote_user {
        cmd.env("REMOTE_USER", user);
    }
    cmd.env("REQUEST_METHOD", method);
    cmd.env("PATH_INFO", path_info);
    if method == "POST" {
        cmd.env("CONTENT_LENGTH", body.len().to_string());
    } else {
        cmd.env("QUERY_STRING", body);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the cgit-manage binary");

    if method == "POST" {
        child
            .stdin
            .as_mut()
            .expect("stdin is piped")
            .write_all(body.as_bytes())
            .expect("write the request body");
    }
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("collect the response");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    let (headers, body) = stdout
        .split_once("\r\n\r\n")
        .map(|(h, b)| (h.to_string(), b.to_string()))
        .unwrap_or_else(|| panic!("response has no header/body split:\n{stdout}"));

    let status = headers
        .lines()
        .find_map(|line| line.strip_prefix("Status: "))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("response has no Status header:\n{headers}"));

    Response {
        status,
        headers,
        body,
        stderr,
    }
}

fn get(root: &TempDir, path_info: &str) -> Response {
    call(Some(root.path()), "GET", path_info, "", None)
}

fn post(root: &TempDir, path_info: &str, body: &str) -> Response {
    call(Some(root.path()), "POST", path_info, body, None)
}

fn post_as(root: &TempDir, user: &str, path_info: &str, body: &str) -> Response {
    call(Some(root.path()), "POST", path_info, body, Some(user))
}

// --- configuration ---

#[test]
fn a_missing_git_root_is_a_server_error() {
    let r = call(None, "GET", "/", "", None);

    assert_eq!(r.status, 500);
    r.assert_has("GIT_ROOT");
}

#[test]
fn every_response_declares_html() {
    let d = TempDir::new("content-type");
    let r = get(&d, "/");

    assert!(
        r.headers.contains("Content-Type: text/html; charset=utf-8"),
        "headers were:\n{}",
        r.headers
    );
}

// --- routing ---

#[test]
fn the_index_reports_an_empty_root() {
    let d = TempDir::new("index-empty");
    let r = get(&d, "/");

    assert_eq!(r.status, 200);
    r.assert_has("No repos yet.");
}

#[test]
fn the_new_page_serves_a_create_form() {
    let d = TempDir::new("new-form");
    let r = get(&d, "/new");

    assert_eq!(r.status, 200);
    r.assert_has(r#"<form method="post" action="create">"#);
}

#[test]
fn an_unknown_path_is_not_found() {
    let d = TempDir::new("unknown");
    let r = get(&d, "/nope");

    assert_eq!(r.status, 404);
}

#[test]
fn a_mutation_refuses_a_get() {
    let d = TempDir::new("get-create");
    let r = get(&d, "/create");

    assert_eq!(r.status, 404, "create must not run on GET");
    assert!(!d.exists("demo.git"));
}

#[test]
fn a_post_to_the_index_is_not_found() {
    let d = TempDir::new("post-index");
    let r = post(&d, "/", "name=demo");

    assert_eq!(r.status, 404);
}

// --- create ---

#[test]
fn create_makes_a_repo_and_lists_it() {
    let d = TempDir::new("create");
    let r = post(&d, "/create", "name=demo&description=A+test+repo");

    assert_eq!(r.status, 200);
    assert!(d.exists("demo.git/HEAD"));
    r.assert_has("Created demo.");
    r.assert_has("<h2>demo</h2>");
    r.assert_has(r#"value="A test repo""#);
}

#[test]
fn create_decodes_a_percent_encoded_description() {
    let d = TempDir::new("create-encoded");
    post(&d, "/create", "name=demo&description=one%2Ftwo+%26+three");

    assert_eq!(d.read("demo.git/description"), "one/two & three\n");
}

#[test]
fn create_records_the_authenticated_user_as_the_owner() {
    let d = TempDir::new("create-owner");
    let r = post_as(&d, "alice", "/create", "name=demo");

    assert_eq!(d.read("demo.git/owner"), "alice\n");
    r.assert_has("as <strong>alice</strong>");
}

#[test]
fn create_refuses_a_duplicate() {
    let d = TempDir::new("create-dup");
    post(&d, "/create", "name=demo");

    let r = post(&d, "/create", "name=demo");

    assert_eq!(r.status, 400);
    r.assert_has("already exists");
}

#[test]
fn create_refuses_a_traversing_name() {
    let d = TempDir::new("create-traverse");
    let outside = d.path().parent().unwrap().join("escape.git");

    let r = post(&d, "/create", "name=..%2F..%2Fescape");

    assert_eq!(r.status, 400);
    assert!(!outside.exists(), "nothing was written outside the root");
}

#[test]
fn create_refuses_the_names_the_allowlist_bars() {
    let d = TempDir::new("create-names");
    let long = "a".repeat(65);
    let cases = [
        ("name=", "an empty name"),
        ("name=.hidden", "a leading dot"),
        ("name=foo.git", "a .git suffix"),
        ("name=a%3Btouch+%2Ftmp%2Fpwned", "shell metacharacters"),
        ("name=caf%C3%A9", "a non-ASCII character"),
        (&format!("name={long}"), "a name over 64 characters"),
    ];

    for (body, what) in cases {
        let r = post(&d, "/create", body);
        assert_eq!(r.status, 400, "{what} must be refused; body was {body:?}");
    }

    let made = std::fs::read_dir(d.path()).unwrap().count();
    assert_eq!(made, 0, "a refused name creates nothing at all");

    let long_name = format!("{}.git", "a".repeat(64));
    post(&d, "/create", &format!("name={}", "a".repeat(64)));
    assert!(d.exists(&long_name), "64 characters is still accepted");
}

// --- nesting ---

#[test]
fn create_nests_a_repo_in_a_folder_and_lists_the_full_name() {
    let d = TempDir::new("nested-create");
    let r = post(&d, "/create", "name=work%2Fsite&description=The+site");

    assert_eq!(r.status, 200);
    assert!(d.exists("work/site.git/HEAD"));
    assert_eq!(d.read("work/site.git/description"), "The site\n");
    r.assert_has("Created work/site.");
    r.assert_has("<h2>work/site</h2>");
}

#[test]
fn the_index_lists_nested_repos_beside_root_ones() {
    let d = TempDir::new("nested-listing");
    post(&d, "/create", "name=top");
    post(&d, "/create", "name=work%2Fa%2Fdeep");
    post(&d, "/create", "name=work%2Fsite");

    let r = get(&d, "/");

    r.assert_has("<h2>top</h2>");
    r.assert_has("<h2>work/site</h2>");
    r.assert_has("<h2>work/a/deep</h2>");
}

#[test]
fn describe_and_delete_address_a_nested_repo_by_its_full_name() {
    let d = TempDir::new("nested-describe");
    post(&d, "/create", "name=work%2Fsite");

    let described = post(&d, "/describe", "name=work%2Fsite&description=hello");
    assert_eq!(described.status, 200);
    assert_eq!(d.read("work/site.git/description"), "hello\n");

    let deleted = post(&d, "/delete", "name=work%2Fsite&confirm=yes");
    assert_eq!(deleted.status, 200);
    assert!(!d.exists("work"), "the emptied folder went with the repo");
}

#[test]
fn rename_moves_a_repo_between_folders() {
    let d = TempDir::new("nested-rename");
    post(&d, "/create", "name=demo");

    let r = post(&d, "/rename", "old=demo&new=archive%2F2024%2Fdemo");

    assert_eq!(r.status, 200);
    assert!(d.exists("archive/2024/demo.git/HEAD"));
    r.assert_has("Renamed demo to archive/2024/demo.");
}

#[test]
fn create_refuses_the_nested_names_the_allowlist_bars() {
    let d = TempDir::new("nested-refused");
    let cases = [
        ("name=%2Fsite", "a leading slash"),
        ("name=work%2F", "a trailing slash"),
        ("name=work%2F%2Fsite", "a doubled slash"),
        ("name=work%2F..%2F..%2Fescape", "a traversal through a folder"),
        ("name=work%2F.hidden", "a hidden component"),
        ("name=work.git%2Fsite", "a folder that looks like a repo"),
        ("name=a%2Fb%2Fc%2Fd%2Fe%2Ff%2Fg%2Fh%2Fi", "nine components"),
    ];

    for (body, what) in cases {
        let r = post(&d, "/create", body);
        assert_eq!(r.status, 400, "{what} must be refused; body was {body:?}");
    }

    let made = std::fs::read_dir(d.path()).unwrap().count();
    assert_eq!(made, 0, "a refused name creates nothing at all");
}

// --- the index listing ---

#[test]
fn the_index_lists_repos_in_order_and_ignores_other_entries() {
    let d = TempDir::new("listing");
    post(&d, "/create", "name=zebra");
    post(&d, "/create", "name=alpha");
    std::fs::create_dir(d.join("not-a-repo")).unwrap();
    d.write("loose-file", "x");

    let r = get(&d, "/");

    let alpha = r.body.find("<h2>alpha</h2>").expect("alpha is listed");
    let zebra = r.body.find("<h2>zebra</h2>").expect("zebra is listed");
    assert!(alpha < zebra, "the listing sorts by name");
    r.assert_lacks("not-a-repo");
    r.assert_lacks("loose-file");
}

#[test]
fn the_index_hides_the_placeholder_description_git_writes() {
    let d = TempDir::new("placeholder");
    post(&d, "/create", "name=demo");

    let r = get(&d, "/");

    assert!(
        d.read("demo.git/description").contains("Unnamed repository"),
        "git wrote its placeholder, so the tool has something to hide"
    );
    r.assert_lacks("Unnamed repository");
    r.assert_has(r#"name="description" value=""#);
}

#[test]
fn the_index_escapes_a_description_that_carries_markup() {
    let d = TempDir::new("escaping");
    post(
        &d,
        "/create",
        "name=demo&description=%3Cscript%3Ealert%281%29%3C%2Fscript%3E",
    );

    let r = get(&d, "/");

    r.assert_lacks("<script>alert(1)</script>");
    r.assert_has("&lt;script&gt;alert(1)&lt;/script&gt;");
}

#[test]
fn the_index_escapes_an_owner_that_carries_markup() {
    let d = TempDir::new("escaping-user");
    let r = post_as(&d, "<b>bob</b>", "/create", "name=demo");

    r.assert_lacks("<b>bob</b>");
    r.assert_has("&lt;b&gt;bob&lt;/b&gt;");
}

// --- describe, rename, delete ---

#[test]
fn describe_updates_the_text_cgit_shows() {
    let d = TempDir::new("describe");
    post(&d, "/create", "name=demo&description=first");

    let r = post(&d, "/describe", "name=demo&description=second");

    assert_eq!(r.status, 200);
    assert_eq!(d.read("demo.git/description"), "second\n");
    r.assert_has("Updated description for demo.");
}

#[test]
fn describe_refuses_a_missing_repo() {
    let d = TempDir::new("describe-missing");
    let r = post(&d, "/describe", "name=demo&description=x");

    assert_eq!(r.status, 400);
    r.assert_has("does not exist");
}

#[test]
fn rename_moves_the_repo() {
    let d = TempDir::new("rename");
    post(&d, "/create", "name=demo");

    let r = post(&d, "/rename", "old=demo&new=archive");

    assert_eq!(r.status, 200);
    assert!(!d.exists("demo.git"));
    assert!(d.exists("archive.git/HEAD"));
    r.assert_has("Renamed demo to archive.");
}

#[test]
fn rename_refuses_a_target_that_exists() {
    let d = TempDir::new("rename-taken");
    post(&d, "/create", "name=demo");
    post(&d, "/create", "name=archive");

    let r = post(&d, "/rename", "old=demo&new=archive");

    assert_eq!(r.status, 400);
    assert!(d.exists("demo.git"), "the source survives");
}

#[test]
fn delete_needs_the_confirmation_field() {
    let d = TempDir::new("delete-unconfirmed");
    post(&d, "/create", "name=demo");

    let r = post(&d, "/delete", "name=demo");

    assert_eq!(r.status, 400);
    r.assert_has("Delete needs confirmation.");
    assert!(d.exists("demo.git"), "the repo survives");
}

#[test]
fn delete_removes_the_repo_when_confirmed() {
    let d = TempDir::new("delete");
    post(&d, "/create", "name=demo");

    let r = post(&d, "/delete", "name=demo&confirm=yes");

    assert_eq!(r.status, 200);
    assert!(!d.exists("demo.git"));
    r.assert_has("Deleted demo.");
}

#[test]
fn delete_refuses_a_missing_repo() {
    let d = TempDir::new("delete-missing");
    let r = post(&d, "/delete", "name=demo&confirm=yes");

    assert_eq!(r.status, 400);
}

// --- the request body ---

#[test]
fn the_reader_stops_at_content_length() {
    let d = TempDir::new("content-length");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cgit-manage"));
    cmd.env_clear();
    for key in ["PATH", "HOME"] {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("GIT_ROOT", d.path())
        .env("REQUEST_METHOD", "POST")
        .env("PATH_INFO", "/create")
        // Declare fewer bytes than the body carries. The tool must read only
        // the declared count, so the smuggled second field never arrives.
        .env("CONTENT_LENGTH", "9");

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run the binary");
    // The tool stops reading at the declared count, so the tail of this write
    // may land in a closed pipe. That is the behavior under test, not a failure.
    let _ = child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"name=demo&description=smuggled");
    drop(child.stdin.take());
    child.wait_with_output().unwrap();

    assert!(d.exists("demo.git"), "the declared bytes were read");
    assert!(
        !d.read("demo.git/description").contains("smuggled"),
        "the bytes past CONTENT_LENGTH were ignored"
    );
}

// --- logging ---

#[test]
fn each_mutation_writes_one_log_line() {
    let d = TempDir::new("logging");

    let created = post_as(&d, "alice", "/create", "name=demo");
    assert_eq!(
        created.stderr.trim(),
        r#"cgit-manage: user=alice action=create target="demo""#
    );

    let renamed = post(&d, "/rename", "old=demo&new=archive");
    assert_eq!(
        renamed.stderr.trim(),
        r#"cgit-manage: user=- action=rename target="demo -> archive""#
    );

    let deleted = post(&d, "/delete", "name=archive&confirm=yes");
    assert_eq!(
        deleted.stderr.trim(),
        r#"cgit-manage: user=- action=delete target="archive""#
    );
}

#[test]
fn a_refused_request_writes_no_log_line() {
    let d = TempDir::new("logging-refused");
    let r = post(&d, "/create", "name=..%2Fescape");

    assert_eq!(r.status, 400);
    assert!(
        !r.stderr.contains("cgit-manage:"),
        "stderr was: {}",
        r.stderr
    );
}
