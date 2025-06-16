FROM rust:1.85-bookworm

RUN apt update && apt install -y libpq-dev
RUN rustup component add clippy rustfmt
RUN cargo install cargo-watch

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
VOLUME /builds/worker/.task-cache/cargo


WORKDIR /builds/worker
