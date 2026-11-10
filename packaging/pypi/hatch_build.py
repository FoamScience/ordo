"""Tags the wheel for the platform whose binaries were vendored into it."""
import os

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        platform = os.environ.get("ORDO_WHEEL_PLATFORM")
        if platform:
            build_data["pure_python"] = False
            build_data["tag"] = f"py3-none-{platform}"
