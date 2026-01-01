import os
import sys
import asyncio
import multiprocessing
import uuid

from wq import LobbyQueue, JobStatus

import handler  # noqa: E402
import checker  # noqa: E402

from enum import Enum

from worlds import AutoWorldRegister
from Options import (
    Choice,
    get_option_groups,
    NamedRange,
    OptionCounter,
    OptionDict,
    OptionList,
    OptionSet,
    Range,
    TextChoice,
    Toggle,
    Visibility,
)


# TODO: Dedupe with self_check
def world_from_apworld_name(apworld_name):
    for name, world in AutoWorldRegister.world_types.items():
        if world.__module__.startswith(f"worlds.{apworld_name}"):
            return name, world

    loaded_worlds = { name: world.__module__ for name, world in AutoWorldRegister.world_types.items() }
    raise Exception(f"Couldn't find loaded world with world: {apworld_name}. Loaded worlds: {loaded_worlds}")


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

    token = os.environ.get("OPTIONS_GEN_QUEUE_TOKEN")
    if token is None:
        print("Please provide a token in `OPTIONS_GEN_QUEUE_TOKEN`")
        sys.exit(1)

    worker_name = str(uuid.uuid4())
    ap_handler = handler.ApHandler(apworlds_dir, custom_apworlds_dir)

    await OptionsGenQueue(ap_handler, root_url, worker_name, token, loop).run()

def safe_json(value):
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, (frozenset, set)):
        return [safe_json(v) for v in value]
    if isinstance(value, list):
        return [safe_json(v) for v in value]
    if isinstance(value, dict):
        return {k: safe_json(v) for k, v in value.items()}
    return value

def get_default(option):
    default = option.default
    # XXX: We might need to broaden this
    if isinstance(default, int) and hasattr(option, 'name_lookup') and default in option.name_lookup:
        return option.name_lookup[default]
    return safe_json(default)

def get_type(option):
    if issubclass(option, Toggle):
        return "bool"
    # NamedRange before Range (subclass)
    if issubclass(option, NamedRange):
        return "named_range"
    if issubclass(option, Range):
        return "range"
    # TextChoice before Choice (subclass)
    if issubclass(option, TextChoice):
        return "text_choice"
    if issubclass(option, Choice):
        return "choice"
    if issubclass(option, OptionDict):
        return "dict"
    if issubclass(option, OptionSet):
        return "set"
    if issubclass(option, OptionCounter):
        return "counter"
    if issubclass(option, OptionList):
        return "list"
    return "text"

def get_range(option):
    if issubclass(option, Range):
        return (option.range_start, option.range_end)
    return None

def get_choices(option):
    if issubclass(option, Choice):
        return [key for key in option.options.keys() if key not in option.aliases]
    return None

def get_suggestions(option):
    # For NamedRange: special named values
    if issubclass(option, NamedRange):
        return list(option.special_range_names.keys())
    # For TextChoice: the choice options (same as get_choices but always returned)
    if issubclass(option, TextChoice):
        return [key for key in option.options.keys() if key not in option.aliases]
    return None

def get_valid_keys(option, world):
    if not issubclass(option, (OptionSet, OptionList, OptionCounter)):
        return None
    valid_keys = list(option.valid_keys)
    if getattr(option, 'verify_item_name', False):
        valid_keys += list(world.item_name_to_id.keys())
    if getattr(option, 'verify_location_name', False):
        valid_keys += list(world.location_name_to_id.keys())
    return valid_keys

class OptionsGenQueue(LobbyQueue):
    def __init__(self, ap_handler, root_url, worker_name, token, loop):
        super().__init__(root_url, "options_gen", worker_name, token, loop)
        self.ap_handler = ap_handler

    def handle_job(self, job):
        try:
            self.ap_handler.load_apworld(*job.params["apworld"])
            name, world = world_from_apworld_name(job.params["apworld"][0])

            game_options = {}
            option_groups = get_option_groups(world, Visibility.template)
            for group, options in option_groups.items():
                option_group_options = {}
                for option_name, option_value in options.items():
                    ty = get_type(option_value)
                    valid_keys = get_valid_keys(option_value, world)

                    display_name = getattr(option_value, "display_name", option_name)
                    description = (option_value.__doc__ or "").strip()
                    option_def = {
                        "default": get_default(option_value),
                        "description": description,
                        "ty": ty,
                        "display_name": display_name,
                    }
                    if range_info := get_range(option_value):
                        option_def["range"] = range_info
                    if choices := get_choices(option_value):
                        option_def["choices"] = choices
                    if suggestions := get_suggestions(option_value):
                        option_def["suggestions"] = suggestions
                    if valid_keys is not None:
                        option_def["valid_keys"] = safe_json(valid_keys)
                    option_group_options[option_name] = option_def
                if option_group_options:
                    game_options[group] = option_group_options
            result = {"options": game_options}
            status = JobStatus.Failure if 'error' in result else JobStatus.Success
            return status, result
        except Exception as e:
            return JobStatus.Failure, {"options": {}, 'error': str(e)}


if __name__ == "__main__":
    multiprocessing.set_start_method('fork')
    loop = asyncio.new_event_loop()
    try:
        loop.run_until_complete(main(loop))
    except KeyboardInterrupt:
        pass

