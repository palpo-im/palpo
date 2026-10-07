#!/usr/bin/env python3
"""Exercise real Matrix appservice retry on a disposable loopback Palpo.

Requires a running homeserver, its real database and private JSON accounts with
`admin` and `manager` entries (user_id/access_token). Creates an isolated room,
bot and temporary appservice; removes the appservice afterward. Never run this
against production. The receiver alone is a controlled HTTP peer.
"""
import argparse
import hashlib
import json
import secrets
import stat
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def binary_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True)
    parser.add_argument("--accounts", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    origin = urllib.parse.urlsplit(args.origin)
    if origin.scheme != "http" or origin.hostname != "127.0.0.1" or origin.path or origin.query or origin.fragment or origin.username:
        parser.error("origin must be an explicit loopback HTTP origin")
    info = args.accounts.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077 or info.st_size > 65536:
        parser.error("accounts must be a private regular JSON file, at most 64 KiB")
    accounts = json.loads(args.accounts.read_text())
    for role in ("admin", "manager"):
        if not accounts[role]["user_id"].endswith(":rinx-adr0011.test"):
            parser.error("only the isolated rinx-adr0011.test accounts are supported")
    binary_hash = binary_digest(args.binary)
    args.output.mkdir(parents=True, exist_ok=False)

    def request(path, body=None, token=None, method=None):
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        req = urllib.request.Request(args.origin + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            # Neither request credentials nor arbitrary server response bodies
            # belong in a public acceptance report.
            raise RuntimeError(f"{req.method} {urllib.parse.urlsplit(path).path}: HTTP {error.code}") from None

    records = []
    accepted = threading.Event()
    hs_token = secrets.token_hex(32)
    as_token = secrets.token_hex(32)
    tag = "adr_retry_" + secrets.token_hex(4)

    class Receiver(BaseHTTPRequestHandler):
        def do_PUT(self):
            url = urllib.parse.urlsplit(self.path)
            if urllib.parse.parse_qs(url.query).get("access_token") != [hs_token] and self.headers.get("Authorization") != "Bearer " + hs_token:
                self.send_error(401)
                return
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            # Matrix recalculates unsigned age on delivery; stable event content
            # and transaction identity, not elapsed time, must survive a retry.
            for event in payload["events"]:
                event.get("unsigned", {}).pop("age", None)
            digest = hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            status = 200 if accepted.is_set() else 503
            records.append({"transaction": url.path, "digest": digest, "status": status,
                            "eventIds": [event.get("event_id") for event in payload["events"]]})
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    registered = False
    report = {"passed": False, "binarySha256": binary_hash,
              "scope": "Actual Palpo Matrix and database; controlled HTTP appservice receiver"}
    try:
        request("/_palpo/admin/v1/appservices", {
            "id": tag, "url": f"http://127.0.0.1:{server.server_port}",
            "as_token": as_token, "hs_token": hs_token, "sender_localpart": tag,
            "namespaces": {"users": [{"exclusive": True, "regex": "^@" + tag + ".*:rinx-adr0011\\.test$"}]},
            "rate_limited": False,
        }, accounts["admin"]["access_token"])
        registered = True
        request("/_matrix/client/v3/account/whoami?" + urllib.parse.urlencode({"user_id": "@" + tag + ":rinx-adr0011.test"}), token=as_token)
        request("/_matrix/client/v3/login", {"type": "m.login.application_service", "identifier": {"type": "m.id.user", "user": tag}, "device_id": "ADR_RETRY"}, as_token)
        request("/_matrix/client/v3/createRoom", {"name": "ADR delivery retry", "preset": "private_chat", "invite": ["@" + tag + ":rinx-adr0011.test"]}, accounts["manager"]["access_token"])
        deadline = time.monotonic() + 20
        while not records and time.monotonic() < deadline:
            time.sleep(.1)
        assert records and records[0]["status"] == 503, "No failed delivery observed"
        failed = records[0]
        accepted.set()
        deadline = time.monotonic() + 75
        while time.monotonic() < deadline:
            if any(r["status"] == 200 and r["transaction"] == failed["transaction"] and r["digest"] == failed["digest"] for r in records):
                break
            time.sleep(.25)
        else:
            raise AssertionError("Refused transaction was not retried with identical events")
        assert binary_digest(args.binary) == binary_hash, "Binary changed during acceptance"
        report.update(passed=True, checks=["HTTP 503 retains the pending transaction", "The same transaction and events retry successfully with HTTP 200"])
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        try:
            if registered:
                request("/_palpo/admin/v1/appservices/" + tag, token=accounts["admin"]["access_token"], method="DELETE")
        finally:
            server.shutdown()
            report["records"] = records
            (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: actual Matrix appservice retry")


if __name__ == "__main__":
    main()
