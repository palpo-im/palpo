import { randomBytes, createHash, createCipheriv, createDecipheriv, timingSafeEqual } from 'node:crypto';
import { ApiError, Palpo } from './service.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
const enc = encodeURIComponent;
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const terminal = new Set(['registered', 'rejected', 'expired', 'name_unavailable']);
const same = (a, b) => typeof a === 'string' && typeof b === 'string' && a.length === b.length && timingSafeEqual(Buffer.from(a), Buffer.from(b));

// Registration is separate from browser login and from HAFleet allocation.
// The worker owns only this private room. It never consumes project chat.
export class Accounts {
  constructor(service, config, { clock = Date.now, intervalMs = 2500 } = {}) {
    this.service = service; this.store = service.store; this.controller = new AbortController();
    this.palpo = new Palpo(service.palpo.url, (target, options) => service.palpo.fetch(target, {
      ...options, signal: AbortSignal.any([this.controller.signal, ...(options.signal ? [options.signal] : [])]),
    }));
    this.config = config; this.clock = clock; this.intervalMs = intervalMs;
    this.running = null; this.stopped = true; this.lastError = null; this.ready = false;
    if (!config) return;
    const { botMxid, botToken, adminToken, approvers, passwordKey, registrationToken } = config;
    service.owner(botMxid);
    if (!botToken || !adminToken || !registrationToken || !/^[a-f0-9]{64}$/.test(passwordKey ?? '')
      || !Array.isArray(approvers) || !approvers.length || approvers.includes(botMxid)) throw new Error('Invalid account approval configuration.');
    for (const approver of approvers) service.owner(approver);
    this.key = Buffer.from(passwordKey, 'hex');
    this.state = this.store.state.accountAccess ??= { requests: {}, cursor: null };
    const binding = hash(JSON.stringify([botMxid, [...new Set(approvers)].sort(), hash(passwordKey)]));
    if (this.state.binding && this.state.binding !== binding) throw new Error('Account approval identity/key binding changed; migrate pending requests explicitly.');
    this.state.binding = binding; this.store.save();
  }
  publicConfig() { return { enabled: !!this.config, ready: this.ready, serverName: this.service.serverName }; }
  adminView() { return { ...this.publicConfig(), roomId: this.state?.roomId ?? null, botMxid: this.config?.botMxid ?? null, lastError: this.lastError,
    requests: Object.values(this.state?.requests ?? {}).map(row => ({ ...this.view(row), displayName: row.displayName, reason: row.reason, decidedBy: row.decidedBy ?? null, lastError: row.lastError ?? null })) }; }
  view(row) { return { id: row.id, userId: row.userId, status: row.status, createdAt: row.createdAt, expiresAt: row.expiresAt }; }
  seal(password, id) {
    const iv = randomBytes(12), cipher = createCipheriv('aes-256-gcm', this.key, iv); cipher.setAAD(Buffer.from(id));
    return { iv: iv.toString('hex'), data: Buffer.concat([cipher.update(password, 'utf8'), cipher.final()]).toString('hex'), tag: cipher.getAuthTag().toString('hex') };
  }
  unseal(row) {
    const decipher = createDecipheriv('aes-256-gcm', this.key, Buffer.from(row.password.iv, 'hex'));
    decipher.setAAD(Buffer.from(row.id)); decipher.setAuthTag(Buffer.from(row.password.tag, 'hex'));
    return Buffer.concat([decipher.update(Buffer.from(row.password.data, 'hex')), decipher.final()]).toString('utf8');
  }
  get(id, receipt) {
    const row = this.state?.requests[id];
    if (!row || !/^[a-f0-9]{64}$/.test(receipt ?? '') || !same(row.receiptHash, hash(receipt))) fail(404, 'account_request_missing', 'Account request not found. Use the browser that submitted it.');
    return row;
  }
  finish(row, status) {
    row.status = status; row.finishedAt = this.clock(); delete row.password; delete row.lastError;
    this.store.audit(row.decidedBy ?? 'account-worker', 'account.' + status, null, row.id, status);
  }
  expire() {
    for (const row of Object.values(this.state.requests)) {
      // An accepted decision cannot expire while its registration is retrying.
      if (['notification_pending', 'pending'].includes(row.status) && this.clock() >= row.expiresAt) this.finish(row, 'expired');
    }
  }
  submit(input) {
    if (!this.config || !this.ready) fail(503, 'account_requests_unavailable', 'Account requests are temporarily unavailable. Please try again later.');
    const { id, receipt, username, password } = input;
    if (!/^[a-f0-9]{32}$/.test(id ?? '') || !/^[a-f0-9]{64}$/.test(receipt ?? '')) fail(400, 'invalid_request', 'A valid request receipt is required.');
    if (typeof username !== 'string' || !/^[a-z][a-z0-9_.=-]{0,63}$/.test(username)) fail(400, 'invalid_username', 'Use 1–64 lowercase letters, digits, dots, underscores, equals signs or hyphens; start with a letter.');
    if (typeof password !== 'string' || password.length < 12 || password.length > 256) fail(400, 'invalid_password', 'Use a password between 12 and 256 characters.');
    const displayName = typeof input.displayName === 'string' ? input.displayName.trim() : '';
    const reason = typeof input.reason === 'string' ? input.reason.trim() : '';
    if (!displayName || displayName.length > 128 || !reason || reason.length > 1000) fail(400, 'invalid_details', 'Enter your name (up to 128 characters) and reason (up to 1000 characters).');
    const userId = `@${username}:${this.service.serverName}`;
    const fingerprint = hash(JSON.stringify([username, displayName, reason]));
    if (this.state.requests[id]) {
      const row = this.get(id, receipt);
      if (row.fingerprint !== fingerprint || (row.password && this.unseal(row) !== password)) fail(409, 'request_changed', 'This request was already submitted with different details.');
      return this.view(row);
    }
    this.expire();
    const rows = Object.values(this.state.requests);
    if (rows.some(row => row.userId === userId && !terminal.has(row.status))) fail(409, 'username_pending', 'An account request for this username is already pending.');
    if (rows.length >= 5000 || rows.filter(row => !terminal.has(row.status)).length >= 200) fail(429, 'account_queue_full', 'The account request queue is full. Please try again later.');
    const createdAt = this.clock(), expiresAt = createdAt + 7 * 86400000;
    const row = { id, userId, username, displayName, reason, fingerprint, receiptHash: hash(receipt), password: this.seal(password, id),
      status: 'notification_pending', createdAt, expiresAt, deviceId: 'PALPO_SIGNUP_' + randomBytes(24).toString('hex') };
    row.digest = hash(JSON.stringify({ id, userId, displayName, reason, expiresAt }));
    this.store.atomic(() => { this.state.requests[id] = row; this.store.audit('applicant', 'account.request', null, id, 'pending'); });
    return this.view(row);
  }
  status(id, receipt) { const row = this.get(id, receipt); this.expire(); return this.view(row); }
  async admin(mxid) {
    const user = await this.palpo.user(mxid, this.config.adminToken);
    return !!user && user.admin === true && !user.deactivated && !user.is_guest && !user.appservice_id;
  }
  async room() {
    const state = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(this.state.roomId)}/state`, this.config.botToken);
    if (!Array.isArray(state)) fail(502, 'invalid_account_room', 'Account room state could not be verified.');
    const get = (type, key = '') => state.find(e => e.type === type && e.state_key === key)?.content;
    const allowed = new Set([this.config.botMxid, ...this.config.approvers]);
    if (get('m.room.join_rules')?.join_rule !== 'invite'
      || !['joined', 'invited'].includes(get('m.room.history_visibility')?.history_visibility)
      || get('m.room.encryption') || get('m.room.member', this.config.botMxid)?.membership !== 'join'
      || state.some(e => e.type === 'm.room.member' && ['join', 'invite'].includes(e.content?.membership) && !allowed.has(e.state_key))) {
      fail(409, 'account_room_not_private', 'Restore the private administrator room membership and settings.');
    }
    return state;
  }
  async initialize() {
    const me = await this.palpo.call('/_matrix/client/v3/account/whoami', this.config.botToken);
    if (me.user_id !== this.config.botMxid || me.is_guest) fail(403, 'account_bot_mismatch', 'Account service identity mismatch.');
    await this.palpo.requireAdmin(this.config.adminToken);
    const allowed = [];
    for (const mxid of this.config.approvers) if (await this.admin(mxid)) allowed.push(mxid);
    if (!allowed.length) fail(403, 'account_admin_unavailable', 'No configured account approver has administrator authority.');
    if (!this.state.roomId) {
      const localpart = 'palpo_account_approvals_' + hash(this.config.botMxid).slice(0,16);
      try {
        const existing = await this.palpo.call(`/_matrix/client/v3/directory/room/${enc('#' + localpart + ':' + this.service.serverName)}`, this.config.botToken);
        this.state.roomId = existing.room_id;
      } catch (error) { if (error.status !== 404) throw error; }
      if (!this.state.roomId) {
        const result = await this.palpo.call('/_matrix/client/v3/createRoom', this.config.botToken, { method: 'POST', body: {
          room_alias_name: localpart, name: 'Palpo · Account approvals', visibility: 'private', preset: 'private_chat', is_direct: false,
          invite: allowed, creation_content: { 'm.federate': false },
          power_level_content_override: { users: { [this.config.botMxid]: 100 }, users_default: 0, invite: 100, events_default: 0, state_default: 100 },
          initial_state: [{ type: 'm.room.history_visibility', state_key: '', content: { history_visibility: 'invited' } }],
        } });
        this.state.roomId = result.room_id;
      }
      this.store.save();
    }
    let state = await this.room();
    // Early installations used joined-only history. An invited administrator
    // arriving later could never see the original card. Plan this migration
    // before changing room state so restart also recovers a lost PUT response.
    if (!this.state.historyUpgrade && state.some(e => e.type === 'm.room.history_visibility' && e.content?.history_visibility === 'joined')) {
      this.state.historyUpgrade = { pending: true, requests: Object.values(this.state.requests).filter(row => row.status === 'pending').map(row => row.id) };
      this.store.save();
    }
    if (this.state.historyUpgrade?.pending) {
      await this.palpo.call(`/_matrix/client/v3/rooms/${enc(this.state.roomId)}/state/m.room.history_visibility`, this.config.botToken,
        { method: 'PUT', body: { history_visibility: 'invited' } });
      state = await this.room();
      if (!state.some(e => e.type === 'm.room.history_visibility' && e.content?.history_visibility === 'invited')) fail(409, 'account_history_unconfirmed', 'Administrator invitation history was not confirmed.');
      this.store.atomic(() => {
        for (const id of this.state.historyUpgrade.requests) {
          const row = this.state.requests[id];
          if (row?.status !== 'pending') continue;
          (row.supersededSourceEventIds ??= []).push(row.sourceEventId);
          row.notificationVersion = (row.notificationVersion ?? 0) + 1;
          row.status = 'notification_pending'; delete row.nextAttemptAt;
        }
        this.state.historyUpgrade.pending = false;
      });
    }
    this.ready = true;
  }
  card(row) {
    return { msgtype: 'm.text', body: `Account request: ${row.userId}\nName: ${row.displayName}\nReason: ${row.reason}\nApprove creates an ordinary Matrix account. Agent resources still require HAFleet approval.`,
      'org.octos.approval_request': { request_id: row.id, tool_name: 'palpo.register_account', tool_args_digest: row.digest,
        title: `Register ${row.userId}`, summary: `${row.displayName}\n${row.reason}\nOrdinary user account; no administrator privileges.`,
        risk_level: 'normal', authorized_approvers: this.config.approvers, expires_at: new Date(row.expiresAt).toISOString(), on_timeout: 'notify' },
      'org.octos.actions': [{ id: 'approve', label: 'Approve', style: 'primary' }, { id: 'deny', label: 'Reject', style: 'danger' }] };
  }
  async send(row, kind, content) {
    return this.palpo.call(`/_matrix/client/v3/rooms/${enc(this.state.roomId)}/send/m.room.message/${enc('account_' + row.id + '_' + kind)}`, this.config.botToken, { method: 'PUT', body: content });
  }
  async notify(row) {
    const existing = await this.palpo.user(row.userId, this.config.adminToken);
    if (existing) { this.finish(row, 'name_unavailable'); return; }
    await this.room();
    const result = await this.send(row, 'request' + (row.notificationVersion ? '_v' + row.notificationVersion : ''), this.card(row));
    if (typeof result.event_id !== 'string') fail(502, 'invalid_account_event', 'The account request event was not confirmed.');
    row.sourceEventId = result.event_id; row.status = 'pending'; this.store.save();
  }
  async decide(event) {
    const response = event.content?.['org.octos.approval_response'];
    if (event.type !== 'm.room.message' || !response || event.sender === this.config.botMxid) return;
    const row = this.state.requests[response.request_id];
    if (!row || row.status !== 'pending') return;
    const valid = event.room_id === this.state.roomId && this.clock() < row.expiresAt
      && this.config.approvers.includes(event.sender) && ['approve', 'deny'].includes(response.decision)
      && response.source_event_id === row.sourceEventId && response.tool_args_digest === row.digest
      && event.content?.['m.relates_to']?.['m.in_reply_to']?.event_id === row.sourceEventId;
    if (!valid) { this.store.audit(event.sender, 'account.invalid_decision', null, row.id, 'refused'); return; }
    const state = await this.room();
    if (!state.some(e => e.type === 'm.room.member' && e.state_key === event.sender && e.content?.membership === 'join') || !await this.admin(event.sender)) {
      this.store.audit(event.sender, 'account.unauthorized_decision', null, row.id, 'refused'); return;
    }
    if (row.status !== 'pending') return;
    if (this.clock() >= row.expiresAt) { this.finish(row, 'expired'); return; }
    row.decidedBy = event.sender; row.decisionEventId = event.event_id; row.decidedAt = this.clock();
    if (response.decision === 'deny') this.finish(row, 'rejected');
    else { row.status = 'approved'; delete row.nextAttemptAt; this.store.audit(event.sender, 'account.approve', null, row.id, 'approved'); }
  }
  async registerCall(body) {
    let response, data;
    try {
      response = await this.palpo.fetch(new URL('/_matrix/client/v3/register', this.palpo.url), { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), redirect: 'error', signal: AbortSignal.timeout(12000) });
      data = await response.json();
    } catch { fail(502, 'registration_unreachable', 'Waiting to confirm account registration.'); }
    if (response.status === 401 && Array.isArray(data.flows) && typeof data.session === 'string') return { challenge: data };
    if (!response.ok) fail(response.status, /^M_[A-Z_]+$/.test(data.errcode) ? data.errcode : 'registration_failed', 'Palpo could not complete account registration.');
    return data;
  }
  async provision(row) {
    // Replay may observe a user created just before a crash. Only our randomly
    // chosen original registration device proves that this request created it.
    const existing = await this.palpo.user(row.userId, this.config.adminToken);
    if (existing) {
      if (row.attempted && !existing.admin && !existing.appservice_id && !existing.deactivated) {
        const proof = await this.palpo.call(`/_palpo/admin/v1/whois/${enc(row.userId)}`, this.config.adminToken);
        if (Object.hasOwn(proof.devices ?? {}, row.deviceId)) { this.finish(row, 'registered'); return; }
      }
      this.finish(row, 'name_unavailable'); return;
    }
    await this.room();
    if (!await this.admin(row.decidedBy)) fail(403, 'account_approver_revoked', 'The approver no longer has administrator authority.');
    const body = { username: row.username, password: this.unseal(row), device_id: row.deviceId, initial_device_display_name: 'Palpo account registration' };
    row.attempted = true; row.status = 'registering'; this.store.save();
    let result = await this.registerCall(body);
    if (result.challenge) {
      const flow = result.challenge.flows.find(flow => Array.isArray(flow.stages) && flow.stages.length === 1 && ['m.login.registration_token', 'm.login.dummy'].includes(flow.stages[0]));
      if (!flow) fail(409, 'unsupported_registration', 'The configured homeserver registration flow is unsupported.');
      const type = flow.stages[0];
      result = await this.registerCall({ ...body, auth: { type, session: result.challenge.session, ...(type === 'm.login.registration_token' ? { token: this.config.registrationToken } : {}) } });
    }
    if (result.user_id !== row.userId || result.device_id !== row.deviceId || !result.access_token) fail(502, 'registration_unconfirmed', 'Waiting to confirm the registered account identity.');
    // Persist success before optional profile/logout effects. No password or
    // session token enters a notification, public response, or audit record.
    this.finish(row, 'registered');
    try { await this.palpo.call(`/_matrix/client/v3/profile/${enc(row.userId)}/displayname`, result.access_token, { method: 'PUT', body: { displayname: row.displayName } }); } catch { /* Account remains usable; the user can edit their profile. */ }
    try { await this.palpo.call('/_matrix/client/v3/logout', result.access_token, { method: 'POST', body: {} }); } catch { /* Never reverse confirmed account creation on logout failure. */ }
  }
  async tick() {
    if (!this.config) return;
    if (this.running) return this.running;
    this.running = (async () => {
      try {
        if (!this.ready) await this.initialize();
        this.expire();
        // Fail closed if room privacy changes. Do not skip unread verdicts.
        await this.room();
        for (const row of Object.values(this.state.requests)) {
          if (this.controller.signal.aborted) break;
          if (row.nextAttemptAt > this.clock()) continue;
          try {
            if (row.status === 'notification_pending') await this.notify(row);
            if (['approved', 'registering'].includes(row.status)) await this.provision(row);
            if (terminal.has(row.status) && row.sourceEventId && !row.resultEventId) {
              const result = await this.send(row, 'result', { msgtype: 'm.notice', body: `${row.userId}: ${row.status.replaceAll('_', ' ')}.`, 'm.relates_to': { 'm.in_reply_to': { event_id: row.sourceEventId } } });
              row.resultEventId = result.event_id; this.store.save();
            }
            delete row.nextAttemptAt; delete row.retryCount;
          } catch (error) {
            if (['M_EXCLUSIVE', 'M_INVALID_USERNAME'].includes(error.code)) { this.finish(row, 'name_unavailable'); continue; }
            row.lastError = error.code ?? 'account_operation_failed';
            row.retryCount = (row.retryCount ?? 0) + 1;
            row.nextAttemptAt = this.clock() + Math.min(60000, 2500 * 2 ** Math.min(row.retryCount - 1, 5));
            this.store.save();
          }
        }
        if (this.controller.signal.aborted) return;
        const query = new URLSearchParams({ dir: 'f', limit: '100', ...(this.state.cursor ? { from: this.state.cursor } : {}) });
        const batch = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(this.state.roomId)}/messages?${query}`, this.config.botToken);
        if (!Array.isArray(batch.chunk)) fail(502, 'invalid_account_history', 'Account approval history could not be read.');
        for (const event of batch.chunk) await this.decide({ ...event, room_id: this.state.roomId });
        if (batch.end) { this.state.cursor = batch.end; this.store.save(); }
        this.lastError = null;
      } catch (error) { this.lastError = error.code ?? 'account_worker_unavailable'; this.ready = false; }
    })().finally(() => { this.running = null; });
    return this.running;
  }
  start() {
    this.stopped = false;
    if (this.controller.signal.aborted) this.controller = new AbortController();
    const run = async () => { await this.tick(); if (!this.stopped) { this.timer = setTimeout(run, this.intervalMs); this.timer.unref(); } };
    void run();
  }
  async stop() { this.stopped = true; clearTimeout(this.timer); this.controller.abort(); await this.running; }
}
