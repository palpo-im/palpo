// ADR 0010: the native host borrows Matrix authority; app sessions never own it.
import { randomBytes, createHash } from 'node:crypto';
import { ApiError, publicFleet } from './service.mjs';

export const APP_ID = 'im.palpo.operations';
export const SERVICES = Object.freeze({
  'palpo.intent.new': 'Prepare a stable identifier for a new request',
  'palpo.session.open': 'Use your signed-in Matrix account in Palpo',
  'palpo.session.disconnect': 'Disconnect this app without signing out of Rinx',
  'palpo.catalog.list': 'Browse published Hagency resources',
  'palpo.projects.list': 'Read projects your Matrix account can access',
  'palpo.projects.create': 'Create projects as your Matrix account',
  'palpo.requests.list': 'Read your agent requests and their progress',
  'palpo.requests.create': 'Request agents for projects you can access',
  'palpo.fleets.list': 'Read fleets you own or administer',
  'palpo.fleets.register': 'Register fleets within your administrator permissions',
  'palpo.fleets.install': 'Retry an authorized fleet installation',
  'palpo.fleets.set_state': 'Pause, resume or revoke fleet registration credentials',
  'palpo.fleets.migrate': 'Migrate a fleet to outbound delivery',
  'palpo.fleets.queue': 'Read a fleet delivery queue summary',
  'palpo.fleets.export': 'Save your fleet configuration through the Rinx file picker',
  'palpo.fleets.connect': 'Verify a connection for a fleet you own',
  'palpo.agents.list': 'Read managed Matrix identities within your permissions',
  'palpo.agents.register': 'Register a Matrix identity for an approved agent',
  'palpo.agents.rename': 'Rename a managed Matrix identity',
  'palpo.agents.retire': 'Retire a managed Matrix identity',
  'palpo.activity.list': 'Read the Palpo administrator audit history',
  'palpo.accounts.list': 'Read pending account signup requests',
  'palpo.inbox.list': 'Read your pending Palpo actions and history',
  'palpo.inbox.submit': 'Submit resource contributions or project requests',
  'palpo.inbox.get': 'Read the latest state of an authorized action',
  'palpo.inbox.decide': 'Approve or reject within your server permissions',
  'palpo.inbox.activate': 'Continue an approved project as its owner',
  'palpo.inbox.seen': 'Mark a notification seen without completing its action',
  'palpo.inbox.snooze': 'Snooze reminders for an action you can take',
});
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const text = (value, name, max = 255) => {
  if (typeof value !== 'string' || !value.trim() || value.length > max || /[\x00-\x1f]/.test(value)) fail(400, 'invalid_arguments', `${name} is invalid.`);
  return value.trim();
};
export function fields(input, names) {
  if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).some(key => !names.includes(key))) fail(400, 'invalid_arguments', 'Unexpected operation arguments.');
}
const credentials = header => /^Bearer ([A-Za-z0-9._~+\/-]+={0,2})$/.exec(header ?? '')?.[1];

export class MiniApp {
  constructor(service, workflow, accounts, inbox, { ttlMs = 15 * 60 * 1000, now = Date.now, maxSessions = 512 } = {}) {
    Object.assign(this, { service, workflow, accounts, inbox, ttlMs, now, maxSessions });
    this.sessions = new Map();
  }
  async admin(token) {
    try { await this.service.palpo.requireAdmin(token); return true; }
    catch (cause) { if (cause.status === 403) return false; throw cause; }
  }
  async open(header, input) {
    fields(input, ['appId', 'bundleDigest', 'services']);
    if (input.appId !== APP_ID || !/^[a-f0-9]{64}$/.test(input.bundleDigest ?? '')
      || !Array.isArray(input.services) || !input.services.length || input.services.length > Object.keys(SERVICES).length
      || new Set(input.services).size !== input.services.length || input.services.some(name => !Object.hasOwn(SERVICES, name))) fail(403, 'app_not_supported', 'This app or its requested services are not supported.');
    const token = credentials(header);
    if (!token || token.length > 8192) fail(401, 'matrix_session_required', 'Use the current Rinx Matrix session.');
    const identity = await this.service.palpo.call('/_matrix/client/v3/account/whoami', token);
    if (identity.is_guest) fail(403, 'guest_forbidden', 'A registered Matrix account is required.');
    const actor = this.service.owner(identity.user_id);
    const isAdmin = await this.admin(token);
    for (const [key, session] of this.sessions) if (session.expiresAt <= this.now()) this.sessions.delete(key);
    if (this.sessions.size >= this.maxSessions) fail(503, 'sessions_busy', 'The app service is busy. Try again later.');
    const sessionToken = randomBytes(32).toString('base64url');
    const session = { token, actor, appId: input.appId, bundleDigest: input.bundleDigest, services: [...input.services], expiresAt: this.now() + this.ttlMs };
    // Store only a hash of the mini-app bearer. Borrowed Matrix token stays server-side.
    this.sessions.set(this.key(sessionToken), session);
    return { sessionToken, expiresAt: session.expiresAt, ...this.identity(session, isAdmin) };
  }
  key(token) { return createHash('sha256').update(token).digest('hex'); }
  identity(session, isAdmin) {
    return { version: 1, userId: session.actor, isAdmin, serverName: this.service.serverName,
      services: session.services, callbackOrigins: isAdmin ? [...this.service.callbackOrigins] : [],
      outboundAvailable: !!(this.service.transportOrigin && this.service.relayOrigin),
      features: { inbox: true, contributions: true, projectApproval: true, remoteAgentDecisions: false, topUps: false } };
  }
  async authenticate(header) {
    const bearer = credentials(header), key = bearer ? this.key(bearer) : '';
    const session = this.sessions.get(key);
    if (!session || session.expiresAt <= this.now()) {
      this.sessions.delete(key); fail(401, 'app_session_expired', 'Reopen the app using your Rinx session.');
    }
    let identity;
    try { identity = await this.service.palpo.call('/_matrix/client/v3/account/whoami', session.token); }
    catch (cause) { if (cause.status === 401) this.sessions.delete(key); throw cause; }
    if (identity.user_id !== session.actor || identity.is_guest) { this.sessions.delete(key); fail(403, 'identity_changed', 'Matrix identity changed. Reopen the app.'); }
    return { key, session };
  }
  async disconnect(header) {
    const { key } = await this.authenticate(header);
    this.sessions.delete(key);
    // Deliberately does not call Matrix /logout on the host's borrowed token.
    return { disconnected: true };
  }
  async call(header, input) {
    fields(input, ['service', 'args']);
    const { session } = await this.authenticate(header);
    if (!session.services.includes(input.service)) fail(403, 'service_not_granted', 'This app session was not granted that operation.');
    const { service, args = {} } = input;
    const actor = session.actor, token = session.token, signal = AbortSignal.timeout(8000);
    const id = () => text(args.fleetId, 'Fleet ID');
    // The existing Service serial queue covers both browser and app mutations.
    const mutate = fn => this.service.serial(async () => {
      // Recheck after waiting in the queue; demotion/expiry must affect queued work.
      await this.authenticate(header);
      return fn();
    });
    switch (service) {
      case 'palpo.intent.new': fields(args, []); return { requestId: randomBytes(20).toString('hex') };
      case 'palpo.session.open': fields(args, []); return this.identity(session, await this.admin(token));
      case 'palpo.session.disconnect': fields(args, []); return this.disconnect(header);
      case 'palpo.catalog.list': {
        fields(args, ['projectId']);
        let project;
        if (args.projectId !== undefined) {
          const projectId = text(args.projectId, 'Project ID');
          project = (await this.workflow.projects(actor, token, signal)).find(p => p.id === projectId);
          if (!project) fail(404, 'project_not_found', 'Project not found.');
          if (!project.canRequest) fail(409, 'project_not_ready', 'This project is not ready for agent requests.');
        }
        const fleets = await this.workflow.catalog(actor, signal);
        return { fleets: fleets.filter(f => !project || f.id === project.fleetId).map(f => ({ ...f,
          capabilities: f.capabilities ? { ...f.capabilities, offers: f.capabilities.offers.map(o => ({ ...o,
            resources: (o.resources ?? []).filter(r => !project?.resourceGrant || project.resourceGrant.resourceIds.includes(r.id)) })) } : null })) };
      }
      case 'palpo.projects.list': fields(args, []); return { projects: await this.workflow.projects(actor, token, signal) };
      case 'palpo.projects.create': fields(args, ['requestId', 'name', 'fleetId', 'roomId']); return mutate(async () => ({ project: await this.workflow.createProject(args, actor, token) }));
      case 'palpo.requests.list': fields(args, []); return { requests: (await this.workflow.requests(actor, token, signal)).map(request => ({ ...request, agentDefinition: request.agentDefinition ?? null })) };
      case 'palpo.requests.create': fields(args, ['requestId', 'projectId', 'role', 'requestedTokens', 'ratePerDay', 'agentDefinition']); return mutate(async () => ({ request: await this.workflow.request(args, actor, token) }));
      case 'palpo.fleets.list': {
        fields(args, []); const isAdmin = await this.admin(token);
        return { fleets: Object.values(this.service.store.state.fleets).filter(fleet => isAdmin || fleet.ownerMxid === actor).map(publicFleet) };
      }
      case 'palpo.fleets.export': fields(args, ['fleetId']); return mutate(() => this.service.credentials(id(), actor));
      case 'palpo.fleets.connect': fields(args, ['fleetId']); return mutate(() => this.workflow.connect(id(), actor, token));
      case 'palpo.inbox.list': fields(args, ['view', 'offset', 'limit']); return this.inbox.list(actor, await this.admin(token), args);
      case 'palpo.inbox.get': fields(args, ['id']); return this.inbox.get(text(args.id, 'Action ID'), actor, await this.admin(token));
      case 'palpo.inbox.submit': return mutate(() => this.inbox.submit(args, actor));
      case 'palpo.inbox.decide': return mutate(async () => { await this.service.palpo.requireAdmin(token); return this.inbox.decide(args, actor, token); });
      case 'palpo.inbox.activate': return mutate(() => this.inbox.activate(args, actor, token));
      case 'palpo.inbox.seen': return this.inbox.seen(args, actor, await this.admin(token));
      case 'palpo.inbox.snooze': return this.inbox.snooze(args, actor, await this.admin(token));
    }
    // These existing operations are server-administrator operations, exactly as
    // in the browser. A package grant never makes an ordinary account an admin.
    await this.service.palpo.requireAdmin(token);
    const adminMutation = fn => mutate(async () => { await this.service.palpo.requireAdmin(token); return fn(); });
    switch (service) {
      case 'palpo.activity.list': fields(args, []); return { events: this.service.store.state.audit.slice(-200).reverse() };
      case 'palpo.accounts.list': fields(args, []); return this.accounts.adminView();
      case 'palpo.fleets.register': fields(args, ['requestId', 'name', 'ownerMxid', 'transportMode', 'callbackUrl']); return adminMutation(async () => ({ fleet: await this.service.create(args, actor, token) }));
      case 'palpo.fleets.install': fields(args, ['fleetId']); return adminMutation(async () => ({ fleet: await this.service.install(id(), actor, token) }));
      case 'palpo.fleets.set_state': {
        fields(args, ['fleetId', 'action']);
        if (!['pause', 'resume', 'revoke'].includes(args.action)) fail(400, 'invalid_action', 'Select pause, resume or revoke.');
        return adminMutation(async () => ({ fleet: await this.service.setState(id(), args.action, actor, token) }));
      }
      case 'palpo.fleets.queue': fields(args, ['fleetId']); return { queue: this.service.outbound.usage(this.service.fleet(id())) };
      case 'palpo.fleets.migrate': fields(args, ['fleetId', 'requestId', 'rotate']); return adminMutation(async () => ({ fleet: await this.service.migrateOutbound(id(), args, actor, token) }));
      case 'palpo.agents.list': fields(args, ['fleetId']); return { agents: await this.service.agents(id(), token) };
      case 'palpo.agents.register': fields(args, ['fleetId', 'agentId', 'role', 'displayName', 'approvedRequestId']); return adminMutation(async () => ({ agent: await this.service.createAgent(id(), args, actor, token) }));
      case 'palpo.agents.rename': fields(args, ['fleetId', 'agentId', 'displayName']); return adminMutation(async () => ({ agent: await this.service.updateAgent(id(), text(args.agentId, 'Agent ID'), args, actor, token) }));
      case 'palpo.agents.retire': fields(args, ['fleetId', 'agentId']); return adminMutation(async () => ({ agent: await this.service.retireAgent(id(), text(args.agentId, 'Agent ID'), actor, token) }));
      default: fail(404, 'service_unavailable', 'This Palpo version does not implement the operation.');
    }
  }
}
