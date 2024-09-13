FROM debian:12-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && \
    apt install -y ca-certificates openssl && \
    rm -rf /var/lib/{apt,dpkg,cache,log}/

WORKDIR /usr/local/bin

COPY --chmod=755 build-result /usr/local/bin/apdiff-viewer
CMD ["/usr/local/bin/apdiff-viewer"]
