import os
import sentry_sdk

import aiohttp
import asyncio
import enum
import sys
from multiprocessing import Process, Pipe

from opentelemetry.trace.propagation.tracecontext import TraceContextTextMapPropagator
from opentelemetry.baggage.propagation import W3CBaggagePropagator
from opentelemetry import trace
from opentelemetry.sdk.resources import SERVICE_NAME, Resource
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from opentelemetry.propagate import set_global_textmap
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from sentry_sdk.integrations.opentelemetry import SentryPropagator, SentrySpanProcessor

tracer = trace.get_tracer("wq")

class JobStatus(enum.Enum):
    Success = "Success"
    Failure = "Failure"
    InternalError = "InternalError"

class JobCancelledException(Exception):
    """Raised when a job has been cancelled"""
    pass

class LobbyQueue:
    def __init__(self, root_url, queue_name, worker_id, token, loop):
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

        self.queue_name = queue_name
        self.worker_id = worker_id
        self.client = aiohttp.ClientSession(root_url)
        self.token = token
        self.loop = loop
        self.resource = Resource(attributes={
            SERVICE_NAME: f"{queue_name}-worker",
        })

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        loop = asyncio.new_event_loop()
        loop.run_until_complete(self.close())

    async def claim_job(self):
        async with self.post("claim_job", json={"worker_id": self.worker_id}) as resp:
            resp.raise_for_status()
            job_raw = await resp.json()
            if job_raw is None:
                return None
            return Job(self, **job_raw)

    def post(self, route, *args, **kwargs):
        route = "/queues/{}/{}".format(self.queue_name, route)
        if 'headers' not in kwargs:
            kwargs['headers'] = {}

        if 'otlp_context' in kwargs:
            W3CBaggagePropagator().inject(kwargs['headers'], kwargs['otlp_context'])
            TraceContextTextMapPropagator().inject(kwargs['headers'], kwargs['otlp_context'])
            del kwargs['otlp_context']

        kwargs['headers']['X-Worker-Auth'] = self.token
        return self.client.post(route, *args, **kwargs)

    async def close(self):
        await self.client.close()

    async def run(self):
        while True:
            try:
                job = await self.claim_job()
            except RuntimeError:
                break
            except Exception as e:
                print(f"Error while claiming job from lobby: {e}. Retrying in 1s...")
                sys.stdout.flush()
                await asyncio.sleep(1)
                continue

            try:
                if job is not None:
                    print(f"Claimed job: {job.job_id}")
                    await self._handle_job(job)
                continue
            except Exception as e:
                print(e)
                sentry_sdk.capture_exception(e)

                try:
                    await job.resolve(JobStatus.InternalError, None)
                except Exception as e:
                    print(e)
                    sentry_sdk.capture_exception(e)
                    continue

    async def _handle_job(self, job):
        rpipe, wpipe = Pipe()
        data_available = asyncio.Event()
        asyncio.get_event_loop().add_reader(rpipe.fileno(), data_available.set)

        job_cancelled = asyncio.Event()

        async def reclaim_loop():
            while True:
                try:
                    await job.reclaim()
                except JobCancelledException as e:
                    # Job was explicitly cancelled
                    print(f"Job {job.job_id} was cancelled: {e}")
                    job_cancelled.set()
                    return
                except Exception as e:
                    # Other reclaim failure
                    print(f"Job {job.job_id} reclaim failed: {e}. Assuming job was cancelled.")
                    job_cancelled.set()
                    return
                await asyncio.sleep(7)

        task = self.loop.create_task(reclaim_loop())

        p = Process(target=self._job_process, args=(job, wpipe))
        p.start()

        # Wait for either job completion or cancellation
        while not rpipe.poll() and not job_cancelled.is_set():
            done, pending = await asyncio.wait(
                [asyncio.create_task(data_available.wait()), asyncio.create_task(job_cancelled.wait())],
                return_when=asyncio.FIRST_COMPLETED
            )

            # Cancel any pending tasks
            for task_to_cancel in pending:
                task_to_cancel.cancel()

            if job_cancelled.is_set():
                # Job was cancelled, terminate the process
                print(f"Job {job.job_id} was cancelled, terminating process")
                p.terminate()
                p.join()
                break

            data_available.clear()

        asyncio.get_event_loop().remove_reader(rpipe.fileno())

        # Only resolve if job completed normally (not cancelled)
        if not job_cancelled.is_set() and rpipe.poll():
            status, result = rpipe.recv()
            task.cancel()
            try:
                await job.resolve(status, result)
                print(f"Resolved job {job.job_id} with status {status}")
            except JobCancelledException as e:
                print(f"Job {job.job_id} was cancelled during resolution: {e}")
            except Exception as e:
                print(f"Failed to resolve job {job.job_id}: {e}")
                sentry_sdk.capture_exception(e)
        else:
            # Job was cancelled, clean up
            task.cancel()
            if p.is_alive():
                p.terminate()
                p.join()
            print(f"Job {job.job_id} was cancelled and terminated")

        sys.stdout.flush()

    def _job_process(self, job, wpipe):
        traceProvider = TracerProvider(resource=self.resource)
        otlp_endpoint = os.environ.get("OTLP_ENDPOINT")
        if otlp_endpoint:
            processor = BatchSpanProcessor(OTLPSpanExporter(endpoint=otlp_endpoint))
            traceProvider.add_span_processor(processor)
        else:
            print("OTLP_ENDPOINT not provided, not enabling otlp exporter")

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
            sentry_processor = SentrySpanProcessor()
            traceProvider.add_span_processor(sentry_processor)
            set_global_textmap(SentryPropagator())

        trace.set_tracer_provider(traceProvider)

        with tracer.start_as_current_span("job") as _span:
            result = self.handle_job(job)
            wpipe.send(result)

        traceProvider.force_flush()
        sentry_sdk.flush()


class Job:
    def __init__(self, queue, job_id, params):
        self._queue = queue
        self.job_id = job_id
        self.params = params
        self.ctx = TraceContextTextMapPropagator().extract(carrier=params['otlp_context'])

    async def resolve(self, status, result):
        should_fallback = False
        try:
            async with self._queue.post("resolve_job", json={"worker_id": self._queue.worker_id, "job_id": self.job_id, "status": status.value, "result": result}) as resp:
                if resp.status == 410:
                    raise JobCancelledException(f"Job {self.job_id} has been cancelled")
                if 400 <= resp.status < 500 and result is not None:
                    print(f"Job {self.job_id}: resolve failed with {resp.status}, resolving as InternalError")
                    # Consume body to properly release connection
                    await resp.read()
                    should_fallback = True
                else:
                    resp.raise_for_status()
        except (OSError, aiohttp.ClientError) as e:
            if result is not None:
                print(f"Job {self.job_id}: resolve failed with {e}, resolving as InternalError")
                should_fallback = True
            else:
                raise

        if should_fallback:
            await self._resolve_internal_error()

    async def _resolve_internal_error(self):
        for attempt in range(3):
            try:
                async with self._queue.post("resolve_job", json={"worker_id": self._queue.worker_id, "job_id": self.job_id, "status": JobStatus.InternalError.value, "result": None}) as resp:
                    resp.raise_for_status()
                    return
            except (OSError, aiohttp.ClientError) as e:
                if attempt < 2:
                    print(f"Job {self.job_id}: fallback resolve attempt {attempt + 1} failed with {e}, retrying...")
                    await asyncio.sleep(0.1)
                else:
                    raise

    async def reclaim(self):
        async with self._queue.post("reclaim_job", json={"worker_id": self._queue.worker_id, "job_id": self.job_id}) as resp:
            if resp.status == 410:
                raise JobCancelledException(f"Job {self.job_id} has been cancelled")
            resp.raise_for_status()

