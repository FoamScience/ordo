# Sample for rulesets/python-google-style.toml: every rule has a violation
# here, most with the compliant form beside it. Not real code.
from . import sibling                  # no-relative-import
from concurrent import futures         # fine

MAX_RETRIES = 3                        # fine: module constant
_CACHE = {}                            # fine: internal, leading underscore
cache = {}                             # mutable-global-state


class Widget:
    @staticmethod                      # no-staticmethod
    def helper():
        return 1

    @classmethod                       # fine
    def create(cls):
        return cls()


def add(a, items=[]):                  # mutable-default-arg
    items.append(a)
    return items


def add_ok(a, items=None):             # fine
    if items is None:
        items = []
    items.append(a)
    return items


def load_plugin(name):
    return __import__(name)            # no-import-hack


def risky():
    try:
        return 1 / 0
    except Exception:                  # no-broad-except
        return None


def risky_ok():
    try:
        return 1 / 0
    except ZeroDivisionError:          # fine
        return None


def is_ready(flag):
    if flag == False:                  # no-bool-literal-compare
        return False
    return True


def is_ready_ok(flag):
    if not flag:                       # fine
        return False
    return True


def iter_keys(adict):
    for k in adict.keys():             # prefer-default-iterator
        print(k)


def iter_keys_ok(adict):
    for k in adict:                    # fine
        print(k)


def pairs(a, b):
    return [x for x in a for y in b]   # single-for-comprehension


def pairs_ok(a):
    return [x for x in a]              # fine


def long_function():                   # function-length (46 lines)
    total = 0
    total += 1
    total += 2
    total += 3
    total += 4
    total += 5
    total += 6
    total += 7
    total += 8
    total += 9
    total += 10
    total += 11
    total += 12
    total += 13
    total += 14
    total += 15
    total += 16
    total += 17
    total += 18
    total += 19
    total += 20
    total += 21
    total += 22
    total += 23
    total += 24
    total += 25
    total += 26
    total += 27
    total += 28
    total += 29
    total += 30
    total += 31
    total += 32
    total += 33
    total += 34
    total += 35
    total += 36
    total += 37
    total += 38
    total += 39
    total += 40
    return total
