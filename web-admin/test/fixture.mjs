import { Store } from '../lib/store.mjs';
import { Palpo, Service } from '../lib/service.mjs';

// Fixtures mirror Palpo 3e4fbd33 route shapes, including omitted empty namespaces
// and App Service whoami identity creation in hoops/auth.rs. No real server calls.
export function fixture({ path = ':memory:', bypassRetirement = false, deliverProbe = true, autoJoinBot = true, transportOrigin, relayOrigin, outboundOptions } = {}) {
  const registrations = new Map(), calls = [], actors = new Map(), users = new Map([
    ['@admin:example.test', { name: '@admin:example.test', admin: true, deactivated: false }],
    ['@owner:example.test', { name: '@owner:example.test', admin: false, deactivated: false }],
    ['@other:example.test', { name: '@other:example.test', admin: false, deactivated: false }],
  ]);
  const rooms = new Map(), aliases = new Map(), events = new Map(), receipts = new Map(), requests = new Map(), transactions = new Map(), publishedOffers = new Map();
  const member = (room, mxid) => room.state.find(event => event.type === 'm.room.member' && event.state_key === mxid)?.content.membership;
  const putState = (room, type, state_key, content, sender) => { const row = { type, state_key, content, sender }; const index = room.state.findIndex(event => event.type === type && event.state_key === state_key); if (index >= 0) room.state[index] = row; else room.state.push(row); };
  let deniedAdmin = false;
  const response = (status, value) => new Response(JSON.stringify(value), { status });
  const missing = () => response(404, { errcode: 'M_NOT_FOUND', error: 'Not found' });
  const fetch = async (target, options) => {
    const url = new URL(target), path = decodeURIComponent(url.pathname), method = options.method;
    const token = options.headers.Authorization?.slice(7), body = options.body ? JSON.parse(options.body) : null;
    calls.push({ path, search: url.search, method, token, body });
    const actingService = [...registrations.values()].find(reg => reg.as_token === token && !reg.disabled);
    const actor = actors.get(token) ?? (token === 'admin-secret' ? '@admin:example.test' : token === 'owner-secret' ? '@owner:example.test' : token === 'other-secret' ? '@other:example.test' : actingService ? url.searchParams.get('user_id') : null);
    if (path.startsWith('/api/fleet/v1')) {
      const reg = [...registrations.values()].find(reg => reg.hs_token === token && !reg.disabled);
      if (!reg) return response(401, { code: 'unauthorized' });
      if (path === '/api/fleet/v1/capabilities') return response(200, { v: 1, fleetId: reg.id, serverName: 'example.test', representativeMxid: `@${reg.sender_localpart}:example.test`, approvalBotMxid: '@approvalbot:example.test', offers: publishedOffers.get(reg.id) ?? [{ role: 'coding', resources: [{ id: `resource_${'a'.repeat(24)}`, name: 'Medium coding resource', framework: 'codex', model: 'fixture-model', reasoning: 'medium' }] }] });
      if (path === '/api/fleet/v1/probe') {
        const receipt = receipts.get(body.sourceEventId);
        if (!receipt || receipt.challenge !== body.challenge || receipt.sourceRoomId !== body.sourceRoomId || receipt.fleetId !== reg.id) return response(409, { code: 'probe_pending' });
        return response(200, { v: 1, received: true, ...receipt, sourceEventId: body.sourceEventId, mode: 'push' });
      }
      if (path === '/api/fleet/v1/requests' && method === 'POST') {
        const event = events.get(body.sourceEventId), { sourceEventId, ownerDmRoomId, ...content } = body;
        if (!event || event.sender !== body.requesterMxid || event.room_id !== body.sourceRoomId || JSON.stringify(event.content) !== JSON.stringify(content)) return response(403, { code: 'source_event_mismatch' });
        let record = requests.get(`${reg.id}:${body.requestId}`);
        if (!record) { record = { v: 1, fleetId: reg.id, requestId: body.requestId, state: 'pending', engagementId: `en_${requests.size + 1}`, targetProjectId: body.targetProjectId, targetRoomId: body.targetRoomId, sourceRoomId: body.sourceRoomId, sourceEventId, role: body.role, requestedTokens: body.requestedTokens, agentMxid: null, bound: false, ready: false, serving: false, fulfillment: null }; requests.set(`${reg.id}:${body.requestId}`, record); }
        if (body.agentDefinition) record.agentDefinition = body.agentDefinition;
        return response(200, record);
      }
      const found = /^\/api\/fleet\/v1\/requests\/([^/]+)$/.exec(path);
      if (found) return requests.has(`${reg.id}:${found[1]}`) ? response(200, requests.get(`${reg.id}:${found[1]}`)) : missing();
    }
    if (path === '/_matrix/client/v3/login') {
      if (body.password !== 'correct-password') return response(403, { errcode: 'M_FORBIDDEN' });
      return response(200, { access_token: body.identifier.user === '@admin:example.test' ? 'admin-secret' : body.identifier.user === '@other:example.test' ? 'other-secret' : 'owner-secret', user_id: body.identifier.user });
    }
    if (path === '/_matrix/client/v3/logout') return response(200, {});
    if (path === '/_matrix/client/v3/account/whoami') {
      if (token === 'admin-secret') return response(200, { user_id: '@admin:example.test' });
      if (token === 'owner-secret') return response(200, { user_id: '@owner:example.test' });
      if (token === 'other-secret') return response(200, { user_id: '@other:example.test' });
      const reg = [...registrations.values()].find(r => r.as_token === token && !r.disabled);
      if (!reg) return response(401, { errcode: 'M_UNKNOWN_TOKEN' });
      const mxid = url.searchParams.get('user_id');
      if (!reg.namespaces.users.some(ns => new RegExp(ns.regex).test(mxid))) return response(403, { errcode: 'M_FORBIDDEN' });
      if (users.get(mxid)?.deactivated && !bypassRetirement) return response(403, { errcode: 'M_USER_DEACTIVATED' });
      if (!users.has(mxid)) users.set(mxid, { name: mxid, appservice_id: reg.id, deactivated: false, displayname: mxid.split(':')[0].slice(1), rooms: [] });
      return response(200, { user_id: mxid });
    }
    if (path === '/_matrix/client/v3/createRoom') {
      if (!actor) return response(401, { errcode: 'M_UNKNOWN_TOKEN' });
      const alias = `#${body.room_alias_name}:example.test`; if (aliases.has(alias)) return response(409, { errcode: 'M_ROOM_IN_USE' });
      const roomId = `!room${rooms.size + 1}:example.test`, room = { id: roomId, state: [] }; rooms.set(roomId, room); aliases.set(alias, roomId);
      putState(room, 'm.room.create', '', { creator: actor }, actor);
      putState(room, 'm.room.member', actor, { membership: 'join' }, actor);
      putState(room, 'm.room.join_rules', '', { join_rule: 'invite' }, actor);
      putState(room, 'm.room.power_levels', '', { users: { [actor]: 100 }, users_default: 0, invite: 0, state_default: 50 }, actor);
      for (const event of body.initial_state ?? []) putState(room, event.type, event.state_key, event.content, actor);
      for (const mxid of body.invite ?? []) putState(room, 'm.room.member', mxid, { membership: autoJoinBot && mxid === '@approvalbot:example.test' ? 'join' : 'invite' }, actor);
      return response(200, { room_id: roomId });
    }
    if (path.startsWith('/_matrix/client/v3/directory/room/')) { const alias = path.slice('/_matrix/client/v3/directory/room/'.length); return aliases.has(alias) ? response(200, { room_id: aliases.get(alias) }) : missing(); }
    if (path === '/_matrix/client/v3/joined_rooms') return response(200, { joined_rooms: [...rooms.values()].filter(room => member(room, actor) === 'join').map(room => room.id) });
    let roomMatch = /^\/_matrix\/client\/v3\/join\/(.+)$/.exec(path);
    if (roomMatch) { const room = rooms.get(roomMatch[1]); if (!room) return missing(); if (!actor || !['invite', 'join'].includes(member(room, actor))) return response(403, { errcode: 'M_FORBIDDEN' }); putState(room, 'm.room.member', actor, { membership: 'join' }, actor); return response(200, { room_id: room.id }); }
    roomMatch = /^\/_matrix\/client\/v3\/rooms\/([^/]+)\/(state|invite|send)(?:\/(.+))?$/.exec(path);
    if (roomMatch) {
      const [, roomId, action, tail] = roomMatch, room = rooms.get(roomId);
      if (!room) return missing(); if (!actor || member(room, actor) !== 'join') return response(403, { errcode: 'M_FORBIDDEN' });
      // Palpo serializes state-content maps with sorted keys, unlike JavaScript
      // object insertion order. Exercise that actual wire behavior in workflows.
      if (action === 'state' && method === 'GET') return response(200, JSON.parse(JSON.stringify(room.state, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
        ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item)));
      if (action === 'state' && method === 'PUT') { const [type, stateKey = ''] = tail.split('/'); putState(room, type, stateKey, body, actor); return response(200, { event_id: `$state${calls.length}` }); }
      if (action === 'invite') { putState(room, 'm.room.member', body.user_id, { membership: 'invite' }, actor); return response(200, {}); }
      if (action === 'send') {
        const [type, txn] = tail.split('/'), transaction = `${actor}:${roomId}:${txn}`;
        if (transactions.has(transaction)) return response(200, { event_id: transactions.get(transaction) });
        const eventId = `$event${events.size + 1}`, event = { event_id: eventId, sender: actor, room_id: roomId, type, content: body };
        events.set(eventId, event); transactions.set(transaction, eventId);
        if (type === 'com.hafleet.connection.probe.v1' && deliverProbe) receipts.set(eventId, { fleetId: body.fleetId, sourceRoomId: roomId, challenge: body.challenge });
        return response(200, { event_id: eventId });
      }
    }
    if (path.startsWith('/_palpo/admin/')) {
      if (token !== 'admin-secret' || deniedAdmin) return response(403, { errcode: 'M_FORBIDDEN', error: 'Requires admin privileges' });
      if (path === '/_palpo/admin/v1/appservices') {
        if (method === 'GET') return response(200, { appservices: [...registrations.values()].map(({ id, url, sender_localpart, disabled }) => ({ id, url, sender_localpart, disabled })) });
        if (registrations.has(body.id)) return response(400, { errcode: 'M_INVALID_PARAM' });
        registrations.set(body.id, { ...body, disabled: false }); return response(200, { id: body.id });
      }
      let match = /^\/_palpo\/admin\/v1\/appservices\/([^/]+)(?:\/(disable|enable|url))?$/.exec(path);
      if (match) {
        const reg = registrations.get(match[1]); if (!reg) return missing();
        if (match[2] === 'url') {
          if (method !== 'PUT') return response(405, { errcode: 'M_UNRECOGNIZED' });
          if (reg.url !== body.expected_url) return response(409, { errcode: 'M_CONFLICT' });
          reg.url = body.url; return response(200, {});
        }
        if (match[2]) { reg.disabled = match[2] === 'disable'; return response(200, {}); }
        const result = structuredClone(reg);
        // serde skips empty namespace collections and serializes map keys in a
        // different order from the JavaScript registration request.
        result.namespaces = { users: reg.namespaces.users.map(({ regex, exclusive }) => ({ regex, exclusive })) };
        return response(200, result);
      }
      match = /^\/_palpo\/admin\/v2\/users\/(.+)$/.exec(path);
      if (match) {
        const user = users.get(match[1]); if (!user) return missing();
        if (method === 'PUT') { if (body.displayname !== undefined) user.displayname = body.displayname; }
        return response(200, user);
      }
      match = /^\/_palpo\/admin\/v1\/users\/(.+)\/joined_rooms$/.exec(path);
      if (match) return response(200, { joined_rooms: users.get(match[1])?.rooms ?? [] });
      match = /^\/_palpo\/admin\/v1\/deactivate\/(.+)$/.exec(path);
      if (match) { const user = users.get(match[1]); if (!user) return missing(); user.deactivated = true; user.rooms = []; return response(200, {}); }
    }
    return missing();
  };
  const store = new Store(path);
  const palpo = new Palpo('http://fixture.invalid', fetch);
  const service = new Service({ store, palpo, serverName: 'example.test', callbackOrigins: ['https://fleet.example.test'], transportOrigin, relayOrigin, outboundOptions });
  const fulfill = (fleetId, requestId) => {
    const request = requests.get(`${fleetId}:${requestId}`), mxid = `@${fleetId}_agent_actual:example.test`;
    Object.assign(request, { state: 'active', agentMxid: mxid, bound: true, serving: { framework: 'codex', model: 'fixture-model', reasoning: 'high', tier: 'strong' }, ready: true, allocatedTokens: request.requestedTokens, fulfillment: { phase: 'ready', incomplete: false } });
    users.set(mxid, { name: mxid, appservice_id: fleetId, deactivated: false, displayname: 'Actual coding agent', rooms: [request.targetRoomId] });
    putState(rooms.get(request.targetRoomId), 'm.room.member', mxid, { membership: 'join' }, mxid);
    return request;
  };
  return { service, store, palpo, actors, registrations, users, calls, fetch, rooms, events, receipts, requests, publishedOffers, putState, fulfill, denyAdmin: () => { deniedAdmin = true; } };
}
export const fleetInput = { requestId: 'request-one', name: 'Coding team', ownerMxid: '@owner:example.test', transportMode: 'callback', callbackUrl: 'https://fleet.example.test/matrix' };
export const agentInput = { agentId: 'coding_01', displayName: 'Coding agent', role: 'coding', approvedRequestId: 'approved-42' };
