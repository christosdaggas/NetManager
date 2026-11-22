import { $, esc, icon, btn, tool, options, modal, foot, invoke, notify } from './ui.js';

import { tr } from './i18n.js';
const row = (title, sub, control, forId) =>
  `<div class="setting-row"><div>${forId ? `<label for="${esc(forId)}">${esc(tr(title))}</label>` : `<strong>${esc(tr(title))}</strong>`}<small>${esc(tr(sub))}</small></div>${control}</div>`;
const group = (title, rows) =>
  `<section class="pref-group"><h2>${esc(tr(title))}</h2><div>${rows.join('')}</div></section>`;
const value = (cfg, k) => k.split('.').reduce((v, p) => v?.[p], cfg);
function control(cfg, k, title, sub, kind, choices) {
  const id = 'opt-' + k,
    v = value(cfg, k),
    attr = `data-setting="${k}" id="${id}"`;
  const html =
    kind === 'check'
      ? `<input type="checkbox" role="switch" ${attr} ${v ? 'checked' : ''}>`
      : kind === 'select'
        ? `<select ${attr}>${options(choices, v ?? '')}</select>`
        : `<input ${attr} type="${kind}" ${kind === 'number' ? `min="${choices[0]}" max="${choices[1]}"` : ''} value="${esc(v)}">`;
  return row(title, sub, html, id);
}
const action = (title, sub, act, glyph = 'chevron') => row(title, sub, tool(act, title, glyph));
const themes = [
  ['system', 'Follow System'],
  ['light', 'Light'],
  ['dark', 'Dark'],
];
export function settingsPage(lib, interval) {
  const c = lib.settings;
  if (!c) return '<p>Reading settings…</p>';
  const opt = (...a) => control(c, ...a);
  return [
    group('Appearance', [
      opt('theme', 'Color Scheme', 'Select the application color scheme', 'select', themes),
      row(
        'Traffic sampling',
        'Time between network activity readings.',
        `<select id="interval">${options(
          [1000, 2000, 5000].map((n) => [n, `${n / 1000} s`]),
          interval,
        )}</select>`,
        'interval',
      ),
    ]),
    group('Auto-Switch', [
      opt('auto_switch_enabled', 'Enable Auto-Switch', 'Activate profiles when conditions match', 'check'),
      opt(
        'auto_switch_interval_secs',
        'Check Interval',
        'Seconds between condition checks',
        'number',
        [5, 300],
      ),
      opt(
        'auto_switch_allow_scripts',
        'Allow Scripts During Auto-Switch',
        'Run automation actions when auto-switching profiles',
        'check',
      ),
      action('Auto-Switch Rules', 'Configure conditions for automatic profile switching', 'rules'),
    ]),
    group('Behavior', [
      opt('show_notifications', 'Desktop Notifications', 'Show notifications for profile changes', 'check'),
      opt('start_minimized', 'Start Minimized', 'Start in the tray when a tray host is available', 'check'),
    ]),
    group('Storage', [
      action('Profiles Directory', lib.storage, 'storage', 'layers'),
      action('Logs File', lib.storage + '/logs.json', 'storage', 'log'),
    ]),
    group('Backup & Restore', [
      action('Export Profiles', 'Save a JSON copy of your profiles', 'export-profiles', 'up'),
      action('Import Profiles', 'Restore profiles from a backup', 'import', 'down'),
    ]),
    group('Scheduling', [
      opt(
        'scheduling_enabled',
        'Enable Scheduled Activation',
        'Activate profiles at specified times',
        'check',
      ),
      action('Manage Schedules', `${c.schedules?.length || 0} schedules`, 'schedules'),
    ]),
    group('Connection Watchdog', [
      opt('watchdog.enabled', 'Enable Watchdog', 'Monitor connectivity and respond to failures', 'check'),
      opt(
        'watchdog.check_interval_secs',
        'Check Interval',
        'Seconds between connectivity checks',
        'number',
        [5, 3600],
      ),
      opt('watchdog.ping_target', 'Ping Target', 'Host to check', 'text'),
      opt(
        'watchdog.failure_threshold',
        'Failure Threshold',
        'Consecutive failures before taking action',
        'number',
        [1, 100],
      ),
      opt('watchdog.failure_action', 'Failure Action', 'What to do when connectivity is lost', 'select', [
        ['notify', 'Notify'],
        ['reconnect', 'Reconnect'],
        ['switch_profile', 'Switch Profile'],
        ['restart_network_manager', 'Restart NetworkManager'],
      ]),
      ...(c.watchdog.failure_action === 'switch_profile'
        ? [
            opt(
              'watchdog.fallback_profile_id',
              'Fallback Profile',
              'Activate this profile on failure',
              'select',
              [['', 'None'], ...lib.profiles.map((p) => [p.id, p.name])],
            ),
          ]
        : []),
    ]),
    group('Security', [
      opt('sandbox_mode', 'Script Sandboxing', 'Isolation for pre- and post-activation scripts', 'select', [
        ['none', 'No Sandboxing'],
        ['bubblewrap', 'Bubblewrap'],
        ['firejail', 'Firejail'],
      ]),
      action(
        'Profile Encryption',
        c.encrypt_profiles ? 'Encrypted on disk' : 'Not encrypted',
        c.encrypt_profiles ? 'disable-encryption' : 'enable-encryption',
        'lock',
      ),
      ...(lib.locked
        ? [action('Unlock Profiles…', 'Unlock this library for the current session', 'unlock')]
        : c.encrypt_profiles
          ? [action('Change Passphrase…', 'Replace the library passphrase', 'change-encryption')]
          : []),
    ]),
    group('Keyboard Shortcuts', [
      opt(
        'hotkeys_enabled',
        'Enable Profile Hotkeys',
        'Use assigned shortcuts to switch profiles',
        'check',
      ),
      action('Configure Hotkeys', `${c.hotkeys?.length || 0} shortcuts`, 'hotkeys'),
    ]),
    group('About', [
      row('Version', lib.version, btn('about', 'About')),
      action('Project Website', 'chrisdaggas.com', 'website', 'ext'),
      action('Report an Issue', 'Open the project issue tracker', 'issues', 'ext'),
    ]),
  ].join('');
}
export function preferences(lib, tab = 'appearance') {
  const c = lib.settings,
    opt = (...a) => control(c, ...a);
  const body =
    tab === 'appearance'
      ? group('Theme', [
          opt('theme', 'Color Scheme', 'Choose the application’s appearance', 'select', themes),
        ])
      : group('Startup', [
          opt(
            'autostart_on_login',
            'Start on Login',
            'Automatically start Network Manager when you log in',
            'check',
          ),
          opt(
            'show_tray_icon',
            'Show System Tray Icon',
            'Display an icon in the system tray (AppIndicator on GNOME)',
            'check',
          ),
        ]) +
        group('Profiles', [
          opt(
            'confirm_profile_switch',
            'Confirm Profile Switch',
            'Ask for confirmation before switching profiles',
            'check',
          ),
        ]);
  modal(
    'Preferences',
    `<div class="segmented">${btn('pref-appearance', 'Appearance', null, tab === 'appearance' ? 'selected' : '')}${btn('pref-behavior', 'Behavior', null, tab === 'behavior' ? 'selected' : '')}</div>${body}`,
  );
}
export async function updateSetting(el, lib) {
  const [field, sub] = el.dataset.setting.split('.');
  let val = el.type === 'checkbox' ? el.checked : el.type === 'number' ? Number(el.value) : el.value;
  if (sub) val = { ...lib.settings[field], [sub]: sub === 'fallback_profile_id' ? val || null : val };
  await invoke('save_setting', { field, value: val });
}
export function encryptionDialog(action) {
  const isUnlock = action === 'unlock',
    isDisable = action === 'disable-encryption',
    needsCurrent = isUnlock || isDisable || action === 'change-encryption';
  modal(
    isUnlock
      ? 'Unlock Profiles'
      : isDisable
        ? 'Disable Profile Encryption'
        : needsCurrent
          ? 'Change Passphrase'
          : 'Enable Profile Encryption',
    `<form id="encryption-form" data-mode="${action}">
    ${needsCurrent ? '<label for="current-key">Current passphrase</label><input id="current-key" type="password" autocomplete="current-password" required>' : ''}
    ${!isUnlock && !isDisable ? '<label for="new-key">New passphrase</label><input id="new-key" type="password" autocomplete="new-password" minlength="8" required><label for="repeat-key">Repeat passphrase</label><input id="repeat-key" type="password" autocomplete="new-password" required>' : ''}
    ${foot(`<button class="primary">${isUnlock ? 'Unlock' : isDisable ? 'Decrypt Profiles' : 'Save Passphrase'}</button>`)}</form>`,
  );
}
export async function saveEncryption() {
  const mode = $('encryption-form').dataset.mode;
  if (mode === 'unlock') await invoke('unlock_profiles', { passphrase: $('current-key').value });
  else {
    const newKey = $('new-key')?.value ?? null;
    if (newKey !== null && newKey !== $('repeat-key').value) throw Error('The passphrases do not match.');
    await invoke('encryption', { current: $('current-key')?.value || '', newKey });
  }
}
export function manager(kind, lib) {
  const schedule = kind === 'schedules',
    entries = lib.settings[kind] || [];
  modal(
    schedule ? 'Manage Schedules' : 'Configure Hotkeys',
    `<div class="manager-list">${
      entries
        .map((e) => {
          const name =
            lib.profiles.find((p) => p.id === e.profile_id)?.name || e.profile_name || 'Missing profile';
          return `<div class="setting-row"><div><strong>${esc(e.description || name)}</strong><small>${esc(schedule ? `${name} · ${e.cron_expression}${e.one_shot ? ' · once' : ''}` : [...e.modifiers, e.key].join('+'))}</small></div><input type="checkbox" aria-label="Enabled" data-entry-toggle="${kind}" data-id="${esc(e.id)}" ${e.enabled ? 'checked' : ''}>${tool('entry-delete', 'Delete', 'trash', `data-kind="${kind}" data-id="${esc(e.id)}"`)}</div>`;
        })
        .join('') || `<p>No ${schedule ? 'schedules' : 'hotkeys'} configured.</p>`
    }</div><div class="dialog-foot">${btn('close', 'Close')}<button class="primary" data-do="entry-add" data-kind="${kind}">${icon('plus')}Add ${schedule ? 'Schedule' : 'Hotkey'}</button></div>`,
  );
}
export function addEntry(kind, lib) {
  if (!lib.profiles.length) throw Error('Create or unlock a profile first.');
  const schedule = kind === 'schedules';
  modal(
    schedule ? 'Add Schedule' : 'Add Hotkey',
    `<form id="entry-form" data-kind="${kind}">
    <label for="entry-profile">Profile</label><select id="entry-profile">${options(
      lib.profiles.map((p) => [p.id, p.name]),
      null,
    )}</select>
    ${
      schedule
        ? `<label for="entry-desc">Description</label><input id="entry-desc"><label for="entry-time">Activation time</label><input type="time" id="entry-time" value="09:00" required>
    <fieldset><legend>Days of Week · none means every day</legend>${['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'].map((d, i) => `<label><input name="day" type="checkbox" value="${i}"> ${d}</label>`).join('')}</fieldset><label><input id="one-shot" type="checkbox"> One-shot</label>`
        : `<fieldset><legend>Modifiers</legend>${['Ctrl', 'Alt', 'Shift', 'Super'].map((k) => `<label><input name="modifier" type="checkbox" value="${k}"> ${k}</label>`).join('')}</fieldset><label for="entry-key">Key</label><input id="entry-key" required maxlength="16" placeholder="e.g. F8">`
    }
    ${foot('<button class="primary">Save</button>')}</form>`,
  );
}
export async function changeEntry(kind, lib, id, toggle) {
  const list = lib.settings[kind] || [];
  const value =
    toggle === undefined
      ? list.filter((e) => e.id !== id)
      : list.map((e) => (e.id === id ? { ...e, enabled: toggle } : e));
  await invoke('save_setting', { field: kind, value });
}
export async function saveEntry(lib) {
  const kind = $('entry-form').dataset.kind,
    profile_id = $('entry-profile').value;
  const e = { id: crypto.randomUUID(), profile_id, enabled: true };
  if (kind === 'schedules') {
    const [h, m] = $('entry-time').value.split(':').map(Number),
      days = [...document.querySelectorAll('[name="day"]:checked')].map((b) => b.value);
    Object.assign(e, {
      description: $('entry-desc').value,
      one_shot: $('one-shot').checked,
      cron_expression: `${m} ${h} * * ${days.join(',') || '*'}`,
    });
  } else {
    const modifiers = [...document.querySelectorAll('[name="modifier"]:checked')].map((b) => b.value);
    const key = $('entry-key').value.trim();
    if (!modifiers.length && !/^F\d{1,2}$/i.test(key))
      throw Error('Choose a modifier or use a function key.');
    Object.assign(e, {
      profile_name: lib.profiles.find((p) => p.id === profile_id)?.name || '',
      modifiers,
      key,
    });
  }
  await invoke('save_setting', { field: kind, value: [...(lib.settings[kind] || []), e] });
  return kind;
}
export function about(version) {
  modal(
    'About Network Manager',
    `<img src="./mark.svg" width="56" height="56" alt=""><h3>Network Manager ${esc(version)}</h3><p>Network and system profile manager for Linux</p><p>© 2026 Christos A. Daggas · MIT License</p><div class="dialog-foot">${btn('website', 'Website', 'ext')}${btn('issues', 'Report an Issue', 'ext')}${btn('close', 'Close')}</div>`,
  );
}
export function rules(lib) {
  modal(
    'Auto-Switch Rules',
    `<p>Rules are stored with each profile. Edit its Auto-Switch Rules section to combine conditions using AND or OR.</p>${group(
      'Available Rule Types',
      ['Wi-Fi SSID', 'Gateway MAC', 'Ping Target', 'Time Window', 'Interface State'].map((t) =>
        row(t, '', ''),
      ),
    )}<div class="dialog-foot">${btn('close', 'Close')}</div>`,
  );
}
