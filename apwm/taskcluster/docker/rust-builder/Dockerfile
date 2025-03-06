FROM rust:1.79-bookworm

RUN rustup component add clippy rustfmt

# Add worker user
RUN mkdir -p /builds && \
    useradd -d /builds/worker -s /bin/bash -m worker && \
    mkdir -p /builds/worker/artifacts && \
    chown worker:worker /builds/worker/artifacts && \
    chown worker:worker /builds/worker

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

# %include-run-task

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/target
VOLUME /builds/worker/.task-cache/cargo

WORKDIR /builds/worker
