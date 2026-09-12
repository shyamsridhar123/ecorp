"""Native Linux receipts and pidfd-only signalling for disposable test servers.

Python 3.9+ exposes the kernel process handle that Node 22 does not expose.
There is deliberately no numeric-PID signal or unsupported-kernel fallback.
Only non-secret ownership metadata is accepted on stdin or emitted on stdout.
"""

import datetime
import json
import os
from pathlib import Path
import re
import select
import signal
import sys
from urllib.parse import urlsplit


REFUSAL = "QA process ownership changed or is unverifiable; refusing server restart."
MANUAL_PORTS = {5432, 54329, 8791, 8793, 5187, 5291, 15191, 15193}


def require(condition):
    if not condition:
        raise ValueError(REFUSAL)


def require_native():
    require(sys.platform == "linux")
    require(callable(getattr(os, "pidfd_open", None)))
    require(callable(getattr(signal, "pidfd_send_signal", None)))


def start_ticks(raw, pid):
    # comm can contain spaces, newlines and closing parentheses. Field 22 is
    # relative to the last closing parenthesis, not a whitespace-split comm.
    opening, closing = raw.find("("), raw.rfind(")")
    require(opening > 0 and closing > opening and raw[:opening].strip() == str(pid))
    fields = raw[closing + 1:].split()
    require(len(fields) >= 20 and fields[0] not in {"Z", "X", "x"})
    require(re.fullmatch(r"[0-9]+", fields[19]) is not None)
    return fields[19]


def live(handle):
    poll = select.poll()
    poll.register(handle, select.POLLIN)
    return not poll.poll(0)


def listener_owned(proc, port):
    sockets = set()
    for entry in (proc / "fd").iterdir():
        try:
            target = os.readlink(entry)
        except FileNotFoundError:
            continue  # A concurrently closed unrelated descriptor is harmless.
        match = re.fullmatch(r"socket:\[([0-9]+)\]", target)
        if match:
            sockets.add(match[1])
    listeners = set()
    for table in ("tcp", "tcp6"):
        for line in (proc / "net" / table).read_text().splitlines()[1:]:
            fields = line.split()
            require(len(fields) >= 10)
            address, encoded_port = fields[1].rsplit(":", 1)
            if fields[3] != "0A" or int(encoded_port, 16) != port:
                continue
            # Refuse wildcard/public listeners, including an additional socket
            # on the same port that is not owned by the recorded process.
            if address not in {"0100007F", "00000000000000000000000001000000"}:
                return False
            listeners.add(fields[9])
    return bool(listeners) and listeners.issubset(sockets)


def inspect_process(pid, port, handle):
    require(live(handle))
    proc = Path("/proc") / str(pid)
    before = start_ticks((proc / "stat").read_text(), pid)
    boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    require(re.fullmatch(r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", boot_id) is not None)
    boot_seconds = next(int(line.split()[1]) for line in Path("/proc/stat").read_text().splitlines()
                        if line.startswith("btime "))
    executable, cwd = os.readlink(proc / "exe"), os.readlink(proc / "cwd")
    require(os.path.isabs(executable) and not executable.endswith(" (deleted)"))
    require(os.path.isabs(cwd) and not cwd.endswith(" (deleted)"))
    network_namespace = os.readlink(proc / "ns" / "net")
    require(network_namespace == os.readlink("/proc/self/ns/net"))
    creation = datetime.datetime.fromtimestamp(
        boot_seconds + int(before) / os.sysconf("SC_CLK_TCK"), datetime.timezone.utc
    ).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    receipt = {
        "platform": "linux", "pid": pid, "executable": executable,
        "cwd": cwd, "uid": proc.stat().st_uid, "creation": creation,
        "boot_id": boot_id, "start_ticks": before,
        "network_namespace": network_namespace, "port_owned": listener_owned(proc, port),
    }
    require(start_ticks((proc / "stat").read_text(), pid) == before and live(handle))
    return receipt


def validate_context(request):
    pid = request.get("pid")
    require(type(pid) is int and pid > 1)
    endpoint = urlsplit(request.get("server", ""))
    require(endpoint.scheme == "http" and endpoint.hostname in {"127.0.0.1", "localhost", "::1"})
    require(endpoint.port is not None and endpoint.port not in MANUAL_PORTS)
    require(endpoint.path in {"", "/"} and not endpoint.query and not endpoint.fragment)
    require(endpoint.username is None and endpoint.password is None)
    root, binary = request.get("root", ""), request.get("binary", "")
    require(os.path.isabs(root) and os.path.isdir(root))
    require(os.path.isabs(binary) and os.path.isfile(binary))
    return pid, endpoint.port, os.path.realpath(root), os.path.realpath(binary)


def operate(request):
    require_native()
    action = request.get("action")
    require(action in {"capabilities", "inspect", "stop"})
    if action == "capabilities":
        # Feature presence is insufficient: the running kernel must support it.
        handle = os.pidfd_open(os.getpid())
        try:
            signal.pidfd_send_signal(handle, 0)
            return {"platform": "linux", "pidfd": True}
        finally:
            os.close(handle)
    pid, port, root, binary = validate_context(request)
    # Acquire a stable kernel handle BEFORE inspecting /proc. If a process exits
    # and its number is reused, this handle can never signal its replacement.
    handle = os.pidfd_open(pid)
    try:
        receipt = inspect_process(pid, port, handle)
        require(receipt["executable"] == binary and receipt["cwd"] == root)
        require(receipt["uid"] == os.getuid())
        if action == "inspect":
            return receipt
        require(pid not in {os.getpid(), os.getppid()} and receipt["port_owned"])
        expected = request.get("expected")
        require(isinstance(expected, dict))
        for field in ("platform", "pid", "executable", "cwd", "uid", "creation",
                      "boot_id", "start_ticks", "network_namespace"):
            require(expected.get(field) == receipt[field])
        require(live(handle))
        signal.pidfd_send_signal(handle, signal.SIGTERM)
        poll = select.poll()
        poll.register(handle, select.POLLIN)
        if not poll.poll(30_000):
            raise TimeoutError("Owned server did not exit in 30 seconds; no force-stop was attempted.")
        return {"stopped": True, "pid": pid, "identity": receipt}
    finally:
        os.close(handle)


if __name__ == "__main__":
    try:
        raw = sys.stdin.read(65_537)
        require(len(raw) <= 65_536)
        request = json.loads(raw)
        require(isinstance(request, dict))
        print(json.dumps(operate(request)))
    except (OSError, ValueError, StopIteration, TypeError, KeyError):
        # Never reflect stdin, process environment, arguments or credential data.
        print(REFUSAL, file=sys.stderr)
        sys.exit(1)
