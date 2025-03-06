FROM mozillareleases/taskgraph:run-task-v14.0.0

RUN apt update && apt install -y --option=Dpkg::Options::=--force-confdef python3 bash git openssh-client openssl curl jq

COPY --chmod=755 build-result /usr/local/bin/apwm

VOLUME /builds/worker/checkouts
