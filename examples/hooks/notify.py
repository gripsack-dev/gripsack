#!/usr/bin/env python3
"""Send one bounded request carrying the stable intent token; never retry automatically."""
import argparse
import ipaddress
import json
import os
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener
from counter_store import intent_identity


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, _request, _response, _code, _message, _headers, _target):
        # A new destination is a new operator decision, not implicit delivery.
        return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--counter", default="example")
    args = parser.parse_args()
    parsed = urlsplit(args.endpoint)
    loopback = False
    try:
        loopback = ipaddress.ip_address(parsed.hostname or "").is_loopback
    except ValueError:
        loopback = parsed.hostname == "localhost"
    if parsed.username is not None or parsed.password is not None or parsed.fragment:
        parser.error("userinfo and fragments are not accepted in notification endpoints")
    if parsed.scheme != "https" and not (parsed.scheme == "http" and loopback):
        parser.error("use HTTPS, or explicit loopback HTTP for the local fixture")
    intent = intent_identity(os.environ.get("GRIPSACK_ACTIVATION_INTENT_ID"))
    if not args.counter or len(args.counter.encode()) > 256:
        parser.error("counter name is empty or exceeds the example budget")
    body = json.dumps({"counter": args.counter}, separators=(",", ":")).encode()
    request = Request(args.endpoint, body, method="POST", headers={
        "Content-Type": "application/json", "Idempotency-Key": intent,
    })
    with build_opener(NoRedirect()).open(request, timeout=5) as response:
        result = response.read(4097)
        if response.status != 200 or len(result) > 4096:
            raise RuntimeError("receiver response did not satisfy the bounded success contract")
    result = json.loads(result)
    if (not isinstance(result, dict) or result.get("intent") != intent
            or type(result.get("value")) is not int or result["value"] < 0
            or type(result.get("applied")) is not bool):
        raise RuntimeError("receiver response did not bind the requested intent")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
