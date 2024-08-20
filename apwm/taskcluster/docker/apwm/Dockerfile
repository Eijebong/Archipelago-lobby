FROM debian:12-slim

WORKDIR /usr/local/bin

ENV DEBIAN_FRONTEND=noninteractive
RUN apt update && apt install -y --option=Dpkg::Options::=--force-confdef python3 bash git openssh-client openssl

# Add worker user
RUN mkdir -p /builds && \
    useradd -d /builds/worker -s /bin/bash -m worker && \
    mkdir -p /builds/worker/artifacts && \
    chown worker:worker /builds/worker/artifacts /builds/worker

ENV SHELL=/bin/bash \
    HOME=/builds/worker \
    USER=worker

COPY run-task/run-task /usr/local/bin/run-task
COPY run-task/fetch-content /usr/local/bin/fetch-content
COPY run-task/robustcheckout.py /usr/local/mercurial/robustcheckout.py
COPY run-task/hgrc /etc/mercurial/hgrc.d/mozilla.rc

COPY --chmod=755 build-result /usr/local/bin/apwm

VOLUME /builds/worker/checkouts
VOLUME /builds/worker/.cache
