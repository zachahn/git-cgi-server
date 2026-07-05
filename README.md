# git-cgi-server

A single container image: **cgit** web UI plus **git smart-HTTP** (clone *and*
anonymous push), served by **lighttpd**.

Published to `ghcr.io/zachahn/git-cgi-server` by GitHub Actions on every push
to `main`.

- **Web UI**: cgit at the container's port 80
- **Clone / push**: `http://<host>/git/<repo>.git`
- **Push is unauthenticated** — anyone who can reach port 80 can push to any
  repo. Intended for a trusted LAN / tailnet only.

## Layout

```
Dockerfile      # cgit + git-http-backend + lighttpd on debian:stable-slim
lighttpd.conf   # routes /git/* to git-http-backend, everything else to cgit
cgitrc          # cgit config (scans /srv/git, advertises clone URLs)
entrypoint.sh   # per-start repo prep, then runs lighttpd
compose.yaml    # minimal example for running the published image locally
.github/workflows/publish.yaml  # builds and pushes the image to ghcr.io
```

## The image

`entrypoint.sh` runs on **every container start**, not at build time. It marks
every `*.git` repo under `/srv/git` as a `safe.directory`, symlinks
`git-http-backend` to a `.cgi` name (lighttpd dispatches CGI by suffix), and
enables `http.receivepack` so each bare repo accepts anonymous push. After
adding a repo, restart the container so the entrypoint prepares it.

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
docker exec git git init --bare /srv/git/myrepo.git
docker restart git   # entrypoint enables push + refreshes safe.directory
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
