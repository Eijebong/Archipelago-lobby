FROM mozillareleases/taskgraph:run-task-v14.0.0

RUN apt update && apt install -y --no-install-recommends --option=Dpkg::Options::=--force-confdef python3 bash git openssh-client openssl curl jq && rm -rf /var/lib/apt/lists/*

COPY --chmod=755 build-result /usr/local/bin/apwm

VOLUME /builds/worker/checkouts
