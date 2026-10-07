import { DatabaseSync } from 'node:sqlite';
import { mkdirSync, chmodSync } from 'node:fs';
import { dirname } from 'node:path';

// One admin service process per database. Matrix calls are serialized by Service.
// Persist the operation plan before calling Palpo so a retry keeps its identity.
export class Store {
  constructor(path, { readOnly = false } = {}) {
    this.readOnly = readOnly;
    if (!readOnly && path !== ':memory:') mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    this.db = new DatabaseSync(path, { readOnly });
    try {
      if (!readOnly) {
        this.requireLegacyWriter();
        if (path !== ':memory:') chmodSync(path, 0o600);
        this.db.exec('PRAGMA journal_mode = DELETE; CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK (id = 1), body TEXT NOT NULL)');
      }
      const row = this.db.prepare('SELECT body FROM state WHERE id = 1').get();
      this.state = row ? JSON.parse(row.body) : { version: 1, fleets: {}, audit: [] };
      if (this.state.version !== 1) throw new Error('Unsupported admin database version');
    } catch (error) { this.db.close(); throw error; }
  }
  requireLegacyWriter() {
    if (this.readOnly) throw new Error('This workflow database is open for inspection only');
    if (this.db.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_writer'").get()
      && this.db.prepare('SELECT 1 FROM workflow_writer WHERE id=1').get()) {
      throw new Error('Rust owns this workflow database. Keep the current database and use a compatible Rust service; do not restore an old snapshot.');
    }
  }
  save() {
    this.requireLegacyWriter();
    this.db.prepare('INSERT INTO state (id, body) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET body = excluded.body').run(JSON.stringify(this.state));
  }
  audit(actor, action, fleetId, objectId, result) {
    this.requireLegacyWriter();
    this.state.audit.push({ at: new Date().toISOString(), actor, action, fleetId, objectId, result });
    this.save();
  }
  atomic(fn) {
    this.requireLegacyWriter();
    const before = structuredClone(this.state);
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = fn();
      if (result?.then) throw new Error('SQLite atomic operations must not await external work');
      this.save(); this.db.exec('COMMIT'); return result;
    } catch (error) { this.db.exec('ROLLBACK'); this.state = before; throw error; }
  }
  close() { this.db.close(); }
}
