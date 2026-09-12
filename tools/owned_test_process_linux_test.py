"""Native pidfd tests. Only disposable local Python/socket children are created."""

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


HELPER = Path(__file__).with_name("owned_test_process_linux.py")
spec = importlib.util.spec_from_file_location("owned_test_process_linux", HELPER)
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)

SOCKET_CHILD = """
import socket,time
s=socket.socket()
s.bind(('127.0.0.1',0))
s.listen()
print(s.getsockname()[1],flush=True)
time.sleep(120)
"""


@unittest.skipUnless(sys.platform == "linux", "Linux-native process handles required")
class OwnershipTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ecorp-pidfd-test-")
        self.addCleanup(self.directory.cleanup)

    def child(self):
        child = subprocess.Popen([sys.executable, "-u", "-c", SOCKET_CHILD],
                                 cwd=self.directory.name, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, text=True)
        handle = os.pidfd_open(child.pid)

        def cleanup():
            try:
                if native.live(handle):
                    signal.pidfd_send_signal(handle, signal.SIGTERM)
                child.wait(timeout=5)
            finally:
                os.close(handle)
                child.stdout.close()
                child.stderr.close()
        self.addCleanup(cleanup)
        port = int(child.stdout.readline())
        request = {"action": "inspect", "pid": child.pid,
                   "server": "http://127.0.0.1:" + str(port),
                   "root": self.directory.name, "binary": os.path.realpath(sys.executable)}
        return child, request

    def test_kernel_capabilities_are_exercised(self):
        self.assertEqual(native.operate({"action": "capabilities"}), {"platform": "linux", "pidfd": True})

    def test_start_ticks_do_not_whitespace_split_process_names(self):
        fields = ["S"] + ["0"] * 18 + ["98765"]
        self.assertEqual(native.start_ticks("123 (odd ) process\nname) " + " ".join(fields), 123), "98765")
        for raw in ["123 no delimiters", "321 (name) " + " ".join(fields),
                    "123 (name) Z " + " ".join(fields[1:]), "123 (name) S 0"]:
            with self.assertRaises(ValueError):
                native.start_ticks(raw, 123)

    def test_real_receipt_and_handle_scoped_stop(self):
        child, request = self.child()
        receipt = native.operate(request)
        self.assertTrue(receipt["port_owned"])
        self.assertEqual(receipt["pid"], child.pid)
        self.assertEqual(receipt["uid"], os.getuid())
        self.assertEqual(receipt["cwd"], self.directory.name)
        result = native.operate({**request, "action": "stop", "expected": receipt})
        self.assertTrue(result["stopped"])
        child.wait(timeout=5)

    def test_stale_receipts_never_signal_the_live_child(self):
        child, request = self.child()
        receipt = native.operate(request)
        for field, value in [
            ("pid", child.pid + 1), ("boot_id", "00000000-0000-0000-0000-000000000000"),
            ("start_ticks", "0"), ("creation", "2000-01-01T00:00:00.000Z"),
            ("uid", os.getuid() + 1), ("cwd", "/different"), ("executable", "/other/python"),
            ("network_namespace", "net:[0]"), ("platform", "win32"),
        ]:
            changed = {**receipt, field: value}
            with mock.patch.object(signal, "pidfd_send_signal", wraps=signal.pidfd_send_signal) as send:
                with self.assertRaises(ValueError):
                    native.operate({**request, "action": "stop", "expected": changed})
                send.assert_not_called()
            self.assertIsNone(child.poll())

    def test_foreign_listener_is_not_process_ownership(self):
        child, request = self.child()
        other, other_request = self.child()
        receipt = native.operate(request)
        foreign = {**request, "server": other_request["server"]}
        self.assertFalse(native.operate(foreign)["port_owned"])
        with self.assertRaises(ValueError):
            native.operate({**foreign, "action": "stop", "expected": receipt})
        self.assertIsNone(child.poll())
        self.assertIsNone(other.poll())

    def test_manual_remote_non_origin_and_legacy_targets_are_refused(self):
        child, request = self.child()
        for change in [
            {"pid": 0}, {"pid": True}, {"pid": "123"}, {"root": "/"},
            {"binary": "/bin/true"}, {"server": "http://127.0.0.1:8791"},
            {"server": "http://127.0.0.1:54329"}, {"server": "http://example.com:18471"},
            {"server": request["server"] + "/path"}, {"server": request["server"] + "?query"},
            {"server": request["server"].replace("http://", "http://user:secret@")},
        ]:
            with self.assertRaises((ValueError, OSError)):
                native.operate({**request, **change})
        with self.assertRaises(ValueError):
            native.operate({**request, "action": "stop", "expected": {"pid": child.pid}})
        self.assertIsNone(child.poll())

    def test_missing_native_support_fails_closed_without_numeric_kill(self):
        with mock.patch.object(os, "pidfd_open", None), mock.patch.object(os, "kill") as kill:
            with self.assertRaises(ValueError):
                native.operate({"action": "capabilities"})
            kill.assert_not_called()

    def test_stop_timeout_never_escalates_to_force_or_numeric_pid_signalling(self):
        child, request = self.child()
        receipt = native.operate(request)
        original_poll = native.select.poll

        class TimeoutPoll:
            def __init__(self):
                self.poller = original_poll()

            def register(self, *args):
                return self.poller.register(*args)

            def poll(self, timeout):
                return [] if timeout == 30_000 else self.poller.poll(timeout)

        with mock.patch.object(native.select, "poll", TimeoutPoll), \
             mock.patch.object(signal, "pidfd_send_signal") as send, \
             mock.patch.object(os, "kill") as kill:
            with self.assertRaises(TimeoutError):
                native.operate({**request, "action": "stop", "expected": receipt})
            send.assert_called_once()
            self.assertEqual(send.call_args.args[1], signal.SIGTERM)
            kill.assert_not_called()
        self.assertIsNone(child.poll())

    def test_cli_receipts_do_not_require_or_disclose_environment_credentials(self):
        child, request = self.child()
        result = subprocess.run([sys.executable, "-B", str(HELPER)], input=json.dumps(request),
                                text=True, capture_output=True, timeout=10,
                                env={**os.environ, "DATABASE_URL": "sentinel-not-a-real-credential"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)["port_owned"])
        self.assertNotIn("sentinel-not-a-real-credential", result.stdout + result.stderr)
        self.assertIsNone(child.poll())


if __name__ == "__main__":
    unittest.main()
