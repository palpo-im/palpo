import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical, outboundStatusCurrent } from './outbound.mjs';
import { fields } from './miniapp.mjs';

const hash = v => createHash('sha256').update(canonical(v)).digest('hex');
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const terminal = new Set(['command_refused', 'cleanup_failed']);

export class AgentLifecycle {
  constructor(inbox) { this.inbox = inbox; this.store = inbox.store; this.agents = inbox.agents; }
  scope(request, actor) {
    const original = this.inbox.state.records[request?.actionId];
    if (original?.kind !== 'agent' || original.state !== 'approved' || original.execution !== 'done' || !original.result?.engagementId) return null;
    const value = this.agents.grant(original.grantId, true);
    if (!value || (value.record.desiredRevision ?? value.grant.revision) !== value.grant.revision) return null;
    const admins = this.agents.administrators({ kind: 'agent_removal', grantId: original.grantId });
    return actor === value.grant.ownerMxid || admins.includes(actor) ? { ...value, original } : null;
  }
  canRemove(request, actor) {
    if (!this.scope(request, actor)) return false;
    const old = this.inbox.state.records[request.removalActionId];
    return !old || terminal.has(old.execution);
  }
  canRetry(row, actor) {
    const request = this.store.state.requests[row?.requestKey];
    return row?.kind === 'agent_removal' && request?.removalActionId === row.id && terminal.has(row.execution) && !!this.scope(request, actor);
  }
  async submit(input, actor, token) {
    fields(input, ['requestId', 'kind', 'agentRequestId', 'reason']);
    if (typeof input.requestId !== 'string' || !/^[A-Za-z0-9_-]{1,80}$/.test(input.requestId)
      || typeof input.reason !== 'string' || !input.reason.trim() || input.reason.length > 1000 || /[\x00-\x1f]/.test(input.reason))
      fail(400, 'invalid_removal', 'Confirm removal with a reason and stable request ID.');
    await this.agents.person(actor, token);
    const id = `action_${hash({ actor, requestId: input.requestId }).slice(0, 32)}`;
    const fingerprint = hash({ kind: 'agent_removal', actor, agentRequestId: input.agentRequestId, reason: input.reason.trim() });
    const old = this.inbox.state.records[id];
    if (old) {
      if (old.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This request ID has different content.');
      return this.inbox.get(id, actor, false);
    }
    const request = this.store.state.requests[input.agentRequestId];
    if (!request || !this.canRemove(request, actor)) fail(404, 'agent_not_available', 'This agent is unavailable for removal or already has removal in progress.');
    const { grant, fleet, project, original } = this.scope(request, actor);
    this.agents.commands.requireSupport(fleet);
    if (Object.keys(this.inbox.state.records).length >= this.inbox.maxRecords) fail(429, 'inbox_full', 'Too many workflow records.');
    this.store.atomic(() => {
      const row = this.inbox.state.records[id] = { id, kind: 'agent_removal', workflowVersion: 1, fingerprint,
        requestKey: request.id, grantId: grant.id, grantRevision: grant.revision, engagementId: original.result.engagementId,
        ownerMxid: grant.ownerMxid, requesterMxid: actor,
        cleanupAttemptBaseline: request.provider?.lifecycle?.cleanupAttempt ?? 0,
        payload: { name: original.payload.name, projectId: project.id, projectName: project.name, fleetId: fleet.id,
          agentRequestId: request.id, reason: input.reason.trim() },
        state: 'approved', execution: 'awaiting_hagency', revision: 1, createdAt: this.inbox.now(), updatedAt: this.inbox.now(),
        decision: { by: actor, at: this.inbox.now(), reason: input.reason.trim() } };
      const entry = this.agents.commands.enqueue(fleet, actor, { kind: 'revoke_agent', grantId: grant.id,
        grantRevision: grant.revision, engagementId: row.engagementId }, { commandId: `remove_${hash(id).slice(0, 40)}`, projectId: project.id, actionId: id });
      entry.actionKind = 'agent_removal'; row.commandRef = entry.command.commandId;
      const previous = this.inbox.state.records[request.removalActionId];
      if (previous) {
        previous.execution = 'superseded'; previous.revision++; previous.updatedAt = this.inbox.now();
        for (const notice of Object.values(this.inbox.state.notices)) if (notice.actionId === previous.id) notice.cancelled = true;
      }
      request.removalActionId = id; request.state = 'removal_queued'; request.usable = false;
      this.inbox.notify(row); this.store.audit(actor, 'agent.remove', fleet.id, id, 'awaiting_hagency');
    });
    return this.inbox.get(id, actor, false);
  }
  authorizes(entry) {
    const row = this.inbox.state.records[entry.actionId], request = this.store.state.requests[row?.requestKey], op = entry.command.operation;
    const value = this.scope(request, entry.command.actorMxid);
    return !!value && row?.kind === 'agent_removal' && request.removalActionId === row.id && row.commandRef === entry.command.commandId
      && row.decision.by === entry.command.actorMxid && row.grantId === op.grantId && row.grantRevision === op.grantRevision
      && op.engagementId === value.original.result.engagementId && row.engagementId === op.engagementId;
  }
  receipt(entry) {
    const row = this.inbox.state.records[entry.actionId];
    if (row?.kind !== 'agent_removal' || row.commandRef !== entry.command.commandId) return;
    row.execution = entry.state === 'applied' ? 'retiring' : 'command_refused';
    row.result = entry.receipt.outcome.result ?? { code: entry.receipt.outcome.code };
    row.updatedAt = this.inbox.now(); row.revision++;
    const request = this.store.state.requests[row.requestKey];
    if (request?.removalActionId === row.id) { request.state = entry.state === 'applied' ? 'retiring' : 'removal_refused'; request.usable = false; }
    this.inbox.notify(row);
  }
  observe(request) {
    const row = this.inbox.state.records[request.removalActionId], status = request.provider?.lifecycle;
    if (row?.kind !== 'agent_removal') return;
    if (row.execution === 'done') { request.state = 'removed'; request.usable = false; return; }
    if (!status) return;
    const entry = this.agents.commands.state.commands[`${request.fleetId}:${this.store.state.fleets[request.fleetId]?.projectWorkflow?.registrationGeneration}:${row.commandRef}`];
    if (entry?.receipt?.outcome.status !== 'applied') return;
    const noIdentity = status.agentMxid === null && status.localCleanup === 'not_required';
    const matrixDone = noIdentity || (status.matrixRetired && request.retirement?.state === 'complete' && request.retirement.mxid === status.agentMxid);
    // A held status page can describe the previous failure after a fresh retry
    // receipt arrives. Only a newer physical effect can offer another retry.
    const newAttempt = status.cleanupAttempt > row.cleanupAttemptBaseline;
    const execution = status.runtimeStopped && matrixDone ? 'done' : newAttempt && status.cleanupRetryable ? 'cleanup_failed'
      : newAttempt && status.localCleanup === 'uncertain' ? 'inspection_required' : 'retiring';
    request.state = execution === 'done' ? 'removed' : 'retiring'; request.usable = false;
    if (execution !== row.execution) {
      row.execution = execution; row.updatedAt = this.inbox.now(); row.revision++;
      this.inbox.notify(row);
    }
  }
  view(request) {
    const row = this.inbox.state.records[request.removalActionId], status = request.provider?.lifecycle;
    const observed = request.outboundStatus?.observedAt;
    const fleet = this.store.state.fleets[request.fleetId];
    return { removalActionId: row?.id ?? null, removalState: row?.execution ?? null,
      status: status ?? null, observedAt: observed ?? null,
      stale: fleet?.transport?.mode !== 'outbound' || !outboundStatusCurrent(fleet, request) };
  }
}

export function lifecycleStatus(value, status) {
  if (value === undefined || value === null) return null;
  const keys = ['v', 'agentMxid', 'localCleanup', 'runtimeStopped', 'cleanupRetryable', 'cleanupAttempt', 'matrixRetired', 'endedAtMs', 'allocatedTokens', 'spentTokensLowerBound', 'quotaPaused'];
  if (!value || typeof value !== 'object' || Object.keys(value).length !== keys.length || !keys.every(k => Object.hasOwn(value, k))
    || value.v !== 1 || value.agentMxid !== (status.agentMxid ?? null)
    || !['not_required', 'pending', 'complete', 'uncertain'].includes(value.localCleanup)
    || !['runtimeStopped', 'cleanupRetryable', 'matrixRetired', 'quotaPaused'].every(k => typeof value[k] === 'boolean')
    || !Number.isSafeInteger(value.allocatedTokens) || value.allocatedTokens <= 0
    || !Number.isSafeInteger(value.cleanupAttempt) || value.cleanupAttempt < 0
    || (value.spentTokensLowerBound !== null && (!Number.isSafeInteger(value.spentTokensLowerBound) || value.spentTokensLowerBound < 0))
    || (value.endedAtMs !== null && (!Number.isSafeInteger(value.endedAtMs) || value.endedAtMs <= 0))
    || (value.runtimeStopped && (status.state !== 'ended' || value.endedAtMs === null || !['not_required', 'complete'].includes(value.localCleanup)))
    || (value.cleanupRetryable && (status.state !== 'ended' || value.localCleanup !== 'pending' || value.runtimeStopped || value.cleanupAttempt === 0))
    || (value.matrixRetired && (!value.runtimeStopped || typeof value.agentMxid !== 'string')))
    fail(409, 'invalid_agent_lifecycle', 'Hagency returned an invalid lifecycle observation.');
  return structuredClone(value);
}
