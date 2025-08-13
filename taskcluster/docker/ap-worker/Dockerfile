ARG PYTHON_VERSION=3.12


FROM alpine:latest AS enemizer

ADD --checksum=sha256:efab6784cbfe4189a01e0e25226943afd7f71e7c2f10f74b88bfa34fdac972ab https://github.com/Ijwu/Enemizer/releases/latest/download/ubuntu.16.04-x64.zip Enemizer.zip
RUN apk update && apk add unzip && unzip Enemizer.zip -d /Enemizer

FROM ghcr.io/astral-sh/uv:python${PYTHON_VERSION}-bookworm AS builder

ENV UV_PYTHON_DOWNLOADS=0

ENV BASE_COMMIT=bcce2b77981c90bfaef907274a494c0994eea78c
ENV FUZZER_COMMIT=3266f5fbd5868275261cc439c82af246b5ace146
ENV LINTER_COMMIT=fa82912d72747407a4cfaf0b5a7f3fc22b5c130d

WORKDIR /ap

# %include ap-worker

# This is a horrible hack to be able to add extra context in docker compose
# The idea is that we want to reuse this dockerfile between docker compose and CI,
# taskgraph adds the source code in /topsrcdir in the main context through
# the %include directive, of course docker compose doesn't have a way to do that so
# instead we use `additional_contexts` there. But kaniko doesn't support that...
# By using the scratch image image which has s empty, we do nothing
# on CI, and use the scratch named extra context in docker compose...
# We also cannot use the scratch image because it has no tag and kaniko doesn't
# like that, so we use that one which is the same image but with a tag...
COPY --from=eijebong/scratch . /topsrcdir/ap-worker/
COPY /topsrcdir /topsrcdir
RUN ls -lah /topsrcdir && cp -R /topsrcdir/ap-worker/ /ap/ap-worker/

COPY prepare_worlds.sh /ap/prepare_worlds.sh
COPY setup.sh /ap/setup.sh

RUN sh /ap/setup.sh ${BASE_COMMIT} ${FUZZER_COMMIT} ${LINTER_COMMIT} && rm /ap/setup.sh

FROM python:${PYTHON_VERSION}-slim-bookworm

RUN apt update && apt install -y curl libtk8.6
RUN curl -fsSL https://github.com/watchexec/watchexec/releases/download/cli-v1.20.5/watchexec-1.20.5-x86_64-unknown-linux-gnu.deb -o /tmp/watchexec.deb
RUN dpkg -i /tmp/watchexec.deb

RUN mkdir /ap
WORKDIR /ap

COPY host.yaml /ap/archipelago/host.yaml
COPY --from=enemizer /Enemizer /ap/enemizer
RUN chmod +x /ap/enemizer/EnemizerCLI.Core

COPY --from=builder /ap/archipelago/ /ap/archipelago/
COPY --from=builder /ap/supported_worlds/ /ap/supported_worlds/
COPY --from=builder /ap/ap-worker/ /ap/ap-worker/

ENV PATH="/ap/.venv/bin:$PATH"

CMD /ap/archipelago/run.sh
