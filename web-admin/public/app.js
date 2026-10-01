let fleetName = row => row.name;
let csrf = null, selectedFleet = null, fleetRows = [], currentSession = null, catalog = [], projects = [], memberView = false;
let requestBusy = false, requestExpiryTimer;
let catalogRefreshPromise, catalogSessionToken, catalogPollTimer;
const connectionChecks = new Map();
const $ = selector => document.querySelector(selector);
const node = (tag, content, className) => { const el = document.createElement(tag); if (content != null) el.textContent = content; if (className) el.className = className; return el; };
function notice(message, error = false) { const el = $('#notice'); el.textContent = message; el.className = error ? 'error' : ''; el.hidden = false; }
async function api(path, method = 'GET', body) {
  const sessionToken = csrf;
  const response = await fetch(`/api${path}`, { method, headers: { 'Content-Type': 'application/json', ...(csrf ? { 'X-CSRF-Token': csrf } : {}) }, body: body === undefined ? undefined : JSON.stringify(body) });
  const value = await response.json();
  if (!response.ok) { if (response.status === 401 && value.code === 'sign_in_required' && csrf === sessionToken) showLogin(); throw new Error(value.error ?? 'The operation failed.'); }
  return value;
}
function showLogin() { memberTab = null; adminTab = null; $('#workspace').hidden = true; $('#member-workspace').hidden = true; $('#view-nav').hidden = true; $('#login-panel').hidden = false; $('#account').replaceChildren(); csrf = null; currentSession = null; connectionChecks.clear(); clearTimeout(requestExpiryTimer); }
function readCatalog() {
  if (!catalogRefreshPromise || catalogSessionToken !== csrf) {
    catalogSessionToken = csrf;
    const refresh = api('/catalog').finally(() => { if (catalogRefreshPromise === refresh) catalogRefreshPromise = null; });
    catalogRefreshPromise = refresh;
  }
  return catalogRefreshPromise;
}
async function refreshResourceCatalog() {
  clearTimeout(catalogPollTimer);
  const sessionToken = csrf;
  try {
    if (!sessionToken || !memberView || document.hidden || requestBusy) return;
    if (projects.some(project => project.readinessError === 'owner_dm_join_pending')) {
      await refreshMember(); return;
    }
    const before = JSON.stringify(catalog.map(fleet => [fleet.id, fleet.capabilities?.offers, fleet.capabilityRead?.state, fleet.readiness]));
    const data = await readCatalog();
    if (csrf !== sessionToken || !memberView) return;
    catalog = data.fleets;
    const after = JSON.stringify(catalog.map(fleet => [fleet.id, fleet.capabilities?.offers, fleet.capabilityRead?.state, fleet.readiness]));
    if (before !== after) setRoles();
    renewOwnerConnections();
  } catch (error) {
    if (csrf === sessionToken && memberView) {
      catalog = catalog.map(fleet => ({ ...fleet, capabilityRead: { ...fleet.capabilityRead, state: 'failed', code: 'catalog_unavailable' } }));
      setRoles();
    }
  } finally { clearTimeout(catalogPollTimer); catalogPollTimer = setTimeout(refreshResourceCatalog, 10000); }
}
async function action(button, fn) {
  button.disabled = true;
  try { await fn(); } catch (error) { notice(error.message, true); } finally { button.disabled = false; }
}
function button(label, onClick, className = 'secondary') { const el = node('button', label, className); el.type = 'button'; el.onclick = () => action(el, onClick); return el; }
function badge(label, kind = '') { return node('span', label, `badge ${kind}`); }
function readable(value) { return String(value ?? '').replaceAll('_', ' '); }
// One semantic color per state family: ok (done/usable), warning (waiting on
// someone), error (failed/revoked), neutral (ended/paused).
function tone(value) {
  const v = String(value ?? '').toLowerCase();
  if (/(revoked|failed|rejected|error|denied|unavailable|name unavailable)/.test(v)) return 'error';
  if (/(submission|unverified|not configured|stale|retiring)/.test(v.replaceAll('_', ' '))) return 'warning';
  if (/(pending|waiting|registering|approved|connecting)/.test(v.replaceAll('_', ' '))) return 'info';
  if (/(queued|paused|ended|retired|expired|deactivated|unknown|offline)/.test(v)) return '';
  if (/(ready|active|verified|installed|registered|complete|online|delivered|authorized|done|started)/.test(v)) return 'ok';
  return '';
}
function stateBadge(value, label = readable(value)) { return badge(label, tone(value)); }
// Codes the server reports as reasons, in plain language. Unknown codes fall
// back to readable text rather than raw snake_case.
const reasons = {
  outbound_status_stale: 'Hagency has not reported this request’s status recently.',
  capabilities_pending: 'Waiting for Hagency to publish its resources.',
  catalog_unavailable: 'The resource list could not be refreshed.',
  owner_dm_join_pending: 'Waiting for the Hagency approval account to join.',
  not_configured: 'Not configured yet.',
  admission: 'Hagency is admitting the request.',
  complete: 'Ready.',
};
function reason(code) { return reasons[code] ?? `${readable(code).replace(/^./, c => c.toUpperCase())}.`; }
// A long identifier shown compactly: full value on hover, one-click copy.
function idChip(value) {
  const chip = node('span', null, 'id'); chip.title = value;
  const text = node('span', value);
  const copy = node('button', 'Copy'); copy.type = 'button'; copy.setAttribute('aria-label', `Copy ${value}`);
  copy.onclick = async () => { try { await navigator.clipboard.writeText(value); copy.textContent = 'Copied'; setTimeout(() => { copy.textContent = 'Copy'; }, 1200); } catch { /* clipboard unavailable */ } };
  chip.append(text, copy); return chip;
}
function facts(rows) {
  const dl = node('dl', null, 'facts');
  for (const [term, value] of rows) { if (value == null || value === '') continue; dl.append(node('dt', term)); const dd = node('dd'); dd.append(typeof value === 'string' ? document.createTextNode(value) : value); dl.append(dd); }
  return dl;
}
// A Matrix ID as people read it: the localpart on this server, full ID elsewhere.
function shortId(mxid) {
  const m = /^@([^:]+):(.+)$/.exec(mxid ?? '');
  return m && m[2] === currentSession?.serverName ? m[1] : (mxid ?? '');
}
function shortIdFor(mxid, server) { const m = /^@([^:]+):(.+)$/.exec(mxid ?? ''); return m && m[2] === server ? m[1] : (mxid ?? ''); }
function person(mxid) { const el = node('span', shortId(mxid)); el.title = mxid ?? ''; return el; }
function relTime(iso) {
  const t = Date.parse(iso ?? ''); if (!Number.isFinite(t)) return '';
  const s = Math.round((Date.now() - t) / 1000), abs = Math.abs(s);
  const [n, unit] = abs < 60 ? [s, 'second'] : abs < 3600 ? [Math.round(s / 60), 'minute'] : abs < 86400 ? [Math.round(s / 3600), 'hour'] : [Math.round(s / 86400), 'day'];
  return new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' }).format(-n, unit);
}
function alertBox(text, kind = 'error') { const el = node('div', null, `alert ${kind}`); el.append(node('p', text)); return el; }
function details(rows, extra, about) { const more = node('details', null, 'more'), summary = node('summary', 'Details'); if (about) summary.setAttribute('aria-label', `Details for ${about}`); more.append(summary, facts(rows)); if (extra) more.append(extra); return more; }
// Names shared by several rows get their creation date so they can be told apart.
function distinctNames(rows) {
  const seen = new Map(); for (const row of rows) seen.set(row.name, (seen.get(row.name) ?? 0) + 1);
  return row => seen.get(row.name) > 1 ? `${row.name} · ${row.createdAt ? `added ${new Date(row.createdAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}` : `#${String(row.id).slice(-4)}`}` : row.name;
}
// The one service status shown on every page (admin list, owner list).
// A heading with the name; anything distinguishing duplicates goes in a muted line.
function nameHeading(tag, label, row) {
  const box = node('div', null, 'title'); box.append(node(tag, row.name));
  if (label !== row.name) box.append(node('span', label.slice(row.name.length).replace(/^ · /, ''), 'sub'));
  return box;
}
function serviceStatus(fleet) {
  if (fleet.state === 'revoked') return { label: 'App Service revoked', kind: 'error' };
  const outbound = fleet.transport?.mode === 'outbound';
  if (fleet.state === 'ready' && outbound && !fleet.transport.online) return { label: `offline${fleet.transport.lastSeenAt ? ` · seen ${relTime(fleet.transport.lastSeenAt)}` : ''}`, kind: 'warning' };
  if (fleet.state === 'ready' && (fleet.readiness?.ready ?? true)) return { label: 'Ready to receive requests', kind: 'ok' };
  return { label: readable(fleet.state), kind: tone(fleet.state) };
}
function statusBadge(fleet) { const s = serviceStatus(fleet); return badge(s.label, s.kind); }
// One section at a time: each workspace has a tab list and matching panels.
let memberTab = null, adminTab = null;
const tabIntro = {
  access: 'Download each Hagency’s configuration, then verify its connection.', projects: 'Projects hold your agents. Each has a room and a private approval room.',
  request: 'Pick a resource from a Hagency’s pool and define your agent.', requests: 'Every agent you asked for, grouped by what needs your attention.', accounts: 'People asking for an account on this server.',
  hagencys: 'Authorize Hagency providers and watch their connections.', activity: 'Everything administrators and owners changed recently.',
};
function showTab(root, name) {
  const visible = [...root.querySelectorAll('[role=tab]')].filter(tab => !tab.hidden);
  if (!visible.some(tab => tab.dataset.tab === name)) name = visible[0]?.dataset.tab;
  for (const tab of root.querySelectorAll('[role=tab]')) { const on = tab.dataset.tab === name; tab.setAttribute('aria-selected', String(on)); tab.tabIndex = on ? 0 : -1; if (on && tab.offsetParent) { const strip = tab.parentElement; strip.scrollLeft = Math.max(0, tab.offsetLeft - strip.offsetLeft - 16); } }
  for (const panel of root.querySelectorAll('[data-panel]')) panel.classList.toggle('current', panel.dataset.panel === name);
  if (root.id === 'member-workspace') memberTab = name; else adminTab = name;
}
for (const list of document.querySelectorAll('[role=tablist]')) {
  const root = list.closest('#member-workspace, #workspace');
  list.addEventListener('click', event => { const tab = event.target.closest('[role=tab]'); if (tab) showTab(root, tab.dataset.tab); });
  list.addEventListener('keydown', event => {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    const tabs = [...list.querySelectorAll('[role=tab]:not([hidden])')], at = tabs.indexOf(document.activeElement);
    const next = tabs[(at + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length]; showTab(root, next.dataset.tab); next.focus();
  });
}
function compact(n) { return Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(n); }
function setNav(view) { for (const [id, name] of [['#show-member', 'member'], ['#show-admin', 'admin']]) { const el = $(id); if (view === name) el.setAttribute('aria-current', 'page'); else el.removeAttribute('aria-current'); } }
async function session() {
  const data = await api('/session'); csrf = data.csrf; currentSession = data; memberView = !data.isAdmin;
  $('#login-panel').hidden = true; $('#account-request-panel').hidden = true; $('#workspace').hidden = !data.isAdmin; $('#member-workspace').hidden = data.isAdmin; $('#view-nav').hidden = !data.isAdmin; setNav(data.isAdmin ? 'admin' : 'member');
  $('#server-name').textContent = data.serverName;
  $('#callback-policy').textContent = data.callbackOrigins.length ? `Allowed callback origins: ${data.callbackOrigins.join(', ')}` : 'No callback origins are allowed yet. Configure the server callback policy before installing.';
  $('#fleet-form [name=transportMode] option[value=outbound]').disabled = !data.outboundAvailable;
  $('#fleet-form [name=transportMode]').value = data.outboundAvailable ? 'outbound' : 'callback';
  updateTransportForm();
  const who = node('span', null, 'who'); who.title = data.userId; const short = shortIdFor(data.userId, data.serverName);
  who.dataset.initial = short.replace(/^@/, '').charAt(0).toUpperCase();
  who.append(node('strong', short), node('small', data.isAdmin ? 'Administrator' : 'Member'), node('span', data.userId, 'sr-only'));
  $('#account').replaceChildren(who, button('Sign out', async () => { await api('/logout', 'POST', {}); showLogin(); }));
  if (!$('#fleet-form [name=requestId]').value) $('#fleet-form [name=requestId]').value = crypto.randomUUID();
  $('#member-server-name').textContent = data.serverName;
  if (data.isAdmin) await refresh(); else await refreshMember();
}
async function refresh() {
  const [fleets, audit, accounts] = await Promise.all([api('/fleets'), api('/audit'), api('/account-requests')]); fleetRows = fleets.fleets;
  $('#account-admin-panel').hidden = !accounts.enabled; $('#nav-accounts').hidden = !accounts.enabled;
  $('#account-admin-room').replaceChildren(...(accounts.roomId ? [roomLink(accounts.roomId, 'Open account approval room in Robrix'), details([['Room ID', idChip(accounts.roomId)]], null, 'the approval room')] : [node('p', 'The private administrator room is being prepared.', 'hint')]));
  if (accounts.lastError) $('#account-admin-room').append(node('p', `Account service needs attention: ${accounts.lastError}`, 'error'));
  $('#account-request-count').textContent = accounts.requests.filter(r => r.status === 'pending').length;
  const accountCard = row => {
    const card = node('article', null, 'fleet');
    const top = node('div', null, 'section-heading'); const who = node('div', null, 'title'); who.append(node('h3', row.displayName || shortId(row.userId)), node('span', shortId(row.userId), 'sub')); top.append(who, stateBadge(row.status)); card.append(top);
    const asked = row.createdAt ? `Requested ${relTime(row.createdAt)}` : '';
    card.append(node('p', [asked, row.decidedBy ? (row.decidedBy === currentSession?.userId ? 'decided by you' : `decided by ${shortId(row.decidedBy)}`) : ''].filter(Boolean).join(' · ').replace(/^./, c => c.toUpperCase()), 'meta'));
    if (row.reason && !row.decidedBy) card.append(node('p', `“${row.reason}”`, 'quote'));
    if (row.lastError) card.append(alertBox(`Waiting to complete: ${row.lastError}`, 'warning'));
    card.append(details([['Matrix ID', idChip(row.userId)], ['Decided by', row.decidedBy ? idChip(row.decidedBy) : null]], null, row.displayName || shortId(row.userId)));
    return card;
  };
  const recent = accounts.requests.slice(-50).reverse(), open = recent.filter(r => r.status === 'pending'), decided = recent.filter(r => r.status !== 'pending');
  const counts = new Map(); for (const r of recent) counts.set(r.status, (counts.get(r.status) ?? 0) + 1);
  const summary = node('p', null, 'summary-line');
  summary.append(node('strong', open.length ? `${open.length} waiting for your decision` : 'Nothing waiting for a decision'));
  const tally = [...counts].filter(([status]) => status !== 'pending').map(([status, n]) => `${n} ${readable(status)}`).join(' · ');
  if (tally) summary.append(node('span', tally, 'muted'));
  $('#account-admin-requests').replaceChildren(...(recent.length ? [summary] : []), ...open.map(accountCard));
  if (decided.length) { const fold = node('details', null, 'more group-fold'); fold.append(node('summary', `Decided (${decided.length})`), ...decided.map(accountCard)); $('#account-admin-requests').append(fold); }
  if (!accounts.requests.length) $('#account-admin-requests').append(node('p', 'No account requests yet.', 'empty'));
  $('#fleet-count').textContent = fleetRows.filter(f => f.state !== 'revoked').length;
  if (!$('#authorize-panel').dataset.ready) { $('#authorize-panel').open = fleetRows.length === 0; $('#authorize-panel').dataset.ready = '1'; }
  fleetName = distinctNames(fleetRows);
  // Active services first; revoked ones in a collapsed group, still there for cleanup.
  const active = fleetRows.filter(f => f.state !== 'revoked'), revoked = fleetRows.filter(f => f.state === 'revoked');
  $('#fleets').replaceChildren(...active.map(renderFleet));
  if (revoked.length) { const group = node('details', null, 'more revoked-group'); group.append(node('summary', `Revoked (${revoked.length})`), ...revoked.map(renderFleet)); $('#fleets').append(group); }
  if (!fleetRows.length) $('#fleets').append(node('p', 'No Hagencys have been authorized yet.', 'empty'));
  renderAudit(audit.events, 25);
  $('#fleet-count-nav').textContent = fleetRows.filter(f => f.state !== 'revoked').length;
  $('#account-pending-nav').textContent = open.length;
  showTab($('#workspace'), adminTab ?? (accounts.enabled ? 'accounts' : 'hagencys'));
  if (selectedFleet) await showAgents(selectedFleet);
}
const operations = {
  'account.request': 'Requested an account', 'account.approve': 'Approved an account', 'account.invalid_decision': 'Ignored an invalid decision', 'account.unauthorized_decision': 'Refused a decision from a non-administrator',
  'agent.authorize': 'Authorized an agent', 'agent.create': 'Created an agent identity', 'agent.profile.update': 'Renamed an agent', 'agent.retire': 'Retired an agent identity',
  'fleet.authorize': 'Authorized a Hagency', 'fleet.connection.verify': 'Verified a Hagency connection', 'fleet.credentials.deliver': 'Delivered configuration to the owner',
  'fleet.install': 'Installed a Hagency', 'fleet.outbound.migrate': 'Moved a Hagency to outbound', 'fleet.pause': 'Paused a Hagency', 'fleet.resume': 'Resumed a Hagency', 'fleet.revoke': 'Revoked a Hagency',
  'project.register': 'Created a project', 'request.enqueue': 'Queued an agent request', 'request.submit': 'Submitted an agent request',
};
function operation(action) { return operations[action] ?? readable(action).replaceAll('.', ' · ').replace(/^./, c => c.toUpperCase()); }
function renderAudit(events, shown) {
  $('#audit-summary').textContent = events.length ? `${events.length} recent ${events.length === 1 ? 'entry' : 'entries'}` : 'None yet';
  if (!events.length) { $('#audit').replaceChildren(node('p', 'Operations will appear here after you authorize a provider.', 'empty')); return; }
  const table = node('table'), head = node('tr');
  for (const title of ['When', 'Who', 'What', 'Hagency', 'Result']) head.append(node('th', title));
  table.append(head);
  let day = '';
  for (const event of events.slice(0, shown)) {
    const at = new Date(event.at), thisDay = at.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
    if (thisDay !== day) { day = thisDay; const head = node('tr', null, 'day'); const cell = node('th', day); cell.colSpan = 5; head.append(cell); table.append(head); }
    const row = node('tr');
    row.append(node('td', at.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }), 'time'));
    const actor = node('td'); actor.append(event.actor?.startsWith('@') ? person(event.actor) : document.createTextNode(event.actor ?? '')); row.append(actor);
    row.append(node('td', operation(event.action)));
    const target = fleetRows.find(f => f.id === event.fleetId); const targetCell = node('td', target ? target.name : '—', 'target'); if (target) targetCell.title = fleetName(target); row.append(targetCell);
    const result = node('td'); result.append(stateBadge(event.result)); row.append(result);
    table.append(row);
  }
  const scroll = node('div', null, 'table-scroll'); scroll.append(table);
  const footer = node('div', null, 'table-footer'); footer.append(node('span', `Showing ${Math.min(shown, events.length)} of ${events.length}`));
  if (shown < events.length) footer.append(button('Show 25 more', async () => renderAudit(events, shown + 25)));
  $('#audit').replaceChildren(scroll, footer);
}
function renderFleet(fleet) {
  const card = node('article', null, 'fleet'); card.dataset.fleetId = fleet.id;
  const outbound = fleet.transport?.mode === 'outbound';
  const top = node('div', null, 'section-heading'); top.append(nameHeading('h3', fleetName(fleet), fleet), fleet.state === 'ready' || fleet.state === 'revoked' ? statusBadge(fleet) : badge(readable(fleet.state), tone(fleet.state))); card.append(top);
  card.append(facts([
    ['Owner', person(fleet.ownerMxid)],
    ['Connection', outbound ? 'Outbound' : 'Legacy callback'],
    ['Credentials', fleet.credentialDeliveredAt ? 'Issued to owner' : 'Waiting for the owner to download them'],
  ]));
  // While offline the checks describe the last connection, so they go neutral.
  const offline = outbound && !fleet.transport.online, chip = (label, kind) => badge(label, offline ? '' : kind);
  const states = node('div', null, 'statusline'); states.hidden = fleet.state === 'revoked'; states.append(chip(`Installation: ${readable(fleet.installation)}`, tone(fleet.installation)), chip(`Identity: ${fleet.readiness.identity}`, tone(fleet.readiness.identity)), chip(`Event delivery: ${fleet.readiness.eventDelivery}`, fleet.readiness.eventDelivery === 'verified' ? 'ok' : 'warning'), chip(`Reception: ${readable(fleet.readiness.reception)}`, fleet.readiness.reception === 'verified' ? 'ok' : 'warning'));
  if (offline) states.append(node('span', 'Last verified before going offline', 'meta'));
  const allPassed = !offline && fleet.state !== 'revoked' && fleet.installation === 'installed' && fleet.readiness.identity === 'verified' && fleet.readiness.eventDelivery === 'verified' && fleet.readiness.reception === 'verified';
  if (allPassed) { const summary = node('div', null, 'statusline'); summary.append(badge('All checks passed', 'ok')); summary.title = [...states.children].map(c => c.textContent).join(' · '); states.hidden = true; card.append(summary); }
  card.append(states);
  if (fleet.state === 'revoked') card.append(node('p', 'The service credential is disabled. Retire each identity separately to remove its Matrix memberships and any independent sessions.', 'hint'));
  if (fleet.lastError) card.append(fleet.installation !== 'installed'
    ? alertBox(`Installation did not finish: ${reason(fleet.lastError.code)} Use Retry installation to continue with the existing registration.`)
    : alertBox(`${reason(fleet.lastError.code)} Palpo retries automatically.`, 'warning'));
  const more = node('details', null, 'more'); more.append(node('summary', 'Details'));
  more.querySelector('summary').setAttribute('aria-label', `Details for ${fleetName(fleet)}`);
  more.append(facts([
    ['Owner', idChip(fleet.ownerMxid)],
    ['Checks', allPassed ? 'Installation, identity, event delivery and reception verified' : null],
    ['Representative', idChip(fleet.representativeMxid)],
    [outbound ? 'Transport' : 'Callback', outbound ? `Generation ${fleet.transport.generation}` : idChip(fleet.callbackUrl ?? '')],
    ['Credential version', String(fleet.credentialVersion)],
    ['Local task stop', readable(fleet.localTaskStop)],
    ['Owner pairing', idChip(`POST /api/pair/${fleet.id}`)],
  ]));
  more.append(node('p', 'The Hagency owner retrieves credentials with their own Matrix session; retries return the same version and no administrator token is shared.', 'hint'));
  card.append(more);
  const actions = node('div', null, 'actions');
  actions.append(button(`Manage ${fleet.agentCount} ${fleet.agentCount === 1 ? 'identity' : 'identities'}`, () => showAgents(fleet.id)));
  const menu = node('details', null, 'menu'), menuSummary = node('summary', 'More'); menuSummary.setAttribute('aria-label', `More actions for ${fleetName(fleet)}`); const menuList = node('div', null, 'menu-list'); menu.append(menuSummary, menuList);
  if (fleet.transport?.mode === 'outbound') menuList.append(button('Inspect delivery capacity', async () => {
    const { queue } = await api(`/fleets/${fleet.id}/outbound`);
    notice(`Delivery queue: ${queue.pending}/${queue.limits.pending} pending, ${queue.records}/${queue.limits.records} retained records, ${queue.bytes}/${queue.limits.bytes} pending bytes. The server operator can increase configured limits while retaining all delivery receipts.`);
  }));
  const mutate = async name => { await api(`/fleets/${fleet.id}/${name}`, 'POST', {}); notice(name === 'revoke' ? 'Matrix service access revoked. Local task stop remains unconfirmed.' : 'Registration state verified on Palpo.'); await refresh(); };
  if (fleet.installation !== 'installed' && !['paused', 'revoked'].includes(fleet.state)) actions.append(button('Retry installation', () => mutate('install')));
  if (fleet.installation === 'installed' && fleet.state !== 'revoked') {
    if (currentSession?.outboundAvailable && fleet.state !== 'paused') menuList.append(button(fleet.transport?.mode === 'outbound' ? 'Rotate transport credential' : 'Migrate to outbound connection', async () => {
      const key = `palpo-outbound-${fleet.id}`, requestId = sessionStorage.getItem(key) ?? crypto.randomUUID(); sessionStorage.setItem(key, requestId);
      await api(`/fleets/${fleet.id}/outbound`, 'POST', { requestId, rotate: fleet.transport?.mode === 'outbound' });
      sessionStorage.removeItem(key); notice('Outbound registration is ready. The owner must download the new configuration into Hagency and verify the Matrix event channel.'); await refresh();
    }));
    actions.append(button(fleet.state === 'paused' ? 'Resume' : 'Pause', () => mutate(fleet.state === 'paused' ? 'resume' : 'pause')));
    actions.append(button('Revoke service', async () => { if (confirm(`Revoke the App Service for ${fleet.name}? Its service token will stop working. Retire individual identities separately to remove memberships and independent sessions. Local tasks require Hagency confirmation.`)) await mutate('revoke'); }, 'danger'));
  }
  if (menuList.children.length) actions.insertBefore(menu, actions.querySelector('.danger'));
  card.append(actions); return card;
}
async function showAgents(id) {
  selectedFleet = id; const fleet = fleetRows.find(item => item.id === id);
  const { agents } = await api(`/fleets/${id}/agents`);
  $('#agent-panel').hidden = false; $('#agent-heading').textContent = `${fleet.name} · Agent identities`;
  $('#agent-form').hidden = !['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed';
  $('#agents').replaceChildren(...agents.map(agent => {
    const card = node('article', null, 'fleet'); card.append(node('h3', agent.displayName ?? agent.id));
    const states = node('div', null, 'statusline'); states.append(stateBadge(agent.state), badge(`Matrix: ${agent.matrixIdentity}`, tone(agent.matrixIdentity)), badge('Runtime health: unknown')); card.append(states);
    card.append(facts([['Matrix ID', idChip(agent.mxid)], ['Role', agent.role], ['Approved request', idChip(agent.approvedRequestId)], ['Observed', new Date(agent.observedAt).toLocaleString()], ['Joined rooms', agent.joinedRooms === null ? 'unknown' : String(agent.joinedRooms.length || 'none')]]));
    if (agent.localTaskStop === 'unconfirmed') card.append(node('p', 'Matrix access is retired; stopping local Hagency tasks remains unconfirmed.', 'hint'));
    if (agent.observationError) card.append(node('p', `Could not observe Matrix state: ${agent.observationError}`, 'hint'));
    const actions = node('div', null, 'actions');
    if (agent.state === 'registered' && ['pending_connection', 'ready'].includes(fleet.state)) actions.append(button('Edit display name', async () => { const displayName = prompt('Display name', agent.displayName); if (!displayName) return; await api(`/fleets/${id}/agents/${agent.id}`, 'PATCH', { displayName }); await refresh(); }));
    if (agent.state !== 'retired') actions.append(button(agent.state === 'retiring' ? 'Retry retirement' : 'Retire identity', async () => { if (!confirm(`Deactivate ${agent.mxid} and remove its Matrix room memberships? History is preserved; local task stop is separate.`)) return; await api(`/fleets/${id}/agents/${agent.id}/retire`, 'POST', {}); await refresh(); }, 'danger'));
    card.append(actions); return card;
  }));
  if (!agents.length) $('#agents').append(node('p', 'No managed identities. Create an identity after approving its request.', 'empty'));
}
$('#login-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => { const form = event.target; const input = Object.fromEntries(new FormData(form)); try { const result = await api('/login', 'POST', input); csrf = result.csrf; await session(); $('#notice').hidden = true; } finally { form.elements.password.value = ''; } }); };
$('#fleet-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => { try { await api('/fleets', 'POST', Object.fromEntries(new FormData(event.target))); notice('App Service and representative verified. Event delivery and reception still require integration.'); event.target.reset(); event.target.elements.requestId.value = crypto.randomUUID(); } finally { await refresh(); } }); };
$('#agent-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => { await api(`/fleets/${selectedFleet}/agents`, 'POST', Object.fromEntries(new FormData(event.target))); event.target.reset(); notice('Matrix identity created. Hagency runtime and project admission are separate.'); await refresh(); }); };
$('#refresh').onclick = event => action(event.target, refresh);
$('#close-agents').onclick = () => { selectedFleet = null; $('#agent-panel').hidden = true; };
function option(value, label, disabled = false) { const el = node('option', label); el.value = value; el.disabled = disabled; return el; }
function roomLink(roomId, label) { const link = node('a', label); link.href = `https://matrix.to/#/${encodeURIComponent(roomId)}`; link.target = '_blank'; link.rel = 'noreferrer'; return link; }
function requestFleet() {
  const project = projects.find(row => row.id === $('#request-form [name=projectId]').value);
  return catalog.find(row => row.id === project?.fleetId);
}
function requestResourcePool() {
  const pool = new Map();
  for (const offer of requestFleet()?.capabilities?.offers ?? []) {
    for (const resource of offer.resources ?? []) {
      if (!pool.has(resource.id)) pool.set(resource.id, { ...resource, roles: [] });
      const entry = pool.get(resource.id);
      if (!entry.roles.includes(offer.role)) entry.roles.push(offer.role);
    }
  }
  return [...pool.values()];
}
function setRoles() {
  const fleet = requestFleet();
  const offers = fleet?.capabilities?.offers ?? [];
  const readFailed = fleet?.capabilityRead?.state === 'failed';
  const resources = requestResourcePool(), select = $('#request-form [name=resourceId]'), previous = select.value;
  select.replaceChildren(option('', 'Choose a resource from the pool'), ...resources.map(resource => option(resource.id, `${resource.name} · ${resource.model} / ${resource.reasoning ?? 'default'}`)));
  if (resources.some(resource => resource.id === previous)) select.value = previous;
  $('#request-role-hint').textContent = readFailed
    ? `Could not refresh roles from this Hagency (${fleet.capabilityRead.code}). ${offers.length ? 'The listed roles are from the last successful check. ' : ''}Refresh status to try again before requesting an agent.`
    : 'No supported roles are currently available from this Hagency. New usable resources are published automatically; its owner can check resource configuration and withdrawn roles.';
  $('#request-role-hint').hidden = !fleet || (!readFailed && offers.length > 0);
  renderRequestResources();
  setResourceRoles();
  updateRequestAvailability();
}
function setResourceRoles() {
  const resource = requestResourcePool().find(row => row.id === $('#request-form [name=resourceId]').value);
  const roles = resource?.roles ?? [], select = $('#request-form [name=role]'), previous = select.value;
  select.replaceChildren(...roles.map(role => option(role, role)));
  select.disabled = !resource;
  if (roles.includes(previous)) select.value = previous;
  else if (roles.includes('coding')) select.value = 'coding';
  $('#request-selected-resource').textContent = resource
    ? `${resource.model} · ${resource.reasoning ?? 'default'} effort · roles: ${roles.join(', ')}`
    : 'Choose one from the pool above; it sets the available roles.';
  for (const card of document.querySelectorAll('.resource-card')) card.classList.toggle('selected', card.dataset.resourceId === resource?.id);
}
function renderRequestResources() {
  const resources = requestResourcePool(), fleet = requestFleet();
  $('#request-resource-hint').hidden = !fleet?.capabilities?.offers?.length || resources.length > 0;
  const panel = $('#request-resources'); panel.replaceChildren();
  if (!resources.length) return;
  panel.append(node('h3', `Hagency resource pool · ${resources.length}`));
  panel.append(node('p', 'New Hagency resources appear automatically. This pool updates every 10 seconds while this page is visible. Choose a resource, then define your Agent and its role. Multiple Agents can use the same resource.', 'hint'));
  const cards = node('div', null, 'resource-pool-list'); panel.append(cards);
  const resourceName = distinctNames(resources);
  for (const resource of resources) {
    const card = node('article', null, 'resource-card'); card.dataset.resourceId = resource.id;
    if ($('#request-form [name=resourceId]').value === resource.id) card.classList.add('selected');
    const detail = [resource.name.includes(resource.model) ? null : resource.model, `${resource.reasoning ?? 'default'} effort`, resource.name.includes(resource.framework) ? null : resource.framework].filter(Boolean).join(' · ');
    const info = node('div', null, 'resource-info');
    info.append(nameHeading('h4', resourceName(resource), resource), node('p', `${detail} · roles: ${resource.roles.join(', ')}`, 'meta'));
    card.append(info);
    const choose = button('Define Agent on this resource', async () => {
      $('#request-form [name=resourceId]').value = resource.id;
      setResourceRoles(); updateRequestAvailability();
      $('#request-form').scrollIntoView({ block: 'start', behavior: 'smooth' }); $('#request-form [name=agentName]').focus({ preventScroll: true });
    });
    choose.disabled = fleet.capabilityRead?.state === 'failed';
    choose.classList.add('ghost');
    card.addEventListener('click', event => { if (event.target === card || info.contains(event.target)) choose.click(); });
    card.append(choose); cards.append(card);
  }
}
function requestNotice(message, error = false) {
  const el = $('#request-status'); el.textContent = message; el.className = error ? 'scope-note request-error' : 'scope-note'; el.hidden = false;
}
function verifyFleetConnection(fleetId) {
  const sessionToken = csrf, previous = connectionChecks.get(fleetId);
  if (previous?.pending && previous.sessionToken === sessionToken) return previous.promise;
  const check = { sessionToken, pending: true, error: null, retryAt: 0 };
  connectionChecks.set(fleetId, check);
  check.promise = Promise.resolve().then(async () => {
    let failure;
    try { await api(`/my/fleets/${fleetId}/connect`, 'POST', {}); }
    catch (error) { failure = error; }
    try { if (csrf === sessionToken) await refreshMember(); }
    catch (error) { failure ??= error; }
    if (failure) {
      check.error = failure.message; check.retryAt = Date.now() + 60000;
      // A failed round trip/readback must not leave a cached proof usable.
      if (csrf === sessionToken) catalog = catalog.map(fleet => fleet.id === fleetId
        ? { ...fleet, readiness: { ...fleet.readiness, ready: false } } : fleet);
    }
    check.pending = false;
    if (csrf === sessionToken) {
      if (!failure && previous?.notice && $('#notice').textContent === previous.notice) $('#notice').hidden = true;
      updateRequestAvailability();
    }
    if (failure) throw failure;
  });
  return check.promise;
}
function renewOwnerConnections() {
  if (!csrf || !memberView || document.hidden || requestBusy) return;
  let started = false;
  for (const fleet of catalog) {
    const check = connectionChecks.get(fleet.id);
    // Renew established connections only. First pairing/reception setup still
    // requires the owner's explicit action; another member cannot renew it.
    if (fleet.transport?.mode === 'outbound' || !fleet.owned || fleet.installation !== 'installed' || !['ready', 'pending_connection'].includes(fleet.state)
      || !fleet.connection?.verifiedAt || !fleet.reception?.roomId
      || (fleet.readiness?.ready === true && !check?.error && Date.parse(fleet.readiness?.expiresAt ?? '') - Date.now() > 60000)
      || check?.pending || check?.retryAt > Date.now()) continue;
    const sessionToken = csrf;
    started = true;
    void verifyFleetConnection(fleet.id).catch(error => {
      if (csrf === sessionToken) {
        const message = `Could not renew ${fleet.name}: ${error.message} We will retry while this page is visible. You can also use Verify connection.`;
        connectionChecks.get(fleet.id).notice = message;
        notice(message, true);
      }
    });
  }
  if (started) updateRequestAvailability();
}
function updateRequestAvailability() {
  clearTimeout(requestExpiryTimer);
  const project = projects.find(row => row.id === $('#request-form [name=projectId]').value);
  const fleet = catalog.find(row => row.id === project?.fleetId);
  const check = connectionChecks.get(fleet?.id);
  const remaining = Date.parse(fleet?.readiness?.expiresAt ?? '') - Date.now();
  const outbound = fleet?.transport?.mode === 'outbound';
  const ready = (outbound ? fleet?.readiness?.canQueue === true : fleet?.readiness?.ready === true && remaining > 0) && !check?.error;
  const offline = outbound && !fleet.transport.online;
  const connection = $('#request-connection'); connection.replaceChildren(); connection.hidden = !fleet || (ready && !offline); connection.classList.toggle('warn', !!fleet && (offline || !ready));
  if (fleet && ready && offline) connection.append(node('p', 'Hagency is offline. These are its last published resources. Your request will be stored in Palpo and delivered when Hagency reconnects; allocation still requires its owner’s decision.'));
  if (fleet && !ready) {
    const expired = remaining <= 0;
    connection.append(node('p', outbound ? 'Waiting for Hagency to receive the actual Matrix verification event through its outbound connection. Download and import the configuration, then verify the connection.' : expired ? 'Connection verification has expired. New agent requests cannot be sent until the connection is verified again.' : 'This Hagency connection is not ready to receive agent requests.'));
    if (check?.pending) connection.append(node('p', 'Renewing the connection automatically… Your request fields will be kept.'));
    if (check?.error) connection.append(node('p', `Connection could not be verified: ${check.error}`, 'request-error'));
    if (fleet.owned) {
      const verify = button('Verify connection', async () => {
        requestNotice('Verifying the connection… Your request fields will be kept.');
        try { await verifyFleetConnection(fleet.id); requestNotice(outbound ? 'Verification event queued. Hagency will confirm receipt automatically; refresh status to check.' : 'Connection verified. Review your request and click Send agent request.'); }
        catch (error) { requestNotice(`Connection could not be verified: ${error.message}`, true); }
      });
      verify.disabled = !!check?.pending;
      connection.append(verify);
    }
    else connection.append(node('p', `Ask the Hagency owner (${fleet.ownerMxid}) to use Verify connection & create reception in My Hagency access, then refresh status here.`));
  }
  const blocked = requestBusy ? 'Sending…' : !project?.canRequest ? 'Choose a ready project.' : !ready ? 'Waiting for the Hagency connection.' : fleet?.capabilityRead?.state === 'failed' ? 'Refresh status to reload resources.' : !$('#request-form [name=resourceId]').value ? 'Choose a resource to continue.' : !$('#request-form [name=role]').value ? 'Choose a role to continue.' : '';
  $('#request-form [type=submit]').disabled = !!blocked;
  $('#request-blocked').textContent = blocked;
  if (ready && !outbound) requestExpiryTimer = setTimeout(updateRequestAvailability, Math.min(remaining + 1, 2147483647));
  renewOwnerConnections();
}
async function refreshMember() {
  const sessionToken = csrf;
  const [ownedData, catalogData, projectData, requestData] = await Promise.all([api('/my/fleets'), readCatalog(), api('/projects'), api('/requests')]);
  if (csrf !== sessionToken || !memberView) return;
  catalog = catalogData.fleets; projects = projectData.projects;
  const ownedName = distinctNames(ownedData.fleets);
  const renderOwned = fleet => {
    const card = node('article', null, 'fleet'); card.dataset.fleetId = fleet.id;
    const top = node('div', null, 'section-heading'); top.append(nameHeading('h3', ownedName(fleet), fleet), statusBadge(fleet)); card.append(top);
    const downloaded = !!fleet.credentialDeliveredAt, verified = fleet.readiness?.ready || !!fleet.connection?.verifiedAt, revoked = fleet.state === 'revoked';
    const steps = node('ol', null, 'steps');
    const stale = fleet.transport?.mode === 'outbound' && !fleet.transport.online;
    for (const [label, done] of [['Download the configuration into Hagency', downloaded], ['Verify the connection and create the reception room', verified]]) { const li = node('li', label); if (done) li.className = stale || revoked ? 'done stale' : 'done'; steps.append(li); }
    card.append(steps);
    if (fleet.reception?.roomId) card.append(roomLink(fleet.reception.roomId, 'Open reception room'));
    if (revoked) card.append(node('p', 'The administrator revoked this service. Its configuration no longer connects.', 'hint'));
    else if (fleet.lastError) card.append(alertBox(`Last check: ${reason(fleet.lastError.code)} Use Verify connection again to continue.`, 'warning'));
    card.append(details([['Representative', idChip(fleet.representativeMxid)]], null, ownedName(fleet)));
    const actions = node('div', null, 'actions');
    actions.append(button('Download Hagency configuration', async () => {
      const result = await api(`/my/fleets/${fleet.id}/pair`, 'POST', {});
      const blob = new Blob([JSON.stringify(result, null, 2)], { type: 'application/json' }), url = URL.createObjectURL(blob);
      const link = document.createElement('a'); link.href = url; link.download = `${fleet.id}-registration.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      notice('Configuration downloaded for this Hagency. Store it in the Hagency credential store; retries preserve the same version.');
    }));
    actions.append(button('Verify connection & create reception', async () => {
      await verifyFleetConnection(fleet.id); notice(fleet.transport?.mode === 'outbound' ? 'Verification event queued for Hagency. Receipt will be confirmed over its outbound connection.' : 'Verified the actual Matrix event round trip and owner/representative reception membership.');
    }, downloaded && !verified && !revoked ? '' : 'secondary'));
    if (!downloaded && !revoked) actions.firstChild.className = '';
    if (downloaded && verified && !revoked) { const again = node('details', null, 'more reconfigure'); again.append(node('summary', 'Set up again'), actions); card.append(again); return card; }
    card.append(actions); return card;
  };
  const ownedActive = ownedData.fleets.filter(f => f.state !== 'revoked'), ownedRevoked = ownedData.fleets.filter(f => f.state === 'revoked');
  $('#my-fleets').replaceChildren(...ownedActive.map(renderOwned));
  if (ownedRevoked.length) { const fold = node('details', null, 'more revoked-group'); fold.append(node('summary', `Revoked (${ownedRevoked.length})`), ...ownedRevoked.map(renderOwned)); $('#my-fleets').append(fold); }
  if (!ownedData.fleets.length) $('#my-fleets').append(node('p', 'No Hagencys are assigned to your Matrix account. You can still request from available providers below.', 'empty'));
  $('#my-fleets-section').hidden = !ownedData.fleets.length; $('#nav-my-fleets').hidden = !ownedData.fleets.length; $('#my-fleet-count').textContent = ownedData.fleets.filter(f => f.state !== 'revoked').length;
  $('#project-count').textContent = projects.length;
  $('#request-needs-project').hidden = projects.length > 0;
  for (const el of [$('#request-form'), $('#request-resources'), $('#request-selected-resource'), $('#agent-name-help')]) el.hidden = projects.length === 0;
  const previousFleet = $('#project-form [name=fleetId]').value;
  const catalogName = distinctNames(catalog);
  $('#project-form [name=fleetId]').replaceChildren(...catalog.map(fleet => option(fleet.id, catalogName(fleet))));
  if (catalog.some(fleet => fleet.id === previousFleet)) $('#project-form [name=fleetId]').value = previousFleet;
  $('#projects').replaceChildren(...projects.map(project => {
    const card = node('article', null, 'fleet');
    const top = node('div', null, 'section-heading'); top.append(node('h3', project.name), project.canRequest ? stateBadge(project.state) : badge(readable(project.state), '')); card.append(top);
    const states = node('div', null, 'statusline'); states.append(badge(`Owner approval: ${project.ownerApproval}`, project.canRequest ? 'ok' : 'warning')); card.append(states);
    const links = node('div', null, 'links');
    if (project.roomId) links.append(roomLink(project.roomId, 'Open project room'));
    if (project.ownerDmRoomId) links.append(roomLink(project.ownerDmRoomId, 'Open private approval room'));
    card.append(links);
    if (project.readinessError) card.append(node('p', project.readinessError === 'owner_dm_join_pending' ? 'Waiting for the Hagency approval account to join. This page checks automatically; you can also refresh status.' : project.readinessError, 'hint'));
    card.append(details([['Owner', idChip(project.ownerMxid)], ['Project room', project.roomId ? idChip(project.roomId) : null]]));
    return card;
  }));
  if (!projects.length) $('#projects').append(node('p', 'Create a project or register an existing room you own.', 'empty'));
  const previousProject = $('#request-form [name=projectId]').value;
  $('#request-form [name=projectId]').replaceChildren(...projects.map(project => option(project.id, `${project.name}${project.canRequest ? '' : ' · not ready'}`, !project.canRequest)));
  if (projects.some(project => project.id === previousProject && project.canRequest)) $('#request-form [name=projectId]').value = previousProject;
  setRoles();
  for (const form of ['project-form', 'request-form']) if (!$(`#${form} [name=requestId]`).value) $(`#${form} [name=requestId]`).value = crypto.randomUUID();
  $('#request-count').textContent = requestData.requests.length; $('#request-count-nav').textContent = requestData.requests.length;
  const stale = requestData.requests.filter(r => r.lastError?.code === 'outbound_status_stale').length;
  $('#request-stale').hidden = !stale;
  $('#request-stale').replaceChildren(node('p', `Hagency has not reported the status of ${stale} ${stale === 1 ? 'request' : 'requests'} recently. Their states below may be out of date; they update when Hagency reconnects.`));
  const group = r => ['submission_pending', 'failed', 'rejected'].includes(r.state) ? 0 : r.state === 'active' && !r.usable ? 3 : ['pending', 'queued'].includes(r.state) ? 1 : r.usable ? 2 : 4;
  const groupNames = ['Needs attention', 'In review', 'Ready to use', 'Waiting for agent to join', 'Ended'];
  const groupOrder = [0, 3, 1, 2, 4];
  const ordered = [...requestData.requests].reverse();
  const cards = ordered.map(request => {
    const card = node('article', null, 'fleet'); card.dataset.requestId = request.requestId;
    const top = node('div', null, 'section-heading'); const title = node('div', null, 'title');
    const dup = requestData.requests.filter(r => (r.agentDefinition?.name ?? r.role) === (request.agentDefinition?.name ?? request.role)).length > 1;
    const when = request.createdAt ? new Date(request.createdAt).toLocaleString(undefined, dup ? { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' } : { month: 'short', day: 'numeric' }) : `#${String(request.requestId).slice(0, 6)}`;
    const projectName = projects.find(p => p.id === request.projectId)?.name;
    const sub = node('span', null, 'sub meta-line');
    [request.agentDefinition ? request.role : null, `${compact(request.requestedTokens)} tokens`, projectName, when].filter(Boolean).forEach(part => sub.append(node('span', part)));
    sub.title = `${request.requestedTokens.toLocaleString()} tokens`;
    title.append(node('h3', request.agentDefinition?.name ?? request.role), sub);
    top.append(title, request.state === 'active' && !request.usable ? badge('active', '') : stateBadge(request.state)); card.append(top);
    if (request.provider?.serving?.model) card.append(node('p', `Configuration: ${[...new Set([request.provider.serving.framework, request.provider.serving.model, request.provider.serving.reasoning, request.provider.serving.tier].filter(Boolean))].join(' · ')}`, 'meta'));
    if (request.resource && !request.provider?.serving?.model) card.append(node('p', `Configuration: ${request.resource.name} · ${request.resource.reasoning ?? 'default'}`, 'meta'));
    if (request.state === 'pending') card.append(node('p', 'Awaiting the Hagency owner’s resource decision.', 'hint'));
    if (request.state === 'queued') card.append(node('p', 'Stored in Palpo. Waiting for Hagency to receive this request; delivery does not approve or allocate an Agent.', 'hint'));
    if (request.state === 'submission_pending') card.append(alertBox('Not delivered to Hagency yet. It retries on its own; use Retry submission if this persists.', 'warning'));
    else if (request.provider?.fulfillment?.phase && request.provider.fulfillment.phase !== 'complete') card.append(node('p', `Preparation: ${reason(request.provider.fulfillment.phase)}`, 'meta'));
    if (request.usable) card.append(roomLink(request.targetRoomId, 'Open project and use agent'));
    if (request.lastError && request.lastError.code !== 'outbound_status_stale') card.append(node('p', reason(request.lastError.code), 'hint'));
    if (request.state === 'submission_pending') card.append(button('Retry submission', async () => {
      await api('/requests', 'POST', { requestId: request.requestId, projectId: request.projectId, role: request.role, requestedTokens: request.requestedTokens, ratePerDay: request.ratePerDay, ...(request.agentDefinition ? { agentDefinition: request.agentDefinition } : {}) }); await refreshMember();
    }));
    card.append(details([['Agent', request.provider?.agentMxid ? idChip(request.provider.agentMxid) : null], ['Request', idChip(request.requestId)]]));
    card.dataset.group = group(request);
    return card;
  });
  // One block per group; Ended is collapsed, long groups show five at first.
  const out = [];
  for (const index of groupOrder) { const name = groupNames[index];
    const members = cards.filter(card => card.dataset.group === String(index));
    if (!members.length) continue;
    const block = node('div', null, 'request-group');
    if (index === 4) {
      const ended = node('details', null, 'more group-fold'); ended.append(node('summary', `${name} (${members.length})`), ...members); block.append(ended);
    } else {
      block.append(node('h4', `${name} · ${members.length}`, 'group-heading'));
      if (index === 3) block.append(node('p', `Not usable yet: ${members.length === 1 ? 'this agent has' : 'these agents have'} not joined the project room. ${members.length === 1 ? 'It moves' : 'They move'} to Ready to use once joined.`, 'group-note'));
      if (index === 0) block.classList.add('attention-group');
      block.append(...members);
      if (members.length > 5) {
        members.slice(5).forEach(card => { card.hidden = true; });
        const more = button(`Show ${members.length - 5} more`, async () => { members.forEach(card => { card.hidden = false; }); more.remove(); }, 'secondary show-more'); block.append(more);
      }
    }
    out.push(block);
  }
  $('#requests').replaceChildren(...out);
  if (!requestData.requests.length) { const empty = node('div', null, 'empty inbox'); empty.append(node('strong', 'No agent requests yet'), document.createTextNode('Requests you send appear here with their review status. ')); const go = node('button', 'Request an agent', 'link'); go.type = 'button'; go.onclick = () => showTab($('#member-workspace'), 'request'); empty.append(go); $('#requests').append(empty); }
  const setupOpen = ownedActive.some(f => !f.credentialDeliveredAt || !(f.readiness?.ready || f.connection?.verifiedAt));
  showTab($('#member-workspace'), memberTab ?? (setupOpen ? 'access' : !projects.length ? 'projects' : requestData.requests.length ? 'requests' : 'request'));
}
$('#project-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => {
  try { await api('/projects', 'POST', Object.fromEntries(new FormData(event.target))); event.target.elements.name.value = ''; event.target.elements.roomId.value = ''; event.target.elements.requestId.value = crypto.randomUUID(); notice('Project and encrypted private approval room created. The Hagency approval account must join before you request an agent.'); }
  finally { await refreshMember(); }
}); };
$('#request-form').onsubmit = async event => {
  event.preventDefault(); updateRequestAvailability();
  if ($('#request-form [type=submit]').disabled) return;
  requestBusy = true; updateRequestAvailability(); requestNotice('Sending agent request…');
  try {
    const result = await api('/requests', 'POST', Object.fromEntries(new FormData(event.target)));
    event.target.elements.requestId.value = crypto.randomUUID();
    event.target.elements.agentName.value = '';
    showTab($('#member-workspace'), 'requests');
    requestNotice(result.request?.state === 'queued' ? `Request ${result.request.requestId} stored in Palpo. It will be delivered when Hagency connects; its owner will review the allocation.` : `Request ${result.request?.requestId ?? result.requestId ?? ''} delivered to Hagency. Its owner will review the resource allocation.`);
  } catch (error) { requestNotice(`Hagency has not confirmed this request: ${error.message} Your request ID and fields have been kept for retry.`, true); }
  finally {
    try { await refreshMember(); }
    catch (error) { notice(`Could not refresh request status: ${error.message}`, true); }
    requestBusy = false; updateRequestAvailability();
  }
};
$('#request-form [name=projectId]').onchange = setRoles;
$('#request-form [name=requestedTokens]').oninput = event => { const n = Number(event.target.value); $('#tokens-hint').textContent = Number.isFinite(n) && n > 0 ? `About ${compact(n)}` : 'Enter a whole number of tokens'; };
$('#request-form [name=role]').onchange = updateRequestAvailability;
$('#request-form [name=resourceId]').onchange = () => { setResourceRoles(); updateRequestAvailability(); };
$('#member-refresh').onclick = event => action(event.target, refreshMember);
function updateTransportForm() {
  const callback = $('#fleet-form [name=transportMode]').value === 'callback';
  $('#callback-field').hidden = !callback;
  $('#fleet-form [name=callbackUrl]').disabled = !callback; $('#fleet-form [name=callbackUrl]').required = callback;
  $('#callback-policy').textContent = callback ? `Allowed callback origins: ${(currentSession?.callbackOrigins ?? []).join(', ')}` : 'Hagency connects to this Palpo server over HTTPS. No inbound Hagency port or reverse tunnel is needed.';
}
$('#fleet-form [name=transportMode]').onchange = updateTransportForm;
$('#show-member').onclick = event => action(event.target, async () => { memberView = true; setNav('member'); $('#workspace').hidden = true; $('#member-workspace').hidden = false; await refreshMember(); });
$('#show-admin').onclick = event => action(event.target, async () => { memberView = false; setNav('admin'); $('#workspace').hidden = false; $('#member-workspace').hidden = true; await refresh(); });
for (const reveal of document.querySelectorAll('.password-field .reveal')) reveal.onclick = () => {
  const input = reveal.parentElement.querySelector('input'), show = input.type === 'password';
  input.type = show ? 'text' : 'password'; reveal.textContent = show ? 'Hide' : 'Show';
  reveal.setAttribute('aria-pressed', String(show)); reveal.setAttribute('aria-label', show ? 'Hide password' : 'Show password');
};
session().catch(error => { if (!csrf) showLogin(); else notice(error.message, true); });
catalogPollTimer = setTimeout(refreshResourceCatalog, 10000);
window.addEventListener('focus', refreshResourceCatalog);
document.addEventListener('visibilitychange', () => { if (!document.hidden) refreshResourceCatalog(); });
for (const link of document.querySelectorAll('[data-goto]')) link.onclick = () => showTab(link.closest('#member-workspace, #workspace'), link.dataset.goto);
$('#open-authorize').onclick = () => {
  const panel = $('#authorize-panel'); panel.open = !panel.open; $('#open-authorize').setAttribute('aria-expanded', String(panel.open));
  if (panel.open) { panel.scrollIntoView({ block: 'start', behavior: 'smooth' }); panel.querySelector('input[name=name]').focus({ preventScroll: true }); }
};
