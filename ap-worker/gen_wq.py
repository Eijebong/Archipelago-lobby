import aiohttp
import asyncio
import handler
import io
import multiprocessing
import os
import random
import sentry_sdk
import shutil
import sys
import tempfile
import traceback
import uuid
import yaml
import zipfile

from argparse import Namespace
from contextlib import redirect_stderr, redirect_stdout
from wq import LobbyQueue, JobStatus
from opentelemetry import trace

import Utils
from worlds import AutoWorldRegister
from Generate import main as GenMain, PlandoOptions
from Main import main as ERmain

ORIG_USER_PATH = Utils.user_path

tracer = trace.get_tracer("generator")

async def main(loop):
    try:
        apworlds_dir = sys.argv[1]
        custom_apworlds_dir = sys.argv[2]
    except IndexError:
        print("Usage gen_wq.py worlds_dir custom_worlds_dir")
        sys.exit(1)

    root_url = os.environ.get("LOBBY_ROOT_URL")
    if root_url is None:
        print("Please provide the lobby root url in `LOBBY_ROOT_URL`")
        sys.exit(1)

    token = os.environ.get("GENERATION_QUEUE_TOKEN")
    if token is None:
        print("Please provide a token in `GENERATION_QUEUE_TOKEN`")
        sys.exit(1)

    api_key = os.environ.get("LOBBY_API_KEY")
    if api_key is None:
        print("Please provide an API key in `LOBBY_API_KEY`")
        sys.exit(1)

    output_dir = os.environ.get("GENERATOR_OUTPUT_DIR")
    if output_dir is None:
        print("Please provide an output dir in `GENERATOR_OUTPUT_DIR`")
        sys.exit(1)

    worker_name = str(uuid.uuid4())
    ap_handler = handler.ApHandler(apworlds_dir, custom_apworlds_dir)
    await GenerationQueue(ap_handler, output_dir, root_url, worker_name, token, loop).run()


class GenerationQueue(LobbyQueue):
    def __init__(self, ap_handler, output_dir, root_url, worker_name, token, loop):
        super().__init__(root_url, "generation", worker_name, token, loop)
        self.root_url = root_url
        self.ap_handler = ap_handler
        self.output_dir = output_dir

    def handle_job(self, job):
        output_path = os.path.join(self.output_dir, job.job_id)
        os.makedirs(output_path, exist_ok=True)
        with tracer.start_as_current_span("generate", context=job.ctx) as _gen_span, open(os.path.join(output_path, "output.log"), "w") as out_file, redirect_stderr(out_file), redirect_stdout(out_file):
            status = JobStatus.Success
            loop = asyncio.new_event_loop()

            # Override Utils.user path so we can customize the logs folder
            def my_user_path(name, *args):
                if name == "logs":
                    return output_path
                return ORIG_USER_PATH(name, *args)


            Utils.user_path = my_user_path

            try:
                room_id = job.params["room_id"]

                players_dir = tempfile.mkdtemp(prefix="apgen")
                loop.run_until_complete(self.gather_resources(room_id, players_dir))

                sys.argv.append("--player_files_path")
                sys.argv.append(players_dir)

                for apworld, version in job.params["apworlds"]:
                    self.ap_handler.load_apworld(apworld, version)

                if job.params.get("meta_file"):
                    filtered_meta = {}
                    meta = yaml.safe_load(job.params["meta_file"])

                    for section, content in meta.items():
                        if section == "meta_description" or section in AutoWorldRegister.world_types or section is None:
                            filtered_meta[section] = content

                    with open(os.path.join(players_dir, "meta.yaml"), "w") as fd:
                        fd.write(yaml.dump(filtered_meta))

                from settings import get_settings
                settings = get_settings()

                args = Namespace(
                    **{
                        "weights_file_path": settings.generator.weights_file_path,
                        "sameoptions": False,
                        "player_files_path": players_dir,
                        "seed": random.randint(10000, 10000000),
                        "multi": 1,
                        "spoiler": 1,
                        "outputpath": output_path,
                        "spoiler_only": False,
                        "race": False,
                        "meta_file_path": os.path.join(players_dir, "meta.yaml"),
                        "log_level": "info",
                        "yaml_output": 1,
                        "plando": PlandoOptions.from_set(frozenset({"bosses", "items", "connections", "texts"})),
                        "skip_prog_balancing": False,
                        "skip_output": False,
                        "csv_output": False,
                        "log_time": False,
                    }
                )

                server_options = {
                    "hint_cost": 10,
                    "release_mode": "auto-enabled",
                    "remaining_mode": "goal",
                    "collect_mode": "disabled",
                    "item_cheat": False,
                    "server_password": None,
                }

                with tracer.start_as_current_span("ap-gen") as _span:
                    erargs, seed = GenMain(args)
                    ERmain(erargs, seed, baked_server_options=server_options)
            except Exception as e:
                traceback.print_exc()
                sentry_sdk.capture_exception(e)
                trace.get_current_span().record_exception(e)

                status = JobStatus.Failure
            finally:
                shutil.rmtree(players_dir)

        return status, None

    async def gather_resources(self, room_id, players_dir):
        yamls_url = f"/room/{room_id}/yamls"
        async with aiohttp.ClientSession(self.root_url) as client:
            response = await client.get(yamls_url, headers = { "X-Api-Key": os.environ["LOBBY_API_KEY"] })
            response.raise_for_status()

            body = io.BytesIO(await response.read())
            z = zipfile.ZipFile(body)
            z.extractall(players_dir)


if __name__ == "__main__":
    multiprocessing.set_start_method("fork")
    loop = asyncio.new_event_loop()
    try:
        loop.run_until_complete(main(loop))
    except KeyboardInterrupt:
        pass
