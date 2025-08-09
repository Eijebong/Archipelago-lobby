FROM rust:1.89-slim-bookworm

COPY setup.sh setup.sh
RUN bash ./setup.sh

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

# %include-run-task

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/target
VOLUME /builds/worker/.task-cache/cargo


WORKDIR /builds/worker
