import { createHash, randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { ApiError, Palpo, publicFleet } from './service.mjs';
import { isOutbound, outboundProven, outboundOnline, outboundStatusCurrent } from './outbound.mjs';

const now = () => new Date().toISOString();
const enc = encodeURIComponent;
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const field = (value, name, max = 128) => {
  if (typeof value !== 'string' || !value.trim() || value.length > max) fail(400, 'invalid_input', `${name} is required (maximum ${max} characters).`);
  return value.trim();
};
const key = (value, name) => { value = field(value, name, 80); if (!/^[a-zA-Z0-9_-]+$/.test(value)) fail(400, 'invalid_input', `${name} must use letters, numbers, underscores and hyphens.`); return value; };
const digest = value => createHash('sha256').update(JSON.stringify(value) ?? 'null').digest('hex');
// Matrix servers may reorder object keys when serializing state. Compare only
// room bindings canonically; persisted operation IDs/fingerprints keep their
// original digest format so existing requests remain resumable.
const canonicalJson = value => JSON.stringify(value, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);
const sameBinding = (actual, expected) => canonicalJson(actual) === canonicalJson(expected);
const stateContent = (state, type, stateKey = '') => state.find(event => event.type === type && event.state_key === stateKey)?.content;

export class Workflow {
  constructor(service, { readTimeoutMs = 8000 } = {}) {
    this.service = service; this.store = service.store; this.palpo = service.palpo;
    this.readTimeoutMs = readTimeoutMs;
    this.store.state.projects ??= {}; this.store.state.requests ??= {};
  }
  fleet(id) {
    const fleet = this.service.fleet(id);
    if (!['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed') fail(409, 'fleet_inactive', 'The Hagency service is not active.');
    return fleet;
  }
  ownerFleet(id, actor) {
    const fleet = this.fleet(id);
    if (fleet.ownerMxid !== actor) fail(404, 'not_found', 'Fleet not found.');
    return fleet;
  }
  rep(fleet, path, options) {
    return this.palpo.call(`${path}${path.includes('?') ? '&' : '?'}user_id=${enc(fleet.representativeMxid)}`, fleet.registration.as_token, options);
  }
  provider(fleet, path, options) {
    if (isOutbound(fleet)) fail(409, 'outbound_callback_forbidden', 'Outbound fleets publish snapshots and poll delivery; reverse callbacks are disabled.');
    // The owner cannot choose a second integration host. The scoped API shares
    // the administrator-approved App Service callback listener and credential.
    return new Palpo(fleet.callbackUrl, this.palpo.fetch, 'Hagency callback').call(`/api/fleet/v1${path}`, fleet.registration.hs_token, options);
  }
  async capabilities(fleet, signal) {
    if (isOutbound(fleet)) {
      if (!fleet.capabilities) fail(409, 'capabilities_pending', 'Waiting for Hagency to publish its resource catalog.');
      return fleet.capabilities;
    }
    const data = await this.provider(fleet, '/capabilities', { signal });
    return this.applyCapabilities(fleet, data);
  }
  applyCapabilities(fleet, data, persist = true) {
    if (!data || typeof data !== 'object' || Array.isArray(data)) fail(400, 'invalid_capabilities', 'Capabilities must be an object.');
    if (data.v !== 1 || data.fleetId !== fleet.id || data.serverName !== this.service.serverName || data.representativeMxid !== fleet.representativeMxid) fail(409, 'provider_identity_mismatch', 'The callback does not identify the installed fleet and server.');
    if (!Array.isArray(data.offers) || data.offers.some(offer => !offer || typeof offer !== 'object' || Array.isArray(offer)
      || (Array.isArray(offer.resources) && offer.resources.some(resource => !resource || typeof resource !== 'object' || Array.isArray(resource))))
      || !data.approvalBotMxid || !/^@[^\s:]+:.+$/.test(data.approvalBotMxid)) fail(409, 'provider_capability_missing', 'Hagency has not published its roles and approval identity.');
    // v1 providers may return only published roles, without a flag. When a
    // provider includes configured-but-withdrawn roles, honor explicit false.
    fleet.capabilities = { v: 1, fleetId: data.fleetId, serverName: data.serverName, representativeMxid: data.representativeMxid, approvalBotMxid: data.approvalBotMxid, offers: data.offers.filter(offer => offer.published !== false).map(offer => ({ role: field(offer.role, 'Role', 80), ...(typeof offer.description === 'string' ? { description: offer.description.slice(0, 500) } : {}),
      ...(Array.isArray(offer.resources) ? { resources: offer.resources.slice(0, 200).filter(resource => /^resource_[a-f0-9]{24}$/.test(resource.id ?? '')).map(resource => ({
        id: resource.id,
        name: field(resource.name, 'Resource name', 128), framework: field(resource.framework, 'Framework', 32),
        model: field(resource.model, 'Model', 256), reasoning: typeof resource.reasoning === 'string' ? resource.reasoning.slice(0, 64) : null,
      })) } : {}),
    })), observedAt: now() };
    fleet.capabilityRead = { state: 'current', observedAt: fleet.capabilities.observedAt };
    if (persist) this.store.save(); return fleet.capabilities;
  }
  async catalog(actor, signal) {
    // A browser refresh must observe newly published roles, not just reread
    // cached capabilities. Share concurrent refreshes and bound callback fanout.
    // The shared refresh owns its deadline; one departing browser cannot cancel
    // the other callers' read. Each HTTP caller also has its own response budget.
    if (!this.catalogRefresh) this.catalogRefresh = this.refreshCapabilities(AbortSignal.timeout(this.readTimeoutMs)).finally(() => { this.catalogRefresh = null; });
    await this.catalogRefresh;
    signal?.throwIfAborted();
    return Object.values(this.store.state.fleets)
      .filter(fleet => fleet.installation === 'installed' && !['revoked', 'paused'].includes(fleet.state))
      .map(fleet => ({ ...publicFleet(fleet), owned: fleet.ownerMxid === actor }));
  }
  async refreshCapabilities(signal) {
    const fleets = Object.values(this.store.state.fleets).filter(fleet => !isOutbound(fleet) && fleet.installation === 'installed' && !['revoked', 'paused'].includes(fleet.state));
    for (let offset = 0; offset < fleets.length; offset += 3) {
      const batch = fleets.slice(offset, offset + 3);
      const results = await Promise.allSettled(batch.map(fleet => this.capabilities(fleet, signal)));
      for (const [index, result] of results.entries()) {
        if (result.status !== 'rejected') continue;
        // Preserve the last valid offers and timestamp; an unavailable or
        // misidentified callback is not evidence that no roles are published.
        batch[index].capabilityRead = { state: 'failed', code: signal?.aborted ? 'read_timeout' : result.reason?.code ?? 'internal_error', failedAt: now(), lastSuccessAt: batch[index].capabilities?.observedAt ?? null };
      }
      this.store.save();
    }
  }
  async roomState(roomId, token, asUser, signal) {
    const data = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(roomId)}/state${asUser ? `?user_id=${enc(asUser)}` : ''}`, token, { signal });
    if (!Array.isArray(data)) fail(502, 'invalid_room_state', 'Matrix room state could not be verified.');
    return data;
  }
  async ensureRoom(plan, { name, actor, token, asUser, invite = [], encrypted = false, binding }) {
    const query = asUser ? `?user_id=${enc(asUser)}` : '';
    const alias = `#${plan.aliasLocalpart}:${this.service.serverName}`;
    if (!plan.roomId) {
      try { plan.roomId = (await this.palpo.call(`/_matrix/client/v3/directory/room/${enc(alias)}`, token)).room_id; }
      catch (error) { if (error.status !== 404) throw error; }
      // Recover interrupted creation before its alias was stored: only an exact
      // server-generated state binding on a room already joined by its creator.
      if (!plan.roomId) {
        const { joined_rooms: rooms } = await this.palpo.call(`/_matrix/client/v3/joined_rooms${query}`, token);
        if (!Array.isArray(rooms) || rooms.length > 1000) fail(409, 'room_recovery_required', 'The joined room inventory cannot safely be reconciled automatically.');
        const matches = [];
        for (const room of rooms) {
          const state = await this.roomState(room, token, asUser);
          if (sameBinding(stateContent(state, 'com.hagency.admin.binding.v1'), binding)) matches.push(room);
        }
        if (matches.length > 1) fail(409, 'room_binding_conflict', 'More than one room claims this durable operation.');
        if (matches.length) plan.roomId = matches[0];
      }
      if (!plan.roomId) {
        const result = await this.palpo.call(`/_matrix/client/v3/createRoom${query}`, token, { method: 'POST', body: {
          name, room_alias_name: plan.aliasLocalpart, visibility: 'private', preset: 'private_chat', room_version: '11', invite,
          initial_state: [
            { type: 'com.hagency.admin.binding.v1', state_key: '', content: binding },
            ...(encrypted ? [{ type: 'm.room.encryption', state_key: '', content: { algorithm: 'm.megolm.v1.aes-sha2' } }] : []),
          ],
        } });
        if (typeof result.room_id !== 'string') fail(502, 'invalid_room_identity', 'Matrix did not return a room ID.');
        plan.roomId = result.room_id;
      }
      this.store.save();
    }
    const state = await this.roomState(plan.roomId, token, asUser);
    const creator = stateContent(state, 'm.room.create')?.creator ?? state.find(event => event.type === 'm.room.create')?.sender;
    if (creator !== actor || !sameBinding(stateContent(state, 'com.hagency.admin.binding.v1'), binding)) fail(409, 'room_binding_conflict', 'The room creator or saved operation binding does not match.');
    if (stateContent(state, 'm.room.join_rules')?.join_rule !== 'invite') fail(409, 'room_privacy_required', 'The managed room must be invite-only.');
    const encryption = stateContent(state, 'm.room.encryption');
    if (encrypted ? encryption?.algorithm !== 'm.megolm.v1.aes-sha2' : encryption !== undefined) fail(409, 'room_encryption_mismatch', encrypted ? 'Owner approval room must use Matrix encryption.' : 'The Hagency reception/project channel must be unencrypted.');
    return { roomId: plan.roomId, state };
  }
  async reception(id, actor, token) {
    const fleet = this.ownerFleet(id, actor);
    fleet.reception ??= { aliasLocalpart: `${id}_reception`, createdAt: now() }; this.store.save();
    const { roomId } = await this.ensureRoom(fleet.reception, { name: `${fleet.name} · Reception`, actor: fleet.representativeMxid, token: fleet.registration.as_token, asUser: fleet.representativeMxid, invite: [actor], binding: { v: 1, fleetId: id, purpose: 'reception' } });
    await this.palpo.call(`/_matrix/client/v3/join/${enc(roomId)}`, token, { method: 'POST', body: {} });
    const state = await this.roomState(roomId, token);
    if (stateContent(state, 'm.room.member', actor)?.membership !== 'join' || stateContent(state, 'm.room.member', fleet.representativeMxid)?.membership !== 'join') fail(409, 'reception_membership_pending', 'The owner and representative must both join the reception room.');
    fleet.reception.verifiedAt = now(); this.store.save(); return roomId;
  }
  async connect(id, actor, token) {
    const fleet = this.ownerFleet(id, actor);
    try {
      await this.capabilities(fleet);
      const roomId = await this.reception(id, actor, token);
      // Outbound proof belongs to a credential generation. Reuse it even after
      // completion so a lost update response or repeated Verify cannot replace
      // the challenge underneath the client's durable publication outbox.
      if (!fleet.probe || (!isOutbound(fleet) && fleet.probe.completedAt)) { fleet.probe = { challenge: randomUUID(), roomId, startedAt: now() }; this.store.save(); }
      const probe = fleet.probe;
      if (probe.roomId !== roomId) fail(409, 'probe_binding_conflict', 'The saved connectivity probe belongs to a different room.');
      if (!probe.eventId) {
        const sent = await this.rep(fleet, `/_matrix/client/v3/rooms/${enc(roomId)}/send/com.hagency.connection.probe.v1/${enc(`probe_${probe.challenge}`)}`, { method: 'PUT', body: { v: 1, fleetId: id, challenge: probe.challenge } });
        if (!sent.event_id) fail(502, 'probe_event_missing', 'Matrix did not acknowledge the connectivity event.');
        probe.eventId = sent.event_id; this.store.save();
      }
      const probePayload = { fleetId: id, sourceRoomId: roomId, sourceEventId: probe.eventId, challenge: probe.challenge };
      if (isOutbound(fleet)) {
        this.service.outbound.enqueue(fleet, 'work', 'probe', `probe_${probe.challenge}`, probePayload);
        if (outboundProven(fleet) && probe.completedAt) {
          fleet.state = 'ready'; fleet.lastError = null; this.store.save();
        }
        return publicFleet(fleet);
      }
      const receipt = await this.waitForProbe(fleet, probePayload);
      if (receipt.received !== true || receipt.fleetId !== id || receipt.sourceRoomId !== roomId || receipt.sourceEventId !== probe.eventId || receipt.challenge !== probe.challenge) fail(409, 'event_delivery_unverified', 'Hagency has not verified receipt of this exact App Service event.');
      this.ownerFleet(id, actor);
      probe.completedAt = now();
      fleet.connection = { verifiedAt: now(), expiresAt: new Date(Date.now() + 30 * 60 * 1000).toISOString(), sourceRoomId: roomId, sourceEventId: probe.eventId, challenge: probe.challenge };
      fleet.state = 'ready'; fleet.lastError = null;
      this.store.audit(actor, 'fleet.connection.verify', id, probe.eventId, 'bidirectional_event_and_membership_verified');
      return publicFleet(fleet);
    } catch (error) {
      if (['ready', 'pending_connection'].includes(fleet.state)) fleet.state = 'pending_connection';
      fleet.lastError = { code: error.code ?? 'internal_error', at: now() };
      this.store.audit(actor, 'fleet.connection.verify', id, id, 'pending'); throw error;
    }
  }
  async waitForProbe(fleet, body) {
    // Matrix acknowledges sending before the App Service receives the event.
    // Retry only that precise pending receipt, preserving the original challenge
    // and event ID. No request, new room, or synthetic readiness is produced.
    for (let attempt = 0; ; attempt++) {
      this.fleet(fleet.id);
      try { return await this.provider(fleet, '/probe', { method: 'POST', body }); }
      catch (error) {
        if (error.code !== 'probe_pending' || attempt >= 19) throw error;
        await delay(500);
      }
    }
  }
  async validateProjectRoom(roomId, actor, token, ownerMxid = actor, requireOwnerAuthority = false, signal) {
    const state = await this.roomState(roomId, token, undefined, signal);
    if (stateContent(state, 'm.room.encryption')) fail(409, 'project_encrypted', 'The current Hagency project channel requires an unencrypted room.');
    if (stateContent(state, 'm.room.join_rules')?.join_rule !== 'invite') fail(409, 'project_privacy_required', 'The project room must be invite-only.');
    if (stateContent(state, 'm.room.member', actor)?.membership !== 'join' || stateContent(state, 'm.room.member', ownerMxid)?.membership !== 'join') fail(403, 'project_membership_required', 'The requester and recorded project owner must be joined members.');
    const power = stateContent(state, 'm.room.power_levels') ?? {};
    const level = mxid => Number(power.users?.[mxid] ?? power.users_default ?? 0);
    if (level(actor) < Number(power.invite ?? 0)) fail(403, 'project_invite_required', 'The requester must be allowed to invite an agent into the project.');
    if (requireOwnerAuthority && level(ownerMxid) < Math.max(100, Number(power.state_default ?? 50), Number(power.invite ?? 0))) fail(403, 'project_owner_authority_required', 'Registering a project requires room owner authority (power level 100).');
    return state;
  }
  async validateOwnerDm(project, token, signal) {
    const state = await this.roomState(project.ownerDmRoomId, token, undefined, signal);
    const members = state.filter(event => event.type === 'm.room.member' && event.content?.membership === 'join').map(event => event.state_key).sort();
    const allowed = [project.ownerMxid, project.approvalBotMxid].sort();
    if (stateContent(state, 'm.room.encryption')?.algorithm !== 'm.megolm.v1.aes-sha2' || stateContent(state, 'm.room.join_rules')?.join_rule !== 'invite') fail(409, 'owner_dm_privacy_required', 'Owner approvals require an encrypted invite-only room.');
    if (state.some(event => event.type === 'm.room.member' && ['join', 'invite'].includes(event.content?.membership) && !allowed.includes(event.state_key))) fail(409, 'owner_dm_not_private', 'The owner approval room contains an unrelated joined or invited member.');
    if (digest(members) !== digest(allowed)) fail(409, 'owner_dm_join_pending', 'Waiting for the Hagency approval identity to join the private owner room.');
    return true;
  }
  async createProject(input, actor, token) {
    const fleet = this.fleet(field(input.fleetId, 'Fleet ID'));
    const requestId = key(input.requestId, 'Project operation ID'), name = field(input.name, 'Project name');
    const existingRoom = typeof input.roomId === 'string' && input.roomId.trim() ? field(input.roomId, 'Room ID', 255) : null;
    const id = `project_${digest({ actor, requestId }).slice(0, 24)}`;
    const fingerprint = digest({ actor, fleetId: fleet.id, name, existingRoom });
    let project = this.store.state.projects[id];
    if (project && project.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This project operation ID is already bound to different content.');
    const capabilities = await this.capabilities(fleet);
    if (!project) {
      project = this.store.state.projects[id] = { id, requestId, fingerprint, fleetId: fleet.id, name, ownerMxid: actor, approvalBotMxid: capabilities.approvalBotMxid, authVersion: 1, createdAt: now(), roomId: existingRoom, state: 'creating', room: { aliasLocalpart: `hf_${id}` }, ownerDm: { aliasLocalpart: `hf_${id}_approvals` } };
      this.store.audit(actor, 'project.register', fleet.id, id, 'started');
    }
    try {
      if (!project.roomId) {
        const room = await this.ensureRoom(project.room, { name, actor, token, invite: [fleet.representativeMxid], binding: { v: 1, projectId: id, purpose: 'project', ownerMxid: actor } });
        project.roomId = room.roomId; this.store.save();
      }
      await this.validateProjectRoom(project.roomId, actor, token, actor, true);
      await this.palpo.call(`/_matrix/client/v3/rooms/${enc(project.roomId)}/state/com.hagency.admin.binding.v1/${enc(fleet.id)}`, token, { method: 'PUT', body: { v: 1, fleetId: fleet.id, purpose: 'project', projectId: project.id, ownerMxid: actor, authVersion: project.authVersion } });
      const targetState = await this.roomState(project.roomId, token);
      const repMembership = stateContent(targetState, 'm.room.member', fleet.representativeMxid)?.membership;
      if (!['join', 'invite'].includes(repMembership)) await this.palpo.call(`/_matrix/client/v3/rooms/${enc(project.roomId)}/invite`, token, { method: 'POST', body: { user_id: fleet.representativeMxid } });
      await this.rep(fleet, `/_matrix/client/v3/join/${enc(project.roomId)}`, { method: 'POST', body: {} });
      const dm = await this.ensureRoom(project.ownerDm, { name: `${name} · Private approvals`, actor, token, invite: [project.approvalBotMxid], encrypted: true, binding: { v: 1, projectId: id, purpose: 'owner_approval', ownerMxid: actor, approvalBotMxid: project.approvalBotMxid } });
      project.ownerDmRoomId = dm.roomId; project.state = 'registered'; project.lastError = null;
      this.store.audit(actor, 'project.register', fleet.id, id, 'registered');
      return await this.projectView(project, actor, token);
    } catch (error) { project.lastError = { code: error.code ?? 'internal_error', at: now() }; project.state = 'partial'; this.store.audit(actor, 'project.register', fleet.id, id, 'partial'); throw error; }
  }
  async projectView(project, actor, token, signal) {
    const { fingerprint, room, ownerDm, ...view } = project;
    // Private room identifiers are only returned to their owner. Project members
    // use the opaque project ID; the server carries the private binding internally.
    if (project.ownerMxid !== actor) delete view.ownerDmRoomId;
    try {
      await this.validateProjectRoom(project.roomId, actor, token, project.ownerMxid, true, signal);
      view.canRequest = true;
      if (actor === project.ownerMxid) {
        await this.validateOwnerDm(project, token, signal); view.ownerApproval = 'ready';
      } else view.ownerApproval = 'verified_by_provider_on_submission';
    } catch (error) { view.canRequest = false; view.readinessError = error.code ?? 'unknown'; view.ownerApproval = 'pending'; }
    return view;
  }
  async projects(actor, token, signal) {
    const views = [];
    for (const project of Object.values(this.store.state.projects)) {
      if (project.ownerMxid === actor) views.push(await this.projectView(project, actor, token, signal));
      else {
        try { await this.validateProjectRoom(project.roomId, actor, token, project.ownerMxid, false, signal); views.push(await this.projectView(project, actor, token, signal)); } catch { /* A room outside this Matrix session is not discoverable through the catalog. */ }
      }
    }
    return views;
  }
  async request(input, actor, token) {
    const requestId = key(input.requestId, 'Request ID');
    const project = this.store.state.projects[field(input.projectId, 'Project ID')];
    if (!project) fail(404, 'not_found', 'Project not found.');
    const fleet = this.fleet(project.fleetId);
    if (!(isOutbound(fleet) ? outboundProven(fleet) : publicFleet(fleet).readiness.ready)) fail(409, 'fleet_not_ready', 'The fleet owner must verify its connection and reception before new requests.');
    await this.validateProjectRoom(project.roomId, actor, token, project.ownerMxid, true);
    if (actor === project.ownerMxid) await this.validateOwnerDm(project, token);
    const capabilities = await this.capabilities(fleet), role = field(input.role, 'Role', 80);
    const requestedTokens = Number(input.requestedTokens), ratePerDay = Number(input.ratePerDay);
    if (!Number.isSafeInteger(requestedTokens) || requestedTokens <= 0 || !Number.isSafeInteger(ratePerDay) || ratePerDay <= 0) fail(400, 'invalid_quota', 'Requested tokens and daily rate must be positive safe integers.');
    let agentDefinition;
    if (input.agentDefinition !== undefined || input.agentName !== undefined || input.resourceId !== undefined) {
      const definition = input.agentDefinition ?? { name: input.agentName, resourceId: input.resourceId };
      const name = typeof definition?.name === 'string' ? definition.name.normalize('NFC') : '';
      if (!definition || typeof definition !== 'object' || Array.isArray(definition)
        || name.length > 64 || !/^\p{L}[\p{L}\p{M}\p{N}_-]*$/u.test(name)
        || typeof definition.resourceId !== 'string' || !/^resource_[a-f0-9]{24}$/.test(definition.resourceId)
        || Object.keys(definition).some(key => !['name', 'resourceId'].includes(key))) {
        fail(400, 'invalid_agent_definition', 'Agent name must start with a letter and use letters (including Chinese), numbers, underscores or hyphens (64 characters maximum). Select a published resource.');
      }
      agentDefinition = { name, resourceId: definition.resourceId };
    }
    const payload = { v: 1, fleetId: fleet.id, requestId, requesterMxid: actor, sourceRoomId: fleet.reception.roomId, targetProjectId: project.id, targetRoomId: project.roomId, ownerMxid: project.ownerMxid, ownerDmRoomId: project.ownerDmRoomId, role, requestedTokens, ratePerDay, authVersion: project.authVersion,
      ...(agentDefinition ? { agentDefinition } : {}) };
    const id = `${fleet.id}:${requestId}`, fingerprint = digest(payload);
    let request = this.store.state.requests[id];
    if (request && request.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This request ID is already bound to different content.');
    if (!request) {
      const offer = capabilities.offers.find(offer => offer.role === role);
      if (!offer) fail(409, 'role_unavailable', 'This role is not currently published by the Hagency.');
      const resource = agentDefinition && offer.resources?.find(resource => resource.id === agentDefinition.resourceId);
      if (agentDefinition && !resource) fail(409, 'resource_unavailable', 'This resource is not currently published for the selected role. Refresh the catalog and review your definition.');
      if (agentDefinition && Object.values(this.store.state.requests).some(other => other.projectId === project.id
        && !['ended', 'rejected'].includes(other.state) && other.payload.agentDefinition?.name === agentDefinition.name)) {
        fail(409, 'agent_name_conflict', 'This project already has an Agent request with that name. Use a distinct name for the next Agent.');
      }
      request = this.store.state.requests[id] = { id, requestId, fleetId: fleet.id, projectId: project.id, requesterMxid: actor, payload, fingerprint, state: 'sending', createdAt: now() };
      if (resource) request.resource = { ...resource };
      this.store.audit(actor, 'request.submit', fleet.id, requestId, 'started');
    }
    try {
      // Membership is granted only after the target project has been verified.
      const receptionState = await this.roomState(payload.sourceRoomId, fleet.registration.as_token, fleet.representativeMxid);
      if (!['invite', 'join'].includes(stateContent(receptionState, 'm.room.member', actor)?.membership)) await this.rep(fleet, `/_matrix/client/v3/rooms/${enc(payload.sourceRoomId)}/invite`, { method: 'POST', body: { user_id: actor } });
      await this.palpo.call(`/_matrix/client/v3/join/${enc(payload.sourceRoomId)}`, token, { method: 'POST', body: {} });
      if (!request.sourceEventId) {
        // The private approval-room address travels only over the authenticated
        // per-fleet HTTP channel, never in plaintext reception events.
        const { ownerDmRoomId, ...eventContent } = payload;
        const event = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(payload.sourceRoomId)}/send/com.hagency.engagement.request.v1/${enc(`request_${digest({ fleetId: fleet.id, actor, requestId })}`)}`, token, { method: 'PUT', body: eventContent });
        if (!event.event_id) fail(502, 'request_event_missing', 'Matrix did not acknowledge the request event.');
        request.sourceEventId = event.event_id; this.store.save();
      }
      if (isOutbound(fleet)) {
        this.service.outbound.enqueue(fleet, 'work', 'request', requestId, { ...payload, sourceEventId: request.sourceEventId });
        request.usable = false; request.statusVerified = false;
        if (!request.provider) request.state = 'queued';
        request.lastError = null; this.store.audit(actor, 'request.enqueue', fleet.id, requestId, 'durably_queued');
        return this.requestView(request, actor);
      }
      const result = await this.provider(fleet, '/requests', { method: 'POST', body: { ...payload, sourceEventId: request.sourceEventId } });
      this.applyStatus(request, result); request.lastError = null;
      this.store.audit(actor, 'request.submit', fleet.id, requestId, request.state);
      return this.requestView(request, actor);
    } catch (error) { request.lastError = { code: error.code ?? 'internal_error', at: now() }; request.state = 'submission_pending'; this.store.audit(actor, 'request.submit', fleet.id, requestId, 'submission_pending'); throw error; }
  }
  applyStatus(request, result, persist = true) {
    if (result.requestId !== request.requestId || result.fleetId !== request.fleetId) fail(409, 'request_binding_conflict', 'Hagency returned a different request binding.');
    if (result.targetProjectId !== request.payload.targetProjectId || result.targetRoomId !== request.payload.targetRoomId || result.sourceRoomId !== request.payload.sourceRoomId || result.sourceEventId !== request.sourceEventId) fail(409, 'request_binding_conflict', 'Hagency returned a different source or target project.');
    if (request.payload.agentDefinition && canonicalJson(result.agentDefinition) !== canonicalJson(request.payload.agentDefinition)) {
      fail(409, 'request_binding_conflict', 'Hagency did not confirm the requested Agent definition. Update its integration and retry this same request.');
    }
    const fields = ['v', 'fleetId', 'requestId', 'engagementId', 'state', 'targetProjectId', 'targetRoomId', 'sourceRoomId', 'sourceEventId', 'role', 'requestedTokens', 'allocatedTokens', 'agentMxid', 'bound', 'ready', 'decidedAt', 'endedAt'];
    request.provider = Object.fromEntries(fields.filter(key => result[key] !== undefined).map(key => [key, result[key]]));
    if (request.payload.agentDefinition) request.provider.agentDefinition = structuredClone(request.payload.agentDefinition);
    request.provider.serving = result.serving && typeof result.serving === 'object' ? Object.fromEntries(['framework', 'model', 'reasoning', 'tier'].filter(key => typeof result.serving[key] === 'string').map(key => [key, result.serving[key].slice(0, 128)])) : null;
    if (result.fulfillment) request.provider.fulfillment = { phase: result.fulfillment.phase, incomplete: result.fulfillment.incomplete, ...(typeof result.fulfillment.error === 'string' ? { error: result.fulfillment.error.slice(0, 500) } : {}) };
    request.state = result.state;
    // Older in-flight publications cannot revive an explicitly retired identity.
    if (request.retirement) {
      request.state = 'ended'; request.provider.state = 'ended'; request.provider.ready = false;
      request.provider.bound = false; request.provider.endedAt = request.retirement.endedAt;
    }
    request.observedAt = now(); if (persist) this.store.save();
  }
  requestView(request, actor) {
    const { fingerprint, payload, ...view } = request;
    return { ...view, role: payload.role, requestedTokens: payload.requestedTokens, ratePerDay: payload.ratePerDay, targetRoomId: payload.targetRoomId,
      ...(payload.agentDefinition ? { agentDefinition: payload.agentDefinition } : {}),
      ...(actor === payload.ownerMxid ? { ownerDmRoomId: payload.ownerDmRoomId } : {}) };
  }
  async requests(actor, token, signal = AbortSignal.timeout(this.readTimeoutMs)) {
    const version = this.service.mutationVersion;
    // Poll copies outside the mutation queue. Commit synchronously only while no
    // mutation is running and all authority/status bindings still match. No
    // awaited callback can overwrite a submission retry, revocation or new owner.
    const authority = fleet => fleet && { id: fleet.id, ownerMxid: fleet.ownerMxid, state: fleet.state,
      installation: fleet.installation, callbackUrl: fleet.callbackUrl, registration: fleet.registration,
      connection: fleet.connection, representativeMxid: fleet.representativeMxid };
    const snapshots = Object.values(this.store.state.requests).filter(request => request.requesterMxid === actor || this.store.state.projects[request.projectId]?.ownerMxid === actor)
      .map(request => ({ request: structuredClone(request), requestBefore: digest(request),
        project: structuredClone(this.store.state.projects[request.projectId]), fleet: structuredClone(this.store.state.fleets[request.fleetId]) }));
    const poll = async ({ request, requestBefore, project, fleet }) => {
      let managedAgent;
      const failed = (row, code) => ({ ...row, usable: false, statusVerified: false, lastError: { code, at: now() } });
      try {
        signal.throwIfAborted();
        await this.validateProjectRoom(project.roomId, actor, token, project.ownerMxid, true, signal);
        this.fleet(request.fleetId);
        request.usable = false; request.statusVerified = false;
        if (isOutbound(fleet)) {
          if (!request.provider) {
            request.usable = false; request.statusVerified = false;
          } else {
            const observedAt = request.observedAt;
            this.applyStatus(request, request.provider, false); request.observedAt = observedAt;
          }
        } else this.applyStatus(request, await this.provider(fleet, `/requests/${enc(request.requestId)}`, { signal }), false);
        if (request.provider?.agentMxid && ['active', 'ready'].includes(request.provider.state)) {
          const agentMxid = request.provider.agentMxid;
          if (!new RegExp(fleet.registration.namespaces.users[0].regex).test(agentMxid)) fail(409, 'agent_namespace_conflict', 'The fulfilled identity is outside this fleet namespace.');
          const state = await this.roomState(project.roomId, token, undefined, signal);
          request.agentJoined = stateContent(state, 'm.room.member', agentMxid)?.membership === 'join';
          if (!request.agentJoined) request.state = 'admission_pending';
          else if (request.provider.ready === true && request.provider.bound === true && request.provider.fulfillment?.incomplete !== true && request.provider.serving?.framework && request.provider.serving?.model) {
            const agentId = `fulfilled_${digest({ requestId: request.requestId }).slice(0, 20)}`;
            managedAgent = { id: agentId, fleetId: fleet.id, mxid: agentMxid, role: request.payload.role, displayName: request.payload.agentDefinition?.name ?? request.provider.agent ?? agentMxid, approvedRequestId: request.requestId, engagementId: request.provider.engagementId, projectId: project.id, authorization: 'verified_hagency_fulfillment', state: 'registered', createdAt: now(), localTaskStop: 'unknown' };
            request.usable = true;
          }
          else request.state = 'preparing';
        }
        request.statusVerified = !!request.provider; request.lastError = null;
        if (isOutbound(fleet)) {
          if (!outboundProven(fleet) || !outboundOnline(fleet)) request.usable = false;
          if (request.provider && !outboundStatusCurrent(fleet, request)) {
            request.usable = false; request.statusVerified = false; request.lastError = { code: 'outbound_status_stale', at: now() };
          }
        }
      } catch (error) { request = failed(request, signal.aborted ? 'read_timeout' : error.code ?? 'internal_error'); }
      const current = this.store.state.requests[request.id], currentProject = this.store.state.projects[request.projectId], currentFleet = this.store.state.fleets[request.fleetId];
      if (!current || (current.requesterMxid !== actor && currentProject?.ownerMxid !== actor)) return null;
      if (signal.aborted || this.service.activeMutation || this.service.mutationVersion !== version || digest(current) !== requestBefore
        || digest(currentProject) !== digest(project) || digest(authority(currentFleet)) !== digest(authority(fleet))) {
        return this.requestView(failed(current, signal.aborted ? 'read_timeout' : 'status_refresh_pending'), actor);
      }
      Object.assign(current, request);
      if (managedAgent && request.usable) currentFleet.agents[managedAgent.id] ??= managedAgent;
      this.store.save();
      return this.requestView(current, actor);
    };
    const output = [];
    for (let offset = 0; offset < snapshots.length; offset += 3) {
      output.push(...await Promise.all(snapshots.slice(offset, offset + 3).map(poll)));
    }
    return output.filter(Boolean);
  }
}
