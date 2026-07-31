# git-cgi-server

A single container image: **cgit** web UI plus **git smart-HTTP** (clone *and*
anonymous push), served by **Apache**, with a small CGI admin tool for creating
and deleting repos.

Published to `ghcr.io/zachahn/git-cgi-server` by GitHub Actions on every push
to `main`.

- **Web UI**: cgit at the container's port 80
- **Clone / push**: `http://<host>/<repo>.git`
- **Browse**: `http://<host>/<repo>.git/` — the same URL, no prefix
- **Manage**: `http://<host>/manage/` creates, deletes, renames, and describes
  repos from the browser
- **Push and manage are both unauthenticated** — anyone who can reach port 80
  can push to any repo and can delete any repo. Intended for a trusted LAN /
  tailnet only.

Clone/push and browse share one URL space. Apache routes git's smart-HTTP
requests (`HEAD`, `info/refs?service=…`, `git-upload-pack`, `git-receive-pack`)
to git-http-backend and sends everything else to cgit, so a single URL serves
both.

## Layout

```
Dockerfile      # cgit + git-http-backend + apache on debian:stable-slim
apache.conf     # routes smart-HTTP to git-http-backend, /manage to cgit-manage, the rest to cgit
cgitrc          # cgit config (scans /srv/git, advertises clone URLs)
create-repo.sh  # single creation path: inits a bare repo as www-data (-> /usr/local/bin/create-repo)
entrypoint.sh   # per-start repo prep (re-runs create-repo per repo), then runs apache
cgit-manage/    # Rust CGI admin tool, built into the image (src/, tests/, see its README)
compose.yaml    # minimal example for running the published image locally
.github/workflows/publish.yaml  # tests cgit-manage, then builds and pushes the image to ghcr.io
```

## The image

`entrypoint.sh` runs on **every container start**, not at build time. It marks
every `*.git` repo under `/srv/git` as a `safe.directory`, hands the repo root to
`www-data` (Apache refuses to run as root, so it serves as `www-data` and needs
write access to push objects), then re-runs `create-repo` for each existing repo
to re-apply push config and heal ownership.

`create-repo <name>` is the **single creation path**. It inits the bare repo as
`www-data` — so the repo and every object are `www-data`-owned from birth, never
root — and enables `http.receivepack`. It is a no-op on an existing repo's
objects, so the entrypoint calls it safely on every start.

Repos live under `/srv/git`, expected to be a mounted volume. A name may carry
slashes, so `work/site` puts the repo at `/srv/git/work/site.git`. cgit scans
the root recursively and lists it as `work/site.git`, and git clones it from
`/work/site.git`. `create-repo` makes the folders such a name needs and hands
any folder an earlier root-run left behind to `www-data`.

## The admin tool

`cgit-manage/` holds a small Rust CGI program. A builder stage compiles it and
the final image copies the binary to `/usr/local/lib/cgit-manage/cgit-manage`.
Apache serves it at `/manage/` and gives it `GIT_ROOT=/srv/git`. The program
creates, deletes, renames, and describes repos, and cgit lists each change on
its next request.

A name with slashes nests the repo in a folder, the same way `create-repo`
does. The tool makes the folders a name needs, removes the ones a delete
empties, and treats a rename into another folder as a move.

The CGI process runs as `www-data`, so a repo it creates is `www-data`-owned
like one `create-repo` makes. It runs a plain `git init --bare` and writes no
per-repo push config, so the image sets `http.receivepack` and
`http.uploadpack` in **system** config. A repo created from the browser accepts
pushes immediately, with no restart.

`/manage` has no password. A repo named `manage.git` still browses at
`/manage.git/`, because the route matches `/manage` only at a slash or the end
of the path. `cgit-manage/README.md` covers the path checks and how to put
basic auth in front of the tool.

## Running

```sh
docker run -d --name git \
  -p 8080:80 \
  -v cgit-repos:/srv/git \
  ghcr.io/zachahn/git-cgi-server:latest
```

Add a repo at `http://<host>/manage/`. It appears in cgit and accepts clones and
pushes right away. `create-repo` remains for the case where the web UI is
unreachable:

```sh
docker exec git create-repo myrepo        # inits as www-data, enables push
docker exec git create-repo work/site     # /srv/git/work/site.git
```

cgit rescans `/srv/git` on every request, so neither route needs a restart.

## Building locally

```sh
docker build -t git-cgi-server .
```

## Publishing

The `publish.yaml` workflow builds a `linux/arm64` image and pushes it to
ghcr.io using the repository's `GITHUB_TOKEN` — no extra secrets needed. Tags:

- `latest` and `main` on every push to the default branch
- `sha-<short>` for each commit
- `vX.Y.Z` when you push a matching git tag

The package is created private on first publish. Make it public (or grant pull
access) under the repo's **Packages** settings if the pulling host is not
authenticated to ghcr.io.
