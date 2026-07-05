#!/bin/sh
# Create (or re-prepare) a bare repo owned by www-data and ready for anonymous
# smart-HTTP clone and push. This is the single creation path: new-repo.sh in the
# stack, the entrypoint's per-start pass, and any manual `docker exec git
# create-repo <name>` all call it, so no repo is ever left root-owned.
#
# Usage: create-repo <name>
#   <name> may omit the .git suffix; it is added if missing.
set -eu

REPO_ROOT=/srv/git
RUN_AS=www-data

if [ $# -ne 1 ]; then
  echo "usage: create-repo <name>" >&2
  exit 1
fi

name=${1%.git}
if ! printf '%s' "$name" | grep -qE '^[A-Za-z0-9._-]+$'; then
  echo "error: name must be alphanumerics, '.', '_', or '-'" >&2
  exit 1
fi

repo="$REPO_ROOT/${name}.git"

# init only when absent; on an existing repo this is a no-op so the entrypoint
# can call create-repo to re-apply config without clobbering objects. Running as
# www-data means the repo and every object written into it are www-data-owned
# from creation — never root — so Apache (which serves as www-data) can push.
if [ ! -d "$repo" ]; then
  setpriv --reuid="$RUN_AS" --regid="$RUN_AS" --init-groups \
    git init --bare "$repo" >/dev/null
fi

# Serve this repo over smart HTTP for anonymous clone and push.
git -C "$repo" config http.receivepack true
git -C "$repo" config http.uploadpack true

# Refresh server info after a push so dumb clients and cgit see current refs.
if [ -f "$repo/hooks/post-update.sample" ] && [ ! -f "$repo/hooks/post-update" ]; then
  cp "$repo/hooks/post-update.sample" "$repo/hooks/post-update"
  chmod +x "$repo/hooks/post-update"
fi

# Belt and suspenders: a repo that predates this script (or a manual root-run
# `git init`) may be root-owned. Hand any such repo to www-data. Already-correct
# repos are untouched, so this stays cheap.
find "$repo" ! -user "$RUN_AS" -exec chown "$RUN_AS:$RUN_AS" {} + 2>/dev/null || true
