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
function showLogin() { $('#workspace').hidden = true; $('#member-workspace').hidden = true; $('#view-nav').hidden = true; $('#login-panel').hidden = false; $('#account').replaceChildren(); csrf = null; currentSession = null; connectionChecks.clear(); clearTimeout(requestExpiryTimer); }
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
function readable(value) { return value.replaceAll('_', ' '); }
async function session() {
  const data = await api('/session'); csrf = data.csrf; currentSession = data; memberView = !data.isAdmin;
  $('#login-panel').hidden = true; $('#account-request-panel').hidden = true; $('#workspace').hidden = !data.isAdmin; $('#member-workspace').hidden = data.isAdmin; $('#view-nav').hidden = !data.isAdmin;
  $('#server-name').textContent = data.serverName;
  $('#callback-policy').textContent = data.callbackOrigins.length ? `Allowed callback origins: ${data.callbackOrigins.join(', ')}` : 'No callback origins are allowed yet. Configure the server callback policy before installing.';
  $('#fleet-form [name=transportMode] option[value=outbound]').disabled = !data.outboundAvailable;
  $('#fleet-form [name=transportMode]').value = data.outboundAvailable ? 'outbound' : 'callback';
  updateTransportForm();
  $('#account').replaceChildren(node('span', data.userId), button('Sign out', async () => { await api('/logout', 'POST', {}); showLogin(); }));
  if (!$('#fleet-form [name=requestId]').value) $('#fleet-form [name=requestId]').value = crypto.randomUUID();
  $('#member-server-name').textContent = data.serverName;
  if (data.isAdmin) await refresh(); else await refreshMember();
}
async function refresh() {
  const [fleets, audit, accounts] = await Promise.all([api('/fleets'), api('/audit'), api('/account-requests')]); fleetRows = fleets.fleets;
  $('#account-admin-panel').hidden = !accounts.enabled;
  $('#account-admin-room').replaceChildren(...(accounts.roomId ? [roomLink(accounts.roomId, 'Open account approval room in Robrix'), node('p', accounts.roomId, 'meta')] : [node('p', 'The private administrator room is being prepared.', 'hint')]));
  if (accounts.lastError) $('#account-admin-room').append(node('p', `Account service needs attention: ${accounts.lastError}`, 'error'));
  $('#account-admin-requests').replaceChildren(...accounts.requests.slice(-50).reverse().map(row => {
    const card = node('article', null, 'fleet'); card.append(node('strong', row.userId), node('p', row.displayName), badge(readable(row.status)));
    if (row.decidedBy) card.append(node('p', `Decision by ${row.decidedBy}`, 'meta'));
    if (row.lastError) card.append(node('p', `Waiting to complete: ${row.lastError}`, 'hint'));
    return card;
  }));
  $('#fleet-count').textContent = fleetRows.length;
  $('#fleets').replaceChildren(...fleetRows.map(renderFleet));
  if (!fleetRows.length) $('#fleets').append(node('p', 'No HAFleets have been authorized yet.', 'empty'));
  const table = node('table'), head = node('tr');
  for (const title of ['When', 'Actor', 'Operation', 'Result']) head.append(node('th', title));
  table.append(head);
  for (const event of audit.events) { const row = node('tr'); for (const value of [new Date(event.at).toLocaleString(), event.actor, event.action, readable(event.result)]) row.append(node('td', value)); table.append(row); }
  $('#audit').replaceChildren(audit.events.length ? table : node('p', 'Operations will appear here after you authorize a provider.', 'empty'));
  if (selectedFleet) await showAgents(selectedFleet);
}
function renderFleet(fleet) {
  const card = node('article', null, 'fleet'); card.dataset.fleetId = fleet.id;
  const top = node('div', null, 'section-heading'); top.append(node('h3', fleet.name), badge(fleet.state === 'revoked' ? 'App Service revoked' : readable(fleet.state), fleet.state === 'revoked' ? 'error' : 'warning')); card.append(top);
  card.append(node('div', `Owner: ${fleet.ownerMxid}`, 'meta'));
  card.append(node('div', `Representative: ${fleet.representativeMxid}`, 'meta'));
  card.append(node('div', fleet.transport?.mode === 'outbound' ? `Outbound connection · ${fleet.transport.online ? 'online' : 'offline'} · generation ${fleet.transport.generation}` : `Callback: ${fleet.callbackUrl}`, 'meta'));
  const states = node('div', null, 'statusline'); states.append(badge(`Installation: ${fleet.installation}`), badge(`Identity: ${fleet.readiness.identity}`), badge(`Event delivery: ${fleet.readiness.eventDelivery}`, fleet.readiness.eventDelivery === 'verified' ? '' : 'warning'), badge(`Reception: ${fleet.readiness.reception}`, fleet.readiness.reception === 'verified' ? '' : 'warning')); card.append(states);
  card.append(node('div', `Credential version ${fleet.credentialVersion} · ${fleet.credentialDeliveredAt ? 'Issued to owner' : 'Awaiting owner pairing'} · Local task stop: ${fleet.localTaskStop}`, 'meta'));
  if (fleet.state === 'revoked') card.append(node('p', 'The service credential is disabled. Retire each identity separately to remove its Matrix memberships and any independent sessions.', 'hint'));
  const pairing = node('details'), summary = node('summary', 'Owner pairing endpoint'); pairing.append(summary, node('p', 'The HAFleet owner uses their own Matrix session to retrieve the assigned credentials. Retries return the same version. No administrator token is shared.', 'hint'), node('code', `POST /api/pair/${fleet.id}`)); card.append(pairing);
  if (fleet.lastError) card.append(node('div', `Last operation failed: ${fleet.lastError.code}. Retry installation with the existing registration.`, 'meta'));
  const actions = node('div', null, 'actions');
  actions.append(button(`Manage ${fleet.agentCount} identities`, () => showAgents(fleet.id)));
  if (fleet.transport?.mode === 'outbound') actions.append(button('Inspect delivery capacity', async () => {
    const { queue } = await api(`/fleets/${fleet.id}/outbound`);
    notice(`Delivery queue: ${queue.pending}/${queue.limits.pending} pending, ${queue.records}/${queue.limits.records} retained records, ${queue.bytes}/${queue.limits.bytes} pending bytes. The server operator can increase configured limits while retaining all delivery receipts.`);
  }));
  const mutate = async name => { await api(`/fleets/${fleet.id}/${name}`, 'POST', {}); notice(name === 'revoke' ? 'Matrix service access revoked. Local task stop remains unconfirmed.' : 'Registration state verified on Palpo.'); await refresh(); };
  if (fleet.installation !== 'installed' && !['paused', 'revoked'].includes(fleet.state)) actions.append(button('Retry installation', () => mutate('install')));
  if (fleet.installation === 'installed' && fleet.state !== 'revoked') {
    if (currentSession?.outboundAvailable && fleet.state !== 'paused') actions.append(button(fleet.transport?.mode === 'outbound' ? 'Rotate transport credential' : 'Migrate to outbound connection', async () => {
      const key = `palpo-outbound-${fleet.id}`, requestId = sessionStorage.getItem(key) ?? crypto.randomUUID(); sessionStorage.setItem(key, requestId);
      await api(`/fleets/${fleet.id}/outbound`, 'POST', { requestId, rotate: fleet.transport?.mode === 'outbound' });
      sessionStorage.removeItem(key); notice('Outbound registration is ready. The owner must download the new configuration into HAFleet and verify the Matrix event channel.'); await refresh();
    }));
    actions.append(button(fleet.state === 'paused' ? 'Resume' : 'Pause', () => mutate(fleet.state === 'paused' ? 'resume' : 'pause')));
    actions.append(button('Revoke service', async () => { if (confirm(`Revoke the App Service for ${fleet.name}? Its service token will stop working. Retire individual identities separately to remove memberships and independent sessions. Local tasks require HAFleet confirmation.`)) await mutate('revoke'); }, 'danger'));
  }
  card.append(actions); return card;
}
async function showAgents(id) {
  selectedFleet = id; const fleet = fleetRows.find(item => item.id === id);
  const { agents } = await api(`/fleets/${id}/agents`);
  $('#agent-panel').hidden = false; $('#agent-heading').textContent = `${fleet.name} · Agent identities`;
  $('#agent-form').hidden = !['pending_connection', 'ready'].includes(fleet.state) || fleet.installation !== 'installed';
  $('#agents').replaceChildren(...agents.map(agent => {
    const card = node('article', null, 'fleet'); card.append(node('h3', agent.displayName ?? agent.id), node('div', agent.mxid, 'meta'));
    const states = node('div', null, 'statusline'); states.append(badge(readable(agent.state)), badge(`Matrix: ${agent.matrixIdentity}`), badge('Runtime health: unknown', 'warning')); card.append(states);
    card.append(node('div', `Role: ${agent.role} · Request: ${agent.approvedRequestId} · Observed: ${new Date(agent.observedAt).toLocaleString()}`, 'meta'));
    card.append(node('div', `Joined rooms: ${agent.joinedRooms === null ? 'unknown' : agent.joinedRooms.join(', ') || 'none'}`, 'meta'));
    if (agent.localTaskStop === 'unconfirmed') card.append(node('p', 'Matrix access is retired; stopping local HAFleet tasks remains unconfirmed.', 'hint'));
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
$('#agent-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => { await api(`/fleets/${selectedFleet}/agents`, 'POST', Object.fromEntries(new FormData(event.target))); event.target.reset(); notice('Matrix identity created. HAFleet runtime and project admission are separate.'); await refresh(); }); };
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
    ? `Could not refresh roles from this HAFleet (${fleet.capabilityRead.code}). ${offers.length ? 'The listed roles are from the last successful check. ' : ''}Refresh status to try again before requesting an agent.`
    : 'No supported roles are currently available from this HAFleet. New usable resources are published automatically; its owner can check resource configuration and withdrawn roles.';
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
    ? `${resource.name} · ${resource.model} / ${resource.reasoning ?? 'default'} · Supported roles: ${roles.join(', ')}`
    : 'Select a Resource first. Role describes the Agent’s job; available roles depend on the selected resource.';
}
function renderRequestResources() {
  const resources = requestResourcePool(), fleet = requestFleet();
  $('#request-resource-hint').hidden = !fleet?.capabilities?.offers?.length || resources.length > 0;
  const panel = $('#request-resources'); panel.replaceChildren();
  if (!resources.length) return;
  panel.append(node('h3', `HAFleet resource pool · ${resources.length}`));
  panel.append(node('p', 'New HAFleet resources appear automatically. This pool updates every 10 seconds while this page is visible. Choose a resource, then define your Agent and its role. Multiple Agents can use the same resource.', 'hint'));
  const cards = node('div', null, 'resource-pool-list'); panel.append(cards);
  for (const resource of resources) {
    const card = node('article', null, 'resource-card'); card.dataset.resourceId = resource.id;
    card.append(node('h4', resource.name), node('p', `${resource.framework} · ${resource.model} · ${resource.reasoning ?? 'default'}`, 'meta'));
    card.append(node('p', `Supported roles: ${resource.roles.join(', ')}`, 'hint'));
    const choose = button('Define Agent on this resource', async () => {
      $('#request-form [name=resourceId]').value = resource.id;
      setResourceRoles(); updateRequestAvailability();
      $('#request-form').scrollIntoView({ block: 'center' }); $('#request-form [name=agentName]').focus();
    });
    choose.disabled = fleet.capabilityRead?.state === 'failed';
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
  const connection = $('#request-connection'); connection.replaceChildren(); connection.hidden = !fleet || (ready && !offline);
  if (fleet && ready && offline) connection.append(node('p', 'HAFleet is offline. These are its last published resources. Your request will be stored in Palpo and delivered when HAFleet reconnects; allocation still requires its owner’s decision.'));
  if (fleet && !ready) {
    const expired = remaining <= 0;
    connection.append(node('p', outbound ? 'Waiting for HAFleet to receive the actual Matrix verification event through its outbound connection. Download and import the configuration, then verify the connection.' : expired ? 'Connection verification has expired. New agent requests cannot be sent until the connection is verified again.' : 'This HAFleet connection is not ready to receive agent requests.'));
    if (check?.pending) connection.append(node('p', 'Renewing the connection automatically… Your request fields will be kept.'));
    if (check?.error) connection.append(node('p', `Connection could not be verified: ${check.error}`, 'request-error'));
    if (fleet.owned) {
      const verify = button('Verify connection', async () => {
        requestNotice('Verifying the connection… Your request fields will be kept.');
        try { await verifyFleetConnection(fleet.id); requestNotice(outbound ? 'Verification event queued. HAFleet will confirm receipt automatically; refresh status to check.' : 'Connection verified. Review your request and click Send agent request.'); }
        catch (error) { requestNotice(`Connection could not be verified: ${error.message}`, true); }
      });
      verify.disabled = !!check?.pending;
      connection.append(verify);
    }
    else connection.append(node('p', `Ask the HAFleet owner (${fleet.ownerMxid}) to use Verify connection & create reception in My HAFleet access, then refresh status here.`));
  }
  $('#request-form [type=submit]').disabled = requestBusy || !project?.canRequest || !ready || fleet?.capabilityRead?.state === 'failed' || !$('#request-form [name=role]').value || !$('#request-form [name=resourceId]').value;
  if (ready && !outbound) requestExpiryTimer = setTimeout(updateRequestAvailability, Math.min(remaining + 1, 2147483647));
  renewOwnerConnections();
}
async function refreshMember() {
  const sessionToken = csrf;
  const [ownedData, catalogData, projectData, requestData] = await Promise.all([api('/my/fleets'), readCatalog(), api('/projects'), api('/requests')]);
  if (csrf !== sessionToken || !memberView) return;
  catalog = catalogData.fleets; projects = projectData.projects;
  $('#my-fleets').replaceChildren(...ownedData.fleets.map(fleet => {
    const card = node('article', null, 'fleet'); card.dataset.fleetId = fleet.id;
    const top = node('div', null, 'section-heading'); top.append(node('h3', fleet.name), badge(fleet.transport?.mode === 'outbound' && !fleet.transport.online ? 'Offline · outbound connection' : fleet.readiness.ready ? 'Ready to receive requests' : readable(fleet.state), fleet.readiness.ready ? '' : 'warning')); card.append(top);
    card.append(node('p', fleet.representativeMxid, 'meta'));
    if (fleet.reception?.roomId) card.append(roomLink(fleet.reception.roomId, 'Open reception room'));
    if (fleet.lastError) card.append(node('p', `Last check: ${fleet.lastError.code}. Use Verify connection again to continue.`, 'hint'));
    const actions = node('div', null, 'actions');
    actions.append(button('Download HAFleet configuration', async () => {
      const result = await api(`/my/fleets/${fleet.id}/pair`, 'POST', {});
      const blob = new Blob([JSON.stringify(result, null, 2)], { type: 'application/json' }), url = URL.createObjectURL(blob);
      const link = document.createElement('a'); link.href = url; link.download = `${fleet.id}-registration.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      notice('Configuration downloaded for this HAFleet. Store it in the HAFleet credential store; retries preserve the same version.');
    }));
    actions.append(button('Verify connection & create reception', async () => {
      await verifyFleetConnection(fleet.id); notice(fleet.transport?.mode === 'outbound' ? 'Verification event queued for HAFleet. Receipt will be confirmed over its outbound connection.' : 'Verified the actual Matrix event round trip and owner/representative reception membership.');
    }));
    card.append(actions); return card;
  }));
  if (!ownedData.fleets.length) $('#my-fleets').append(node('p', 'No HAFleets are assigned to your Matrix account. You can still request from available providers below.', 'empty'));
  const previousFleet = $('#project-form [name=fleetId]').value;
  $('#project-form [name=fleetId]').replaceChildren(...catalog.map(fleet => option(fleet.id, `${fleet.name}${fleet.readiness.ready ? ' · ready' : ' · pending connection'}`)));
  if (catalog.some(fleet => fleet.id === previousFleet)) $('#project-form [name=fleetId]').value = previousFleet;
  $('#projects').replaceChildren(...projects.map(project => {
    const card = node('article', null, 'fleet'); card.append(node('h3', project.name), node('p', `Owner: ${project.ownerMxid}`, 'meta'));
    const states = node('div', null, 'statusline'); states.append(badge(readable(project.state)), badge(`Owner approval: ${project.ownerApproval}`, project.canRequest ? '' : 'warning')); card.append(states);
    if (project.roomId) card.append(roomLink(project.roomId, 'Open project room'));
    if (project.ownerDmRoomId) { card.append(node('span', ' · '), roomLink(project.ownerDmRoomId, 'Open private approval room')); }
    if (project.readinessError) card.append(node('p', project.readinessError === 'owner_dm_join_pending' ? 'Waiting for the HAFleet approval account to join. This page checks automatically; you can also refresh status.' : project.readinessError, 'hint'));
    return card;
  }));
  if (!projects.length) $('#projects').append(node('p', 'Create a project or register an existing room you own.', 'empty'));
  const previousProject = $('#request-form [name=projectId]').value;
  $('#request-form [name=projectId]').replaceChildren(...projects.map(project => option(project.id, `${project.name}${project.canRequest ? '' : ' · not ready'}`, !project.canRequest)));
  if (projects.some(project => project.id === previousProject && project.canRequest)) $('#request-form [name=projectId]').value = previousProject;
  setRoles();
  for (const form of ['project-form', 'request-form']) if (!$(`#${form} [name=requestId]`).value) $(`#${form} [name=requestId]`).value = crypto.randomUUID();
  $('#requests').replaceChildren(...requestData.requests.map(request => {
    const card = node('article', null, 'fleet'); card.dataset.requestId = request.requestId;
    const top = node('div', null, 'section-heading'); top.append(node('h3', `${request.agentDefinition ? request.agentDefinition.name + ' · ' : ''}${request.role} · ${request.requestedTokens.toLocaleString()} tokens`), badge(readable(request.state), request.state === 'active' && request.agentJoined ? '' : 'warning')); card.append(top);
    if (request.resource) card.append(node('p', `Requested resource: ${request.resource.name} · ${request.resource.model} / ${request.resource.reasoning ?? 'default'}`, 'meta'));
    card.append(node('p', `Request ${request.requestId}`, 'meta'));
    if (request.state === 'pending') card.append(node('p', 'Awaiting the HAFleet owner’s resource decision.', 'hint'));
    if (request.state === 'queued') card.append(node('p', 'Stored in Palpo. Waiting for HAFleet to receive this request; delivery does not approve or allocate an Agent.', 'hint'));
    if (request.provider?.agentMxid) card.append(node('p', `Agent: ${request.provider.agentMxid}`, 'meta'));
    if (request.provider?.serving?.model) card.append(node('p', `Configuration: ${[request.provider.serving.framework, request.provider.serving.model, request.provider.serving.reasoning, request.provider.serving.tier].filter(Boolean).join(' · ')}`, 'meta'));
    if (request.provider?.fulfillment?.phase) card.append(node('p', `Preparation: ${request.provider.fulfillment.phase}`, 'meta'));
    if (request.usable) card.append(roomLink(request.targetRoomId, 'Open project and use agent'));
    if (request.lastError) card.append(node('p', `Status: ${request.lastError.code}`, 'hint'));
    if (request.state === 'submission_pending') card.append(button('Retry submission', async () => {
      await api('/requests', 'POST', { requestId: request.requestId, projectId: request.projectId, role: request.role, requestedTokens: request.requestedTokens, ratePerDay: request.ratePerDay, ...(request.agentDefinition ? { agentDefinition: request.agentDefinition } : {}) }); await refreshMember();
    }));
    return card;
  }));
}
$('#project-form').onsubmit = event => { event.preventDefault(); action(event.submitter, async () => {
  try { await api('/projects', 'POST', Object.fromEntries(new FormData(event.target))); event.target.elements.name.value = ''; event.target.elements.roomId.value = ''; event.target.elements.requestId.value = crypto.randomUUID(); notice('Project and encrypted private approval room created. The HAFleet approval account must join before you request an agent.'); }
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
    requestNotice(result.request?.state === 'queued' ? `Request ${result.request.requestId} stored in Palpo. It will be delivered when HAFleet connects; its owner will review the allocation.` : `Request ${result.request?.requestId ?? result.requestId ?? ''} delivered to HAFleet. Its owner will review the resource allocation.`);
  } catch (error) { requestNotice(`HAFleet has not confirmed this request: ${error.message} Your request ID and fields have been kept for retry.`, true); }
  finally {
    try { await refreshMember(); }
    catch (error) { notice(`Could not refresh request status: ${error.message}`, true); }
    requestBusy = false; updateRequestAvailability();
  }
};
$('#request-form [name=projectId]').onchange = setRoles;
$('#request-form [name=role]').onchange = updateRequestAvailability;
$('#request-form [name=resourceId]').onchange = () => { setResourceRoles(); updateRequestAvailability(); };
$('#member-refresh').onclick = event => action(event.target, refreshMember);
function updateTransportForm() {
  const callback = $('#fleet-form [name=transportMode]').value === 'callback';
  $('#callback-field').hidden = !callback;
  $('#fleet-form [name=callbackUrl]').disabled = !callback; $('#fleet-form [name=callbackUrl]').required = callback;
  $('#callback-policy').textContent = callback ? `Allowed callback origins: ${(currentSession?.callbackOrigins ?? []).join(', ')}` : 'HAFleet connects to this Palpo server over HTTPS. No inbound HAFleet port or reverse tunnel is needed.';
}
$('#fleet-form [name=transportMode]').onchange = updateTransportForm;
$('#show-member').onclick = event => action(event.target, async () => { memberView = true; $('#workspace').hidden = true; $('#member-workspace').hidden = false; await refreshMember(); });
$('#show-admin').onclick = event => action(event.target, async () => { memberView = false; $('#workspace').hidden = false; $('#member-workspace').hidden = true; await refresh(); });
session().catch(error => { if (!csrf) showLogin(); else notice(error.message, true); });
catalogPollTimer = setTimeout(refreshResourceCatalog, 10000);
window.addEventListener('focus', refreshResourceCatalog);
document.addEventListener('visibilitychange', () => { if (!document.hidden) refreshResourceCatalog(); });
