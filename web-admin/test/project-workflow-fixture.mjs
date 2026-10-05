// Explicit fake Hagency for frontend/service tests. It is not live integration
// evidence; real reservation enforcement is exercised in the Rust store suite.
import { fleetInput } from './fixture.mjs';
import { createHash } from 'node:crypto';
export const projectResource = `resource_${'a'.repeat(24)}`;
export async function contributedFleet(f, workflow, inbox, requestId = 'hagency-registration') {
  const created = await f.service.create({ ...fleetInput, requestId, transportMode: 'callback' }, '@admin:example.test', 'admin-secret');
  await workflow.connect(created.id, '@owner:example.test', 'owner-secret');
  const fleet = f.service.fleet(created.id), at = inbox.now();
  fleet.transport = f.service.outbound.transport(fleet.id);
  fleet.connection = { ...fleet.connection, verifiedAt: new Date(at).toISOString(), generation: fleet.transport.generation };
  fleet.state = 'ready';
  const capabilities = { ...fleet.capabilities, projectWorkflow: { v: 1, registrationGeneration: 1, unusedRelease: true } };
  const grant = { v: 1, id: 'contribution_native', revision: 1, fleetId: fleet.id, registrationGeneration: 1,
    issuer: 'example.test', resourceId: projectResource, limits: { tokens: 2000000, maxAgents: 10, maxRatePerDay: 200000 }, expiresAtMs: at + 30 * 86400000 };
  // Fixture clocks can be deterministic, so keep both adapter clocks aligned.
  inbox.projectCommands.now = () => inbox.now();
  await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: 1, heartbeat: true, capabilities,
    contributionPage: { v: 1, registrationGeneration: 1, observedAtMs: at, after: '', nextAfter: null,
      contributions: [{ grant, state: 'active', reserved: { tokens: 0, maxAgents: 0, maxRatePerDay: 0 } }] } }, workflow);
  return fleet;
}
export function projectBudget() {
  return [{ contributionId: 'contribution_native', resourceId: projectResource,
    limits: { tokens: 400000, maxAgents: 4, maxRatePerDay: 40000 }, durationHours: 24 }];
}
export const projectAdministrators = { administrators: ['@other:example.test'], allowSelfApproval: false };
export async function acceptProjectReservations(f, workflow, inbox, minAgeMs = 0) {
  for (const fleet of Object.values(f.store.state.fleets)) {
    const entries = Object.values(inbox.projectCommands.state.commands).filter(e => !e.receipt && inbox.now() - e.createdAt >= minAgeMs && e.command.fleetId === fleet.id && e.command.operation.kind === 'reserve_project');
    if (!entries.length) continue;
    const commandReceipts = entries.slice(0, 8).map(e => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId,
      commandDigest: e.digest, completedAtMs: inbox.now(), outcome: { status: 'applied', result: { kind: 'grant', grant: e.command.operation.grant } } }));
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true, commandReceipts }, workflow);
  }
}
export async function refreshContributions(f, workflow, inbox) {
  for (const fleet of Object.values(f.store.state.fleets)) {
    const contributions = Object.values(inbox.projectCommands.state.contributions).filter(r => r.grant.fleetId === fleet.id)
      .map(r => ({ grant: r.grant, state: r.state, reserved: r.reserved, ...(r.released ? { released: r.released } : {}) }));
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true,
      contributionPage: { v: 1, registrationGeneration: 1, observedAtMs: inbox.now(), after: '', nextAfter: null, contributions } }, workflow);
  }
}

export async function acceptAgentDecisions(f, workflow, inbox) {
  for (const fleet of Object.values(f.store.state.fleets)) {
    const entries = Object.values(inbox.projectCommands.state.commands).filter(e => !e.receipt && e.command.fleetId === fleet.id
      && ['approve_agent', 'reject_agent', 'top_up_agent'].includes(e.command.operation.kind));
    if (!entries.length) continue;
    const commandReceipts = entries.slice(0, 8).map(e => {
      const op = e.command.operation;
      const agent = op.kind === 'top_up_agent' ? inbox.state.records[e.actionId] : null;
      const request = agent && f.store.state.requests[agent.requestKey];
      return { v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest, completedAtMs: inbox.now(),
        outcome: { status: 'applied', result: { kind: 'agent', engagementId: op.engagementId ?? `en_${createHash('sha256').update(JSON.stringify([fleet.id, op.request.requestId])).digest('hex').slice(0, 32)}`,
          state: op.kind === 'reject_agent' ? 'rejected' : 'reserved', allocatedTokens: request ? inbox.agents.allocation(request).tokens + op.addTokens : op.allocatedTokens ?? op.request.requestedTokens,
          cleanup: 'not_required' } } };
    });
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true, commandReceipts }, workflow);
  }
}

export async function acceptAgentRemovals(f, workflow, inbox) {
  for (const fleet of Object.values(f.store.state.fleets)) {
    const entries = Object.values(inbox.projectCommands.state.commands).filter(e => !e.receipt && e.command.fleetId === fleet.id && e.command.operation.kind === 'revoke_agent');
    if (!entries.length) continue;
    const commandReceipts = entries.slice(0, 8).map(e => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1,
      commandId: e.command.commandId, commandDigest: e.digest, completedAtMs: inbox.now(), outcome: { status: 'applied', result: {
        kind: 'agent', engagementId: e.command.operation.engagementId, state: 'revoked', cleanup: 'pending',
        allocatedTokens: inbox.agents.allocation(f.store.state.requests[inbox.state.records[e.actionId].requestKey]).tokens } } }));
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true, commandReceipts }, workflow);
  }
}

export async function publishAgentCleanup(f, workflow, inbox, stage) {
  for (const request of Object.values(f.store.state.requests)) {
    const row = inbox.state.records[request.removalActionId];
    if (!row || !['retiring', 'cleanup_failed'].includes(row.execution)) continue;
    const fleet = f.service.fleet(request.fleetId), agentMxid = `@${fleet.id}_${row.engagementId}:example.test`;
    f.users.set(agentMxid, f.users.get(agentMxid) ?? { name: agentMxid, appservice_id: fleet.id, deactivated: false, displayname: row.payload.name, rooms: [request.payload.targetRoomId] });
    const attempts = Object.values(inbox.projectCommands.state.commands).filter(e => e.command.operation.kind === 'revoke_agent' && e.command.operation.engagementId === row.engagementId).length;
    const failed = stage === 'failed' && attempts === 1;
    const lifecycle = { v: 1, agentMxid, localCleanup: stage === 'complete' ? 'complete' : 'pending', runtimeStopped: stage === 'complete',
      cleanupRetryable: failed, cleanupAttempt: attempts, matrixRetired: false, endedAtMs: Date.now(), allocatedTokens: inbox.agents.allocation(request).tokens,
      spentTokensLowerBound: null, quotaPaused: false };
    const status = { ...request.payload, sourceEventId: request.sourceEventId, engagementId: row.engagementId, state: 'ended',
      agentMxid, ready: false, bound: false, observedAt: new Date().toISOString(), lifecycle };
    const publish = () => f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation,
      sequence: fleet.transport.sequence + 1, heartbeat: true, statuses: [status] }, workflow);
    await publish(); // The real retirement handler first needs the exact identity observation.
    if (stage === 'complete') {
      await f.service.retireAllocatedAgent(fleet, { requestId: request.requestId, agentMxid, endedAt: lifecycle.endedAtMs, localStopped: true }, 'admin-secret');
      lifecycle.matrixRetired = true; await publish();
    }
  }
}

// Explicit readiness fixture: business receipts alone never call this helper.
// Publishes a provider observation AND puts the identity in the fake Matrix room.
export async function publishReadyAgent(f, workflow, inbox, request) {
  const action = inbox.state.records[request.actionId];
  if (action?.execution !== 'done' || !action.result?.engagementId || request.removalActionId) return;
  const fleet = f.service.fleet(request.fleetId), agentMxid = `@${fleet.id}_${action.result.engagementId}:example.test`;
  f.users.set(agentMxid, { name: agentMxid, appservice_id: fleet.id, deactivated: false, displayname: request.payload.agentDefinition.name, rooms: [request.payload.targetRoomId] });
  f.putState(f.rooms.get(request.payload.targetRoomId), 'm.room.member', agentMxid, { membership: 'join' }, agentMxid);
  const status = { ...request.payload, sourceEventId: request.sourceEventId, engagementId: action.result.engagementId,
    state: 'active', agentMxid, ready: true, bound: true, allocatedTokens: inbox.agents.allocation(request).tokens,
    observedAt: new Date().toISOString(), serving: { framework: 'codex', model: 'fixture-model' },
    fulfillment: { phase: 'ready', incomplete: false } };
  await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation,
    sequence: fleet.transport.sequence + 1, heartbeat: true, statuses: [status] }, workflow);
  return agentMxid;
}
