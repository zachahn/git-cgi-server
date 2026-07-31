# cgit-manage: the CGI admin tool, built from source in cgit-manage/.
#
# The crate has no dependencies, so this stage compiles four files and nothing
# else. It builds for the target platform under emulation rather than
# cross-compiling, which keeps the stage to three lines; the crate is small
# enough that the emulated compile stays short.
FROM rust:1-slim AS cgit-manage
WORKDIR /src
COPY cgit-manage/ ./
RUN cargo build --release --locked

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

COPY --from=cgit-manage /src/target/release/cgit-manage /usr/local/lib/cgit-manage/cgit-manage

# Default `git init` (including the bare repos this server hosts) to `main`.
#
# Serve every repo for anonymous clone and push from system config as well.
# create-repo writes the same two settings per repo, but cgit-manage creates a
# repo with a plain `git init --bare`, so without the system default a repo made
# from the web UI would refuse pushes until the next container start.
RUN git config --system init.defaultBranch main \
    && git config --system http.receivepack true \
    && git config --system http.uploadpack true

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
