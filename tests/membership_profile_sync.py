#!/usr/bin/env python3
"""Verify profile updates do not create false sync gaps on disposable Palpo.

Uses real Matrix HTTP and storage. Requires private manager/owner test accounts
on the loopback rinx-adr0011.test homeserver; never targets a deployed server.
"""
import argparse
import hashlib
import json
import secrets
import stat
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True)
    parser.add_argument("--accounts", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    origin = urllib.parse.urlsplit(args.origin)
    if (origin.scheme != "http" or origin.hostname != "127.0.0.1"
            or origin.path or origin.query or origin.fragment or origin.username):
        parser.error("origin must be an explicit loopback HTTP origin")
    info = args.accounts.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077 or info.st_size > 65536:
        parser.error("accounts must be a private regular JSON file, at most 64 KiB")
    accounts = json.loads(args.accounts.read_text())
    for role in ("manager", "owner"):
        if not accounts[role]["user_id"].endswith(":rinx-adr0011.test"):
            parser.error("only isolated rinx-adr0011.test accounts are supported")
    digest = hashlib.sha256()
    with args.binary.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)

    def request(role, path, body=None, method=None):
        req = urllib.request.Request(
            args.origin + "/_matrix/client/v3/" + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json",
                     "Authorization": "Bearer " + accounts[role]["access_token"]},
            method=method,
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"{req.method} {req.selector.split('?')[0]}: HTTP {error.code}") from None

    manager = accounts["manager"]["user_id"]
    owner = accounts["owner"]["user_id"]
    room = request("manager", "createRoom", {
        "name": "Membership profile sync regression " + secrets.token_hex(4),
        "preset": "private_chat", "invite": [owner],
        "power_level_content_override": {"users": {manager: 100, owner: 100}},
    })["room_id"]
    encoded_room = urllib.parse.quote(room, safe="")
    request("owner", "join/" + encoded_room, {})
    checks = []

    def check(name, passed, **evidence):
        checks.append({"name": name, "passed": passed, **evidence})
        print(("PASS " if passed else "FAIL ") + name, flush=True)

    def sync(role, since=None, limit=200):
        query = {"timeout": 0, "full_state": "true", "filter": json.dumps({
            "room": {"rooms": [room], "timeline": {"limit": limit},
                     "ephemeral": {"types": []}}, "presence": {"types": []},
        })}
        if since:
            query["since"] = since
        return request(role, "sync?" + urllib.parse.urlencode(query))

    def timeline(response):
        return response["rooms"]["join"][room]["timeline"]

    def send(body):
        return request("owner", "rooms/" + encoded_room + "/send/m.room.message/"
                       + secrets.token_hex(8), {"msgtype": "m.text", "body": body}, "PUT")["event_id"]

    member_path = "rooms/" + encoded_room + "/state/m.room.member/" + urllib.parse.quote(manager, safe="")
    report = {"passed": False, "binarySha256": digest.hexdigest(), "roomId": room, "checks": checks}
    try:
        cursors = {role: sync(role)["next_batch"] for role in ("manager", "owner")}
        changes = [{"displayname": "Renamed once"}, {"displayname": "Renamed twice"},
                   {"avatar_url": "mxc://rinx-adr0011.test/profile-regression"}]
        for index, change in enumerate(changes):
            content = request("manager", member_path)
            assert content["membership"] == "join"
            content.update(change)
            event_id = request("manager", member_path, content, "PUT")["event_id"]
            for role in ("manager", "owner"):
                response = sync(role, cursors[role])
                events = timeline(response)
                check(f"profile_{index}_{role}_continuous", events.get("limited") is False,
                      limited=events.get("limited"), eventCount=len(events["events"]))
                check(f"profile_{index}_{role}_visible",
                      any(e["event_id"] == event_id and all(e["content"].get(k) == v for k, v in change.items())
                          for e in events["events"]))
                cursors[role] = response["next_batch"]
            event_id = send("Message after profile change " + str(index))
            response = sync("manager", cursors["manager"])
            events = timeline(response)
            check(f"message_after_profile_{index}", events.get("limited") is False
                  and any(e["event_id"] == event_id for e in events["events"]))
            cursors["manager"] = response["next_batch"]

        # Several updates before the next sync exercise the uncached history
        # chain, as happens when a client reconnects after being offline.
        changed_events = set()
        for name in ("Batched rename one", "Batched rename two"):
            content = request("manager", member_path)
            content["displayname"] = name
            changed_events.add(request("manager", member_path, content, "PUT")["event_id"])
        for role in ("manager", "owner"):
            response = sync(role, cursors[role])
            events = timeline(response)
            check(f"batched_profiles_{role}_continuous", events.get("limited") is False)
            check(f"batched_profiles_{role}_visible",
                  changed_events.issubset({e["event_id"] for e in events["events"]}))
            cursors[role] = response["next_batch"]

        # Leaving and rejoining between syncs really does start a new joined
        # period. A fix must not suppress that gap just because prior state
        # at the old cursor also said "join".
        old_cursor = cursors["manager"]
        request("manager", "rooms/" + encoded_room + "/leave", {})
        request("owner", "rooms/" + encoded_room + "/invite", {"user_id": manager})
        request("manager", "join/" + encoded_room, {})
        response = sync("manager", old_cursor)
        events = timeline(response)
        check("real_rejoin_remains_limited", events.get("limited") is True
              and len(events["events"]) < 200, eventCount=len(events["events"]))
        cursors["manager"] = response["next_batch"]
        content = request("manager", member_path)
        content["displayname"] = "Renamed after rejoin"
        request("manager", member_path, content, "PUT")
        response = sync("manager", cursors["manager"])
        check("profile_after_rejoin_continuous", timeline(response).get("limited") is False)
        cursors["manager"] = response["next_batch"]

        # Real timeline truncation must still be signalled and its omitted
        # messages must remain available to normal backwards pagination.
        messages = [send("Genuine gap " + str(i)) for i in range(5)]
        response = sync("manager", cursors["manager"], limit=2)
        events = timeline(response)
        check("real_truncation_remains_limited", events.get("limited") is True)
        page = request("manager", "rooms/" + encoded_room + "/messages?" + urllib.parse.urlencode({
            "dir": "b", "from": events["prev_batch"], "limit": 20,
        }))
        recovered = {e["event_id"] for e in events["events"] + page["chunk"]}
        check("truncated_messages_recoverable", set(messages).issubset(recovered))
        report["passed"] = all(c["passed"] for c in checks)
    finally:
        report_path = args.output / "report.json"
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        report_path.chmod(0o600)
        for role in ("manager", "owner"):
            request(role, "rooms/" + encoded_room + "/leave", {})
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
