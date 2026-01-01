FROM debian:13-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && \
    apt install -y libpq5 ca-certificates openssl && \
    rm -rf /var/lib/{apt,dpkg,cache,log}/

WORKDIR /usr/local/bin

COPY --chmod=755 build-result /usr/local/bin/community-ap-tools
CMD ["/usr/local/bin/community-ap-tools"]
