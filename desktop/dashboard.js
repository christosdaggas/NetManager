import { btn, esc, icon, pill, tool, typeName } from './ui.js';
import { tr } from './i18n.js';
import { chartCard } from './traffic.js';
import { latest } from './profiles.js';

const label = (key) => esc(tr(key));

export function dashboard(net, lib) {
  const adapter = net.adapters.find((item) => item.name === net.primary);
  const connection = net.connection && net.connection !== '--' ? net.connection : net.primary;
  const active = latest(lib.profiles);
  const facts = [
    ['monitor', 'IP Address', net.address],
    ['route', 'Gateway', net.gateway],
    ['server', 'DNS Servers', net.dns.join(', ')],
  ];
  return `<section class="connection-panel" aria-labelledby="connection-heading">
    <div class="connection-summary">
      <div class="eyebrow">${icon('globe')}${label('Current Network')}</div>
      <h2 id="connection-heading">${connection ? esc(connection) : label('Disconnected')}</h2>
      ${net.primary ? `<div class="connection-meta"><span>${icon(adapter?.adapter_type === 'wifi' ? 'wifi' : 'port')}${label(typeName(adapter?.adapter_type))}</span><code>${esc(net.primary)}</code></div>` : ''}
    </div>
    <dl id="network-facts">${facts.map(([glyph, key, v]) => `<div><dt>${icon(glyph)}${label(key)}</dt><dd>${esc(v || '—')}</dd></div>`).join('')}</dl>
  </section>
  <div class="monitor-grid">${chartCard()}
    <section class="card profile-status">
      <div class="card-head"><h2>${icon('layers')}${label('Active Profile')}</h2>${pill(active ? 'Active' : 'Inactive', !!active)}</div>
      <div class="profile-current"><strong>${active ? esc(active.name) : label(lib.locked ? 'Profiles locked' : 'No profile active')}</strong>
      <p>${active ? `${label('Applied:')} ${esc(new Date(active.applied_at).toLocaleString())}` : label('Never applied')}</p></div>
      ${btn('switch', 'Switch Profile', 'route', 'switch-action')}
      <div class="service-state">${icon('server')}<span>${label('Service Status')}</span><strong>${label(net.service == null ? 'Checking…' : net.service ? 'Running' : 'Not Running')}</strong></div>
      ${net.service === false ? btn('start-service', 'Start Service') : ''}
    </section>
  </div>
  <section class="interfaces"><div class="section-head"><h2>${label('Network Interfaces')}</h2>${tool('copy-network', 'Copy Network Info', 'copy')}</div>
    <div class="interface-list">${net.adapters.map((a) => `<div class="interface"><span class="emblem">${icon(a.adapter_type === 'wifi' ? 'wifi' : 'port')}</span><div><strong>${esc(a.name)}</strong><small>${label(typeName(a.adapter_type))}</small></div>${pill(a.is_connected ? 'Connected' : 'Disconnected', a.is_connected)}</div>`).join('') || `<p>${label('No network adapters detected.')}</p>`}</div>
  </section>`;
}
