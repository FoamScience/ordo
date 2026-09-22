"""The edges of the python rules that replaced walker code: one statement per
block, `# => names` on its first line saying what fires (`none` for nothing).
tests/catalog.rs runs every block as its own file."""
import subprocess, yaml, ssl, requests, httpx, asyncio, contextlib
from contextlib import suppress

subprocess.run(cmd, shell=True)  # => shell-injection

subprocess.run(["ls"], shell=False)  # => none

subprocess.check_output(cmd, cwd=d,  # => shell-injection
                        shell=True)

other.run(cmd, shell=True)  # => none

subprocess.run(cmd, env=mk(shell=True))  # => none

subprocess.run(cmd, shell=True if x else False)  # => none

subprocess.run(  # => none
    cmd,  # was shell=True
)

subprocess.sub.run(cmd, shell=True)  # => shell-injection

yaml.load(text)  # => yaml-load

yaml.load_all(text)  # => yaml-load

yaml.load(text, Loader=yaml.SafeLoader)  # => none

yaml.load(text, Loader=CSafeLoader)  # => none

yaml.load(text, Loader=yaml.FullLoader)  # => yaml-load

yaml.load(text, Loader=(SafeLoader if x else Loader))  # => none

yaml.safe_load(text)  # => none

ctx = ssl._create_unverified_context()  # => tls-no-verify

requests.get(url, verify=False)  # => tls-no-verify

httpx.stream(url, verify=False)  # => tls-no-verify

session.send(r, verify=False)  # => tls-no-verify

client.request("GET", url, verify=False)  # => tls-no-verify

get(url, verify=False)  # => tls-no-verify

requests.get(url, verify=True)  # => none

fetch(url, verify=False)  # => none

requests.get(  # => none
    url,  # verify=False in the old code
)

with suppress(Exception):  # => broad-suppress
    pass

with contextlib.suppress(BaseException):  # => broad-suppress
    pass

with suppress(KeyError):  # => none
    pass

with suppress(errors.Exception):  # => none
    pass

with suppress(pick(Exception)):  # => none
    pass

Cls = type("Cls", (Base,), {"a": 1})  # => dynamic-type

t = type(x)  # => none

Cls2 = type("Cls", (Base,), {"a": 1}, extra)  # => none

try:  # => bare-except
    f()
except:
    pass

try:  # => empty-catch
    f()
except ValueError:
    pass

try:  # => none
    f()
except ValueError as e:
    log(e)

try:  # => bare-except, empty-catch
    f()
except (A, B):
    pass
except:
    handle()

asyncio.create_task(job())  # => fire-and-forget-task

asyncio.ensure_future(job())  # => fire-and-forget-task

loop.create_task(job())  # => fire-and-forget-task

t = asyncio.create_task(job())  # => none

await asyncio.create_task(job())  # => none

cur.execute(f"select {x}")  # => sql-injection

cur.execute("select %s" % x)  # => sql-injection

cur.execute("select " + x)  # => sql-injection

cur.execute("select {}".format(x))  # => sql-injection

cur.execute(
    "select {}".format(x))  # => sql-injection

cur.execute("select 1")  # => none

cur.execute(sql, params)  # => none

cur.executemany(q + x, rows)  # => sql-injection

db.conn.executescript(f"{x}")  # => sql-injection

execute(f"{x}")  # => none

cur.execute(build(x))  # => none
