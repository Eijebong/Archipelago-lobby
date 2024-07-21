from voluptuous import ALLOW_EXTRA, Required

from taskgraph.transforms.base import TransformSequence
from taskgraph.util.schema import Schema

transforms = TransformSequence()

@transforms.add
def add_rust_arg(config, tasks):
    arg = "--debug"
    if config.params.get("tasks_for") == "":
        arg = "--release"

    for task in tasks:
        task.setdefault("args", {})["CARGO_FLAGS"] = arg

        yield task
