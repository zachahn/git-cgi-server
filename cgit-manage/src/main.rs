//! cgit-manage: a CGI tool that creates, deletes, renames, and describes the
//! bare git repos that cgit serves.
//!
//! The tool authenticates no one. It reads whatever identity the web server
//! put in `REMOTE_USER` and records it; this image serves the tool without a
//! password, so that name is usually empty. It reads one config value,
//! `GIT_ROOT`, and confines every operation to that directory. cgit with
//! `scan-path` picks up each change on its next request.

mod actions;
mod cgi;
mod repo;
#[cfg(test)]
mod tmpdir;

use cgi::{Request, escape_html, respond};
use std::path::{Path, PathBuf};

fn main() {
    let root = match std::env::var("GIT_ROOT") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            respond(
                500,
                "Internal Server Error",
                &page("Configuration error", "<p>The server did not set <code>GIT_ROOT</code>.</p>"),
            );
            return;
        }
    };

    let req = Request::read();
    route(&root, &req);
}

/// Send the request to the handler its path names.
fn route(root: &Path, req: &Request) {
    let path = req.path_info.trim_matches('/');
    match (req.method.as_str(), path) {
        ("GET", "") => index(root, req, None),
        ("GET", "new") => new_form(req),
        ("POST", "create") => do_create(root, req),
        ("POST", "delete") => do_delete(root, req),
        ("POST", "rename") => do_rename(root, req),
        ("POST", "describe") => do_describe(root, req),
        _ => respond(404, "Not Found", &page("Not found", "<p>No such action.</p>")),
    }
}

/// List every repo under the root, with per-repo forms.
fn index(root: &Path, req: &Request, notice: Option<&str>) {
    let repos = list_repos(root);
    let mut body = String::new();

    if let Some(msg) = notice {
        body.push_str(&format!("<p class=\"notice\">{}</p>", escape_html(msg)));
    }

    body.push_str(&format!(
        "<p>Managing repos under <code>{}</code>",
        escape_html(&root.display().to_string())
    ));
    if let Some(user) = &req.remote_user {
        body.push_str(&format!(" as <strong>{}</strong>", escape_html(user)));
    }
    body.push_str(".</p>");
    body.push_str("<p><a href=\"new\">Create a repo</a></p>");

    if repos.is_empty() {
        body.push_str("<p>No repos yet.</p>");
    } else {
        for r in &repos {
            body.push_str(&repo_card(r));
        }
    }

    respond(200, "OK", &page("Repos", &body));
}

/// One repo's block: description form, rename form, delete form.
fn repo_card(r: &RepoInfo) -> String {
    let name = escape_html(&r.name);
    let desc = escape_html(&r.description);
    format!(
        r#"<section class="repo">
  <h2>{name}</h2>
  <form method="post" action="describe">
    <input type="hidden" name="name" value="{name}">
    <label>Description
      <input type="text" name="description" value="{desc}" maxlength="512">
    </label>
    <button type="submit">Save description</button>
  </form>
  <form method="post" action="rename">
    <input type="hidden" name="old" value="{name}">
    <label>Rename to
      <input type="text" name="new" required pattern="[A-Za-z0-9][A-Za-z0-9._-]*" maxlength="64">
    </label>
    <button type="submit">Rename</button>
  </form>
  <form method="post" action="delete" onsubmit="return confirm('Delete {name}? This cannot be undone.');">
    <input type="hidden" name="name" value="{name}">
    <input type="hidden" name="confirm" value="yes">
    <button type="submit" class="danger">Delete</button>
  </form>
</section>"#
    )
}

/// The create form.
fn new_form(_req: &Request) {
    let body = r#"<h2>Create a repo</h2>
<form method="post" action="create">
  <label>Name
    <input type="text" name="name" required pattern="[A-Za-z0-9][A-Za-z0-9._-]*" maxlength="64" autofocus>
  </label>
  <label>Description
    <input type="text" name="description" maxlength="512">
  </label>
  <button type="submit">Create</button>
</form>
<p><a href="./">Back to repos</a></p>"#;
    respond(200, "OK", &page("Create a repo", body));
}

fn do_create(root: &Path, req: &Request) {
    let name = req.field("name");
    let description = req.field("description");
    let owner = req.remote_user.as_deref();
    match actions::create(root, name, description, owner) {
        Ok(()) => index(root, req, Some(&format!("Created {name}."))),
        Err(e) => fail(&e.to_string()),
    }
}

fn do_delete(root: &Path, req: &Request) {
    if req.field("confirm") != "yes" {
        fail("Delete needs confirmation.");
        return;
    }
    let name = req.field("name");
    match actions::delete(root, name, req.remote_user.as_deref()) {
        Ok(()) => index(root, req, Some(&format!("Deleted {name}."))),
        Err(e) => fail(&e.to_string()),
    }
}

fn do_rename(root: &Path, req: &Request) {
    let old = req.field("old");
    let new = req.field("new");
    match actions::rename(root, old, new, req.remote_user.as_deref()) {
        Ok(()) => index(root, req, Some(&format!("Renamed {old} to {new}."))),
        Err(e) => fail(&e.to_string()),
    }
}

fn do_describe(root: &Path, req: &Request) {
    let name = req.field("name");
    let description = req.field("description");
    match actions::set_description(root, name, description, req.remote_user.as_deref()) {
        Ok(()) => index(root, req, Some(&format!("Updated description for {name}."))),
        Err(e) => fail(&e.to_string()),
    }
}

/// Render an error page with a 400 status.
fn fail(message: &str) {
    let body = format!(
        "<p class=\"error\">{}</p><p><a href=\"./\">Back to repos</a></p>",
        escape_html(message)
    );
    respond(400, "Bad Request", &page("Error", &body));
}

/// What the index shows for one repo.
struct RepoInfo {
    name: String,
    description: String,
}

/// Read every bare repo directly under `root`.
///
/// A bare repo directory ends in `.git`. The listing strips that suffix for
/// display, the same name the forms submit back. The function reads each
/// repo's `description` file when present.
fn list_repos(root: &Path) -> Vec<RepoInfo> {
    let mut repos = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return repos,
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let file_name = entry.file_name();
        let dir = match file_name.to_str() {
            Some(d) => d,
            None => continue,
        };
        let name = match dir.strip_suffix(".git") {
            Some(n) => n.to_string(),
            None => continue,
        };

        let description = std::fs::read_to_string(entry.path().join("description"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // git seeds a placeholder line; treat it as no description.
        let description = if description.starts_with("Unnamed repository") {
            String::new()
        } else {
            description
        };

        repos.push(RepoInfo { name, description });
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

/// Wrap body content in the page shell with inline styles.
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · cgit-manage</title>
<style>
  body {{ font: 16px/1.5 system-ui, sans-serif; max-width: 46rem; margin: 2rem auto; padding: 0 1rem; color: #222; }}
  h1 {{ font-size: 1.4rem; }}
  h2 {{ font-size: 1.1rem; margin: 0 0 .5rem; }}
  section.repo {{ border: 1px solid #ddd; border-radius: 6px; padding: 1rem; margin: 1rem 0; }}
  form {{ display: flex; gap: .5rem; align-items: end; flex-wrap: wrap; margin: .5rem 0; }}
  label {{ display: flex; flex-direction: column; font-size: .85rem; gap: .2rem; flex: 1; }}
  input[type=text] {{ font-size: 1rem; padding: .3rem .4rem; }}
  button {{ font-size: .9rem; padding: .4rem .8rem; cursor: pointer; }}
  button.danger {{ color: #a00; border-color: #a00; }}
  .notice {{ background: #e7f5e7; padding: .6rem .8rem; border-radius: 4px; }}
  .error {{ background: #fbe3e3; padding: .6rem .8rem; border-radius: 4px; }}
  a {{ color: #06c; }}
  code {{ background: #f2f2f2; padding: .1rem .3rem; border-radius: 3px; }}
</style>
</head>
<body>
<h1><a href="./">cgit-manage</a></h1>
{body}
</body>
</html>
"#
    )
}
