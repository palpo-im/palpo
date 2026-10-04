// Explicit fake Hagency for frontend/service tests. It is not live integration
// evidence; real reservation enforcement is exercised in the Rust store suite.
import { fleetInput } from './fixture.mjs';
export const projectResource = `resource_${'a'.repeat(24)}`;
export async function contributedFleet(f, workflow, inbox, requestId = 'hagency-registration') {
  const created = await f.service.create({ ...fleetInput, requestId, transportMode: 'callback' }, '@admin:example.test', 'admin-secret');
  await workflow.connect(created.id, '@owner:example.test', 'owner-secret');
  const fleet = f.service.fleet(created.id), at = inbox.now();
  fleet.transport = f.service.outbound.transport(fleet.id);
  fleet.connection = { ...fleet.connection, verifiedAt: new Date(at).toISOString(), generation: fleet.transport.generation };
  fleet.state = 'ready';
  const capabilities = { ...fleet.capabilities, projectWorkflow: { v: 1, registrationGeneration: 1 } };
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
      .map(r => ({ grant: r.grant, state: r.state, reserved: r.reserved }));
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true,
      contributionPage: { v: 1, registrationGeneration: 1, observedAtMs: inbox.now(), after: '', nextAfter: null, contributions } }, workflow);
  }
}
