"""Exercises every rule in rulesets/catalog/python.toml."""
import os
import pickle


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
