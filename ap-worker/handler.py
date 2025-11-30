from opentelemetry import trace
import requests
import shutil
import tempfile
import os
import sys
import zipfile
import json

ap_path = os.path.abspath(os.path.dirname(sys.argv[0]))
sys.path.insert(0, ap_path)

from worlds import WorldSource  # noqa: E402
from worlds.AutoWorld import AutoWorldRegister  # noqa: E402
from worlds.Files import APWorldContainer, InvalidDataError  # noqa: E402
from Utils import tuplize_version  # noqa: E402
import worlds  # noqa: E402



# Some **supported** apworlds try to get stuff from external APIs. We do not want that as it currently times out in prod
# Until I have a better solution, just return an error immediately when someone tries to use requests
def no_internet(*args, **kwargs):
    raise RuntimeError("The apworld tried to contact the internet which isn't supported with YAML validation.")

requests.get = no_internet
requests.post = no_internet
requests.put = no_internet
requests.head = no_internet
requests.options = no_internet
requests.delete = no_internet

tracer = trace.get_tracer("ap-handler")

class ApHandler:
    def __init__(self, apworlds_dir, custom_apworlds_dir):
        self.apworlds_dir = apworlds_dir
        self.custom_apworlds_dir = custom_apworlds_dir
        self.refresh_netdata_package()
        self.tempdir = tempfile.mkdtemp()

    def __del__(self):
        shutil.rmtree(self.tempdir)

    def check_apworld_directory_name(self, apworld_path, apworld_name):
        with zipfile.ZipFile(apworld_path, "r") as zf:
            for name in zf.namelist():
                parts = name.split('/')
                if len(parts) > 1 and parts[0] == apworld_name:
                    return

            root_dirs = {name.split('/')[0] for name in zf.namelist() if '/' in name}
            raise Exception(f"Apworld must contain a directory named '{apworld_name}'. Found: {sorted(root_dirs)}")

    def read_apworld_manifest(self, apworld_path):
        """
        Read the archipelago.json manifest from an apworld file.
        Returns a tuple of (game_name, world_version) or (None, None) if not found.

        We have to use a custom method here instead of `APWorldContainer` because
        core world manifests don't have a `compatible_version` in sources, it
        only gets put there when they build AP. So we assume that manifests without
        one are core and if they're not the I tried my best.
        """
        with zipfile.ZipFile(apworld_path, "r") as zf:
            for info in zf.infolist():
                if info.filename.endswith("archipelago.json"):
                    with zf.open(info, "r") as f:
                        manifest = json.load(f)

                    # If compatible_version is present, validate it
                    if "compatible_version" in manifest:
                        container_version = APWorldContainer.version
                        if manifest["compatible_version"] > container_version:
                            raise Exception(
                                f"Apworld requires container version "
                                f"{manifest['compatible_version']} but we only support {container_version}"
                            )

                    world_game = manifest.get("game")
                    world_version = None
                    if "world_version" in manifest:
                        try:
                            world_version = tuplize_version(manifest["world_version"])
                        except:
                            # Version string is not in expected format (e.g., "alpha02b")
                            # Leave as None, world will use default 0.0.0
                            pass

                    return world_game, world_version

        return None, None

    @tracer.start_as_current_span("load_apworld")
    def load_apworld(self, apworld_name, apworld_version):
        span = trace.get_current_span()
        span.set_attribute("apworld_name", apworld_name)
        span.set_attribute("apworld_version", apworld_version)

        if '/' in apworld_name:
            raise Exception("Invalid apworld name")

        if '/' in apworld_version:
            raise Exception("Invalid apworld version")

        apworld_path = f"{self.custom_apworlds_dir}/{apworld_name}-{apworld_version}.apworld"
        supported_apworld_path = f"{self.apworlds_dir}/{apworld_name}-{apworld_version}.apworld"
        dest_path = f"{self.tempdir}/{apworld_name}.apworld"

        if os.path.isfile(apworld_path):
            shutil.copy(apworld_path, dest_path)
        elif os.path.isfile(supported_apworld_path):
            shutil.copy(supported_apworld_path, dest_path)
        else:
            if "worlds." + apworld_name in sys.modules:
                return
            raise Exception("Invalid apworld: {}, version {}".format(apworld_name, apworld_version))

        world_game, world_version = self.read_apworld_manifest(dest_path)
        WorldSource(dest_path, is_zip=True, relative=False).load()

        if world_game and world_game in AutoWorldRegister.world_types:
            if world_version:
                AutoWorldRegister.world_types[world_game].world_version = world_version

        self.refresh_netdata_package()

    def refresh_netdata_package(self):
        for world_name, world in AutoWorldRegister.world_types.items():
            if world_name not in worlds.network_data_package["games"]:
                worlds.network_data_package["games"][world_name] =  world.get_data_package_data()


