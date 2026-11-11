#!/usr/bin/env python3
"""Closed-loop HTTP load probe for a disposable loopback Palpo homeserver.

Prepare once with --prepare, then reuse the fixture for before/after runs.
Credentials stay in the fixture; reports contain only timings and binary hashes.
"""
import argparse
import concurrent.futures
import hashlib
import http.client
import json
import math
import threading
import time
import urllib.parse
import uuid
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", default="http://127.0.0.1:24686")
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--include-writes", action="store_true", help="Also send messages into a separate room")
    parser.add_argument("--password", help="Disposable owner account password, prepare only")
    parser.add_argument("--requests", type=int, default=64)
    parser.add_argument("--concurrency", type=int, nargs="+", default=[1, 8, 32])
    args = parser.parse_args()
    origin = urllib.parse.urlsplit(args.origin)
    if (origin.scheme != "http" or origin.hostname != "127.0.0.1" or origin.path
            or origin.query or origin.fragment or origin.username):
        parser.error("origin must be a loopback HTTP origin")
    if args.requests < 1 or any(n < 1 for n in args.concurrency):
        parser.error("requests and concurrency must be positive")
    if not args.binary.is_file() or args.output.exists():
        parser.error("binary must exist and output must be a new file")
    local = threading.local()

    def request(path, token=None, body=None, method="GET", fresh=False):
        if not hasattr(local, "connection"):
            local.connection = http.client.HTTPConnection(origin.hostname, origin.port, timeout=60)
        connection = (http.client.HTTPConnection(origin.hostname, origin.port, timeout=60)
                      if fresh else local.connection)
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        try:
            connection.request(method, path if path.startswith("/") else "/_matrix/client/" + path,
                               None if body is None else json.dumps(body), headers)
            response = connection.getresponse()
            payload = response.read()
        except (OSError, http.client.HTTPException):
            connection.close()
            raise
        finally:
            if fresh:
                connection.close()
        if response.status != 200:
            raise RuntimeError(f"HTTP {response.status} for {method} {path.split('?')[0]}")
        return json.loads(payload)

    if args.prepare:
        if not args.password or args.fixture.exists():
            parser.error("prepare requires --password and a new fixture path")
        login = request("v3/login", body={"type": "m.login.password",
                        "identifier": {"type": "m.id.user", "user": "owner"},
                        "password": args.password}, method="POST")
        if not login["user_id"].endswith(".test"):
            parser.error("prepare requires a disposable .test homeserver")
        token = login["access_token"]
        room = request("v3/createRoom", token, {"preset": "private_chat",
                       "name": "Performance fixture"}, "POST")["room_id"]
        room_path = "v3/rooms/" + urllib.parse.quote(room, safe="")
        for i in range(120):
            request(room_path + "/send/m.room.message/perf-" + str(i), token,
                    {"msgtype": "m.text", "body": "Load fixture " + str(i)}, "PUT")
        writer_room = request("v3/createRoom", token, {"preset": "private_chat",
                              "name": "Performance write fixture"}, "POST")["room_id"]
        fixture = {"access_token": token, "room_id": room, "writer_room_id": writer_room}
        args.fixture.parent.mkdir(parents=True, exist_ok=True)
        args.fixture.write_text(json.dumps(fixture), encoding="utf-8")
        args.fixture.chmod(0o600)
    fixture = json.loads(args.fixture.read_text(encoding="utf-8"))
    token = fixture["access_token"]
    room_path = "v3/rooms/" + urllib.parse.quote(fixture["room_id"], safe="")
    sync_filter = json.dumps({"room": {"rooms": [fixture["room_id"]],
                             "timeline": {"limit": 20}}, "presence": {"types": []}})
    sync_path = "v3/sync?" + urllib.parse.urlencode({"timeout": 0, "filter": sync_filter})
    since = request(sync_path, token)["next_batch"]
    scenarios = [("versions", "versions", None),
                 ("messages_20", room_path + "/messages?dir=b&limit=20", 20),
                 ("messages_100", room_path + "/messages?dir=b&limit=100", 100),
                 ("sync_initial", sync_path, None),
                 ("sync_idle", sync_path + "&since=" + urllib.parse.quote(since), None)]
    runs = [(n, scenarios) for n in args.concurrency]
    if args.include_writes:
        writer_path = "v3/rooms/" + urllib.parse.quote(fixture["writer_room_id"], safe="")
        writes = [("send_message", writer_path + "/send/m.room.message/", None)]
        runs.extend((n, writes) for n in args.concurrency)
    report = {"binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "requests_per_scenario": args.requests, "message_response_sha256": {}, "results": []}
    def diagnostics():
        return request("/diagnostics", fresh=True)

    report["database_pools"] = diagnostics()["readiness"]["database_pools"]
    def stable(value):
        if isinstance(value, dict):
            return {key: stable({k: v for k, v in item.items() if k != "age"}
                                if key == "unsigned" and isinstance(item, dict) else item)
                    for key, item in value.items()}
        if isinstance(value, list):
            return [stable(item) for item in value]
        return value
    for name, path, expected_count in scenarios:
        if expected_count is not None:
            response = request(path, token)
            encoded = json.dumps(stable(response), sort_keys=True, separators=(",", ":")).encode()
            report["message_response_sha256"][name] = hashlib.sha256(encoded).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for concurrency, group in runs:
        for name, path, expected_count in group:
            with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
                def sample(_):
                    start = time.perf_counter()
                    try:
                        if name == "send_message":
                            body = request(path + uuid.uuid4().hex, token,
                                           {"msgtype": "m.text", "body": "Load write"}, "PUT")
                            assert "event_id" in body
                        else:
                            body = request(path, token)
                        if expected_count is not None:
                            assert len(body["chunk"]) == expected_count
                            assert len({e["event_id"] for e in body["chunk"]}) == expected_count
                        if name == "sync_initial":
                            assert body["next_batch"]
                            timeline = body["rooms"]["join"][fixture["room_id"]]["timeline"]
                            assert len(timeline["events"]) == 20
                        if name == "sync_idle":
                            assert body["next_batch"]
                            rooms = body.get("rooms", {}).get("join", {})
                            assert not rooms.get(fixture["room_id"], {}).get("timeline", {}).get("events")
                        return (time.perf_counter() - start) * 1000, None
                    except Exception as error:
                        return (time.perf_counter() - start) * 1000, f"{type(error).__name__}: {error}"
                warmup = list(pool.map(sample, range(concurrency)))
                if any(error for _, error in warmup):
                    raise RuntimeError(f"{name} warmup failed: {[e for _, e in warmup if e]}")
                before = diagnostics()["query_pool_wait"]
                start = time.perf_counter()
                samples = list(pool.map(sample, range(args.requests)))
                elapsed = time.perf_counter() - start
                after = diagnostics()["query_pool_wait"]
                timings = sorted(ms for ms, error in samples if error is None)
                errors = [error for _, error in samples if error is not None]
                def percentile(p):
                    return round(timings[max(0, math.ceil(len(timings) * p) - 1)], 3) if timings else None
                row = {"scenario": name, "concurrency": concurrency, "errors": len(errors),
                       "error_types": sorted(set(errors)), "successful_rps": round(len(timings) / elapsed, 3),
                       "p50_ms": percentile(.5), "p95_ms": percentile(.95), "p99_ms": percentile(.99)}
                row["query_pool_delta"] = {key: after[key] - before[key]
                                           for key in ("completed", "failed", "cancelled", "total_us")}
                report["results"].append(row)
                print(json.dumps(row), flush=True)
                row["successful_latencies_ms"] = [round(ms, 3) for ms in timings]
                args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if any(row["errors"] for row in report["results"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
