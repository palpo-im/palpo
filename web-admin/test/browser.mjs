import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';

// The production application has no package dependencies. For this optional
// browser check, install playwright-core or provide its module path explicitly.
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright-core');
const reservation = createServer().listen(0, '127.0.0.1');
await once(reservation, 'listening'); const port = reservation.address().port;
await new Promise(resolve => reservation.close(resolve));
const f = fixture();
const origin = `http://127.0.0.1:${port}`;
const server = createApp({ service: f.service, publicOrigin: origin });
server.listen(port, '127.0.0.1'); await once(server, 'listening');
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_EXECUTABLE ? { executablePath: process.env.CHROME_EXECUTABLE } : { channel: 'chrome' }) });
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await context.newPage();
const pageErrors = [], apiResponses = [];
page.on('pageerror', error => pageErrors.push(error.message));
page.on('response', async response => { if (response.url().includes('/api/')) apiResponses.push(await response.text().catch(() => '')); });
const evidence = resolve('test-results'); await mkdir(evidence, { recursive: true });
try {
  await page.goto(origin);
  await page.getByLabel('Matrix ID', { exact: true }).fill('@admin:example.test');
  await page.getByLabel('Password', { exact: true }).fill('correct-password');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('heading', { name: 'HAFleet connections' }).waitFor();
  // Regression: an AS credential failure must preserve the administrator login
  // and durable failed operation, as seen during the first real deployment.
  let denyAsOnce = true;
  f.palpo.fetch = async (target, options) => {
    if (denyAsOnce && new URL(target).pathname === '/_matrix/client/v3/account/whoami' && [...f.registrations.values()].some(reg => options.headers.Authorization === `Bearer ${reg.as_token}`)) {
      denyAsOnce = false;
      return new Response(JSON.stringify({ errcode: 'M_UNKNOWN_TOKEN' }), { status: 401 });
    }
    return f.fetch(target, options);
  };
  const fleetForm = page.locator('#fleet-form');
  await fleetForm.getByLabel('Name', { exact: true }).fill('Octos coding team');
  await fleetForm.getByLabel('Owner Matrix ID').fill('@owner:example.test');
  await fleetForm.getByLabel('HAFleet callback URL').fill('https://fleet.example.test/matrix');
  await fleetForm.getByRole('button', { name: 'Authorize and install' }).click();
  const card = page.locator('#fleets article').filter({ hasText: 'Octos coding team' });
  await card.getByRole('button', { name: 'Retry installation', exact: true }).waitFor();
  assert.equal(await page.locator('#login-panel').isVisible(), false);
  assert.equal(await page.locator('#workspace').isVisible(), true);
  await card.getByRole('button', { name: 'Retry installation', exact: true }).click();
  await card.getByText('Identity: verified', { exact: true }).waitFor();
  assert.match(await card.innerText(), /pending connection/);
  assert.match(await card.innerText(), /Event delivery: unverified/);
  await page.screenshot({ path: resolve(evidence, 'fleets.png'), fullPage: true });
  await card.getByRole('button', { name: 'Manage 0 identities' }).click();
  const agentForm = page.locator('#agent-form');
  await agentForm.getByLabel('Stable agent ID').fill('coding_01');
  await agentForm.getByLabel('Display name').fill('Coding agent');
  await agentForm.getByLabel('Public role').fill('coding');
  await agentForm.getByLabel('Approved request reference').fill('approval-42');
  await agentForm.getByRole('button', { name: 'Create identity' }).click();
  await page.locator('#agents').getByText('Matrix: active', { exact: true }).waitFor();
  page.once('dialog', dialog => dialog.accept('Coding agent updated'));
  await page.getByRole('button', { name: 'Edit display name' }).click();
  await page.locator('#agents').getByRole('heading', { name: 'Coding agent updated' }).waitFor();
  await page.screenshot({ path: resolve(evidence, 'agent-created.png'), fullPage: true });
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Retire identity' }).click();
  await page.locator('#agents').getByText('Matrix: deactivated', { exact: true }).waitFor();
  assert.match(await page.locator('#agents').innerText(), /local HAFleet tasks remains unconfirmed/);
  await card.getByRole('button', { name: 'Pause', exact: true }).click();
  await card.getByText('paused', { exact: true }).waitFor();
  await card.getByRole('button', { name: 'Resume', exact: true }).click();
  await card.getByText('pending connection', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(evidence, 'agent-retired.png'), fullPage: true });
  // Continue as the actual fleet owner, then a different project owner. The
  // provider's resource verdict is a controlled fixture; no real approval occurs.
  const signIn = async mxid => {
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
    await page.getByLabel('Matrix ID', { exact: true }).fill(mxid);
    await page.getByLabel('Password', { exact: true }).fill('correct-password');
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('heading', { name: 'HAFleet services', exact: true }).waitFor();
  };
  await signIn('@owner:example.test');
  await page.getByRole('button', { name: 'Verify connection & create reception', exact: true }).click();
  await page.locator('#my-fleets').getByText('Ready to receive requests', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(evidence, 'owner-connected.png'), fullPage: true });
  await signIn('@other:example.test');
  const projectForm = page.locator('#project-form');
  await projectForm.getByLabel('Project name', { exact: true }).fill('Owner UI project');
  await projectForm.getByRole('button', { name: 'Create project and approval room', exact: true }).click();
  await page.locator('#projects').getByText('Owner approval: ready', { exact: true }).waitFor();
  const requestForm = page.locator('#request-form');
  // An event-ready provider can still have no advertised role. Keep readiness
  // truthful while guiding the requester and preventing an empty submission.
  const actualFleetId = [...f.registrations.keys()][0];
  f.publishedOffers.set(actualFleetId, [{ role: 'coding', published: false }]);
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click();
  await page.locator('#request-role-hint').waitFor({ state: 'visible' });
  assert.match(await page.locator('#request-role-hint').innerText(), /New usable resources are published automatically/);
  assert.equal(await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).isEnabled(), false);
  assert.equal(await requestForm.getByLabel('Role', { exact: true }).locator('option').count(), 0);
  const advertised = await page.evaluate(async () => (await (await fetch('/api/catalog')).json()).fleets);
  assert.equal(advertised.find(fleet => fleet.id === actualFleetId).readiness.ready, true);
  const projectCount = Object.keys(f.store.state.projects).length, roomCount = f.rooms.size;
  const medium = {
    id: `resource_${'a'.repeat(24)}`, name: 'Medium coding resource', framework: 'codex', model: 'gpt-5.6-sol', reasoning: 'medium',
    agents: ['fast-one', 'fast-two'].map(name => ({ name, role: 'coding', status: 'defined' })),
  };
  const strong = { ...medium, id: `resource_${'b'.repeat(24)}`, name: 'Strong architecture resource', reasoning: 'high' };
  const light = { ...medium, id: `resource_${'c'.repeat(24)}`, name: 'Light documentation resource', reasoning: 'low' };
  const poolOffers = [{ role: 'coding', published: true, resources: [medium, strong] },
    { role: 'architect', published: true, resources: [strong] },
    { role: 'testing', published: true, resources: [medium, strong] },
    { role: 'documentation', published: true, resources: [medium, strong, light] }];
  f.publishedOffers.set(actualFleetId, poolOffers);
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click();
  await page.locator('#request-role-hint').waitFor({ state: 'hidden' });
  const pool = page.locator('#request-resources');
  await pool.getByRole('heading', { name: 'HAFleet resource pool · 3', exact: true }).waitFor();
  assert.equal(await pool.locator('article').count(), 3); // Shared resources appear once, independent of Role.
  assert.equal(await requestForm.getByLabel('Role', { exact: true }).isDisabled(), true);
  assert.equal(await requestForm.getByLabel('Resource', { exact: true }).locator('option').count(), 4);
  await pool.locator(`article[data-resource-id="${light.id}"]`).getByRole('button', { name: 'Define Agent on this resource', exact: true }).click();
  assert.deepEqual(await requestForm.getByLabel('Role', { exact: true }).locator('option').allTextContents(), ['documentation']);
  await pool.locator(`article[data-resource-id="${strong.id}"]`).getByRole('button', { name: 'Define Agent on this resource', exact: true }).click();
  assert.ok((await requestForm.getByLabel('Role', { exact: true }).locator('option').allTextContents()).includes('architect'));
  await pool.locator(`article[data-resource-id="${medium.id}"]`).getByRole('button', { name: 'Define Agent on this resource', exact: true }).click();
  assert.ok(!(await requestForm.getByLabel('Role', { exact: true }).locator('option').allTextContents()).includes('architect'));
  await requestForm.getByLabel('Role', { exact: true }).selectOption('coding');
  assert.equal(await pool.locator('article').count(), 3); // Role changes never hide the rest of the pool.
  await requestForm.getByLabel('Agent name', { exact: true }).fill('fast-one');
  await requestForm.getByLabel('Resource', { exact: true }).selectOption(`resource_${'a'.repeat(24)}`);
  assert.equal(await requestForm.getByLabel('Role', { exact: true }).inputValue(), 'coding');
  assert.equal(await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).isEnabled(), true);
  assert.equal(Object.keys(f.store.state.projects).length, projectCount);
  assert.equal(f.rooms.size, roomCount);
  await page.locator('#request-resources').getByRole('heading', { name: 'Medium coding resource', exact: true }).waitFor();
  assert.doesNotMatch(await page.locator('#request-resources').innerText(), /fast-one|fast-two|provider can define/);
  f.palpo.fetch = async (target, options) => new URL(target).pathname === '/api/fleet/v1/capabilities'
    ? new Response(JSON.stringify({ code: 'provider_unavailable' }), { status: 503 }) : f.fetch(target, options);
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click();
  await page.getByText(/Could not refresh roles from this HAFleet/).waitFor();
  assert.match(await page.locator('#request-role-hint').innerText(), /last successful check/);
  assert.equal(await requestForm.locator('select[name=role]').inputValue(), 'coding');
  assert.equal(await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).isEnabled(), false);
  f.palpo.fetch = f.fetch;
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click();
  await page.locator('#request-role-hint').waitFor({ state: 'hidden' });
  await requestForm.getByLabel('Request ID', { exact: true }).fill('ui-request-1');
  await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).click();
  await page.locator('#requests').getByText('Awaiting the HAFleet owner’s resource decision.', { exact: true }).waitFor();
  f.fulfill(actualFleetId, 'ui-request-1');
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click();
  await page.getByRole('link', { name: 'Open project and use agent', exact: true }).waitFor();
  await page.locator('#requests').getByText('Configuration: codex · fixture-model · high · strong', { exact: true }).waitFor();
  await requestForm.getByLabel('Agent name', { exact: true }).fill('fast-two');
  await requestForm.getByLabel('Request ID', { exact: true }).fill('ui-request-2');
  await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).click();
  await page.locator('#requests article[data-request-id="ui-request-2"]').getByText('Awaiting the HAFleet owner’s resource decision.', { exact: true }).waitFor();
  const definitions = Object.values(f.store.state.requests).map(r => r.payload.agentDefinition);
  assert.deepEqual(definitions, ['fast-one', 'fast-two'].map(name => ({ name, resourceId: `resource_${'a'.repeat(24)}` })));
  assert.equal(f.requests.size, 2);
  // Refresh removes a withdrawn resource and prevents another submission while
  // preserving the user's name/request ID for deliberate recovery.
  await requestForm.getByLabel('Agent name', { exact: true }).fill('retained-third');
  const retainedRequestId = await requestForm.getByLabel('Request ID', { exact: true }).inputValue();
  f.publishedOffers.set(actualFleetId, poolOffers.map(offer => ({ ...offer, resources: offer.resources.filter(r => r.id !== medium.id) })));
  // The timer must observe withdrawal without a manual refresh.
  await pool.getByRole('heading', { name: 'HAFleet resource pool · 2', exact: true }).waitFor({ timeout: 20000 });
  assert.equal(await requestForm.getByLabel('Resource', { exact: true }).inputValue(), '');
  assert.equal(await requestForm.getByLabel('Role', { exact: true }).isDisabled(), true);
  assert.equal(await requestForm.getByRole('button', { name: 'Send agent request', exact: true }).isEnabled(), false);
  assert.equal(await requestForm.getByLabel('Agent name', { exact: true }).inputValue(), 'retained-third');
  assert.equal(await requestForm.getByLabel('Request ID', { exact: true }).inputValue(), retainedRequestId);
  f.publishedOffers.set(actualFleetId, poolOffers);
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await pool.getByRole('heading', { name: 'HAFleet resource pool · 3', exact: true }).waitFor();
  assert.equal(await requestForm.getByLabel('Agent name', { exact: true }).inputValue(), 'retained-third');
  assert.equal(await requestForm.getByLabel('Request ID', { exact: true }).inputValue(), retainedRequestId);
  assert.equal(f.requests.size, 2); // Refresh never submits a draft.
  const projectRecord = Object.values(f.store.state.projects)[0];
  for (const event of f.events.values()) assert.ok(!JSON.stringify(event).includes(projectRecord.ownerDmRoomId));
  await page.screenshot({ path: resolve(evidence, 'project-request-ready.png'), fullPage: true });
  assert.deepEqual(pageErrors, []);
  for (const registration of f.registrations.values()) {
    assert.ok(!apiResponses.join('\n').includes(registration.as_token));
    assert.ok(!apiResponses.join('\n').includes(registration.hs_token));
  }
  assert.ok(!apiResponses.join('\n').includes('admin-secret'));
  const summary = { result: 'passed', flows: ['administrator sign-in', 'AS401 preserves user session and durable install retry', 'authorize/install fleet', 'verify actual representative identity', 'create Matrix agent', 'edit profile', 'retire identity', 'pause/resume fleet', 'fleet owner Matrix sign-in', 'actual-event receipt and reception workflow', 'separate project owner sign-in', 'project and encrypted private approval room creation', 'manual-pending role request', 'fixture-approved agent admission tracking'], realPalpo: false, pageErrors, credentialLeak: false };
  await writeFile(resolve(evidence, 'browser-summary.json'), JSON.stringify(summary, null, 2));
  console.log(JSON.stringify(summary));
} finally {
  await context.close(); await browser.close();
  await new Promise(resolve => server.close(resolve)); f.store.close();
}
