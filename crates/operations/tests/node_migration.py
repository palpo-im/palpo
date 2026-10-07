#!/usr/bin/env python3
"""Exercise the actual Node -> Rust executable -> Node database boundary.

Only temporary fixture databases and loopback listeners are used. Node is a
migration test dependency, never part of the Rust executable's runtime.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import URLError
from urllib.request import ProxyHandler, build_opener


SEED = r"""
import {pathToFileURL} from 'node:url';
const {Store} = await import(pathToFileURL(process.argv[1]));
const {Service, Palpo} = await import(pathToFileURL(process.argv[2]));
const store = new Store(process.argv[3]);
new Service({store, palpo: new Palpo('https://matrix.example.test'), serverName: 'example.test'});
store.state.fleets.legacy = {
  id: 'legacy', owner: '@provider:example.test', state: 'ready',
  registration: {as_token: 'FAKE_FIXTURE_AS_TOKEN', hs_token: 'FAKE_FIXTURE_HS_TOKEN'},
  agents: {agent1: {state: 'approved', requestId: 'old-request'}}
};
store.state.actionInbox = {records: {
  pending_project:{id:'pending_project',requestId:'old-pending',kind:'project',state:'requested',execution:'pending',ownerMxid:'@manager:example.test',payload:{fleetId:'legacy',name:'Pending project'},revision:1},
  approved_project:{id:'approved_project',requestId:'old-approved',kind:'project',state:'approved',execution:'done',ownerMxid:'@manager:example.test',payload:{fleetId:'legacy',name:'Approved project'},result:{projectId:'legacy_project'},decision:{by:'@admin:example.test',at:100,reason:'Original verdict'},revision:2}
}, notices:{}, rooms:{}};
store.state.projects = {legacy_project:{id:'legacy_project',fleetId:'legacy',ownerMxid:'@manager:example.test'}};
store.state.requests = {'legacy:old-request':{id:'legacy:old-request',fleetId:'legacy',requestId:'old-request',projectId:'legacy_project',provider:{engagementId:'native_agent_allocation'},state:'active'}};
store.state.accountAccess = {requests:{signup:{id:'signup',status:'approved',password:{iv:'FIXTURE',data:'FIXTURE',tag:'FIXTURE'}}},cursor:'original_sync_cursor'};
store.state.unknownExtension = {version: 7, preserve: ['原有记录', {count: 42}]};
store.audit('@admin:example.test', 'fixture', 'legacy', 'agent1', 'success');
store.db.prepare(`INSERT INTO fleet_delivery
  (fleet,generation,lane,id,kind,digest,payload,bytes,consumer,token,expires,acked)
  VALUES (?,?,?,?,?,?,?,?,?,?,?,?)`).run(
    'legacy', 2, 'work', 'old-command', 'fixture', 'a'.repeat(64),
    '{"keep":true}', 13, 'consumer', 'FAKE_FIXTURE_LEASE', 100, null);
store.close();
"""

REOPEN = r"""
import {pathToFileURL} from 'node:url';
const {Store} = await import(pathToFileURL(process.argv[1]));
const store = new Store(process.argv[2]);
if (!store.state.rustWorkflows || !store.state.fleets.legacy) throw Error('Migration lost state');
store.save();
store.close();
"""

INSPECT_FENCED = r"""
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
import {DatabaseSync} from 'node:sqlite';
const {Store} = await import(pathToFileURL(process.argv[1]));
const path = process.argv[2];
assert.throws(() => new Store(path), /Rust owns/);
const store = new Store(path,{readOnly:true});
assert.equal(store.state.requests['legacy:old-request'].provider.engagementId,'native_agent_allocation');
assert.equal(store.state.accountAccess.cursor,'original_sync_cursor');
assert.throws(() => store.save(), /inspection only/);
store.close();
// Simulate an old, unpatched binary which knows nothing of the ownership marker.
const old = new DatabaseSync(path);
for (const sql of ['UPDATE state SET body=body','DELETE FROM fleet_delivery','DELETE FROM workflow_writer','DELETE FROM workflow_migrations'])
  assert.throws(() => old.exec(sql), /palpo_rust_workflow_writer_v1/);
old.close();
"""


def run(command, env=None, success=True):
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
    # Do not include process output: a regression must not print a fixture DB's
    # credential fields, nor train operators to paste their own database output.
    if (result.returncode == 0) != success:
        raise AssertionError(f"Unexpected process exit: {result.returncode}")
    return result


def snapshot(path):
    with sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True) as db:
        state = json.loads(db.execute("SELECT body FROM state WHERE id=1").fetchone()[0])
        deliveries = db.execute("SELECT * FROM fleet_delivery ORDER BY ordinal").fetchall()
    return state, deliveries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", default="node", help="Node 24 or newer")
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    binary = str(args.binary.resolve(strict=True))
    root = Path(__file__).resolve().parents[3]
    node_store = str(root / "crates/operations/tests/fixtures/legacy_node/lib/store.mjs")
    node_service = str(root / "crates/operations/tests/fixtures/legacy_node/lib/service.mjs")

    with tempfile.TemporaryDirectory(prefix="palpo-rust-migration-") as directory:
        directory = Path(directory)
        database = directory / "admin.sqlite"
        lock = directory / "admin.sqlite.lock"
        authority = directory / "authority.json"
        authority.write_text(json.dumps({"engagements": {}, "resources": {}, "projects": {}}))
        run([args.node, "--input-type=module", "-e", SEED, node_store, node_service, str(database)])
        original, delivery_rows = snapshot(database)
        env = {
            **{key: value for key, value in os.environ.items() if not key.startswith("PALPO_")},
            "PALPO_SERVER_NAME": "example.test",
            "PALPO_URL": "https://matrix.example.test",
            "PALPO_ADMIN_DATABASE": str(database),
        }
        command = [binary, "import-authority", str(authority)]
        # Use precisely the lock filename/content created by server.mjs.
        lock.write_text(f"{os.getpid()}\n")
        result = run(command, env, success=False)
        assert "workflow_database_in_use" in result.stderr
        assert lock.read_text() == f"{os.getpid()}\n"
        assert snapshot(database) == (original, delivery_rows)
        lock.unlink()

        run(command, env)
        assert not lock.exists(), "Import retained the process lock"
        migrated, migrated_rows = snapshot(database)
        legacy = dict(migrated)
        workflows = legacy.pop("rustWorkflows")
        assert workflows["authority"] == {"engagements": {}, "resources": {}, "projects": {}}
        assert legacy == original, "Rust changed legacy JSON records"
        assert migrated_rows == delivery_rows, "Rust changed the Node delivery table"
        if os.name == "posix":
            assert database.stat().st_mode & 0o777 == 0o600
        run([args.node, "--input-type=module", "-e", REOPEN, node_store, str(database)])
        assert snapshot(database) == (migrated, delivery_rows), "Node lost Rust state"

        inventory_command = [binary, 'migration-inventory']
        inventory = json.loads(run(inventory_command, env).stdout)
        assert inventory['counts']['requests'] == 1 and inventory['counts']['accounts'] == 1
        assert inventory['counts']['actions'] == 2
        assert inventory['deliveryCount'] == 1
        assert any(row['agentAllocationId'] == 'native_agent_allocation' and row['serverEngagementId'] == 'legacy' for row in inventory['mappings'])
        assert 'FAKE_FIXTURE' not in json.dumps(inventory), 'Inventory disclosed credential contents'
        # The audit is explicit fixture evidence, not a fabricated capacity grant.
        plan = {'version':1,'id':'reviewed_fixture_handoff','inventory':inventory,
                'nativeAuditDigest':hashlib.sha256(b'isolated native accounting fixture audit').hexdigest()}
        handoff = directory / 'handoff.json'
        handoff.write_text(json.dumps(plan))
        transfer = [binary, 'handoff-store', str(handoff)]
        receipt = json.loads(run(transfer, env).stdout)
        assert snapshot(database) == (migrated, delivery_rows), 'Handoff rewrote original state or delivery custody'
        run([args.node, '--input-type=module', '-e', INSPECT_FENCED, node_store, str(database)])
        refused_node = run([args.node, str(root / 'crates/operations/tests/fixtures/legacy_node/server.mjs')], dict(env, PUBLIC_ORIGIN='http://127.0.0.1:8090'), success=False)
        assert 'Rust owns' in refused_node.stderr
        assert not lock.exists(), 'A refused old Node startup stranded the service lock'
        assert json.loads(run(transfer, env).stdout) == receipt
        # SQLite restore copies the current database, including ownership and
        # handoff receipts. A copied fixture can reopen/replay without Node writes.
        restored = directory / 'restored.sqlite'
        with sqlite3.connect(database) as source, sqlite3.connect(restored) as target:
            source.backup(target)
        restored_env = dict(env, PALPO_ADMIN_DATABASE=str(restored))
        assert json.loads(run(transfer, restored_env).stdout) == receipt
        run([args.node, '--input-type=module', '-e', INSPECT_FENCED, node_store, str(restored)])

        # Launch the actual compiled server, check its loopback HTTP endpoint,
        # terminate gracefully, and confirm that the database can be reopened.
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        env["PALPO_OPERATIONS_LISTEN"] = f"127.0.0.1:{port}"
        env["PUBLIC_ORIGIN"] = f"http://127.0.0.1:{port}"
        opener = build_opener(ProxyHandler({}))
        with subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
            try:
                deadline = time.monotonic() + 15
                while True:
                    assert process.poll() is None, "Rust HTTP server exited before readiness"
                    try:
                        with opener.open(f"http://127.0.0.1:{port}/healthz", timeout=1) as response:
                            assert response.status == 200 and response.read() == b"ok"
                            break
                    except URLError:
                        if time.monotonic() >= deadline:
                            raise AssertionError("Rust HTTP server did not become ready") from None
                        time.sleep(0.05)
                result = run(command, env, success=False)
                assert "workflow_database_in_use" in result.stderr
                process.send_signal(signal.SIGTERM)
                assert process.wait(timeout=15) == 0, "Rust server did not exit cleanly"
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
        assert not lock.exists(), "Graceful shutdown retained the database lock"
        run(command, env)
        assert snapshot(database) == (migrated, delivery_rows)
    print("PASS: Node -> Rust ownership, stable identity mapping, private inventory, unchanged custody, fenced Node reads, copied restore/replay, HTTP startup and SIGTERM cleanup")


if __name__ == "__main__":
    main()
