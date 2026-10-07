"""Tiny CLI over the GhidraMCP HTTP API (port 8089).

usage: python tools/gh.py <endpoint> [key=value ...] [--post] [--raw]
"""
import json
import sys
import urllib.parse
import urllib.request

BASE = "http://127.0.0.1:8089/"


def call(endpoint, params=None, post=False, timeout=300):
    params = params or {}
    url = BASE + endpoint.lstrip("/")
    if post:
        req = urllib.request.Request(url, data=json.dumps(params).encode(),
                                     headers={"Content-Type": "application/json"})
    else:
        if params:
            url += "?" + urllib.parse.urlencode(params)
        req = urllib.request.Request(url)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        txt = r.read().decode("utf-8", "replace")
    try:
        return json.loads(txt)
    except ValueError:
        return txt


def main():
    args = sys.argv[1:]
    post = "--post" in args
    raw = "--raw" in args
    args = [a for a in args if a not in ("--post", "--raw")]
    ep = args[0]
    params = dict(a.split("=", 1) for a in args[1:])
    res = call(ep, params, post=post)
    if isinstance(res, str):
        print(res)
    elif raw:
        print(json.dumps(res))
    elif isinstance(res, dict) and "decompiled" in res:
        print(res["decompiled"])
    else:
        print(json.dumps(res, indent=1))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
