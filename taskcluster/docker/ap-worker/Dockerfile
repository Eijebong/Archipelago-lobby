ARG PYTHON_VERSION=3.12
ARG SRC=topsrcdir
ARG DOCKER_SRC=/


FROM alpine:latest AS enemizer

ADD --checksum=sha256:efab6784cbfe4189a01e0e25226943afd7f71e7c2f10f74b88bfa34fdac972ab https://github.com/Ijwu/Enemizer/releases/latest/download/ubuntu.16.04-x64.zip Enemizer.zip
RUN apk update && apk add unzip && unzip Enemizer.zip -d /Enemizer

FROM ghcr.io/astral-sh/uv:python${PYTHON_VERSION}-trixie AS builder
ARG SRC
ARG DOCKER_SRC

ENV UV_PYTHON_DOWNLOADS=0

ENV BASE_COMMIT=f811b364f19d16df7d34b953ca772cfa2b34ddd5
ENV FUZZER_COMMIT=cd92b8f1d16a6baa5aa96ca46a40a34109b127a0
ENV LINTER_COMMIT=9e9c0e2b65ce60200436c7aa54d702a87eca67eb

WORKDIR /ap

# %include ap-worker

COPY ${SRC}/ap-worker /ap/ap-worker
COPY ${DOCKER_SRC}/prepare_worlds.sh /ap/prepare_worlds.sh
COPY ${DOCKER_SRC}/setup.sh /ap/setup.sh

RUN sh /ap/setup.sh ${BASE_COMMIT} ${FUZZER_COMMIT} ${LINTER_COMMIT} && rm /ap/setup.sh

FROM python:${PYTHON_VERSION}-slim-trixie
ARG DOCKER_SRC

RUN apt update && apt install -y curl libtk8.6
RUN curl -fsSL https://github.com/watchexec/watchexec/releases/download/cli-v1.20.5/watchexec-1.20.5-x86_64-unknown-linux-gnu.deb -o /tmp/watchexec.deb
RUN dpkg -i /tmp/watchexec.deb

RUN mkdir /ap
WORKDIR /ap

COPY ${DOCKER_SRC}/host.yaml /ap/archipelago/host.yaml
COPY --from=enemizer /Enemizer /ap/enemizer
RUN chmod +x /ap/enemizer/EnemizerCLI.Core

COPY --from=builder /ap/archipelago/ /ap/archipelago/
COPY --from=builder /ap/supported_worlds/ /ap/supported_worlds/
COPY --from=builder /ap/ap-worker/ /ap/ap-worker/

ENV PATH="/ap/.venv/bin:$PATH"
