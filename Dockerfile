# cgit web UI + git smart-HTTP (clone and anonymous push), served by lighttpd.
FROM debian:stable-slim

RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get update -qq \
    && apt-get install -y --no-install-recommends cgit git lighttpd ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /srv/git /var/cache/lighttpd/uploads /run

COPY lighttpd.conf /etc/lighttpd/lighttpd.conf
COPY cgitrc        /etc/cgitrc
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Responsive theme (git submodule, GPL-3.0). Overwrite the stock stylesheet
# and add the viewport meta tag its media queries need.
COPY vendor/responsive-cgit-css/cgit.css /usr/share/cgit/cgit.css
COPY vendor/responsive-cgit-css/head.html /etc/cgit/head.html

EXPOSE 80
VOLUME /srv/git
WORKDIR /srv/git

LABEL org.opencontainers.image.title="git-cgi-server" \
      org.opencontainers.image.description="cgit + git-http-backend + lighttpd"

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
