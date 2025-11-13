from opentelemetry import trace
import requests
import shutil
import tempfile
import os
import sys

ap_path = os.path.abspath(os.path.dirname(sys.argv[0]))
sys.path.insert(0, ap_path)

from worlds import WorldSource  # noqa: E402
from worlds.AutoWorld import AutoWorldRegister  # noqa: E402
from worlds.Files import APWorldContainer  # noqa: E402
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

        apworld_container = APWorldContainer(dest_path)
        apworld_container.read()

        WorldSource(dest_path, is_zip=True, relative=False).load()

        if apworld_container.game and apworld_container.game in AutoWorldRegister.world_types:
            if apworld_container.world_version:
                AutoWorldRegister.world_types[apworld_container.game].world_version = apworld_container.world_version

        self.refresh_netdata_package()

    def refresh_netdata_package(self):
        for world_name, world in AutoWorldRegister.world_types.items():
            if world_name not in worlds.network_data_package["games"]:
                worlds.network_data_package["games"][world_name] =  world.get_data_package_data()


