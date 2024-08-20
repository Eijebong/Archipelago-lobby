FROM rust:1.79-bookworm

RUN apt update && apt install libpq-dev
RUN rustup component add clippy rustfmt

# Add worker user
RUN mkdir -p /builds && \
    useradd -d /builds/worker -s /bin/bash -m worker && \
    mkdir -p /builds/worker/artifacts && \
    chown -R worker:worker /builds/worker

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

# %include-run-task

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/target
VOLUME /builds/worker/.cargo/registry
VOLUME /builds/worker/.cache

WORKDIR /builds/worker
