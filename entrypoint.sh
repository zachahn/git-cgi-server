#!/bin/sh
set -eu

REPO_ROOT=/srv/git
mkdir -p "$REPO_ROOT"

# git-http-backend runs while the repos in the mounted volume may be owned by
# anyone. Trust every repo under the root so git's "dubious ownership" guard
# does not refuse to serve them.
git config --system --add safe.directory '*'

# lighttpd triggers mod_cgi by the .cgi suffix, so expose git-http-backend
# under a *.cgi name it can alias to.
if [ ! -e /usr/lib/git-core/git-http-backend.cgi ]; then
  ln -s git-http-backend /usr/lib/git-core/git-http-backend.cgi
fi

# Prepare every bare repo found under the root for anonymous smart-HTTP push.
find "$REPO_ROOT" -type d -name '*.git' -prune | while read -r repo; do
  git -C "$repo" config http.receivepack true
  git -C "$repo" config http.uploadpack true
  # Refresh server info so dumb clients and cgit see current refs after a push.
  if [ -f "$repo/hooks/post-update.sample" ] && [ ! -f "$repo/hooks/post-update" ]; then
    cp "$repo/hooks/post-update.sample" "$repo/hooks/post-update"
    chmod +x "$repo/hooks/post-update"
  fi
done

exec lighttpd -D -f /etc/lighttpd/lighttpd.conf
