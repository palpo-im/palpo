(() => {
  const find = id => document.getElementById(id), storageKey = 'palpo.accountRequest.v1';
  let receipt, request, timer, refreshing = false;
  const random = bytes => [...crypto.getRandomValues(new Uint8Array(bytes))].map(v => v.toString(16).padStart(2, '0')).join('');
  try { receipt = JSON.parse(localStorage.getItem(storageKey)); } catch { receipt = null; }
  if (!/^[a-f0-9]{32}$/.test(receipt?.id ?? '') || !/^[a-f0-9]{64}$/.test(receipt?.receipt ?? '')) receipt = null;
  const status = find('account-request-status'), form = find('account-request-form');
  function message(text) { status.hidden = false; status.textContent = text; }
  async function call(path, body) {
    const response = await fetch('/api/' + path, { method: body ? 'POST' : 'GET', headers: { 'Content-Type': 'application/json' }, ...(body ? { body: JSON.stringify(body) } : {}), signal: AbortSignal.timeout(15000) });
    const result = await response.json();
    if (!response.ok) { const error = new Error(result.error ?? 'The request could not be completed.'); error.code = result.code; throw error; }
    return result;
  }
  function showRequest(value) {
    request = value; form.hidden = true;
    const messages = {
      notification_pending: 'Waiting for administrator approval. Your request is saved; delivery to the administrator room is pending.',
      pending: 'Waiting for administrator approval. Your request has been delivered to the private administrator room.',
      approved: 'Approved. Palpo is creating your account.', registering: 'Approved. Palpo is confirming account registration.',
      registered: 'Your account is ready. Sign in with the password you chose. You can then create a project and request Agents.',
      rejected: 'Your account request was rejected. No account was created.',
      expired: 'Your account request expired before approval. Submit a new request to try again.',
      name_unavailable: 'This username is already taken. Submit a new request with a different username.',
    };
    message(`${messages[value.status] ?? 'Checking request status.'}\n${value.userId}`);
    const terminal = ['registered', 'rejected', 'expired', 'name_unavailable'].includes(value.status);
    find('refresh-account-request').hidden = terminal; find('new-account-request').hidden = !terminal;
    find('account-back-login').textContent = value.status === 'registered' ? 'Sign in to your new account' : 'Back to sign in';
    if (value.status === 'registered') find('login-form').elements.username.value = value.userId;
    clearTimeout(timer);
    if (!terminal && !find('account-request-panel').hidden && !document.hidden) timer = setTimeout(refresh, 4000);
  }
  async function refresh() {
    if (!receipt || refreshing) return;
    refreshing = true; clearTimeout(timer);
    try { const result = await call('account-requests/status', receipt); showRequest(result.request); }
    catch (error) {
      message(`${error.message} Your request receipt has been kept. Refresh status to retry.`); find('refresh-account-request').hidden = false;
      if (error.code === 'account_request_missing') form.hidden = false;
    }
    finally { refreshing = false; }
  }
  find('open-account-request').onclick = () => {
    find('login-panel').hidden = true; find('account-request-panel').hidden = false;
    if (receipt) { form.hidden = true; void refresh(); } else { form.hidden = false; status.hidden = true; }
  };
  find('account-back-login').onclick = () => {
    clearTimeout(timer); find('account-request-panel').hidden = true; find('login-panel').hidden = false;
    form.elements.password.value = ''; form.elements.confirmPassword.value = '';
  };
  find('refresh-account-request').onclick = refresh;
  find('new-account-request').onclick = () => {
    receipt = null; request = null; localStorage.removeItem(storageKey); form.reset(); form.hidden = false;
    status.hidden = true; find('new-account-request').hidden = true; find('refresh-account-request').hidden = true;
  };
  form.onsubmit = async event => {
    event.preventDefault(); const button = event.submitter, fields = Object.fromEntries(new FormData(form));
    if (fields.password !== fields.confirmPassword) { message('The passwords do not match.'); return; }
    delete fields.confirmPassword;
    receipt ??= { id: random(16), receipt: random(32) };
    // Save only the status capability, never the password or signup details.
    try { localStorage.setItem(storageKey, JSON.stringify(receipt)); }
    catch { message('This browser cannot save your request receipt. Enable local storage before submitting.'); return; }
    button.disabled = true;
    try { const result = await call('account-requests', { ...receipt, ...fields }); showRequest(result.request); form.elements.password.value = ''; form.elements.confirmPassword.value = ''; }
    catch (error) { message(error.message + ' Your request ID has been kept for retry.'); }
    finally { button.disabled = false; }
  };
  document.addEventListener('visibilitychange', () => { if (!document.hidden && !find('account-request-panel').hidden && receipt) void refresh(); else clearTimeout(timer); });
  void call('account-access').then(config => {
    find('open-account-request').hidden = !config.enabled;
    find('account-server-name').textContent = `Your Matrix ID will be @username:${config.serverName}`;
  }).catch(() => {});
})();
