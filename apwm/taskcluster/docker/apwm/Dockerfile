FROM $DOCKER_IMAGE_PARENT AS builder

# %include .
COPY /topsrcdir /src
WORKDIR /src
RUN cargo build --bin apwm --features cli --release

FROM debian:12-slim

WORKDIR /usr/local/bin

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && apt install -y python3 bash

# %include-run-task
COPY --from=builder /src/target/release/apwm /usr/local/bin/
