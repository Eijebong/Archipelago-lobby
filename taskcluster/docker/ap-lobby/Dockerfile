FROM $DOCKER_IMAGE_PARENT AS builder

# %include .
COPY /topsrcdir /src
WORKDIR /src
RUN cargo build --release

FROM debian:12-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && \
    apt install -y libpq5 ca-certificates openssl && \
    rm -rf /var/lib/{apt,dpkg,cache,log}/

WORKDIR /usr/local/bin

COPY --from=builder /src/target/release/ap-lobby /usr/local/bin/
CMD ["/usr/local/bin/ap-lobby"]
