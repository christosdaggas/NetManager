import { $, icon, esc, btn, empty, profileMark, modal, notify } from './ui.js';

export function libraryList(library, query = '', compact = false) {
  if (library.locked)
    return `${empty('lock', 'Profile library locked', 'Unlock your encrypted profiles for this session.')}<div class="empty-action">${btn('unlock', 'Unlock library', 'lock')}</div>`;
  const profiles = library.profiles.filter((p) =>
    `${p.name} ${p.description}`.toLowerCase().includes(query.toLowerCase()),
  );
  if (!profiles.length)
    return empty(
      'layers',
      query ? 'No matching profiles' : 'No saved profiles yet',
      query ? 'Try another name or description.' : 'Save a configuration for home, work, or travel.',
    );
  const latest = library.profiles
    .filter((p) => p.applied_at)
    .sort((a, b) => Date.parse(b.applied_at) - Date.parse(a.applied_at))[0];
  const visible = compact
    ? [...profiles].sort((a, b) => Number(b === latest) - Number(a === latest)).slice(0, 3)
    : profiles;
  return `<ul class="profile-list">${visible.map((p) => `<li><button data-profile="${esc(p.id)}">${profileMark()}<span class="profile-text"><strong>${esc(p.name)}</strong><small>${esc(p.description || `${p.actions.length} configured action${p.actions.length === 1 ? '' : 's'}`)}</small></span>${p === latest ? '<span class="latest">Last applied</span>' : ''}${icon('chevron')}</button></li>`).join('')}</ul>`;
}

export function detail(p) {
  modal(
    p.name,
    `<p>${esc(p.description || 'Saved network configuration.')}</p><div class="facts">
    <div><small>Last applied</small><strong>${p.applied_at ? esc(new Date(p.applied_at).toLocaleString()) : 'Never'}</strong></div>
    <div><small>Configuration</small><strong>${p.actions.length} actions</strong></div></div>
    <h3>Profile actions</h3>${p.actions.length ? `<ol class="action-list">${p.actions.map((a) => `<li>${esc(a)}</li>`).join('')}</ol>` : '<p>This profile records a location without changing network settings.</p>'}
    <div class="dialog-foot">${btn('remove', 'Delete', null, 'danger')}${btn('edit', 'Edit profile')}${btn('apply', 'Apply profile', 'arrow', 'primary')}</div>`,
  );
}

export function editor(p, adapters, primary) {
  const c = p
    ? p.connection
    : {
        interface: primary || adapters[0]?.name || '',
        method: 'unchanged',
        address: '',
        gateway: '',
        dns: '',
      };
  const options = [...new Set([c?.interface, ...adapters.map((a) => a.name)].filter(Boolean))];
  modal(
    p ? 'Edit profile' : 'Create a profile',
    `<p>Keep the connection settings for a place together.</p><form id="editor">
    <label for="p-name">Profile name</label><input id="p-name" name="name" maxlength="100" required autofocus placeholder="e.g. Studio network" value="${esc(p?.name)}">
    <label for="p-description">Description <small>optional</small></label><input name="description" id="p-description" maxlength="500" value="${esc(p?.description)}" placeholder="What is this configuration for?">
    ${
      c
        ? `<fieldset><legend>Network settings</legend><div class="form-grid"><div><label for="p-interface">Interface</label><select id="p-interface" name="interface"><option value="">Select interface</option>${options.map((x) => `<option ${x === c.interface ? 'selected' : ''} value="${esc(x)}">${esc(x)}</option>`).join('')}</select></div>
    <div><label for="p-method">IPv4 assignment</label><select name="method" id="p-method">${[
      ['unchanged', 'Leave unchanged'],
      ['auto', 'Automatic (DHCP)'],
      ['manual', 'Static address'],
    ]
      .map(([v, t]) => `<option value="${v}" ${c.method === v ? 'selected' : ''}>${t}</option>`)
      .join('')}</select></div></div>
    <div id="manual" class="form-grid"><div><label for="p-addr">IPv4 address / prefix</label><input name="address" id="p-addr" value="${esc(c.address)}" placeholder="192.168.1.20/24"></div>
    <div><label for="p-gw">Gateway <small>optional</small></label><input name="gateway" id="p-gw" value="${esc(c.gateway)}" placeholder="192.168.1.1"></div></div>
    <label for="p-dns">DNS servers <small>optional</small></label><input id="p-dns" name="dns" value="${esc(c.dns)}" placeholder="1.1.1.1, 9.9.9.9"></fieldset>`
        : '<p class="info">This profile has advanced network actions. You can rename it here; use the classic editor to change those actions.</p>'
    }
    <div class="dialog-foot">${btn('close', 'Cancel')}<button class="primary" type="submit">Save profile</button></div></form>`,
  );
  if (c) {
    const update = () => {
      const manual = $('p-method').value === 'manual';
      $('manual').hidden = !manual;
      $('p-addr').required = manual;
    };
    $('p-method').addEventListener('change', update);
    update();
  }
}

export function confirmAction(type, p) {
  if (type === 'remove')
    modal(
      'Delete profile?',
      `<p><strong>${esc(p.name)}</strong> will be removed from this computer. Current network settings will stay as they are.</p><div class="dialog-foot">${btn('close', 'Cancel')}${btn('confirm-remove', 'Delete profile', null, 'destructive')}</div>`,
    );
  else
    modal(
      'Apply profile?',
      `<p>Apply <strong>${esc(p.name)}</strong> to this computer.</p>${p.actions.length ? `<ol class="action-list">${p.actions.map((a) => `<li>${esc(a)}</li>`).join('')}</ol>` : '<p>No network settings will be changed.</p>'}
    ${p.automation ? '<label class="check"><input id="allow-auto" type="checkbox">I reviewed these automation actions and want to run them.</label>' : ''}
    <div class="dialog-foot">${btn('close', 'Cancel')}<button class="primary" data-do="confirm-apply" ${p.automation ? 'disabled' : ''}>Apply profile</button></div>`,
    );
  $('allow-auto')?.addEventListener(
    'change',
    (e) => (document.querySelector('[data-do="confirm-apply"]').disabled = !e.target.checked),
  );
}

export function unlock() {
  modal(
    'Unlock your library',
    `<p>Your passphrase stays in memory for this session.</p><form id="unlock-form">
    <label for="passphrase">Passphrase</label><input id="passphrase" type="password" autocomplete="current-password" required autofocus>
    <div class="dialog-foot">${btn('close', 'Cancel')}<button class="primary">Unlock profiles</button></div></form>`,
  );
}

export async function submitProfile(e, p, invoke, done) {
  if (e.target.id !== 'editor' && e.target.id !== 'unlock-form') return;
  e.preventDefault();
  const f = new FormData(e.target);
  if (e.target.id === 'unlock-form') {
    await invoke('unlock_profiles', { passphrase: $('passphrase').value });
    notify('Profile library unlocked.');
  } else {
    const connection = f.has('method')
      ? Object.fromEntries(['interface', 'method', 'address', 'gateway', 'dns'].map((k) => [k, f.get(k)]))
      : null;
    await invoke('save_profile', {
      draft: {
        id: p?.id,
        revision: p?.revision,
        name: f.get('name'),
        description: f.get('description'),
        connection,
      },
    });
    notify('Profile saved.');
  }
  await done();
}
