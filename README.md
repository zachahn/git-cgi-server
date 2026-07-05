# git-cgi-server

A single container image: **cgit** web UI plus **git smart-HTTP** (clone *and*
anonymous push), served by **Apache**.

Published to `ghcr.io/zachahn/git-cgi-server` by GitHub Actions on every push
to `main`.

- **Web UI**: cgit at the container's port 80
- **Clone / push**: `http://<host>/<repo>.git`
- **Browse**: `http://<host>/<repo>.git/` — the same URL, no prefix
- **Push is unauthenticated** — anyone who can reach port 80 can push to any
  repo. Intended for a trusted LAN / tailnet only.

Clone/push and browse share one URL space. Apache routes git's smart-HTTP
requests (`HEAD`, `info/refs?service=…`, `git-upload-pack`, `git-receive-pack`)
to git-http-backend and sends everything else to cgit, so a single URL serves
both.

## Layout

```
Dockerfile      # cgit + git-http-backend + apache on debian:stable-slim
apache.conf     # routes smart-HTTP to git-http-backend, everything else to cgit
cgitrc          # cgit config (scans /srv/git, advertises clone URLs)
create-repo.sh  # single creation path: inits a bare repo as www-data (-> /usr/local/bin/create-repo)
entrypoint.sh   # per-start repo prep (re-runs create-repo per repo), then runs apache
compose.yaml    # minimal example for running the published image locally
.github/workflows/publish.yaml  # builds and pushes the image to ghcr.io
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

Repos live under `/srv/git`, expected to be a mounted volume.

## Running

```sh
docker run -d --name git \
  -p 8080:80 \
  -v cgit-repos:/srv/git \
  ghcr.io/zachahn/git-cgi-server:latest
```

Add a bare repo and it appears in cgit and becomes clonable / pushable:

```sh
docker exec git create-repo myrepo   # inits as www-data, enables push
docker restart git                   # cgit picks up the new repo
```

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
