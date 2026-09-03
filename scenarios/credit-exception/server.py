#!/usr/bin/env python3
"""Launcher for the credit-policy exception service.

Start:  python server.py --port 9313
Stop:   Ctrl-C (SIGINT) or send SIGTERM to the printed PID.
"""

from __future__ import annotations

import argparse
import signal
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from creditexc import build_server  # noqa: E402


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Credit-policy exception service")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=9313)
    parser.add_argument("--verbose", action="store_true", help="log every request")
    args = parser.parse_args(argv)

    httpd = build_server(args.host, args.port, verbose=args.verbose)
    stopping = threading.Event()

    def shutdown(signum, _frame):
        if stopping.is_set():
            return
        stopping.set()
        print(f"\nReceived signal {signum}; shutting down.", flush=True)
        threading.Thread(target=httpd.shutdown, daemon=True).start()

    signal.signal(signal.SIGINT, shutdown)
    if hasattr(signal, "SIGTERM"):
        signal.signal(signal.SIGTERM, shutdown)

    host, port = httpd.server_address[0], httpd.server_address[1]
    print(f"credit-exception listening on http://{host}:{port}/ (pid {__import__('os').getpid()})",
          flush=True)
    try:
        httpd.serve_forever(poll_interval=0.2)
    finally:
        httpd.server_close()
        print("credit-exception stopped.", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
