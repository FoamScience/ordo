"""Exercises every rule in rulesets/catalog/python.toml."""
import asyncio
import os
import pickle
import subprocess
from contextlib import suppress

import requests
import yaml


class Bag:
    def __eq__(self, other):  # eq-without-hash
        return True

    def __del__(self):  # del-finalizer
        pass

    def __getattribute__(self, name):  # getattribute-override
        return object.__getattribute__(self, name)


def collect(items=[]):  # mutable-default-arg
    assert isinstance(items, list)  # assert-validation
    return items


def run(expr, blob, cmd):
    value = eval(expr)  # eval/exec
    loaded = pickle.loads(blob)  # pickle
    os.system(cmd)  # os-system
    return value, loaded


async def job():
    return 1


async def risky(cmd, text, url, x, cur):
    subprocess.run(cmd, shell=True)  # shell-injection
    loaded = yaml.load(text)  # yaml-load
    page = requests.get(url, verify=False)  # tls-no-verify
    with suppress(Exception):  # broad-suppress
        page.raise_for_status()
    Made = type("Made", (object,), {"x": 1})  # dynamic-type
    try:
        cur.execute(f"select {x}")  # sql-injection
    except:  # noqa: E722 — bare-except
        raise
    try:
        cur.execute("select 1")
    except ValueError:  # empty-catch
        pass
    asyncio.create_task(job())  # fire-and-forget-task
    return loaded, Made
