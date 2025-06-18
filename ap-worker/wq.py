import os
import sentry_sdk

if "SENTRY_DSN" in os.environ:
    try:
        with open("version") as fd:
            version = fd.read().strip()
    except FileNotFoundError:
        version = None

    sentry_sdk.init(
        dsn=os.environ["SENTRY_DSN"],
        instrumenter="otel",
        traces_sample_rate=1.0,
        environment=os.environ.get("ENVIRONMENT", "dev"),
        release=version
    )

import aiohttp
import asyncio
import enum

from opentelemetry.trace.propagation.tracecontext import TraceContextTextMapPropagator

class JobStatus(enum.Enum):
    Success = "Success"
    Failure = "Failure"
    InternalError = "InternalError"

class LobbyQueue:
    def __init__(self, root_url, queue_name, worker_id, token, loop):
        self.queue_name = queue_name
        self.worker_id = worker_id
        self.client = aiohttp.ClientSession(root_url)
        self.token = token

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        loop = asyncio.new_event_loop()
        loop.run_until_complete(self.close())

    async def claim_job(self):
        resp = await self.post("claim_job", json={"worker_id": self.worker_id})
        resp.raise_for_status()

        job_raw = await resp.json()
        if job_raw is None:
            return None

        return Job(self, **job_raw)

    async def post(self, route, *args, **kwargs):
        route = "/queues/{}/{}".format(self.queue_name, route)
        if 'headers' not in kwargs:
            kwargs['headers'] = {}

        if 'otlp_context' in kwargs:
            W3CBaggagePropagator().inject(kwargs['headers'], kwargs['otlp_context'])
            TraceContextTextMapPropagator().inject(kwargs['headers'], kwargs['otlp_context'])
            del kwargs['otlp_context']

        kwargs['headers']['X-Worker-Auth'] = self.token
        result = await self.client.post(route, *args, **kwargs)
        result.raise_for_status()
        return result

    async def close(self):
        await self.client.close()


class Job:
    def __init__(self, queue, job_id, params):
        self._queue = queue
        self.job_id = job_id
        self.params = params
        self.ctx = TraceContextTextMapPropagator().extract(carrier=params['otlp_context'])

    async def resolve(self, status, result):
        await self._queue.post("resolve_job", json={"worker_id": self._queue.worker_id, "job_id": self.job_id, "status": status.value, "result": result})

    async def reclaim(self):
        await self._queue.post("reclaim_job", json={"worker_id": self._queue.worker_id, "job_id": self.job_id})

