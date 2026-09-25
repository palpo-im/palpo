import { randomBytes } from 'node:crypto';
import { fixture } from './fixture.mjs';
import { Accounts } from '../lib/accounts.mjs';

export const applicant = (username = 'alice') => ({ id: randomBytes(16).toString('hex'), receipt: randomBytes(32).toString('hex'), username, displayName: 'Alice 张', reason: 'Work on our project', password: 'a-long-test-password-2026' });
export function accountFixture(options = {}) {
  const f = fixture(options), original = f.palpo.fetch;
  const config = { botMxid: '@owner:example.test', botToken: 'owner-secret', adminToken: 'admin-secret', approvers: ['@admin:example.test'], registrationToken: 'invitation-secret', passwordKey: 'a'.repeat(64) };
  const credentials = new Map(), devices = new Map(), registrationCalls = [];
  const output = (status, value) => new Response(JSON.stringify(value), { status });
  let loseRegistration = false, failMessages = false;
  f.palpo.fetch = async (target, args) => {
    const url = new URL(target), path = decodeURIComponent(url.pathname), body = args.body ? JSON.parse(args.body) : null;
    if (path === '/_matrix/client/v3/register') {
      registrationCalls.push(body);
      const mxid = `@${body.username}:example.test`;
      if (f.users.has(mxid)) return output(400, { errcode: 'M_USER_IN_USE' });
      if (!body.auth) return output(401, { flows: [{ stages: ['m.login.registration_token'] }], session: 'uiaa-session' });
      if (body.auth.session !== 'uiaa-session' || body.auth.token !== config.registrationToken) return output(403, { errcode: 'M_FORBIDDEN' });
      credentials.set(mxid, body.password); devices.set(mxid, body.device_id);
      f.actors.set('new-user:' + mxid, mxid);
      f.users.set(mxid, { name: mxid, admin: false, deactivated: false, displayname: body.username });
      if (loseRegistration) { loseRegistration = false; throw new Error('Response lost after commit'); }
      return output(200, { user_id: mxid, device_id: body.device_id, access_token: 'new-account-secret' });
    }
    if (path.startsWith('/_palpo/admin/v1/whois/')) {
      const mxid = path.slice('/_palpo/admin/v1/whois/'.length);
      return output(200, { user_id: mxid, devices: devices.has(mxid) ? { [devices.get(mxid)]: {} } : {} });
    }
    if (/^\/_matrix\/client\/v3\/profile\/.+\/displayname$/.test(path)) return output(200, {});
    if (path === '/_matrix/client/v3/login' && credentials.has(body.identifier.user)) {
      if (credentials.get(body.identifier.user) !== body.password) return output(403, { errcode: 'M_FORBIDDEN' });
      return output(200, { user_id: body.identifier.user, access_token: 'new-user:' + body.identifier.user });
    }
    if (args.headers.Authorization?.startsWith('Bearer new-user:')) {
      const user = args.headers.Authorization.slice('Bearer new-user:'.length);
      if (path === '/_matrix/client/v3/account/whoami') return output(200, { user_id: user });
      if (path.startsWith('/_palpo/admin/')) return output(403, { errcode: 'M_FORBIDDEN' });
    }
    const messages = /^\/_matrix\/client\/v3\/rooms\/([^/]+)\/messages$/.exec(path);
    if (messages) {
      if (failMessages) throw new Error('History offline');
      const events = [...f.events.values()].filter(event => event.room_id === messages[1]);
      const start = Number(url.searchParams.get('from') ?? 0), chunk = events.slice(start, start + 100);
      return output(200, { start: String(start), end: String(start + chunk.length), chunk });
    }
    return original(target, args);
  };
  const accounts = new Accounts(f.service, config, options.accountOptions);
  const joinAdmin = () => f.putState(f.rooms.get(accounts.state.roomId), 'm.room.member', '@admin:example.test', { membership: 'join' }, '@admin:example.test');
  const decision = (row, changes = {}) => ({ event_id: '$decision' + f.events.size, type: 'm.room.message', room_id: accounts.state.roomId, sender: '@admin:example.test', content: {
    msgtype: 'm.text', body: '[Approval: approve] Register account',
    'org.octos.approval_response': { request_id: row.id, decision: 'approve', source_event_id: row.sourceEventId, tool_args_digest: row.digest },
    'm.relates_to': { 'm.in_reply_to': { event_id: row.sourceEventId } },
  }, ...changes });
  return { ...f, accounts, config, credentials, devices, registrationCalls, joinAdmin, decision,
    loseRegistration: () => { loseRegistration = true; }, failMessages: value => { failMessages = value; } };
}
