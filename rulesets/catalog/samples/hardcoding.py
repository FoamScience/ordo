"""Exercises the python half of rulesets/catalog/hardcoding.toml."""
import time

SDK = "https://cdn.example.com/v2.4.1/client.js"  # hardcoded-version
CACHE = "/var/cache/app"                          # absolute-path


def poll(retries):
    if retries > 86400:                           # magic-number
        return None
    time.sleep(300)                               # magic-argument
    return retries
