import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical, isOutbound } from './outbound.mjs';

const fail = (status, code, message) => { throw new ApiError(status, code, message); };
export const commandDigest = value => createHash('sha256').update(canonical(value)).digest('hex');
const plain = value => value && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => plain(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
const id = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const positive = value => Number.isSafeInteger(value) && value > 0;
const matrix = (value, prefix, issuer) => typeof value === 'string' && Buffer.byteLength(value) <= 255
  && value.startsWith(prefix) && value.slice(1).includes(':') && value.slice(value.indexOf(':') + 1) === issuer && !/[\s\x00-\x1f\x7f]/.test(value);
const user = (value, issuer) => matrix(value, '@', issuer);
export function validateLimits(value) {
  return closed(value, ['tokens', 'maxAgents', 'maxRatePerDay']) && Object.values(value).every(positive) && value.maxAgents <= 10000;
}
export function validateGrant(value, issuer) {
  return closed(value, ['v', 'id', 'revision', 'delegationId', 'delegationRevision', 'projectId', 'roomId', 'ownerMxid', 'administratorMxids', 'allowSelfApproval', 'limits', 'expiresAtMs'])
    && value.v === 1 && id(value.id) && id(value.delegationId) && id(value.projectId) && positive(value.revision) && positive(value.delegationRevision)
    && matrix(value.roomId, '!', issuer) && user(value.ownerMxid, issuer) && typeof value.allowSelfApproval === 'boolean'
    && Array.isArray(value.administratorMxids) && value.administratorMxids.length > 0 && value.administratorMxids.length <= 16
    && new Set(value.administratorMxids).size === value.administratorMxids.length && value.administratorMxids.every(actor => user(actor, issuer))
    && validateLimits(value.limits) && positive(value.expiresAtMs);
}
function validateRequest(value, fleetId, issuer) {
  return closed(value, ['v', 'fleetId', 'requestId', 'requesterMxid', 'sourceRoomId', 'targetProjectId', 'targetRoomId', 'ownerMxid', 'ownerDmRoomId', 'role', 'requestedTokens', 'ratePerDay', 'authVersion', 'sourceEventId', 'agentDefinition'])
    && value.v === 1 && value.authVersion === 1 && value.fleetId === fleetId && id(value.requestId) && value.requestId.length <= 96
    && id(value.targetProjectId) && user(value.requesterMxid, issuer) && user(value.ownerMxid, issuer)
    && ['sourceRoomId', 'targetRoomId', 'ownerDmRoomId'].every(key => matrix(value[key], '!', issuer))
    && new Set([value.sourceRoomId, value.targetRoomId, value.ownerDmRoomId]).size === 3
    && /^[a-z][a-z0-9_-]{0,63}$/.test(value.role) && positive(value.requestedTokens) && positive(value.ratePerDay)
    && typeof value.sourceEventId === 'string' && /^\$[^\s\x00-\x1f]{1,254}$/.test(value.sourceEventId)
    && closed(value.agentDefinition, ['name', 'resourceId']) && typeof value.agentDefinition.name === 'string'
    && value.agentDefinition.name === value.agentDefinition.name.normalize('NFC') && value.agentDefinition.name.length <= 64
    && /^\p{L}[\p{L}\p{M}\p{N}_-]*$/u.test(value.agentDefinition.name) && /^resource_[a-f0-9]{24}$/.test(value.agentDefinition.resourceId);
}
export function validateProjectCommand(command, fleet, issuer) {
  let valid = closed(command, ['v', 'commandId', 'fleetId', 'registrationGeneration', 'issuer', 'actorMxid', 'expiresAtMs', 'operation'])
    && command.v === 1 && id(command.commandId) && command.fleetId === fleet.id && command.issuer === issuer && user(command.actorMxid, issuer)
    && positive(command.registrationGeneration) && command.registrationGeneration === fleet.projectWorkflow?.registrationGeneration && positive(command.expiresAtMs);
  const op = command?.operation;
  if (!plain(op)) valid = false;
  else switch (op.kind) {
    case 'reserve_project': valid &&= closed(op, ['kind', 'grant']) && validateGrant(op.grant, issuer); break;
    case 'assign_project_admins': valid &&= closed(op, ['kind', 'grantId', 'expectedRevision', 'administrators', 'allowSelfApproval'])
      && id(op.grantId) && positive(op.expectedRevision) && typeof op.allowSelfApproval === 'boolean'
      && Array.isArray(op.administrators) && op.administrators.length > 0 && op.administrators.length <= 16
      && new Set(op.administrators).size === op.administrators.length && op.administrators.every(actor => user(actor, issuer)); break;
    case 'approve_agent': case 'reject_agent': valid &&= closed(op, ['kind', 'grantId', 'grantRevision', 'request', ...(op.kind === 'approve_agent' ? ['allocatedTokens'] : [])])
      && id(op.grantId) && positive(op.grantRevision) && validateRequest(op.request, fleet.id, issuer)
      && (op.kind === 'reject_agent' || positive(op.allocatedTokens)); break;
    case 'top_up_agent': valid &&= closed(op, ['kind', 'grantId', 'grantRevision', 'engagementId', 'requesterMxid', 'addTokens'])
      && id(op.grantId) && positive(op.grantRevision) && /^en_[a-f0-9]{32}$/.test(op.engagementId) && user(op.requesterMxid, issuer) && positive(op.addTokens); break;
    case 'revoke_agent': valid &&= closed(op, ['kind', 'grantId', 'grantRevision', 'engagementId'])
      && id(op.grantId) && positive(op.grantRevision) && /^en_[a-f0-9]{32}$/.test(op.engagementId); break;
    case 'revoke_project': valid &&= closed(op, ['kind', 'grantId', 'expectedRevision']) && id(op.grantId) && positive(op.expectedRevision); break;
    default: valid = false;
  }
  if (!valid || Buffer.byteLength(canonical(command)) > 48 * 1024) fail(400, 'invalid_project_command', 'Use a supported, bounded project operation.');
  return command;
}
const refusalCodes = new Set(['expired', 'grant_expired', 'grant_revoked', 'authority', 'conflict', 'not_found', 'insufficient_capacity', 'resource_unavailable', 'invalid', 'state']);
const projectOperations = new Set(['reserve_project', 'assign_project_admins', 'revoke_project']);

export class ProjectCommands {
  constructor(service, inbox, { now = Date.now, adminToken, maxRecords = 100000 } = {}) {
    Object.assign(this, { service, inbox, now, adminToken, maxRecords });
    this.store = service.store;
    this.store.state.projectWorkflow ??= { commands: {}, grants: {}, contributions: {} };
  }
  get state() { return this.store.state.projectWorkflow; }
  requireSupport(fleet) {
    if (!isOutbound(fleet) || fleet.projectWorkflow?.v !== 1 || !positive(fleet.projectWorkflow.registrationGeneration)) {
      fail(409, 'project_workflow_unavailable', 'This Hagency has not advertised project approval support.');
    }
  }
  // Called within the same Store.atomic transaction as the human decision.
  enqueue(fleet, actor, operation, { commandId, projectId, actionId = null, expiresAtMs = this.now() + 7 * 86400000 } = {}) {
    if (!this.store.db.isTransaction) throw new Error('Project decisions and outbound commands require one Store.atomic transaction');
    this.requireSupport(fleet);
    const command = validateProjectCommand({ v: 1, commandId, fleetId: fleet.id, registrationGeneration: fleet.projectWorkflow.registrationGeneration,
      issuer: this.service.serverName, actorMxid: actor, expiresAtMs, operation }, fleet, this.service.serverName);
    if (expiresAtMs > this.now() + 7 * 86400000 || !id(projectId) || (actionId !== null && !id(actionId))) fail(400, 'invalid_project_command', 'A project and action binding are required.');
    const key = `${fleet.id}:${command.registrationGeneration}:${commandId}`, digest = commandDigest(command), old = this.state.commands[key];
    if (old) {
      if (old.digest !== digest || old.projectId !== projectId || old.actionId !== actionId) fail(409, 'command_conflict', 'This command ID already has different content.');
      return old;
    }
    if (Object.keys(this.state.commands).length >= this.maxRecords) fail(503, 'command_store_full', 'The durable project command store is full.');
    const entry = { command, digest, projectId, actionId, state: 'queued', createdAt: this.now(), receipt: null };
    this.state.commands[key] = entry;
    this.service.outbound.enqueue(fleet, 'work', 'workflow', `workflow_${commandId}`, command);
    return entry;
  }
  entry(fleet, commandId) { return this.state.commands[`${fleet.id}:${fleet.projectWorkflow?.registrationGeneration}:${commandId}`]; }
  async authorize(fleet, input) {
    if (!closed(input, ['commandId', 'commandDigest']) || !id(input.commandId) || !/^[a-f0-9]{64}$/.test(input.commandDigest)) fail(400, 'invalid_command_authorization', 'Identify the original command and digest.');
    this.requireSupport(fleet);
    const entry = this.entry(fleet, input.commandId);
    if (!entry || entry.digest !== input.commandDigest) fail(409, 'command_binding_conflict', 'The command does not match this fleet.');
    if (!this.adminToken) fail(503, 'workflow_authority_unavailable', 'The server must configure its workflow authority lookup.');
    let identity;
    try { identity = await this.service.palpo.user(entry.command.actorMxid, this.adminToken); }
    catch { fail(503, 'workflow_authority_unavailable', 'Current Matrix authority is unavailable. Retry this command.'); }
    const { command } = entry, op = command.operation, project = this.store.state.projects?.[entry.projectId];
    let allowed = !!identity && !identity.deactivated && !identity.locked && !identity.appservice_id && !entry.cancelled
      && command.expiresAtMs > this.now() && !!project && project.fleetId === fleet.id;
    if (projectOperations.has(op.kind)) allowed &&= !!identity?.admin && this.inbox.canApproveProjects(command.actorMxid, true);
    const record = op.grantId ? this.state.grants[op.grantId] : null, grant = record?.grant;
    if (op.kind === 'reserve_project') {
      const action = this.inbox.state.records[entry.actionId];
      allowed &&= op.grant.projectId === project?.id && op.grant.ownerMxid === project?.ownerMxid && op.grant.roomId === project?.roomId
        && action?.state === 'approved' && action.ownerMxid === project.ownerMxid && action.decision?.by === command.actorMxid;
    } else {
      allowed &&= record?.fleetId === fleet.id && grant?.projectId === project?.id;
      if (op.kind !== 'revoke_project' && op.kind !== 'assign_project_admins') {
        allowed &&= (record?.desiredRevision ?? grant?.revision) === op.grantRevision;
        const admins = record?.desiredAdministrators ?? grant?.administratorMxids ?? [];
        if (op.kind === 'revoke_agent') allowed &&= command.actorMxid === grant?.ownerMxid || admins.includes(command.actorMxid);
        else {
          const requester = op.request?.requesterMxid ?? op.requesterMxid;
          allowed &&= record?.state === 'accepted' && grant?.expiresAtMs > this.now() && admins.includes(command.actorMxid)
            && (grant?.allowSelfApproval || requester !== command.actorMxid);
        }
      } else allowed &&= grant?.revision === op.expectedRevision;
    }
    // This short lease linearizes the fresh role lookup. New decisions cannot
    // use cached authority; Hagency rechecks the deadline after its writer lock.
    return { v: 1, commandId: command.commandId, commandDigest: entry.digest, allowed: !!allowed, validUntilMs: this.now() + 10000 };
  }
  validateReceipts(fleet, receipts) {
    if (!Array.isArray(receipts) || receipts.length > 8) fail(400, 'invalid_command_receipts', 'At most eight command receipts are accepted per update.');
    if (new Set(receipts.map(r => r?.commandId)).size !== receipts.length) fail(409, 'command_receipt_conflict', 'Duplicate command receipts are not accepted.');
    return receipts.map(receipt => {
      const entry = this.entry(fleet, receipt?.commandId);
      if (!closed(receipt, ['v', 'fleetId', 'registrationGeneration', 'commandId', 'commandDigest', 'completedAtMs', 'outcome'])
        || receipt.v !== 1 || receipt.fleetId !== fleet.id || receipt.registrationGeneration !== fleet.projectWorkflow?.registrationGeneration
        || !entry || receipt.commandDigest !== entry.digest || !positive(receipt.completedAtMs) || receipt.completedAtMs > this.now() + 5000) fail(409, 'command_receipt_mismatch', 'The receipt does not match its original command.');
      if (entry.receipt && canonical(entry.receipt) !== canonical(receipt)) fail(409, 'command_receipt_conflict', 'The command already has a different result.');
      // A historical receipt is immutable even after later assignment/revoke
      // results advance the current grant. Replaying it never restores state.
      if (entry.receipt) return { entry, receipt };
      const outcome = receipt.outcome, op = entry.command.operation;
      if (closed(outcome, ['status', 'code']) && outcome.status === 'refused' && refusalCodes.has(outcome.code)) return { entry, receipt };
      if (!closed(outcome, ['status', 'result']) || outcome.status !== 'applied') fail(400, 'invalid_command_receipt', 'Use a typed business result.');
      const result = outcome.result;
      let valid = false;
      if (op.kind === 'reserve_project') valid = closed(result, ['kind', 'grant']) && result.kind === 'grant' && canonical(result.grant) === canonical(op.grant);
      else if (op.kind === 'assign_project_admins') {
        const existing = this.state.grants[op.grantId]?.grant;
        const expected = existing && { ...existing, revision: op.expectedRevision + 1, administratorMxids: op.administrators, allowSelfApproval: op.allowSelfApproval };
        valid = closed(result, ['kind', 'grant']) && result.kind === 'grant' && expected && canonical(result.grant) === canonical(expected);
      } else if (op.kind === 'revoke_project') valid = closed(result, ['kind', 'grantId', 'revision']) && result.kind === 'revoked_project' && result.grantId === op.grantId && result.revision === op.expectedRevision;
      else {
        const engagementId = op.engagementId ?? `en_${createHash('sha256').update(JSON.stringify([fleet.id, op.request.requestId])).digest('hex').slice(0, 32)}`;
        const states = { approve_agent: ['reserved'], reject_agent: ['rejected'], top_up_agent: ['reserved', 'active'], revoke_agent: ['revoked'] };
        valid = closed(result, ['kind', 'engagementId', 'state', 'allocatedTokens', 'cleanup']) && result.kind === 'agent'
          && result.engagementId === engagementId && states[op.kind]?.includes(result.state) && positive(result.allocatedTokens)
          && ['not_required', 'pending', 'complete', 'uncertain'].includes(result.cleanup)
          && (op.kind !== 'approve_agent' || result.allocatedTokens === op.allocatedTokens);
      }
      if (result?.kind === 'grant' && this.state.grants[result.grant?.id]?.fleetId && this.state.grants[result.grant.id].fleetId !== fleet.id) valid = false;
      if (!valid) fail(409, 'command_result_mismatch', 'The result is outside its command scope.');
      return { entry, receipt };
    });
  }
  // Called within the same update transaction as the fleet sequence and status.
  applyReceipts(records) {
    if (!this.store.db.isTransaction) throw new Error('Command receipts and outbound updates require one Store.atomic transaction');
    for (const { entry, receipt } of records) {
      if (entry.receipt) continue;
      entry.receipt = structuredClone(receipt); entry.state = receipt.outcome.status;
      const result = receipt.outcome.result;
      if (result?.kind === 'grant') this.state.grants[result.grant.id] = { fleetId: receipt.fleetId, grant: structuredClone(result.grant), state: 'accepted' };
      if (result?.kind === 'revoked_project' && this.state.grants[result.grantId]) this.state.grants[result.grantId].state = 'revoked';
      this.onReceipt?.(entry);
      this.store.audit(entry.command.actorMxid, 'project.command_result', receipt.fleetId, receipt.commandId, entry.state);
    }
  }
}
