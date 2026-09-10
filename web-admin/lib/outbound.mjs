import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { ApiError } from './service.mjs';

export const canonical = value => JSON.stringify(value, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);
const digest = value => createHash('sha256').update(canonical(value)).digest('hex');
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const sameSecret = (left, right) => typeof left === 'string' && typeof right === 'string'
  && Buffer.byteLength(left) === Buffer.byteLength(right) && timingSafeEqual(Buffer.from(left), Buffer.from(right));
const validId = value => typeof value === 'string' && value.length > 0 && value.length <= 255 && !/[\x00-\x1f]/.test(value);
export const isOutbound = fleet => fleet?.transport?.mode === 'outbound';
export const outboundProven = fleet => isOutbound(fleet) && fleet.connection?.generation === fleet.transport.generation && !!fleet.connection?.verifiedAt;
export const outboundOnline = fleet => isOutbound(fleet) && Date.now() - Date.parse(fleet.transport.lastSeenAt ?? '') < 90000;
export const outboundStatusCurrent = (fleet, request) => {
  const status = request.outboundStatus, received = Date.parse(status?.receivedAt ?? ''), observed = Date.parse(status?.observedAt ?? '');
  // A frozen publication may first arrive long after Hagency observed it. Its
  // delivery timestamp cannot make that old observation current again. Allow a
  // small clock skew, but cap the effective timestamp at local receipt time.
  return status?.generation === fleet.transport.generation && Number.isFinite(observed)
    && observed <= received + 5000 && Date.now() - Math.min(observed, received) < 90000;
};

export class Outbound {
  constructor(service, { leaseMs = 30000, maxPending = 1000, maxRecords = 10000, maxBytes = 16 * 1024 * 1024 } = {}) {
    this.service = service; this.store = service.store; this.leaseMs = leaseMs;
    this.maxPending = maxPending; this.maxRecords = maxRecords; this.maxBytes = maxBytes;
    this.store.db.exec(`CREATE TABLE IF NOT EXISTS fleet_delivery (
      ordinal INTEGER PRIMARY KEY AUTOINCREMENT, fleet TEXT NOT NULL, generation INTEGER NOT NULL,
      lane TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL, digest TEXT NOT NULL,
      payload TEXT, bytes INTEGER NOT NULL, consumer TEXT, token TEXT, expires INTEGER, acked INTEGER,
      UNIQUE(fleet,generation,lane,id));
      CREATE INDEX IF NOT EXISTS fleet_delivery_pending ON fleet_delivery(fleet,generation,lane,acked,ordinal)`);
  }
  configured() {
    if (!this.service.transportOrigin || !this.service.relayOrigin) fail(501, 'outbound_unconfigured', 'The server operator must configure the public outbound transport and internal Matrix relay origins.');
  }
  transport(id, generation = 1) {
    this.configured();
    return { mode: 'outbound', url: `${this.service.transportOrigin}/api/fleet/v2/${id}`, token: randomBytes(32).toString('base64url'), generation, sequence: 0 };
  }
  relayUrl(id) { this.configured(); return `${this.service.relayOrigin}/api/relay/v2/${id}`; }
  usage(fleet) {
    const totals = this.store.db.prepare('SELECT count(*) records, coalesce(sum(acked IS NULL),0) pending, coalesce(sum(CASE WHEN acked IS NULL THEN bytes ELSE 0 END),0) bytes FROM fleet_delivery WHERE fleet=?').get(fleet.id);
    return { ...totals, limits: { records: this.maxRecords, pending: this.maxPending, bytes: this.maxBytes } };
  }
  authenticate(id, token, generation, relay = false) {
    const fleet = this.service.store.state.fleets[id];
    if (!fleet || !isOutbound(fleet) || !['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed'
      || !sameSecret(token, relay ? fleet.registration.hs_token : fleet.transport.token)) fail(401, 'transport_unauthorized', 'The fleet transport credential is not active.');
    if (!relay && (!/^[1-9][0-9]*$/.test(String(generation ?? '')) || Number(generation) !== fleet.transport.generation)) fail(409, 'generation_conflict', 'Import the current fleet transport generation.');
    return fleet;
  }
  enqueue(fleet, lane, kind, id, payload) {
    if (!validId(id) || !['matrix', 'work'].includes(lane)) fail(400, 'invalid_delivery', 'Invalid delivery identity or lane.');
    const key = [fleet.id, fleet.transport.generation, lane, id], hash = digest(payload);
    const existing = lane === 'matrix'
      ? this.store.db.prepare("SELECT digest FROM fleet_delivery WHERE fleet=? AND lane='matrix' AND id=? ORDER BY ordinal DESC LIMIT 1").get(fleet.id, id)
      : this.store.db.prepare('SELECT digest FROM fleet_delivery WHERE fleet=? AND generation=? AND lane=? AND id=?').get(...key);
    if (existing) {
      if (existing.digest !== hash) fail(409, 'delivery_conflict', 'The delivery identity already has different content.');
      return id;
    }
    const raw = canonical(payload), bytes = Buffer.byteLength(raw);
    const count = this.usage(fleet);
    if (count.records >= this.maxRecords || count.pending >= this.maxPending || count.bytes + bytes > this.maxBytes) fail(503, 'queue_full', 'The durable fleet queue is full. Retry the same operation after delivery or operator capacity maintenance.');
    this.store.db.prepare('INSERT INTO fleet_delivery(fleet,generation,lane,id,kind,digest,payload,bytes) VALUES(?,?,?,?,?,?,?,?)').run(...key, kind, hash, raw, bytes);
    return id;
  }
  transaction(fleet, transactionId, body) {
    if (!Array.isArray(body.events) || body.events.length > 1000 || body.events.some(event => !event || typeof event !== 'object' || Array.isArray(event))) fail(400, 'invalid_transaction', 'A bounded Matrix events array is required.');
    const probe = fleet.probe, before = probe && structuredClone(probe);
    this.store.db.exec('BEGIN IMMEDIATE');
    try {
      this.enqueue(fleet, 'matrix', 'transaction', transactionId, { transactionId, body });
      // Only exact probe events in the representative's reception are eligible
      // proof evidence. Commit that evidence with the original transaction.
      for (const event of body.events) {
        if (event.type !== 'com.hagency.connection.probe.v1' || event.room_id !== probe?.roomId || !validId(event.event_id) || (probe?.eventId && event.event_id !== probe.eventId)
          || event.sender !== fleet.representativeMxid || event.content?.fleetId !== fleet.id || event.content?.challenge !== probe?.challenge) continue;
        if (probe.matrixTransactionId && probe.matrixEventId === event.event_id) continue;
        probe.matrixTransactionId = transactionId; probe.matrixEventId = event.event_id; this.store.save();
      }
      this.store.db.exec('COMMIT');
    } catch (error) {
      this.store.db.exec('ROLLBACK');
      // Keep the probe object held by an in-flight Matrix send valid. No other
      // task can run during this synchronous transaction/restore section.
      if (probe) { for (const key of Object.keys(probe)) delete probe[key]; Object.assign(probe, before); }
      throw error;
    }
  }
  claim(fleet, lane, consumer) {
    if (!['matrix', 'work'].includes(lane) || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(consumer ?? '')) fail(400, 'invalid_poll', 'Use a valid lane and stable UUID consumer.');
    const row = this.store.db.prepare('SELECT * FROM fleet_delivery WHERE fleet=? AND generation=? AND lane=? AND acked IS NULL ORDER BY ordinal LIMIT 1').get(fleet.id, fleet.transport.generation, lane);
    if (!row || row.expires > Date.now()) return null;
    const token = randomBytes(24).toString('base64url'), expires = Date.now() + this.leaseMs;
    this.store.db.prepare('UPDATE fleet_delivery SET consumer=?,token=?,expires=? WHERE ordinal=?').run(consumer, token, expires, row.ordinal);
    return { id: row.id, lane, token, expiresAt: new Date(expires).toISOString(), kind: row.kind, payload: JSON.parse(row.payload) };
  }
  async poll(id, token, generation, { lane, consumer, wait = '25000' }, signal) {
    if (!/^\d+$/.test(String(wait)) || Number(wait) > 25000) fail(400, 'invalid_poll', 'Poll wait must be between zero and 25000 milliseconds.');
    const until = Date.now() + Number(wait);
    for (;;) {
      const fleet = this.authenticate(id, token, generation);
      const delivery = this.claim(fleet, lane, consumer);
      if (delivery || Date.now() >= until) return { v: 2, generation: fleet.transport.generation, delivery };
      await delay(Math.min(100, until - Date.now()), undefined, { signal });
    }
  }
  ack(fleet, input) {
    const row = this.store.db.prepare('SELECT * FROM fleet_delivery WHERE fleet=? AND generation=? AND lane=? AND id=?').get(fleet.id, fleet.transport.generation, input.lane ?? '', input.id ?? '');
    if (!row || !sameSecret(row.token, input.token) || (!row.acked && row.expires <= Date.now())) fail(409, 'stale_lease', 'This delivery lease is no longer current.');
    if (!row.acked) this.store.db.prepare('UPDATE fleet_delivery SET acked=?,payload=NULL WHERE ordinal=?').run(Date.now(), row.ordinal);
    return { ok: true };
  }
  validateReceipt(fleet, receipt) {
    if (!receipt || typeof receipt !== 'object' || Array.isArray(receipt)) fail(400, 'invalid_receipt', 'Each probe receipt must be an object.');
    const probe = fleet.probe;
    if (!probe || receipt.received !== true || receipt.fleetId !== fleet.id || receipt.sourceRoomId !== probe.roomId || probe.matrixEventId !== probe.eventId
      || receipt.sourceEventId !== probe.eventId || receipt.challenge !== probe.challenge) fail(409, 'probe_binding_conflict', 'The receipt does not identify the current exact Matrix probe.');
    const row = this.store.db.prepare('SELECT acked FROM fleet_delivery WHERE fleet=? AND generation=? AND lane=? AND id=?').get(fleet.id, fleet.transport.generation, 'matrix', probe.matrixTransactionId ?? '');
    if (!row?.acked) fail(409, 'matrix_receipt_pending', 'The original Matrix transaction must be durably received and acknowledged first.');
  }
  async updates(fleet, input, workflow) {
    if (input.v !== 2 || input.generation !== fleet.transport.generation || !Number.isSafeInteger(input.sequence) || input.sequence < 1
      || input.heartbeat !== true || (input.statuses !== undefined && (!Array.isArray(input.statuses) || input.statuses.length > 200))
      || (input.probeReceipts !== undefined && (!Array.isArray(input.probeReceipts) || input.probeReceipts.length > 10))) fail(400, 'invalid_update', 'Use a bounded version 2 update with a positive sequence and heartbeat.');
    const hash = digest(input), last = fleet.transport.sequence;
    if (input.sequence < last) fail(409, 'stale_sequence', 'A newer transport update is already committed.');
    if (input.sequence === last) {
      if (hash !== fleet.transport.updateDigest) fail(409, 'sequence_conflict', 'This update sequence already has different content.');
      return { ok: true };
    }
    const copy = structuredClone(fleet);
    if (input.capabilities !== undefined) workflow.applyCapabilities(copy, input.capabilities, false);
    const records = [];
    for (const status of input.statuses ?? []) {
      if (!status || typeof status !== 'object' || status.v !== 1) fail(400, 'invalid_status', 'Each status must be a version 1 request observation.');
      const request = this.store.state.requests?.[`${fleet.id}:${status.requestId}`];
      if (!request || status.fleetId !== fleet.id) fail(409, 'unknown_request', 'Updates must identify an existing request in this fleet.');
      const record = structuredClone(request);
      if (status.role !== request.payload.role || status.requestedTokens !== request.payload.requestedTokens) fail(409, 'request_binding_conflict', 'The status role or quota does not match the registered request.');
      workflow.applyStatus(record, status, false);
      if (status.agentMxid && !fleet.registration.namespaces.users.some(ns => new RegExp(ns.regex).test(status.agentMxid))) fail(409, 'agent_namespace_conflict', 'The serving identity is outside this fleet namespace.');
      const observed = typeof status.observedAt === 'string' ? Date.parse(status.observedAt) : NaN;
      record.outboundStatus = { generation: fleet.transport.generation, receivedAt: new Date().toISOString(),
        observedAt: Number.isFinite(observed) ? new Date(observed).toISOString() : null };
      record.usable = false; record.statusVerified = true; record.lastError = null; records.push(record);
    }
    for (const receipt of input.probeReceipts ?? []) this.validateReceipt(fleet, receipt);
    if (input.probeReceipts?.length) {
      const state = await workflow.roomState(fleet.probe.roomId, fleet.registration.as_token, fleet.representativeMxid);
      const joined = mxid => state.some(event => event.type === 'm.room.member' && event.state_key === mxid && event.content?.membership === 'join');
      if (!joined(fleet.ownerMxid) || !joined(fleet.representativeMxid)) fail(409, 'reception_membership_pending', 'The owner and representative must still be joined before connection verification.');
      copy.probe.completedAt = new Date().toISOString();
      copy.connection = { verifiedAt: copy.probe.completedAt, generation: copy.transport.generation, sourceRoomId: copy.probe.roomId, sourceEventId: copy.probe.eventId, challenge: copy.probe.challenge };
      copy.state = 'ready'; copy.lastError = null;
    }
    copy.transport.sequence = input.sequence; copy.transport.updateDigest = hash; copy.transport.lastSeenAt = new Date().toISOString();
    return this.store.atomic(() => {
      Object.assign(fleet, copy);
      for (const record of records) Object.assign(this.store.state.requests[record.id], record);
      return { ok: true };
    });
  }
}
