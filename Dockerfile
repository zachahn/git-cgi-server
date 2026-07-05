# cgit web UI + git smart-HTTP (clone and anonymous push), served by Apache.
FROM debian:stable-slim

RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get update -qq \
    && apt-get install -y --no-install-recommends cgit git apache2 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /srv/git /run/apache2

# Replace the stock site with our single-file config. Disable the default
# ports.conf and site so only apache.conf drives the server.
COPY apache.conf   /etc/apache2/apache2.conf
COPY cgitrc        /etc/cgitrc
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY create-repo.sh /usr/local/bin/create-repo
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/create-repo \
    && rm -f /etc/apache2/ports.conf /etc/apache2/sites-enabled/* /etc/apache2/conf-enabled/*

# Default `git init` (including the bare repos this server hosts) to `main`.
RUN git config --system init.defaultBranch main

# Responsive theme (git submodule, GPL-3.0). Overwrite the stock stylesheet
# and add the viewport meta tag its media queries need.
COPY vendor/responsive-cgit-css/cgit.css /usr/share/cgit/cgit.css
COPY vendor/responsive-cgit-css/head.html /etc/cgit/head.html

EXPOSE 80
VOLUME /srv/git
WORKDIR /srv/git

LABEL org.opencontainers.image.title="git-cgi-server" \
      org.opencontainers.image.description="cgit + git-http-backend + apache"

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
