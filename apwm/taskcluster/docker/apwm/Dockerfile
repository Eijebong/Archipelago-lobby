FROM $DOCKER_IMAGE_PARENT AS builder

# %include .
COPY /topsrcdir /src
WORKDIR /src
RUN cargo build --release

FROM debian:12-slim

WORKDIR /usr/local/bin

COPY --from=builder /src/target/release/apwm /usr/local/bin/
