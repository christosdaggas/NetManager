import { $, btn, esc, icon, tool, modal, empty, pill, options, foot, invoke } from './ui.js';

import { tr } from './i18n.js';
const templates = [
  'Blank (Empty)',
  'Home Network',
  'Office Network',
  'Public WiFi',
  'VPN Only',
  'Development',
];
const arr = (p, k) => p[k] || [];
const field = (key, label, value = '', placeholder = '') =>
  `<label for="${key}">${esc(tr(label))}</label><input id="${key}" value="${esc(value)}" placeholder="${esc(placeholder)}">`;
const select = (key, label, pairs, value) =>
  `<label for="${key}">${esc(tr(label))}</label><select id="${key}">${options(pairs, value)}</select>`;
const section = (title, html, open = false) =>
  `<details class="editor-section" ${open ? 'open' : ''}><summary>${esc(title)}</summary><div>${html}</div></details>`;
export const latest = (list) =>
  list.filter((p) => p.applied_at).sort((a, b) => Date.parse(b.applied_at) - Date.parse(a.applied_at))[0];
export function profileList(lib, query = '') {
  if (lib.locked)
    return (
      empty('lock', 'Profiles are locked', 'Enter your passphrase to view this library.') +
      btn('unlock', 'Unlock Profiles', 'lock', 'primary')
    );
  const list = lib.profiles.filter((p) =>
    [p.name, p.group, p.description].some((v) => v?.toLowerCase().includes(query.toLowerCase())),
  );
  if (!list.length)
    return (
      empty(
        'users',
        query ? 'No Matching Profiles' : 'No Profiles Yet',
        query ? 'Try another name or group.' : 'Create your first network profile to get started',
      ) +
      (!query ? `<div class="empty-action">${btn('new', 'Create Profile', 'plus', 'primary')}</div>` : '')
    );
  const active = latest(lib.profiles);
  return `<ul class="profile-rows">${list
    .map(
      (
        p,
      ) => `<li data-id="${esc(p.id)}"><button class="row-name" data-do="apply"><span>${esc(p.name)}</span><small>${esc(p.group || p.description || '')}</small></button>${pill(active === p ? 'Active' : 'Inactive', active === p)}
    ${tool('apply', 'Apply Profile', 'play', active === p ? 'disabled' : '')}${tool('edit', 'Edit Profile', 'edit')}${tool('duplicate', 'Duplicate Profile', 'copy')}${tool('remove', 'Delete Profile', 'trash')}</li>`,
    )
    .join('')}</ul>`;
}

let model, existing, dirty, devices;
function adapterRows(net) {
  const ns = arr(model, 'network_actions');
  devices = [
    ...(ns.some((a) => !a.interface && ['ipv4_config', 'dns_servers', 'wifi_connect'].includes(a.action))
      ? [null]
      : []),
    ...new Set([...net.adapters.map((a) => a.name), ...ns.map((a) => a.interface).filter(Boolean)]),
  ];
  return (
    devices
      .map((dev, i) => {
        const a = ns.filter((a) => (a.interface ?? null) === dev),
          ip = a.find((a) => a.action === 'ipv4_config'),
          dns = a.find((a) => a.action === 'dns_servers'),
          enabled = a.find((a) => a.action === 'interface_enable'),
          wifi = a.find((a) => a.action === 'wifi_connect');
        const prefix = `adapter-${i}`,
          state = enabled ? (enabled.enabled ? 'on' : 'off') : a.length ? 'on' : 'keep';
        return section(
          dev ?? 'Default interface',
          `<div data-change="${prefix}">${select(
            prefix + '-state',
            'Adapter state',
            [
              ['keep', 'Leave unchanged'],
              ['on', 'Enable Adapter'],
              ...(dev ? [['off', 'Disable Adapter']] : []),
            ],
            state,
          )}
      <div class="adapter-fields">${select(
        prefix + '-ip',
        'IPv4 Configuration',
        [
          ['auto', 'DHCP (Automatic)'],
          ['manual', 'Static (Manual)'],
          ['disabled', 'Disabled'],
          ['linklocal', 'Link-local only'],
        ],
        ip?.method || 'auto',
      )}
      <div class="static-fields">${field(prefix + '-address', 'IP Address', ip?.addresses?.[0]?.address)}${field(prefix + '-prefix', 'Subnet Mask or Prefix', ip?.addresses?.[0]?.prefix ?? 24)}${field(prefix + '-gateway', 'Gateway', ip?.gateway)}</div>
      ${select(
        prefix + '-dns',
        'DNS Configuration',
        [
          ['auto', 'Automatic (from DHCP)'],
          ['manual', 'Manual'],
        ],
        dns?.servers?.length ? 'manual' : 'auto',
      )}
      <div class="dns-fields">${field(prefix + '-dns1', 'Primary DNS', dns?.servers?.[0])}${field(prefix + '-dns2', 'Secondary DNS', dns?.servers?.slice(1).join(', '))}</div>
      ${net.adapters.find((a) => a.name === dev)?.adapter_type === 'wifi' || wifi ? field(prefix + '-ssid', 'WiFi Network (SSID)', wifi?.ssid) : ''}</div></div>`,
        );
      })
      .join('') || '<p>No network adapters detected.</p>'
  );
}
function syncFields() {
  devices.forEach((_, i) => {
    const k = `adapter-${i}`,
      g = document.querySelector(`[data-change="${k}"]`);
    g.querySelector('.adapter-fields').hidden = $(k + '-state').value !== 'on';
    g.querySelector('.static-fields').hidden = $(k + '-ip').value !== 'manual';
    g.querySelector('.dns-fields').hidden = $(k + '-dns').value !== 'manual';
  });
  $('proxy-manual').hidden = $('proxy-mode').value !== 'manual';
  $('proxy-auto').hidden = $('proxy-mode').value !== 'auto';
  for (const k of ['vpn', 'pre_script', 'post_script', 'run_program'])
    $(k + '-fields').hidden = !$(k + '-enabled').checked;
}
function populate(net, template) {
  const m = model.metadata,
    vpn = arr(model, 'network_actions').find((a) => a.action === 'vpn_connect');
  const proxy = arr(model, 'system_actions').find((a) => a.action === 'proxy_config') || {};
  const automation = arr(model, 'automation_actions');
  modal(
    existing ? 'Edit Profile' : 'Create Profile',
    `<form id="profile-editor">
    ${
      !existing
        ? select(
            'template',
            'Start From Template',
            templates.map((t, i) => [i, t]),
            template,
          )
        : ''
    }
    <fieldset><legend>Profile Details</legend>${field('p-name', 'Name', m.name)}${field('p-description', 'Description', m.description)}${field('p-group', 'Group (optional)', m.group?.name)}</fieldset>
    <h3>Network Adapters</h3>${adapterRows(net)}
    ${section('VPN Connection', `<div data-change="vpn"><label><input id="vpn-enabled" type="checkbox" ${vpn ? 'checked' : ''}> ${esc(tr('Connect to VPN'))}</label><div id="vpn-fields">${field('vpn-name', 'VPN Connection Name', vpn?.connection_name)}</div></div>`)}
    ${section(
      'Proxy Settings',
      `<div data-change="proxy">${select(
        'proxy-mode',
        'Proxy Mode',
        [
          ['keep', 'Leave unchanged'],
          ['none', 'None'],
          ['manual', 'Manual'],
          ['auto', 'Automatic (PAC)'],
        ],
        proxy.mode || 'keep',
      )}
      <div id="proxy-manual">${['http', 'https', 'ftp', 'socks'].map((k) => field(k + '-proxy', k.toUpperCase() + ' Proxy', proxy[k + '_proxy'])).join('')}${field('no-proxy', 'No Proxy For', proxy.no_proxy?.join(', '))}</div><div id="proxy-auto">${field('pac', 'PAC URL', proxy.pac_url)}</div></div>`,
    )}
    ${section(
      'Scripts and Programs',
      ['pre_script', 'post_script', 'run_program']
        .map((kind, i) => {
          const a = automation.find((a) => a.action === kind);
          return `<div data-change="${kind}"><label><input type="checkbox" id="${kind}-enabled" ${a ? 'checked' : ''}> ${esc(tr(['Run Pre-Script', 'Run Post-Script', 'Run Program'][i]))}</label><div id="${kind}-fields">${field(kind, ['Pre-Script Path', 'Post-Script Path', 'Program Path'][i], a?.path || a?.program)}${field(kind + '-args', i === 2 ? 'Program Arguments' : 'Arguments', a?.args?.join(' '))}</div></div>`;
        })
        .join(''),
    )}
    ${section('Auto-Switch Rules', `<div data-change="rules"><label for="rules">Rule set (JSON)</label><textarea id="rules" rows="5" placeholder='{"operator":"and","conditions":[{"type":"wifi_ssid","ssid":"Office"}]}'>${esc(model.auto_switch_rules ? JSON.stringify(model.auto_switch_rules, null, 2) : '')}</textarea></div>`)}
    ${foot(`<button class="primary">${existing ? 'Save' : 'Create'}</button>`)}</form>`,
  );
  $('p-name').required = true;
  $('p-name').maxLength = 100;
  $('p-name').focus();
  syncFields();
  $('profile-editor').addEventListener('input', (e) => {
    const scope = e.target.closest('[data-change]');
    if (scope) dirty.add(scope.dataset.change);
  });
  $('profile-editor').addEventListener('change', async (e) => {
    const scope = e.target.closest('[data-change]');
    if (scope) dirty.add(scope.dataset.change);
    if (e.target.id === 'template') {
      const t = Number(e.target.value),
        name = $('p-name').value;
      e.target.disabled = true;
      try {
        const next = await invoke('profile_form', { template: t });
        if (!$('profile-editor')) return;
        model = next;
        if (name) model.metadata.name = name;
        dirty = new Set();
        populate(net, t);
      } catch (err) {
        $('dialog-error').hidden = false;
        $('dialog-error').textContent = String(err);
        e.target.disabled = false;
      }
    } else syncFields();
  });
}
export async function editor(p, net) {
  model = await invoke('profile_form', { id: p?.id, template: 0 });
  existing = p;
  dirty = new Set();
  populate(net, 0);
}
function prefixLength(v) {
  if (/^\d+$/.test(v) && +v <= 32) return +v;
  const octets = v.split('.');
  if (octets.length === 4 && octets.every((o) => /^\d+$/.test(o) && +o <= 255)) {
    const bits = octets.map((o) => (+o).toString(2).padStart(8, '0')).join('');
    if (/^1*0*$/.test(bits)) return bits.indexOf('0') < 0 ? 32 : bits.indexOf('0');
  }
  throw Error('Enter a valid subnet mask or prefix (0–32).');
}
export async function saveEditor() {
  const draft = structuredClone(model),
    m = draft.metadata,
    val = (k) => $(k)?.value.trim() || '';
  m.name = val('p-name');
  m.description = val('p-description') || null;
  m.group = val('p-group') ? { ...m.group, name: val('p-group') } : null;
  let ns = arr(draft, 'network_actions'),
    sys = arr(draft, 'system_actions'),
    aut = arr(draft, 'automation_actions');
  devices.forEach((dev, i) => {
    const k = `adapter-${i}`;
    if (!dirty.has(k)) return;
    const extra =
      ns.find((a) => (a.interface ?? null) === dev && a.action === 'ipv4_config')?.addresses?.slice(1) ||
      [];
    ns = ns.filter(
      (a) =>
        (a.interface ?? null) !== dev ||
        !['ipv4_config', 'dns_servers', 'interface_enable', 'wifi_connect'].includes(a.action),
    );
    const state = val(k + '-state');
    if (state === 'keep') return;
    if (dev) ns.push({ action: 'interface_enable', interface: dev, enabled: state === 'on' });
    if (state === 'off') return;
    const method = val(k + '-ip');
    ns.push({
      action: 'ipv4_config',
      interface: dev,
      method,
      addresses:
        method === 'manual'
          ? [{ address: val(k + '-address'), prefix: prefixLength(val(k + '-prefix')) }, ...extra]
          : [],
      gateway: method === 'manual' ? val(k + '-gateway') || null : null,
    });
    const servers =
      val(k + '-dns') === 'manual'
        ? [val(k + '-dns1'), val(k + '-dns2')]
            .join(',')
            .split(/[,\s]+/)
            .filter(Boolean)
        : [];
    if (val(k + '-dns') === 'manual' && !servers.length) throw Error('Enter a DNS server.');
    ns.push({ action: 'dns_servers', interface: dev, servers });
    if (val(k + '-ssid')) ns.push({ action: 'wifi_connect', interface: dev, ssid: val(k + '-ssid') });
  });
  if (dirty.has('vpn')) {
    ns = ns.filter((a) => a.action !== 'vpn_connect');
    if ($('vpn-enabled').checked) {
      if (!val('vpn-name')) throw Error('Enter a VPN connection name.');
      ns.push({ action: 'vpn_connect', connection_name: val('vpn-name') });
    }
  }
  if (dirty.has('proxy')) {
    sys = sys.filter((a) => a.action !== 'proxy_config');
    const mode = val('proxy-mode');
    if (mode !== 'keep')
      sys.push({
        action: 'proxy_config',
        mode,
        pac_url: val('pac') || null,
        no_proxy: val('no-proxy')
          .split(',')
          .map((x) => x.trim())
          .filter(Boolean),
        ...Object.fromEntries(
          ['http', 'https', 'ftp', 'socks'].map((k) => [k + '_proxy', val(k + '-proxy') || null]),
        ),
      });
  }
  for (const kind of ['pre_script', 'post_script', 'run_program'])
    if (dirty.has(kind)) {
      const index = aut.findIndex((a) => a.action === kind),
        prior = aut[index] || { action: kind };
      if (index >= 0) aut.splice(index, 1);
      if ($(kind + '-enabled').checked) {
        if (!val(kind)) throw Error('Enter a script or program path.');
        const original = prior.args || [];
        const args =
          val(kind + '-args') === original.join(' ')
            ? original
            : val(kind + '-args')
                .split(/\s+/)
                .filter(Boolean);
        aut.splice(index < 0 ? aut.length : index, 0, {
          ...prior,
          [kind === 'run_program' ? 'program' : 'path']: val(kind),
          args,
        });
      }
    }
  Object.assign(draft, { network_actions: ns, system_actions: sys, automation_actions: aut });
  if (dirty.has('rules')) draft.auto_switch_rules = val('rules') ? JSON.parse(val('rules')) : null;
  await invoke('save_profile', { id: existing?.id, revision: existing?.revision, draft });
}
