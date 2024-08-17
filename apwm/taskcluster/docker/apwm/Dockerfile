FROM $DOCKER_IMAGE_PARENT AS builder

# %include .
COPY /topsrcdir /src
WORKDIR /src
RUN cargo build --bin apwm --features cli --release

FROM debian:12-slim

WORKDIR /usr/local/bin

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && apt install -y python3 bash git openssh-client openssl

COPY --from=builder /src/target/release/apwm /usr/local/bin/

# Add worker user
RUN mkdir /builds && \
    useradd -d /builds/worker -s /bin/bash -m worker && \
    mkdir /builds/worker/artifacts && \
    chown worker:worker /builds/worker/artifacts /build/workers

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

# %include-run-task

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/.cache


