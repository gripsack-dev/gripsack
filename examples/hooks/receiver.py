#!/usr/bin/env python3
"""Loopback-only demonstration receiver; authenticate/authorize real services separately."""
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import sqlite3
from counter_store import increment, intent_identity


class Receiver(BaseHTTPRequestHandler):
    def do_POST(self):
        self.connection.settimeout(5)
        try:
            if self.path != "/increment":
                raise ValueError("unknown operation")
            length = int(self.headers.get("Content-Length", "-1"))
            if not 0 < length <= 1024:
                raise ValueError("request size is outside the example budget")
            if len(self.headers.get_all("Idempotency-Key", [])) != 1:
                raise ValueError("exactly one idempotency key is required")
            intent = intent_identity(self.headers.get("Idempotency-Key"))
            body = self.rfile.read(length)
            if len(body) != length:
                raise ValueError("incomplete request")
            request = json.loads(body)
            if not isinstance(request, dict) or set(request) != {"counter"} or not isinstance(request["counter"], str):
                raise ValueError("invalid operation payload")
            # The response is sent only after token + counter commit together.
            result = increment(self.server.state_directory, intent, request["counter"])
            response = json.dumps(result, sort_keys=True).encode()
            self.send_response(200)
        except (ValueError, OverflowError, OSError, sqlite3.Error):
            response = b'{"error":"request or durable operation failed"}'
            self.send_response(400)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, _format, *_args):
        # Requests/tokens are not diagnostic logging fields.
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", required=True, type=Path)
    parser.add_argument("--port", type=int, default=8123)
    args = parser.parse_args()
    server = HTTPServer(("127.0.0.1", args.port), Receiver)
    server.state_directory = args.state_dir
    print(f"LISTEN_PORT={server.server_address[1]}", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
