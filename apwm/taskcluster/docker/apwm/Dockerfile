FROM debian:12-slim

WORKDIR /usr/local/bin

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && apt install -y --option=Dpkg::Options::=--force-confdef python3 bash git openssh-client openssl

COPY build-result /usr/local/bin/apwm

# Add worker user
RUN mkdir -p /builds && \
    useradd -d /builds/worker -s /bin/bash -m worker && \
    mkdir -p /builds/worker/artifacts && \
    chown worker:worker /builds/worker/artifacts /builds/worker

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

# %include-run-task

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/.cache
