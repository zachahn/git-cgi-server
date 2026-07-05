#!/bin/sh
set -eu

REPO_ROOT=/srv/git
mkdir -p "$REPO_ROOT"

# The mounted volume comes up root-owned. Hand the repo root to www-data so
# create-repo (which runs as www-data) can make repos in it, and so every repo
# lives under a www-data-owned tree. This is a chown of one directory, not a
# recursive pass over repo contents.
chown www-data:www-data "$REPO_ROOT"

# git-http-backend runs while the repos in the mounted volume may be owned by
# anyone. Trust every repo under the root so git's "dubious ownership" guard
# does not refuse to serve them.
git config --system --add safe.directory '*'

# Re-prepare every existing repo through the single creation path. create-repo
# is a no-op on an existing repo's objects, re-applies the push config, and
# fixes ownership for any repo that predates this image (Apache serves as
# www-data and must own the repos to push into them).
find "$REPO_ROOT" -type d -name '*.git' -prune | while read -r repo; do
  name=${repo#"$REPO_ROOT"/}
  create-repo "$name"
done

# Apache writes its runtime state here; mpm_event needs the dir on every start.
mkdir -p /run/apache2

exec apache2 -D FOREGROUND -f /etc/apache2/apache2.conf
