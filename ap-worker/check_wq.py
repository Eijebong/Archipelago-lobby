import os
import sys
import asyncio
import multiprocessing
import uuid

from wq import LobbyQueue, JobStatus

import handler  # noqa: E402
import checker  # noqa: E402


async def main(loop):
    try:
        apworlds_dir = sys.argv[1]
        custom_apworlds_dir = sys.argv[2]
    except IndexError:
        print("Usage wq.py worlds_dir custom_worlds_dir")
        sys.exit(1)

    root_url = os.environ.get("LOBBY_ROOT_URL")
    if root_url is None:
        print("Please provide the lobby root url in `LOBBY_ROOT_URL`")
        sys.exit(1)

    token = os.environ.get("YAML_VALIDATION_QUEUE_TOKEN")
    if token is None:
        print("Please provide a token in `YAML_VALIDATION_QUEUE_TOKEN`")
        sys.exit(1)

    worker_name = str(uuid.uuid4())
    ap_handler = handler.ApHandler(apworlds_dir, custom_apworlds_dir)

    await YamlCheckerQueue(ap_handler, root_url, worker_name, token, loop).run()

class YamlCheckerQueue(LobbyQueue):
    def __init__(self, ap_handler, root_url, worker_name, token, loop):
        super().__init__(root_url, "yaml_validation", worker_name, token, loop)
        self.ap_handler = ap_handler
        self.ap_checker = checker.YamlChecker(ap_handler)

    def handle_job(self, job):
        result = self.ap_checker.run_check_for_job(job)
        status = JobStatus.Failure if 'error' in result else JobStatus.Success
        return status, result


if __name__ == "__main__":
    multiprocessing.set_start_method('fork')
    loop = asyncio.new_event_loop()
    try:
        loop.run_until_complete(main(loop))
    except KeyboardInterrupt:
        pass

