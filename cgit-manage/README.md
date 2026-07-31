# cgit-manage

A CGI tool that creates, deletes, renames, and describes the bare git repos
that [cgit](https://git.zx2c4.com/cgit/) serves. It is part of this image, not
a separate program you deploy. Apache serves it at `/manage/`, and cgit shows
each change on its next request.

The tool does one thing per request. It reads the request as a CGI program,
checks the repo name against a strict allowlist, and runs one filesystem
operation inside `GIT_ROOT`.

## Folders

A repo name may carry slashes, so `work/site` names the repo `site` in the
folder `work`. The tool creates `<GIT_ROOT>/work/site.git`, cgit scans the root
recursively and lists it as `work/site.git`, and git clones it from
`/work/site.git`. A name nests at most 8 levels deep.

The tool manages the folders itself. Create makes the folders a name needs.
Delete removes the folders the repo leaves empty, up to but not including the
root. Rename accepts a new name in a different folder, so it also moves a repo:
`demo` to `archive/2024/demo` files the repo away and `work/site` to `site`
lifts it back out.

## Where it runs

The `Dockerfile` builds this crate in a Rust stage and copies the one binary to
`/usr/local/lib/cgit-manage/cgit-manage`. `apache.conf` routes `/manage` to it
and sets `GIT_ROOT=/srv/git`. The CGI process runs as `www-data`, the user
Apache serves as, so every repo the tool creates is `www-data`-owned from birth
and `git-http-backend` can push into it.

The tool creates a repo with a plain `git init --bare`. It does not write the
per-repo `http.receivepack` that `create-repo.sh` writes, so the image sets that
setting in system config instead. A repo made from the web UI accepts pushes
right away, with no container restart.

## Security model

**This deployment runs the tool without authentication.** Anyone who reaches
port 80 can create, rename, and delete a repo. That matches the anonymous push
the rest of the image already allows, and it assumes a trusted tailnet.
`REMOTE_USER` stays empty, so the log lines and each repo's `owner` file record
no name.

The tool itself never authenticates anyone. It reads the caller's identity from
`REMOTE_USER` and trusts whatever the web server put there. To require a
password, edit the `<Location "/manage">` block in `apache.conf`: add
`mod_auth_basic`, `mod_authn_core`, `mod_authn_file`, and `mod_authz_user` to
the `LoadModule` list, then replace `Require all granted` with `AuthType Basic`,
`AuthUserFile`, and `Require valid-user`. Apache then passes the authenticated
name in `REMOTE_USER` and the tool records it as each repo's owner.

Three independent checks confine every path, whoever the caller is:

- A character allowlist splits the name on `/` and accepts a component only
  when it matches `^[a-zA-Z0-9][a-zA-Z0-9._-]*$`, stays within 64 characters,
  is not a dot component, and does not end in `.git`. A name carries at most 8
  components, so no empty, hidden, or `..` component survives.
- The tool canonicalizes `GIT_ROOT` and asserts the built path still sits
  under it, at the depth the name asked for.
- The tool canonicalizes the deepest part of the path that exists on disk and
  asserts that too sits under the root, so a folder that is a symlink cannot
  point an operation out of the tree.

The tool never builds a shell command. It runs `git` with an argument list and
calls the standard library for every filesystem change, so an attacker cannot
inject a command through a repo name.

## One limitation

The tool runs as `www-data` and deletes a repo with an ordinary recursive
remove. A repo that someone created by hand as root, with `docker exec git git
init --bare`, holds root-owned files that `www-data` cannot unlink, so delete
answers 400. Restart the container and `entrypoint.sh` hands the repo to
`www-data`; the delete then succeeds. Creating repos through this tool or
through `create-repo` avoids the situation, since both create as `www-data`.

## Configure

The tool reads one environment variable, which `apache.conf` sets:

- `GIT_ROOT` — the directory that holds the bare repos, `/srv/git` here. The
  directory must exist before the tool runs.

`cgitrc` points `scan-path` at the same directory. The tool writes each repo's
`description` and `owner` files, which cgit reads natively, so a created repo
shows its description on the cgit index right away.

## Logs

The tool writes one line to standard error for each create, delete, rename, and
description change. Apache captures standard error in its error log, which this
image sends to the container log.

```
cgit-manage: user=- action=create target="demo"
```

## Test

```sh
cargo test
```

The tests need `git` on `PATH`, because `create` runs `git init --bare`. They
need nothing else. The crate has no dependencies and the test suite adds none:
`src/tmpdir.rs` is a 40-line throwaway directory that deletes itself on drop,
and `tests/cgi.rs` stands in for the web server with `std::process`.

The suite has two layers:

- **Unit tests** in `src/`. `repo.rs` covers the name allowlist, the depth
  bound, the path confinement, and the symlinked folder. `cgi.rs` covers form
  decoding and HTML escaping. `actions.rs` covers create, delete, rename, and
  describe against a real directory, including that a refused name writes
  nothing outside the root and that a delete prunes the folders it empties.
- **End-to-end tests** in `tests/cgi.rs`. Each one runs the compiled binary the
  way Apache runs it: CGI variables in the environment, the form body on
  standard input, the response parsed off standard output. They cover routing,
  status codes, the rule that mutations refuse GET, nested names through every
  action, escaping of a description and an owner that carry markup, the
  `CONTENT_LENGTH` bound, and the log line each mutation writes.

The build stage compiles the crate, so a compile error fails `docker build`.
The GitHub Actions workflow runs the tests natively before the image build.
