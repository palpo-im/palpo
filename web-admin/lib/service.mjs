import { randomBytes, randomUUID, createHash } from 'node:crypto';
import { Outbound, isOutbound, outboundProven, outboundOnline } from './outbound.mjs';

export class ApiError extends Error {
  constructor(status, code, message) { super(message); this.status = status; this.code = code; }
}
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const now = () => new Date().toISOString();
const encode = encodeURIComponent;
const secret = () => randomBytes(32).toString('base64url');
const escapeRegex = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const text = (value, name, max = 128) => {
  if (typeof value !== 'string' || !value.trim() || value.length > max) fail(400, 'invalid_input', `${name} is required (maximum ${max} characters).`);
  return value.trim();
};
const key = (value, name) => {
  value = text(value, name, 80);
  if (!/^[a-zA-Z0-9_-]+$/.test(value)) fail(400, 'invalid_input', `${name} must use letters, numbers, underscores or hyphens.`);
  return value;
};
export function fixedTransportOrigin(value, publicOnly = false) {
  if (!value) return null;
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.pathname !== '/' || url.search || url.hash
    || (publicOnly && url.protocol !== 'https:' && !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname))) throw new Error('Transport origins must be fixed credential-free origins; public transport requires HTTPS.');
  return url.origin;
}

export class Palpo {
  constructor(url, fetchImpl = fetch, label = 'Palpo') { this.url = new URL(url); this.fetch = fetchImpl; this.label = label; }
  async call(path, token, { method = 'GET', body, signal } = {}) {
    let response, raw;
    try {
      signal?.throwIfAborted();
      response = await this.fetch(new URL(path, this.url), {
        method, headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body), redirect: 'error',
        signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(12000)]) : AbortSignal.timeout(12000),
      });
      raw = await response.text();
      signal?.throwIfAborted();
    } catch {
      if (signal?.aborted) throw signal.reason;
      fail(502, this.label === 'Palpo' ? 'palpo_unreachable' : 'hagency_unreachable', `The configured ${this.label} did not respond.`);
    }
    let data;
    try { data = raw ? JSON.parse(raw) : {}; } catch { data = {}; }
    if (!response.ok) {
      // Do not forward upstream text: admin detail/error payloads can contain tokens.
      const code = /^M_[A-Z_]+$/.test(data.errcode) ? data.errcode : /^[a-z][a-z0-9_]{0,79}$/.test(data.code) ? data.code : 'upstream_error';
      throw new ApiError(response.status, code, `${this.label} request failed (HTTP ${response.status}, ${code}).`);
    }
    return data;
  }
  async requireAdmin(token) {
    try { await this.call('/_palpo/admin/v1/appservices', token); }
    catch (error) {
      if (error.status === 404) fail(501, 'capability_unavailable', 'This Palpo deployment does not provide the required App Service admin API.');
      throw error;
    }
  }
  async user(mxid, token) {
    try { return await this.call(`/_palpo/admin/v2/users/${encode(mxid)}`, token); }
    catch (error) { if (error.status === 404) return null; throw error; }
  }
}

export function publicFleet(fleet) {
  const { registration: r, agents, fingerprint, transport, outboundChange, ...publicFields } = fleet;
  const canQueue = fleet.state === 'ready' && outboundProven(fleet);
  const ready = fleet.state === 'ready' && (isOutbound(fleet) ? canQueue && outboundOnline(fleet) : Date.parse(fleet.connection?.expiresAt ?? '') > Date.now());
  return {
    ...publicFields,
    state: fleet.state === 'ready' && !ready && !canQueue ? 'pending_connection' : fleet.state,
    ...(isOutbound(fleet) ? { transport: { mode: 'outbound', url: transport.url, generation: transport.generation, online: outboundOnline(fleet), lastSeenAt: transport.lastSeenAt ?? null }, migration: outboundChange ? { requestId: outboundChange.requestId, state: outboundChange.state } : null } : { transport: { mode: 'callback' } }),
    registration: { id: r.id, url: r.url, sender_localpart: r.sender_localpart, namespaces: r.namespaces },
    agentCount: Object.keys(agents).length,
    readiness: { ready, canQueue, identity: fleet.representativeVerifiedAt ? 'verified' : 'unknown', eventDelivery: fleet.connection?.verifiedAt ? 'verified' : 'unverified', reception: fleet.reception?.verifiedAt ? 'verified' : 'not_configured', verifiedAt: fleet.connection?.verifiedAt ?? null, expiresAt: fleet.connection?.expiresAt ?? null },
  };
}

export class Service {
  constructor({ store, palpo, serverName, callbackOrigins = [], transportOrigin, relayOrigin, outboundOptions }) {
    this.store = store; this.palpo = palpo; this.serverName = serverName;
    const binding = { serverName, palpoOrigin: palpo.url.origin };
    if (store.state.serverBinding && JSON.stringify(store.state.serverBinding) !== JSON.stringify(binding)) {
      throw new Error('This admin database belongs to a different Palpo server. Use a separate database.');
    }
    store.state.serverBinding = binding; store.save();
    this.callbackOrigins = new Set(callbackOrigins);
    this.transportOrigin = fixedTransportOrigin(transportOrigin, true); this.relayOrigin = fixedTransportOrigin(relayOrigin);
    this.outbound = new Outbound(this, outboundOptions);
    this.pending = Promise.resolve();
    this.mutationVersion = 0; this.activeMutation = false;
  }
  serial(fn) {
    const next = this.pending.then(async () => {
      this.activeMutation = true; this.mutationVersion++;
      try { return await fn(); }
      finally { this.activeMutation = false; this.mutationVersion++; }
    });
    this.pending = next.catch(() => {});
    return next;
  }
  fleet(id) {
    const fleet = this.store.state.fleets[id];
    if (!fleet) fail(404, 'not_found', 'Fleet not found.');
    return fleet;
  }
  owner(value) {
    value = text(value, 'Owner Matrix ID', 255);
    if (!value.startsWith('@') || value.slice(value.indexOf(':') + 1) !== this.serverName || !/^@[^\s:]+:.+$/.test(value)) {
      fail(400, 'invalid_owner', 'The owner must be a full local Matrix ID on the configured server.');
    }
    return value;
  }
  callback(value) {
    let url;
    try { url = new URL(text(value, 'Callback URL', 1024)); } catch { fail(400, 'invalid_callback', 'Enter a valid callback URL.'); }
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash || !this.callbackOrigins.has(url.origin)) {
      fail(400, 'callback_policy', 'The callback origin must be explicitly allowed by this server. Credentials, queries and fragments are not allowed.');
    }
    return url.href.replace(/\/$/, '');
  }
  async create(input, actor, token) {
    const requestId = key(input.requestId, 'Request ID');
    const name = text(input.name, 'Fleet name');
    const ownerMxid = this.owner(input.ownerMxid);
    const mode = input.transportMode ?? 'outbound';
    if (!['outbound', 'callback'].includes(mode)) fail(400, 'invalid_transport', 'Select outbound or legacy callback transport.');
    const callbackUrl = mode === 'callback' ? this.callback(input.callbackUrl) : null;
    if (mode === 'outbound') this.outbound.configured();
    const fingerprint = createHash('sha256').update(JSON.stringify(mode === 'callback' ? { name, ownerMxid, callbackUrl } : { name, ownerMxid, transportMode: mode })).digest('hex');
    const existing = Object.values(this.store.state.fleets).find(f => f.requestId === requestId);
    if (existing) {
      if (existing.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This request ID is already bound to different content.');
      return this.install(existing.id, actor, token);
    }
    const owner = await this.palpo.user(ownerMxid, token);
    if (!owner || owner.deactivated || owner.locked || owner.appservice_id) fail(400, 'invalid_owner', 'Select an active local human account as the fleet owner.');
    const id = `hf_${randomUUID().replaceAll('-', '')}`;
    const registration = {
      id, url: mode === 'outbound' ? this.outbound.relayUrl(id) : callbackUrl, as_token: secret(), hs_token: secret(), sender_localpart: `${id}_representative`,
      namespaces: { users: [{ exclusive: true, regex: `^@${id}_[a-z0-9_]+:${escapeRegex(this.serverName)}$` }], aliases: [], rooms: [] },
      rate_limited: true, receive_ephemeral: false,
    };
    this.store.state.fleets[id] = {
      id, requestId, fingerprint, name, ownerMxid, callbackUrl, registration, agents: {},
      state: 'authorized', installation: 'pending', credentialVersion: 1, credentialDeliveredAt: null,
      representativeMxid: `@${registration.sender_localpart}:${this.serverName}`,
      createdAt: now(), createdBy: actor, lastError: null, localTaskStop: 'unknown',
      ...(mode === 'outbound' ? { transport: this.outbound.transport(id) } : {}),
    };
    this.store.audit(actor, 'fleet.authorize', id, id, 'authorized');
    return this.install(id, actor, token);
  }
  async assertNamespaceAvailable(fleet, token) {
    const { appservices } = await this.palpo.call('/_palpo/admin/v1/appservices', token);
    if (!Array.isArray(appservices)) fail(502, 'invalid_upstream', 'Palpo returned an invalid App Service list.');
    const prefix = `${fleet.id}_`;
    for (const summary of appservices) {
      if (summary.id === fleet.id) continue;
      const other = await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(summary.id)}`, token);
      if (other.sender_localpart?.startsWith(prefix)) fail(409, 'namespace_conflict', 'An existing representative conflicts with this namespace.');
      for (const namespace of other.namespaces?.users ?? []) {
        // General regular expression intersection is not a safe runtime check.
        // Accept only anchored, provably disjoint literal prefixes; fail closed otherwise.
        const matched = /^\^@([a-z0-9_]+)/.exec(namespace.regex);
        const literal = matched?.[1];
        const suffix = matched ? namespace.regex.slice(matched[0].length) : '';
        if (!literal || /^[?*+{]/.test(suffix) || /[|]/.test(namespace.regex) || literal.startsWith(prefix) || prefix.startsWith(literal)) {
          fail(409, 'namespace_policy', 'An existing namespace cannot be proven disjoint. An administrator must review its registration before installation.');
        }
      }
    }
  }
  matches(actual, expected) {
    return ['id', 'url', 'as_token', 'hs_token', 'sender_localpart'].every(k => actual[k] === expected[k])
      && ['users', 'aliases', 'rooms'].every(kind => {
        const left = actual.namespaces?.[kind] ?? [], right = expected.namespaces[kind] ?? [];
        return left.length === right.length && left.every((row, index) => row.regex === right[index].regex && row.exclusive === right[index].exclusive);
      })
      && actual.rate_limited === expected.rate_limited
      && !actual.receive_ephemeral && !actual['io.element.msc4190'] && !(actual.protocols?.length);
  }
  async installed(fleet, token) {
    const actual = await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(fleet.id)}`, token);
    if (!this.matches(actual, fleet.registration)) fail(409, 'registration_drift', 'The installed registration differs from the saved operation. No credentials were replaced.');
    return actual;
  }
  async install(id, actor, token) {
    const fleet = this.fleet(id);
    if (['paused', 'revoked'].includes(fleet.state)) fail(409, 'fleet_inactive', 'This fleet is paused or revoked.');
    fleet.installation = 'installing'; this.store.save();
    try {
      let actual;
      try { actual = await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(id)}`, token); }
      catch (error) { if (error.status !== 404) throw error; }
      if (!actual) {
        await this.assertNamespaceAvailable(fleet, token);
        await this.palpo.call('/_palpo/admin/v1/appservices', token, { method: 'POST', body: fleet.registration });
      }
      actual = await this.installed(fleet, token);
      if (actual.disabled) fail(409, 'registration_disabled', 'The installed App Service is disabled.');
      fleet.installation = 'installed';
      await this.ensureIdentity(fleet, fleet.representativeMxid, token);
      fleet.representativeVerifiedAt = now(); fleet.state = 'pending_connection'; fleet.lastError = null;
      this.store.audit(actor, 'fleet.install', id, id, 'installed_pending_connection');
      return publicFleet(fleet);
    } catch (error) {
      fleet.lastError = { code: error.code ?? 'internal_error', at: now() };
      fleet.installation = 'failed'; this.store.audit(actor, 'fleet.install', id, id, 'failed');
      throw error;
    }
  }
  async ensureIdentity(fleet, mxid, token) {
    const existing = await this.palpo.user(mxid, token);
    if (existing && existing.appservice_id !== fleet.id) fail(409, 'identity_conflict', 'The Matrix account exists with different ownership.');
    const identity = await this.palpo.call(`/_matrix/client/v3/account/whoami?user_id=${encode(mxid)}`, fleet.registration.as_token);
    const observed = await this.palpo.user(mxid, token);
    if (identity.user_id !== mxid || observed?.appservice_id !== fleet.id || observed.deactivated || observed.locked) {
      fail(409, 'identity_unverified', 'Palpo did not verify the active App Service identity and ownership.');
    }
    return observed;
  }
  async credentials(id, ownerMxid) {
    const fleet = this.fleet(id);
    if (fleet.ownerMxid !== ownerMxid) fail(404, 'not_found', 'Fleet not found.');
    if (!['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed') fail(409, 'fleet_inactive', 'The fleet is not installed or is inactive.');
    const identity = await this.palpo.call(`/_matrix/client/v3/account/whoami?user_id=${encode(fleet.representativeMxid)}`, fleet.registration.as_token);
    if (identity.user_id !== fleet.representativeMxid) fail(409, 'identity_unverified', 'The representative identity could not be verified before credential delivery.');
    fleet.credentialDeliveredAt ??= now();
    fleet.credentialLastDeliveredAt = now();
    this.store.audit(ownerMxid, 'fleet.credentials.deliver', id, id, 'same_version_resumable_delivery');
    return { fleetId: id, serverName: this.serverName, credentialVersion: fleet.credentialVersion, registration: fleet.registration,
      ...(isOutbound(fleet) ? { transport: { mode: 'outbound', url: fleet.transport.url, token: fleet.transport.token, generation: fleet.transport.generation } } : {}) };
  }
  async migrateOutbound(id, input, actor, token) {
    const fleet = this.fleet(id), requestId = key(input.requestId, 'Migration operation ID');
    this.outbound.configured();
    if (!['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed') fail(409, 'fleet_inactive', 'Only an installed active fleet can migrate.');
    let plan = fleet.outboundChange;
    if (plan?.requestId === requestId && plan.rotate !== (input.rotate === true)) fail(409, 'idempotency_conflict', 'This migration operation has different content.');
    if (!plan || plan.requestId !== requestId) {
      if (plan && plan.state !== 'done') fail(409, 'migration_pending', 'Retry the existing migration operation first.');
      if (isOutbound(fleet) && !input.rotate) return publicFleet(fleet);
      plan = fleet.outboundChange = { requestId, rotate: input.rotate === true, state: 'pending', previousRegistration: structuredClone(fleet.registration),
        transport: this.outbound.transport(id, (fleet.transport?.generation ?? 0) + 1) };
      this.store.audit(actor, 'fleet.outbound.migrate', id, requestId, 'pending');
    }
    if (plan.state === 'done') return publicFleet(fleet);
    const desired = { ...plan.previousRegistration, url: this.outbound.relayUrl(id) };
    const current = await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(id)}`, token);
    if (current.disabled) fail(409, 'registration_disabled', 'The App Service is disabled on Palpo.');
    if (!this.matches(current, desired)) {
      if (!this.matches(current, plan.previousRegistration)) fail(409, 'registration_drift', 'Registration changed outside this migration.');
      try { await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(id)}/url`, token, { method: 'PUT', body: { url: desired.url, expected_url: current.url } }); }
      catch (error) { if ([404, 405].includes(error.status)) fail(501, 'capability_unavailable', 'Upgrade Palpo to support atomic App Service URL migration.'); throw error; }
    }
    const verified = await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(id)}`, token);
    if (verified.disabled || !this.matches(verified, desired)) fail(409, 'registration_drift', 'The migrated registration could not be verified.');
    return this.store.atomic(() => {
      const oldGeneration = fleet.transport?.generation;
      // A rotation retains every unacknowledged delivery while invalidating its
      // old lease. ACKed tombstones retain their original generation.
      if (oldGeneration) this.store.db.prepare("UPDATE fleet_delivery SET generation=?,consumer=NULL,token=NULL,expires=NULL WHERE fleet=? AND generation=? AND acked IS NULL AND kind!='probe'").run(plan.transport.generation, id, oldGeneration);
      fleet.registration = desired; fleet.callbackUrl = null; fleet.transport = plan.transport;
      fleet.state = 'pending_connection'; fleet.connection = null; fleet.probe = null;
      // Every existing source-bound request is replayed into the new generation,
      // including previously delivered ones whose ongoing statuses must be tracked.
      for (const request of Object.values(this.store.state.requests ?? {})) {
        if (request.fleetId === id && request.sourceEventId) this.outbound.enqueue(fleet, 'work', 'request', request.requestId, { ...request.payload, sourceEventId: request.sourceEventId });
      }
      plan.state = 'done'; this.store.audit(actor, 'fleet.outbound.migrate', id, requestId, 'done');
      return publicFleet(fleet);
    });
  }
  async setState(id, action, actor, token) {
    const fleet = this.fleet(id);
    if (fleet.state === 'revoked') fail(409, 'fleet_revoked', 'Revocation is final in this release.');
    await this.installed(fleet, token);
    const disable = action !== 'resume';
    await this.palpo.call(`/_palpo/admin/v1/appservices/${encode(id)}/${disable ? 'disable' : 'enable'}`, token, { method: 'POST', body: {} });
    const actual = await this.installed(fleet, token);
    if (actual.disabled !== disable) fail(502, 'state_unverified', 'Palpo did not confirm the registration state.');
    fleet.state = action === 'revoke' ? 'revoked' : disable ? 'paused' : 'pending_connection';
    fleet.localTaskStop = 'unconfirmed';
    fleet.revocationScope = action === 'revoke' ? 'appservice_credentials_only' : null;
    this.store.audit(actor, `fleet.${action}`, id, id, fleet.state);
    return publicFleet(fleet);
  }
  async activeFleet(id, token) {
    const fleet = this.fleet(id);
    if (!['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed') fail(409, 'fleet_inactive', 'Identity changes require an installed, active registration.');
    if ((await this.installed(fleet, token)).disabled) fail(409, 'registration_disabled', 'The App Service is disabled on Palpo.');
    return fleet;
  }
  async createAgent(id, input, actor, token) {
    const fleet = await this.activeFleet(id, token);
    const agentId = key(input.agentId, 'Agent ID').toLowerCase();
    if (!/^[a-z0-9_]+$/.test(agentId)) fail(400, 'invalid_input', 'Agent ID must use lowercase letters, numbers and underscores.');
    const displayName = text(input.displayName, 'Display name');
    const role = text(input.role, 'Public role', 80);
    const approvedRequestId = key(input.approvedRequestId, 'Approved request reference');
    let agent = fleet.agents[agentId];
    if (agent && (agent.approvedRequestId !== approvedRequestId || agent.role !== role || agent.initialDisplayName !== displayName)) {
      fail(409, 'idempotency_conflict', 'This Agent ID is already bound to different creation content.');
    }
    if (agent?.state === 'retired') fail(409, 'agent_retired', 'Retired identities cannot be reused.');
    if (!agent) {
      agent = fleet.agents[agentId] = { id: agentId, fleetId: id, mxid: `@${id}_agent_${agentId}:${this.serverName}`, role,
        displayName, initialDisplayName: displayName, approvedRequestId, authorization: 'administrator_attested', state: 'creating', createdAt: now(), localTaskStop: 'unknown' };
      this.store.audit(actor, 'agent.authorize', id, agentId, 'authorized');
    }
    try {
      await this.ensureIdentity(fleet, agent.mxid, token);
      await this.palpo.call(`/_palpo/admin/v2/users/${encode(agent.mxid)}`, token, { method: 'PUT', body: { displayname: agent.displayName } });
      agent.state = 'registered'; agent.lastError = null;
      this.store.audit(actor, 'agent.create', id, agentId, 'registered');
      return await this.observeAgent(fleet, agent, token);
    } catch (error) {
      agent.state = 'failed'; agent.lastError = { code: error.code ?? 'internal_error', at: now() };
      this.store.audit(actor, 'agent.create', id, agentId, 'failed'); throw error;
    }
  }
  async observeAgent(fleet, agent, token) {
    const output = { ...agent, runtimeHealth: 'unknown', observedAt: now() };
    const user = await this.palpo.user(agent.mxid, token);
    if (!user) return { ...output, matrixIdentity: 'missing', joinedRooms: null };
    if (user.appservice_id !== fleet.id) fail(409, 'identity_conflict', 'Observed identity ownership differs from the managed fleet.');
    const rooms = await this.palpo.call(`/_palpo/admin/v1/users/${encode(agent.mxid)}/joined_rooms`, token);
    if (!Array.isArray(rooms.joined_rooms)) fail(502, 'invalid_upstream', 'Palpo did not return Matrix membership observations.');
    return { ...output, displayName: user.displayname ?? null, matrixIdentity: user.deactivated ? 'deactivated' : user.locked ? 'locked' : 'active', joinedRooms: rooms.joined_rooms };
  }
  async agents(id, token) {
    const fleet = this.fleet(id);
    return Promise.all(Object.values(fleet.agents).map(async agent => {
      try { return await this.observeAgent(fleet, agent, token); }
      catch (error) { return { ...agent, matrixIdentity: 'unknown', joinedRooms: null, runtimeHealth: 'unknown', observationError: error.code ?? 'internal_error', observedAt: now() }; }
    }));
  }
  async updateAgent(id, agentId, input, actor, token) {
    const fleet = await this.activeFleet(id, token);
    const agent = fleet.agents[agentId];
    if (!agent) fail(404, 'not_found', 'Agent not found.');
    if (agent.state !== 'registered') fail(409, 'agent_inactive', 'Only a registered identity can be edited.');
    if (Object.keys(input).some(k => k !== 'displayName')) fail(400, 'immutable_identity', 'Only the display name is editable. Identity, ownership and runtime configuration cannot be changed here.');
    const displayName = text(input.displayName, 'Display name');
    await this.ensureIdentity(fleet, agent.mxid, token);
    await this.palpo.call(`/_palpo/admin/v2/users/${encode(agent.mxid)}`, token, { method: 'PUT', body: { displayname: displayName } });
    const observed = await this.observeAgent(fleet, agent, token);
    if (observed.displayName !== displayName) fail(502, 'profile_unverified', 'The updated Matrix profile could not be verified.');
    agent.displayName = displayName;
    this.store.audit(actor, 'agent.profile.update', id, agentId, 'updated');
    return observed;
  }
  async retireAllocatedAgent(fleet, input, token) {
    if (!token) fail(503, 'retirement_unconfigured', 'The server operator must configure an administrator credential for Agent retirement.');
    const request = this.store.state.requests?.[`${fleet.id}:${input.requestId}`];
    const prefix = `@${fleet.id}_agent_`, suffix = `:${this.serverName}`;
    if (!request || request.fleetId !== fleet.id || typeof input.agentMxid !== 'string'
      || request.provider?.agentMxid !== input.agentMxid || !input.agentMxid.startsWith(prefix)
      || !input.agentMxid.endsWith(suffix) || !Number.isSafeInteger(input.endedAt) || input.endedAt <= 0) {
      fail(403, 'retirement_scope_mismatch', 'Retirement must identify the exact Agent of a confirmed fleet request.');
    }
    const localpart = input.agentMxid.slice(prefix.length, -suffix.length);
    if (!/^[a-z0-9_]+$/.test(localpart)) fail(403, 'retirement_scope_mismatch', 'Invalid managed Agent identity.');
    // Fulfilled requests use a request-derived management key. Matrix localpart
    // and management key need not be equal; preserve existing history links.
    const agentId = Object.entries(fleet.agents).find(([, agent]) => agent.mxid === input.agentMxid)?.[0] ?? localpart;
    const others = Object.values(this.store.state.requests).filter(row => row !== request && row.fleetId === fleet.id
      && row.provider?.agentMxid === input.agentMxid && ['active', 'pending'].includes(row.state));
    if (others.length) fail(409, 'agent_still_allocated', 'Another allocation still uses this Agent.');
    await this.palpo.requireAdmin(token);
    const user = await this.palpo.user(input.agentMxid, token);
    if (!user || user.appservice_id !== fleet.id || user.admin) fail(409, 'identity_conflict', 'The Agent must be owned by this App Service.');
    const existing = fleet.agents[agentId];
    if (existing && existing.mxid !== input.agentMxid) fail(409, 'identity_conflict', 'The managed Agent identity differs.');
    fleet.agents[agentId] ??= { id: agentId, fleetId: fleet.id, mxid: input.agentMxid,
      displayName: user.displayname, role: request.payload.role, approvedRequestId: request.requestId,
      authorization: 'verified_fleet_request', createdAt: now(), state: 'retiring' };
    for (const agent of Object.values(fleet.agents)) if (agent.mxid === input.agentMxid) agent.state = 'retiring';
    request.retirement ??= { state: 'pending', mxid: input.agentMxid, endedAt: input.endedAt };
    request.state = 'ended'; request.provider = { ...request.provider, state: 'ended', ready: false, bound: false, endedAt: request.retirement.endedAt };
    request.usable = false; this.store.save();
    try {
      const agent = await this.retireAgent(fleet.id, agentId, `fleet:${fleet.id}`, token);
      request.retirement.state = 'complete';
      for (const record of Object.values(fleet.agents)) if (record.mxid === input.agentMxid) {
        Object.assign(record, { state: 'retired', retiredAt: record.retiredAt ?? agent.retiredAt,
          localTaskStop: input.localStopped === true ? 'confirmed' : 'unconfirmed', lastError: null });
      }
      this.store.save();
      return { ok: true, fleetId: fleet.id, requestId: request.requestId,
        agent: { ...agent, localTaskStop: fleet.agents[agentId].localTaskStop, appserviceAccess: 'revoked' } };
    } catch (error) {
      request.retirement.state = 'failed'; this.store.save(); throw error;
    }
  }

  async retireAgent(id, agentId, actor, token) {
    const fleet = this.fleet(id), agent = fleet.agents[agentId];
    if (!agent) fail(404, 'not_found', 'Agent not found.');
    const user = await this.palpo.user(agent.mxid, token);
    if (!user || user.appservice_id !== id) fail(409, 'identity_conflict', 'The managed identity and ownership must exist before retirement.');
    agent.state = 'retiring'; this.store.audit(actor, 'agent.retire', id, agentId, 'started');
    try {
      await this.palpo.call(`/_palpo/admin/v1/deactivate/${encode(agent.mxid)}`, token, { method: 'POST', body: { erase: false } });
      const observed = await this.observeAgent(fleet, agent, token);
      if (observed.matrixIdentity !== 'deactivated' || observed.joinedRooms.length) fail(502, 'retirement_unverified', 'Matrix deactivation or room removal is incomplete.');
      let denied = false;
      try { await this.palpo.call(`/_matrix/client/v3/account/whoami?user_id=${encode(agent.mxid)}`, fleet.registration.as_token); }
      catch (error) { if ([401, 403].includes(error.status)) denied = true; else throw error; }
      if (!denied) fail(502, 'retirement_unverified', 'The retired identity still authenticates through its App Service.');
      agent.state = 'retired'; agent.localTaskStop = 'unconfirmed'; agent.retiredAt ??= now(); agent.lastError = null;
      this.store.audit(actor, 'agent.retire', id, agentId, 'matrix_access_revoked_local_stop_unconfirmed');
      return { ...observed, ...agent };
    } catch (error) {
      agent.lastError = { code: error.code ?? 'internal_error', at: now() };
      this.store.audit(actor, 'agent.retire', id, agentId, 'incomplete'); throw error;
    }
  }
}
