import { copyText, $, btn, esc, icon, tool, modal, options, foot, invoke, notify, empty } from './ui.js';
import { sample, stop, togglePause } from './traffic.js';
import { profileList, editor, saveEditor } from './profiles.js';
import { dashboard } from './dashboard.js';
import {
  scannerPage,
  mountScanner,
  scannerDefaults,
  scanAction,
  scannerInput,
  deviceDetails,
  inspectDevice,
} from './scanners.js';
import {
  settingsPage,
  preferences,
  about,
  rules,
  manager,
  addEntry,
  changeEntry,
  saveEntry,
  encryptionDialog,
  saveEncryption,
  updateSetting,
} from './settings.js';
import { helpPage, shortcuts } from './help.js';
import { tr, loadCatalog, localize } from './i18n.js';

const pages = {
  dashboard: ['Dashboard', 'grid', 'Network overview', 'Manage your system connectivity'],
  profiles: ['Profiles', 'layers', 'Profiles', 'Manage saved network configurations'],
  scanner: ['Scanner', 'search', 'Network Scanner', 'Discover devices on your local network'],
  ports: ['Port Scanner', 'monitor', 'Port Scanner', 'Find the open TCP ports of a device'],
  logs: ['Logs', 'log', 'Application Logs', ''],
  settings: ['Settings', 'settings', '', ''],
  help: ['Help', 'info', 'Help', 'Learn how to use Network Manager'],
};
let page = Object.hasOwn(pages, localStorage.page) ? localStorage.page : 'dashboard';
let lib = { profiles: [], logs: [], locked: false },
  net = { adapters: [], dns: [] },
  query = '',
  level = 'ALL',
  selected,
  automatic = false,
  busy = false,
  managerKind,
  importData;
let interval = [1000, 2000, 5000].includes(+localStorage.interval) ? +localStorage.interval : 2000;
const menu = $('menu'),
  dialog = $('dialog'),
  scheme = matchMedia('(prefers-color-scheme: dark)'),
  narrow = matchMedia('(max-width:800px)');
const utility = window.__NM_UTILITY__;
const closeUtility = () => window.__TAURI__.window.getCurrentWindow().close();
const changed = (profilesPage = false) =>
  utility ? invoke('workspace_changed', { profilesPage }) : Promise.resolve();
const openWindow = (kind, data = null) => invoke('open_utility', { kind, data });
function applyTheme() {
  const t = lib.settings?.theme || localStorage.theme || 'system';
  document.documentElement.dataset.theme = t === 'system' ? (scheme.matches ? 'dark' : 'light') : t;
}
function sidebar() {
  const compact = localStorage.sidebar ? localStorage.sidebar === 'compact' : narrow.matches;
  document.documentElement.dataset.sidebar = compact ? 'compact' : 'expanded';
  const b = $('sidebar-toggle'),
    label = compact ? 'Expand sidebar' : 'Collapse sidebar';
  b.setAttribute('aria-expanded', String(!compact));
  b.setAttribute('aria-label', label);
  b.title = label + ' (Ctrl B)';
  b.innerHTML = icon(compact ? 'expand' : 'collapse');
}
function drawMenu() {
  const theme = lib.settings?.theme || 'system';
  menu.innerHTML = `<div class="swatches">${['system', 'light', 'dark'].map((v) => `<button data-do="theme-${v}" data-theme-choice="${v}" aria-pressed="${theme === v}" aria-label="${v} theme" title="${v} theme">${theme === v ? icon('check') : ''}</button>`).join('')}</div>
    ${[
      ['preferences', 'Preferences', 'settings'],
      ['about', 'About', 'help'],
      ['quit', 'Quit', 'close'],
    ]
      .map(([a, t, g]) => btn(a, t, g))
      .join('')}`;
}
function render() {
  if (utility) {
    applyTheme();
    return;
  }
  stop();
  const [label, , title, sub] = pages[page];
  applyTheme();
  drawMenu();
  const navGroup = (name, keys) =>
    `<div class="nav-group" role="group" aria-label="${esc(tr(name))}"><span class="nav-label" aria-hidden="true">${esc(tr(name))}</span>${keys
      .map((key) => {
        const [n, g] = pages[key];
        return `<button data-page="${key}" title="${esc(tr(n))}" aria-label="${esc(tr(n))}" ${key === page ? 'aria-current="page"' : ''}>${icon(g)}<span class="side-label">${esc(tr(n))}</span></button>`;
      })
      .join('')}</div>`;
  $('nav').innerHTML =
    navGroup('Workspace', ['dashboard', 'profiles', 'scanner', 'ports', 'logs']) +
    navGroup('Application', ['settings', 'help']);
  document.body.dataset.page = page;
  document.title = `${tr(label)} · ${tr('Network Manager')}`;
  $('heading').textContent = tr(label);
  $('version').textContent = tr('Version {version}').replace('{version}', lib.version || '1.0.0');
  $('page-title').textContent = tr(title || label);
  $('subtitle').textContent = tr(sub);
  $('toolbar').innerHTML =
    page === 'dashboard'
      ? btn('refresh', 'Refresh', 'refresh') + btn('new', 'New Profile', 'plus', 'primary')
      : ['scanner', 'ports'].includes(page)
        ? tool('scan-export', 'Export Results', 'up')
        : '';
  if (page === 'dashboard') {
    $('view').innerHTML = dashboard(net, lib);
    sample(invoke, '*', interval);
  } else if (page === 'profiles' || page === 'logs') {
    const p = page === 'profiles';
    $('view').innerHTML =
      `<div class="list-toolbar"><input id="query" type="search" aria-label="Search ${page}" value="${esc(query)}" placeholder="Search ${page}…">
    ${
      p
        ? btn('new', 'Create Profile', 'plus', 'primary') +
          tool('import', 'Import Profiles', 'down') +
          tool('export-profiles', 'Export Profiles', 'up')
        : `<select id="level" aria-label="Log level">${options(
            [
              ['ALL', 'All'],
              ['INFO', 'Info'],
              ['WARN', 'Warning'],
              ['ERROR', 'Error'],
            ],
            level,
          )}</select>${tool('clear', 'Clear Logs', 'trash')}${tool('export-logs', 'Export Logs', 'up')}`
    }</div><div id="results"></div>`;
    filter();
  } else if (page === 'settings') $('view').innerHTML = settingsPage(lib, interval);
  else if (page === 'help') $('view').innerHTML = helpPage();
  else {
    $('view').innerHTML = scannerPage(page);
    mountScanner(page);
  }
  localize($('view'));
}
function logLines() {
  return lib.logs
    .filter(
      (e) =>
        (level === 'ALL' || e.level === level || (level === 'WARN' && e.level === 'WARNING')) &&
        `${e.timestamp} ${e.message}`.toLowerCase().includes(query.toLowerCase()),
    )
    .map((e) => `[${e.timestamp}] [${e.level}] ${e.message}`)
    .join('\n');
}
function filter() {
  if ($('results'))
    $('results').innerHTML =
      page === 'profiles'
        ? profileList(lib, query)
        : `<pre class="log-text">${esc(logLines() || 'No matching log entries.')}</pre>`;
}
function navigate(next) {
  if (!Object.hasOwn(pages, next)) return;
  page = localStorage.page = next;
  query = '';
  level = 'ALL';
  render();
  document.querySelector('main').scrollTop = 0;
  ($('page-title').offsetHeight ? $('page-title') : $('nav').querySelector('[aria-current]')).focus({
    preventScroll: true,
  });
}
async function reload() {
  lib = await invoke('library_view');
}
async function refresh() {
  const [n, l] = await Promise.allSettled([invoke('network_snapshot'), invoke('library_view')]);
  const errs = [];
  if (n.status === 'fulfilled') net = n.value;
  else errs.push(n.reason);
  if (l.status === 'fulfilled') lib = l.value;
  else errs.push(l.reason);
  $('error').hidden = !errs.length;
  $('error').textContent = errs.join('\n');
  render();
}
async function done(message) {
  if (utility) {
    await changed(utility.kind === 'new' || utility.kind === 'edit');
    return closeUtility();
  }
  await reload();
  if (dialog.open) dialog.close();
  render();
  if (message) notify(message);
}
function unlock() {
  encryptionDialog('unlock');
}
function switcher() {
  if (lib.locked) return unlock();
  modal(
    'Switch Profile',
    lib.profiles.length
      ? `<div class="switcher">${lib.profiles.map((p) => `<button data-id="${esc(p.id)}" data-do="apply">${icon('users')}${esc(p.name)}${icon('chevron')}</button>`).join('')}</div>`
      : empty('users', 'No Profiles Yet', 'Create a profile to switch network settings.') +
          btn('new', 'Create Profile', 'plus', 'primary'),
  );
}
async function apply(p, auto = false, confirmed = false) {
  selected = p;
  automatic = auto;
  const hasAutomation = p.automation && (!auto || lib.settings.auto_switch_allow_scripts);
  if (!confirmed && (hasAutomation || (p.actions.length && lib.settings.confirm_profile_switch))) {
    modal(
      `Apply ‘${p.name}’?`,
      `<p>This will apply ${p.actions.length} actions to your system.</p><ol class="action-list">${p.actions.map((a) => `<li>${esc(a)}</li>`).join('')}</ol>
    ${hasAutomation ? '<label><input type="checkbox" id="allow-auto"> I reviewed these executable actions and want to run them.</label>' : ''}<div class="dialog-foot">${btn('close', 'Cancel')}<button class="primary" data-do="confirm-apply" ${hasAutomation ? 'disabled' : ''}>Apply</button></div>`,
    );
    return;
  }
  const message = await invoke('apply_profile', {
    id: p.id,
    revision: p.revision,
    allowAutomation: !!$('allow-auto')?.checked,
    automatic: auto,
  });
  await done(message);
}
function diagnostics() {
  modal(
    'Network Diagnostics',
    `<form id="diag-form">${[
      ['ping', 'Connectivity Test', 'Host to ping', '8.8.8.8', 'Run Ping Test'],
      ['dns', 'DNS Lookup', 'Domain to lookup', 'example.com', 'Lookup DNS'],
      ['speed', 'Speed Test', '', '', 'Run Speed Test'],
    ]
      .map(
        ([key, title, label, v, b]) =>
          `<fieldset><legend>${title}</legend>${label ? `<label for="diag-${key}">${label}</label><input id="diag-${key}" value="${v}">` : '<p>Measure download speed using Cloudflare.</p>'}${btn('diag-' + key, b)}<pre id="out-${key}"></pre></fieldset>`,
      )
      .join(
        '',
      )}<fieldset><legend>Quick Test Results</legend><p id="quick-tests">Checking connectivity and DNS…</p></fieldset></form>`,
  );
  const el = $('quick-tests');
  Promise.allSettled([
    invoke('diagnostic', { kind: 'ping', target: '8.8.8.8' }),
    invoke('diagnostic', { kind: 'dns', target: 'example.com' }),
  ]).then(
    (r) =>
      (el.textContent = `Internet: ${r[0].status === 'fulfilled' ? 'Connected' : 'No response'} · DNS: ${r[1].status === 'fulfilled' ? 'Working' : 'Failed'}`),
  );
}
async function action(act, b) {
  const row = b?.closest('[data-id]'),
    p = row ? lib.profiles.find((p) => p.id === row.dataset.id) : selected;
  if (act.startsWith('scan-'))
    return scanAction(act, utility?.kind === 'device' ? 'scanner' : page, b, navigate);
  if (act.startsWith('theme-')) {
    await invoke('save_setting', { field: 'theme', value: act.slice(6) });
    await reload();
    render();
    return;
  }
  if (act.startsWith('diag-')) {
    const kind = act.slice(5),
      el = $('out-' + kind);
    el.textContent = 'Running…';
    try {
      el.textContent = await invoke('diagnostic', { kind, target: $('diag-' + kind)?.value || '' });
    } catch (e) {
      el.textContent = String(e);
    }
    return;
  }
  switch (act) {
    case 'close':
      if (utility) return closeUtility();
      dialog.close();
      return;
    case 'pause-traffic':
      return togglePause();
    case 'sidebar':
      localStorage.sidebar =
        document.documentElement.dataset.sidebar === 'compact' ? 'expanded' : 'compact';
      sidebar();
      return;
    case 'minimize':
      return window.__TAURI__.window.getCurrentWindow().minimize();
    case 'maximize':
      return window.__TAURI__.window.getCurrentWindow().toggleMaximize();
    case 'window-close':
      return window.__TAURI__.window.getCurrentWindow().close();
    case 'quit':
      return invoke('quit_app');
    case 'refresh':
      await refresh();
      return notify('Refreshed.');
    case 'new':
      return lib.locked ? unlock() : openWindow('new');
    case 'edit':
      return openWindow('edit', p.id);
    case 'apply':
      return apply(p);
    case 'confirm-apply':
      return apply(selected, automatic, true);
    case 'switch':
      return lib.locked ? unlock() : openWindow('switch');
    case 'remove':
      selected = p;
      return modal(
        'Delete Profile',
        `<p>Delete <strong>${esc(p.name)}</strong> from this computer?</p>${foot(btn('confirm-remove', 'Delete', null, 'destructive'))}`,
      );
    case 'confirm-remove':
    case 'duplicate':
      await invoke('profile_operation', {
        kind: act === 'duplicate' ? 'duplicate' : 'delete',
        id: p.id,
        revision: p.revision,
      });
      return done(act === 'duplicate' ? 'Profile duplicated.' : 'Profile deleted.');
    case 'import':
      if (lib.locked) return unlock();
      $('import-file').value = '';
      $('import-file').click();
      return;
    case 'import-safe':
    case 'import-all': {
      const n = await invoke('import_profiles', {
        contents: importData,
        includeScripts: act === 'import-all',
      });
      importData = null;
      return done(`Imported ${n} profiles.`);
    }
    case 'export-profiles':
      if (lib.locked) return unlock();
      if (!lib.profiles.length) throw Error('No profiles to export.');
      if (lib.settings.encrypt_profiles)
        return modal(
          'Export Profiles',
          `<p>The exported JSON file will contain unencrypted profiles.</p>${foot(btn('export-plain', 'Export', 'up', 'primary'))}`,
        );
    // fall through to the native save dialog
    case 'export-plain':
    case 'export-logs': {
      if (dialog.open) dialog.close();
      if (
        await invoke('export_file', {
          kind: act === 'export-logs' ? 'logs' : 'profiles',
          contents: act === 'export-logs' ? logLines() : '',
        })
      )
        notify('Export complete.');
      return;
    }
    case 'clear':
      return modal(
        'Clear Logs',
        `<p>Remove the recorded application log?</p>${foot(btn('confirm-clear', 'Clear Logs', null, 'destructive'))}`,
      );
    case 'confirm-clear':
      await invoke('clear_logs');
      return done('Logs cleared.');
    case 'start-service':
      await invoke('diagnostic', { kind: act, target: '' });
      await refresh();
      return;
    case 'copy-network':
      await copyText(
        `IP Address: ${net.address || '—'}\nGateway: ${net.gateway || '—'}\nDNS Servers: ${net.dns.join(', ')}\nConnection: ${net.connection || net.primary || 'Disconnected'}`,
      );
      return notify('Network info copied to clipboard.');
    case 'diagnostics':
      return openWindow('diagnostics');
    case 'shortcuts':
      return shortcuts();
    case 'preferences':
      return openWindow('preferences');
    case 'pref-appearance':
      return preferences(lib);
    case 'pref-behavior':
      return preferences(lib, 'behavior');
    case 'about':
      return about(lib.version);
    case 'rules':
      return openWindow('rules');
    case 'website':
    case 'issues':
    case 'storage':
    case 'releases':
      return invoke('open_location', { kind: act });
    case 'unlock':
    case 'enable-encryption':
    case 'change-encryption':
    case 'disable-encryption':
      return encryptionDialog(act);
    case 'schedules':
    case 'hotkeys':
      return openWindow(act);
    case 'entry-add':
      return addEntry(b.dataset.kind, lib);
    case 'entry-delete':
      await changeEntry(b.dataset.kind, lib, b.dataset.id);
      await changed();
      await reload();
      render();
      return manager(managerKind, lib);
  }
}
const pending = [];
function drain() {
  if (busy || dialog.open || !pending.length) return;
  const [id, auto] = pending.shift(),
    p = lib.profiles.find((p) => p.id === id);
  if (p) run(() => apply(p, auto));
}
async function run(fn, b) {
  if (busy) return;
  busy = true;
  document.body.setAttribute('aria-busy', 'true');
  if ($('dialog-error')) $('dialog-error').hidden = true;
  if (b) b.disabled = true;
  try {
    await fn();
  } catch (e) {
    if (utility && !dialog.open) modal('Network Manager', '');
    const dest = dialog.open ? $('dialog-error') : $('error');
    dest.hidden = false;
    dest.textContent = String(e);
  } finally {
    if (b?.isConnected) b.disabled = false;
    busy = false;
    document.body.removeAttribute('aria-busy');
    queueMicrotask(drain);
  }
}
document.addEventListener('click', (e) => {
  const b = e.target.closest('button');
  if (!b || busy || b.disabled) return;
  if (menu.contains(b)) menu.hidePopover?.();
  if (b.dataset.page) {
    navigate(b.dataset.page);
    return;
  }
  if (b.dataset.sort) {
    run(() => scanAction('scan-sort', page, b, navigate));
    return;
  }
  if (b.dataset.do) run(() => action(b.dataset.do, b), b.dataset.do === 'pause-traffic' ? null : b);
});
document.addEventListener('submit', (e) => {
  e.preventDefault();
  run(async () => {
    if (e.target.id === 'profile-editor') {
      await saveEditor();
      await done('Profile saved.');
    } else if (e.target.id === 'entry-form') {
      const kind = await saveEntry(lib);
      await changed();
      await reload();
      render();
      manager(kind, lib);
    } else if (e.target.id === 'encryption-form') {
      await saveEncryption();
      await done('Profile library updated.');
    } else if (e.target.id === 'scan-form') await scanAction('scan-start', page, null, navigate);
  }, e.submitter);
});
document.addEventListener('input', (e) => {
  if (e.target.id === 'query') {
    query = e.target.value;
    filter();
  } else if (e.target.id.startsWith('scan-') && e.target.tagName === 'INPUT') scannerInput(e.target);
});
document.addEventListener('change', (e) => {
  const el = e.target;
  if (el.id === 'allow-auto') dialog.querySelector('[data-do="confirm-apply"]').disabled = !el.checked;
  else if (el.dataset.setting)
    run(async () => {
      try {
        await updateSetting(el, lib);
        await changed();
      } finally {
        await reload();
        render();
        if (dialog.open) preferences(lib, el.dataset.setting === 'theme' ? 'appearance' : 'behavior');
      }
    });
  else if (el.dataset.entryToggle)
    run(async () => {
      await changeEntry(el.dataset.entryToggle, lib, el.dataset.id, el.checked);
      await changed();
      await reload();
      render();
      manager(managerKind, lib);
    });
  else if (el.id === 'interval') interval = +(localStorage.interval = el.value);
  else if (el.id === 'level') {
    level = el.value;
    filter();
  } else if (el.id.startsWith('scan-') && el.tagName === 'SELECT') scannerInput(el);
  else if (el.id === 'import-file' && el.files[0])
    run(async () => {
      const file = el.files[0];
      if (file.size > 16 * 1024 * 1024) throw Error('Backup exceeds 16 MiB.');
      importData = await file.text();
      const v = JSON.parse(importData),
        list = Array.isArray(v) ? v : v.profiles;
      if (!Array.isArray(list)) throw Error('Invalid profile backup.');
      if (list.some((p) => p.automation_actions?.length))
        modal(
          'Import Warning',
          `<p>This backup includes executable scripts or programs.</p>${foot(btn('import-safe', 'Import Without Scripts', null, 'primary') + btn('import-all', 'Import With Scripts'))}`,
        );
      else await action('import-safe');
    });
});
dialog.addEventListener('cancel', (e) => {
  if (busy) e.preventDefault();
});
dialog.addEventListener('close', () => {
  if (!dialog.open) {
    dialog.replaceChildren();
    delete dialog.dataset.device;
    importData = null;
    drain();
  }
});
document.addEventListener('keydown', (e) => {
  if (utility && e.key === 'Escape' && !busy) {
    e.preventDefault();
    closeUtility();
    return;
  }
  if (busy || dialog.open || e.repeat) return;
  const k = e.key.toLowerCase(),
    mod = e.ctrlKey || e.metaKey;
  const hot =
    lib.settings?.hotkeys_enabled &&
    lib.settings.hotkeys?.find(
      (h) =>
        h.enabled &&
        h.key.toLowerCase() === k &&
        [
          ['Ctrl', e.ctrlKey],
          ['Alt', e.altKey],
          ['Shift', e.shiftKey],
          ['Super', e.metaKey],
        ].every(([m, on]) => h.modifiers.includes(m) === on),
    );
  if (hot) {
    const p = lib.profiles.find((p) => p.id === hot.profile_id);
    if (p) {
      e.preventDefault();
      run(() => apply(p));
    }
    return;
  }
  if (mod && !e.altKey && /^[1-7]$/.test(k)) {
    e.preventDefault();
    navigate(Object.keys(pages)[+k - 1]);
    return;
  }
  if (k === 'f10') {
    e.preventDefault();
    $('main-menu').click();
    return;
  }
  if (k === 'f1') {
    e.preventDefault();
    navigate('help');
    return;
  }
  const a =
    k === 'f5'
      ? 'refresh'
      : mod && !e.altKey
        ? {
            n: 'new',
            p: 'switch',
            b: 'sidebar',
            r: 'refresh',
            d: 'diagnostics',
            q: 'quit',
            '?': 'shortcuts',
            k: 'shortcuts',
            c: e.shiftKey ? 'copy-network' : null,
          }[k]
        : null;
  if (a) {
    e.preventDefault();
    run(() => action(a));
  } else if (mod && k === 'f' && ($('query') || $('scan-query'))) {
    e.preventDefault();
    ($('query') || $('scan-query')).focus();
  }
});
scheme.addEventListener('change', applyTheme);
narrow.addEventListener('change', sidebar);
document.addEventListener('visibilitychange', () => {
  if (utility) return;
  if (document.hidden) stop();
  else if (page === 'dashboard' && !busy && !dialog.open) refresh();
});
async function boot() {
  await loadCatalog();
  document.querySelector('.side-header strong').textContent = tr('Network Manager');
  if (utility) {
    document.body.classList.add('utility');
    [net, lib] = await Promise.all([invoke('network_snapshot'), invoke('library_view')]);
    applyTheme();
    switch (utility.kind) {
      case 'new':
      case 'edit': {
        const p = lib.profiles.find((p) => p.id === utility.data);
        if (utility.kind === 'edit' && !p) throw Error('Profile no longer available.');
        await editor(p, net);
        break;
      }
      case 'preferences':
        preferences(lib);
        break;
      case 'rules':
        rules(lib);
        break;
      case 'switch':
        switcher();
        break;
      case 'hotkeys':
      case 'schedules':
        managerKind = utility.kind;
        manager(managerKind, lib);
        break;
      case 'device':
        deviceDetails(utility.data);
        break;
      case 'diagnostics':
        diagnostics();
        break;
    }
    return;
  }
  $('main-menu').innerHTML = icon('menu');
  $('location-root').textContent = tr('Workspace');
  const guide = $('shortcut-guide');
  guide.title = tr('Keyboard Shortcuts') + ' (Ctrl+?)';
  guide.setAttribute('aria-label', tr('Keyboard Shortcuts'));
  guide.innerHTML = icon('command') + `<span class="side-label">${esc(tr('Keyboard Shortcuts'))}</span>`;
  sidebar();
  render();
  await window.__TAURI__.event.listen('profile-request', ({ payload }) => {
    pending.push(payload);
    drain();
  });
  await window.__TAURI__.event.listen('service-message', ({ payload }) => notify(payload));
  await window.__TAURI__.event.listen('workspace-changed', async ({ payload }) => {
    await refresh();
    if (payload) navigate('profiles');
  });
  await window.__TAURI__.event.listen('inspect-device', ({ payload }) =>
    run(() => inspectDevice(payload, navigate)),
  );
  await refresh();
  invoke('detect_range')
    .then((r) => {
      scannerDefaults(r, net.gateway);
      if (['scanner', 'ports'].includes(page) && !document.querySelector('#scan-target:focus')) render();
    })
    .catch(() => scannerDefaults(null, net.gateway));
  if (lib.locked) unlock();
  invoke('check_update')
    .then((v) => {
      if (v) {
        $('update').hidden = false;
        $('update').textContent = `v${v} available`;
      }
    })
    .catch(() => {});
}
run(boot);
