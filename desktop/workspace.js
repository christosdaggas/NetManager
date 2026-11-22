import { $, icon, esc, btn, empty, pill, typeName, notify, modal } from './ui.js';
import { chartCard, sample, stop, togglePause } from './traffic.js';
import { libraryList, detail, editor, confirmAction, unlock, submitProfile } from './profiles.js';

const pages = {
  overview: ['Overview', 'grid', 'Overview', ''],
  profiles: [
    'Profiles',
    'layers',
    'Network profiles',
    'Save and switch between configurations for different places.',
  ],
  interfaces: [
    'Interfaces',
    'port',
    'Network interfaces',
    'Inspect the physical network adapters on this computer.',
  ],
  activity: [
    'Activity log',
    'log',
    'Activity log',
    'Review recent profile changes and application events.',
  ],
  settings: [
    'Preferences',
    'settings',
    'Workspace preferences',
    'Adjust the appearance and sampling interval of your workspace.',
  ],
};
const invoke = (name, args = {}) =>
  window.__TAURI__?.core?.invoke(name, args) ??
  Promise.reject('Open the Tauri desktop app to connect to local network data.');
let page = pages[localStorage.page] ? localStorage.page : 'overview';
let library = { profiles: [], locked: false, logs: [] },
  net = { adapters: [], dns: [] };
let current = null,
  busy = false,
  loading = true,
  query = '',
  level = 'ALL';
let interval = [1000, 2000, 5000].includes(+localStorage.interval) ? +localStorage.interval : 2000;
const scheme = window.matchMedia('(prefers-color-scheme: dark)');
function theme() {
  const pref = localStorage.theme || 'system';
  document.documentElement.dataset.theme = pref === 'system' ? (scheme.matches ? 'dark' : 'light') : pref;
}
theme();
scheme.addEventListener('change', theme);
const mainAdapter = () => net.adapters.find((a) => a.name === net.primary);
const host = () => net.host || 'This computer';
let updatedAt;
const narrowWindow = matchMedia('(max-width: 860px)');
function syncSidebar() {
  const compact = localStorage.sidebar ? localStorage.sidebar === 'compact' : narrowWindow.matches;
  document.documentElement.dataset.sidebar = compact ? 'compact' : 'expanded';
  const toggle = $('sidebar-toggle');
  const label = compact ? 'Expand sidebar' : 'Collapse sidebar';
  toggle.setAttribute('aria-expanded', String(!compact));
  toggle.setAttribute('aria-label', label);
  toggle.title = `${label} (Ctrl B)`;
  toggle.firstElementChild.innerHTML = icon(compact ? 'expand' : 'collapse');
  toggle.lastElementChild.textContent = label;
}
narrowWindow.addEventListener('change', syncSidebar);
function drawNav() {
  $('nav').innerHTML = Object.entries(pages)
    .map(
      ([key, [name, glyph]]) =>
        `<button data-page="${key}" title="${name}" aria-label="${name}" ${key === page ? 'aria-current="page"' : ''}>${icon(glyph)}<span class="side-label">${name}</span></button>`,
    )
    .join('');
}
function interfaces() {
  return `<section class="card"><div class="card-head"><div class="section-title">${icon('port')}<h2>Adapter inventory</h2></div><span class="tag">${net.adapters.length} TOTAL</span></div>
  ${!net.adapters.length ? empty('port', 'No physical adapters detected', 'Refresh after connecting a network adapter.') : `<div class="table-wrap" role="region" tabindex="0" aria-label="Adapter inventory"><table><thead><tr>${['Interface', 'Type', 'Link state', 'Link speed', 'MAC address'].map((c) => `<th scope="col">${c}</th>`).join('')}</tr></thead><tbody>${net.adapters.map((a) => `<tr><th scope="row">${esc(a.name)}</th><td>${typeName(a.adapter_type)}</td><td>${pill(a.is_connected ? 'Up' : 'Down', a.is_connected)}</td><td>${a.speed_mbps ? `${a.speed_mbps} Mb/s` : '—'}</td><td class="mono">${esc(a.mac_address || '—')}</td></tr>`).join('')}</tbody></table></div>`}</section>`;
}
function overview() {
  const a = mainAdapter();
  const connected = !!net.gateway || !!a?.is_connected;
  const title = loading
    ? 'Reading connection…'
    : a
      ? typeName(a.adapter_type)
      : net.primary
        ? 'Network link'
        : 'Disconnected';
  const facts = [
    ['IPv4 address', net.address],
    ['Gateway', net.gateway],
    ['DNS resolver', net.dns.join(' · ')],
    ['Link speed', a?.speed_mbps ? `${a.speed_mbps.toLocaleString()} Mb/s` : null],
  ];
  return `<div class="dashboard">
    ${chartCard()}
    <section class="card connection"><div class="card-head"><h2>Connection</h2>${pill(loading ? 'Reading' : connected ? 'Link up' : 'Link down', connected)}</div>
      <div class="link-identity"><span class="device-icon">${icon(a?.adapter_type === 'wifi' ? 'wifi' : 'port')}</span><div><strong id="link-type">${title}</strong><small id="link-device">${esc(net.primary || 'No active interface')}</small></div></div>
      <dl class="link-facts">${facts.map(([t, v]) => `<div><dt>${t}</dt><dd>${esc(v || '—')}</dd></div>`).join('')}</dl>
    </section>
    <section class="card saved"><div class="card-head"><h2>Saved profiles <span class="count">${library.locked ? icon('lock') : library.profiles.length}</span></h2><button class="jump" data-page="profiles">All profiles ${icon('arrow')}</button></div>
      ${libraryList(library, '', true)}</section>
    <section class="card adapters"><div class="card-head"><h2>Interfaces <span class="count">${net.adapters.length}</span></h2><button class="jump" data-page="interfaces">Inspect ${icon('arrow')}</button></div>
    ${
      net.adapters.length
        ? `<ul class="adapter-list">${[...net.adapters]
            .sort((x, y) => Number(y.is_connected) - Number(x.is_connected))
            .slice(0, 3)
            .map(
              (a) =>
                `<li><span class="adapter-icon">${icon(a.adapter_type === 'wifi' ? 'wifi' : 'port')}</span><div><strong>${esc(a.name)}</strong><small>${typeName(a.adapter_type)}</small></div>${pill(a.is_connected ? 'Up' : 'Down', a.is_connected)}</li>`,
            )
            .join('')}</ul>`
        : empty(
            'port',
            loading ? 'Reading adapters…' : 'No physical adapters detected',
            'Connect an adapter, then refresh.',
          )
    }</section></div>`;
}
function logs() {
  const logs = library.logs.filter(
    (l) =>
      (level === 'ALL' || l.level === level) &&
      `${l.message} ${l.timestamp}`.toLowerCase().includes(query.toLowerCase()),
  );
  return logs.length
    ? `<ol class="logs">${logs.map((l) => `<li><time>${esc(l.timestamp)}</time><span class="tag">${esc(l.level)}</span><span>${esc(l.message)}</span></li>`).join('')}</ol>`
    : empty('log', 'No matching events', 'Profile operations and classic application events appear here.');
}
function preferences() {
  return `<section class="card prefs"><div class="card-head"><div class="section-title">${icon('settings')}<h2>Presentation</h2></div></div>
    <div><label for="theme">Color scheme<small>Follow your desktop or choose a fixed appearance.</small></label><select id="theme">${['system', 'light', 'dark'].map((t) => `<option ${t === (localStorage.theme || 'system') ? 'selected' : ''} value="${t}">${t[0].toUpperCase() + t.slice(1)}</option>`).join('')}</select></div>
    <div><label for="interval">Traffic sampling<small>Read local interface counters while Overview is visible.</small></label><select id="interval">${[1000, 2000, 5000].map((v) => `<option value="${v}" ${interval === v ? 'selected' : ''}>Every ${v / 1000} second${v === 1000 ? '' : 's'}</option>`).join('')}</select></div></section>
    <section class="card prefs"><div class="card-head"><div class="section-title">${icon('book')}<h2>Classic tools</h2></div></div><div><div><strong>Advanced configuration</strong><small>Manage automatic switching, schedules, watchdog, scanners, and encryption settings in the GTK app.</small></div>${btn('classic', 'Open classic app', 'ext')}</div></section>`;
}
function render() {
  stop();
  drawNav();
  const [, , title, sub] = pages[page];
  $('page-title').textContent = title;
  if (page === 'overview') {
    $('subtitle').innerHTML =
      `${icon('monitor')}<span id="host">${esc(host())}</span><span class="meta-divider" aria-hidden="true"></span><span>${updatedAt ? `Updated ${esc(updatedAt)}` : 'Reading network details…'}</span>`;
  } else $('subtitle').textContent = sub;
  document.title = `${title} · Network Manager`;
  $('toolbar').innerHTML =
    page === 'settings'
      ? ''
      : btn('refresh', 'Refresh', 'refresh') +
        (['overview', 'profiles'].includes(page)
          ? btn(
              library.locked ? 'unlock' : 'new',
              library.locked ? 'Unlock profiles' : 'New profile',
              library.locked ? 'lock' : 'plus',
              'primary',
            )
          : '');
  if (page === 'overview') $('view').innerHTML = overview();
  if (page === 'interfaces') $('view').innerHTML = interfaces();
  if (page === 'settings') $('view').innerHTML = preferences();
  if (page === 'profiles' || page === 'activity') {
    const isProfile = page === 'profiles';
    $('view').innerHTML =
      `<section class="card"><div class="card-head"><div class="section-title">${icon(isProfile ? 'layers' : 'clock')}<h2>${isProfile ? 'Saved configurations' : 'Recorded events'}</h2></div><span class="tag">${isProfile ? library.profiles.length : library.logs.length} ${isProfile ? 'PROFILES' : 'RECENT'}</span></div>
      <div class="filters"><label class="search">${icon('search')}<span class="sr">Search ${isProfile ? 'profiles' : 'events'}</span><input type="search" id="query" value="${esc(query)}" placeholder="Search ${isProfile ? 'profiles' : 'events'}…"></label>
      ${isProfile ? '' : `<label>Level <select id="level">${['ALL', ...new Set(library.logs.map((l) => l.level))].map((l) => `<option ${level === l ? 'selected' : ''}>${esc(l)}</option>`).join('')}</select></label>`}</div><div id="results"></div></section>`;
    filter();
  }
  if (page === 'overview' && !loading) sample(invoke, net.primary, interval);
}
function filter() {
  $('results').innerHTML = page === 'profiles' ? libraryList(library, query) : logs();
}
function navigate(next) {
  if (!pages[next]) return;
  page = localStorage.page = next;
  query = '';
  level = 'ALL';
  render();
  document.querySelector('main').scrollTop = 0;
  $('page-title').focus({ preventScroll: true });
}
let request = 0;
async function refresh() {
  const id = ++request;
  const [network, data] = await Promise.allSettled([invoke('network_snapshot'), invoke('library_view')]);
  if (id !== request) return;
  const errors = [];
  if (network.status === 'fulfilled') {
    net = network.value;
    updatedAt = new Date().toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  } else errors.push(`Network: ${network.reason}`);
  if (data.status === 'fulfilled') library = data.value;
  else {
    library = { profiles: [], locked: false, logs: [] };
    errors.push(`Library: ${data.reason}`);
  }
  loading = false;
  $('error').textContent = errors.join('\n');
  $('error').hidden = !errors.length;
  const focused = document.activeElement?.closest('button');
  const key = focused?.dataset.do,
    nav = focused?.dataset.page;
  render();
  if (!$('dialog').open && (key || nav))
    [...document.querySelectorAll('button')]
      .find((b) => (key ? b.dataset.do === key : b.dataset.page === nav))
      ?.focus({ preventScroll: true });
}
function commands() {
  modal(
    'Workspace commands',
    `<div class="command-list">${Object.entries(pages)
      .map(
        ([key, [name, glyph]], i) =>
          `<button data-page="${key}">${icon(glyph)}${esc(name)}<kbd>Ctrl ${i + 1}</kbd></button>`,
      )
      .join('')}
    ${[
      ['new', 'plus', 'New profile', 'Ctrl N'],
      ['sidebar-toggle', 'panel', 'Toggle sidebar', 'Ctrl B'],
      ['refresh', 'refresh', 'Refresh workspace', 'F5'],
      ['classic', 'ext', 'Classic tools', '↗'],
    ]
      .map(
        ([action, i, text, k]) => `<button data-do="${action}">${icon(i)}${text}<kbd>${k}</kbd></button>`,
      )
      .join('')}</div>`,
  );
}
function close() {
  $('dialog').close();
}
async function complete() {
  close();
  current = null;
  await refresh();
  if (!document.activeElement || document.activeElement === document.body)
    $('page-title').focus({ preventScroll: true });
}
async function action(id) {
  if (id === 'close') return close();
  if (id === 'commands') return commands();
  if (id === 'sidebar-toggle') {
    if ($('dialog').open) close();
    localStorage.sidebar = document.documentElement.dataset.sidebar === 'compact' ? 'expanded' : 'compact';
    syncSidebar();
    return;
  }
  if (id === 'pause-traffic') return togglePause();
  if (id === 'refresh') {
    if ($('dialog').open) close();
    await refresh();
    if ($('error').hidden) notify('Workspace refreshed.');
    return;
  }
  if (id === 'new' || id === 'edit') {
    if (library.locked) return unlock();
    if (id === 'new') current = null;
    return editor(current, net.adapters, net.primary);
  }
  if (id === 'unlock') return unlock();
  if (id === 'classic') {
    modal(
      'Classic tools',
      `<p>Open Network Manager’s GTK window for network and port scanning, scheduled switches, watchdog, and the full profile editor.</p><p>After making changes there, refresh this workspace to see them.</p><div class="dialog-foot">${btn('close', 'Cancel')}${btn('launch', 'Open classic app', 'ext', 'primary')}</div>`,
    );
    return;
  }
  if (id === 'launch') {
    await invoke('open_classic');
    close();
    return notify('Classic Network Manager opened.');
  }
  if (!current) return;
  if (id === 'remove' || id === 'apply') return confirmAction(id, current);
  const args = { id: current.id, revision: current.revision };
  if (id === 'confirm-remove') {
    await invoke('delete_profile', args);
    notify('Profile deleted.');
  } else if (id === 'confirm-apply')
    notify(await invoke('apply_profile', { ...args, allowAutomation: !!$('allow-auto')?.checked }));
  else return;
  await complete();
}
async function run(fn) {
  if (busy) return;
  busy = true;
  const dialog = $('dialog'),
    wasOpen = dialog.open;
  const button = document.activeElement?.closest('button');
  const progress =
    button &&
    ({ 'confirm-apply': 'Applying…', 'confirm-remove': 'Deleting…', launch: 'Opening…' }[
      button.dataset.do
    ] ||
      (button.type === 'submit' && button.form
        ? button.form.id === 'editor'
          ? 'Saving…'
          : 'Unlocking…'
        : ''));
  const original = button?.textContent;
  if (progress) {
    button.textContent = progress;
    button.disabled = true;
  }
  if ($('dialog-error')) $('dialog-error').hidden = true;
  try {
    await fn();
  } catch (e) {
    const el = dialog.open ? $('dialog-error') : $('error');
    el.hidden = false;
    el.textContent = String(e);
  } finally {
    busy = false;
    if (progress && button.isConnected) {
      button.textContent = original;
      button.disabled = false;
      button.focus();
    }
    if (wasOpen && !dialog.open) dialog.replaceChildren();
  }
}
document.addEventListener('click', (e) => {
  const b = e.target.closest('button');
  if (!b || busy) return;
  if (b.dataset.page) {
    if ($('dialog').open) close();
    return navigate(b.dataset.page);
  }
  if (b.dataset.profile) {
    current = library.profiles.find((p) => p.id === b.dataset.profile);
    if (current) detail(current);
    return;
  }
  run(() => action(b.dataset.do || b.id));
});
document.addEventListener('submit', (e) => {
  e.preventDefault();
  run(() => submitProfile(e, current, invoke, complete));
});
$('dialog').addEventListener('cancel', (e) => {
  if (busy) e.preventDefault();
});
$('dialog').addEventListener('close', () => {
  if (!$('dialog').open) $('dialog').replaceChildren();
});
document.addEventListener('input', (e) => {
  if (e.target.id === 'query') {
    query = e.target.value;
    filter();
  }
});
document.addEventListener('change', (e) => {
  if (e.target.id === 'level') {
    level = e.target.value;
    filter();
  }
  if (e.target.id === 'theme') {
    localStorage.theme = e.target.value;
    theme();
  }
  if (e.target.id === 'interval') interval = +(localStorage.interval = e.target.value);
});
document.addEventListener('keydown', (e) => {
  if (busy || $('dialog').open || e.altKey || e.shiftKey) return;
  const k = e.key.toLowerCase(),
    mod = e.ctrlKey || e.metaKey;
  if (mod && /^[1-5]$/.test(k)) {
    e.preventDefault();
    navigate(Object.keys(pages)[+k - 1]);
    return;
  }
  if (!(mod && ['k', 'n', 'r', 'f', 'b'].includes(k)) && k !== 'f5') return;
  if (k === 'f' && !$('query')) return;
  e.preventDefault();
  if (k === 'f') return $('query').focus();
  run(() => action(k === 'k' ? 'commands' : k === 'n' ? 'new' : k === 'b' ? 'sidebar-toggle' : 'refresh'));
});
document.addEventListener('visibilitychange', () => {
  if (document.hidden) stop();
  else refresh();
});
document.querySelectorAll('[data-icon]').forEach((el) => (el.innerHTML = icon(el.dataset.icon)));
syncSidebar();
render();
refresh();
