import { tr } from './i18n.js';
const paths = {
  settingsTools:
    'm14 7 3-4a5 5 0 0 0-6 6L3 17a2.5 2.5 0 0 0 4 4l8-9a5 5 0 0 0 6-6l-4 3zM3 3l4 2 1 3-2 1-3-4zm10 12 6 6a1.5 1.5 0 0 0 2-2l-6-6',
  info: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0zM12 11v6m0-10v1',
  users: 'M15 20H3v-2c0-3 3-5 6-5s6 2 6 5zm-2-13a4 4 0 1 1-8 0 4 4 0 0 1 8 0zm5 3v6m-3-3h6',
  scan: 'M3 8V3h5m8 0h5v5m0 8v5h-5M8 21H3v-5m4-4h10',
  edit: 'm4 16 11-11 4 4L8 20H4zm9-9 4 4',
  copy: 'M8 8h12v13H8zM4 16H3V3h12v2',
  trash: 'M3 6h18M6 6l1 15h10l1-15M9 6V3h6v3m-5 4v7m4-7v7',
  check: 'm3 12 6 6L21 5',
  menu: 'M4 6h16M4 12h16M4 18h16',
  help: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0zM9 9a3 3 0 0 1 6 0c0 2-3 2-3 5m0 3v.1',
  collapse: 'M3 4h18v16H3zM9 4v16m7-11-3 3 3 3',
  expand: 'M3 4h18v16H3zM9 4v16m4-11 3 3-3 3',
  pause: 'M8 5v14M16 5v14',
  play: 'm7 4 13 8-13 8z',
  panel: 'M3 4h18v16H3zM9 4v16',
  grid: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z',
  layers: 'm12 3 9 5-9 5-9-5zM3 12l9 5 9-5M3 16l9 5 9-5',
  port: 'M4 5h16v11h-4l-2 3h-4l-2-3H4zM8 6v4m4-4v4m4-4v4',
  log: 'M4 5h16v15H4zM8 9h1m3 0h4m-8 4h1m3 0h4m-8 4h6',
  settings: 'M4 6h8m4 0h4M4 12h2m4 0h10M4 18h10m4 0h2M12 3v6M7 9v6m8 0v6',
  monitor: 'M3 4h18v13H3zM12 17v4m-5 0h10',
  command: 'M8 8H5a3 3 0 1 1 3-3v14a3 3 0 1 1-3-3h14a3 3 0 1 1-3 3V5a3 3 0 1 1 3 3H8z',
  plus: 'M12 5v14M5 12h14',
  refresh: 'M20 8a8 8 0 0 0-14-2L3 9m0-6v6h6M4 16a8 8 0 0 0 14 2l3-3m0 6v-6h-6',
  arrow: 'M5 12h14m-6-6 6 6-6 6',
  up: 'M12 19V5m-6 6 6-6 6 6',
  down: 'M12 5v14m-6-6 6 6 6-6',
  wifi: 'M2 8a17 17 0 0 1 20 0M5 12a12 12 0 0 1 14 0m-11 4a6 6 0 0 1 8 0m-4 4h.01',
  pulse: 'M2 12h5l3-8 4 16 3-8h5',
  server: 'M3 4h18v6H3zM3 14h18v6H3zM7 7h1m-1 10h1',
  route: 'M3 17h6l6-10h5M4 14l-2 3 2 3m14-16 3 3-3 3',
  pin: 'M12 21s7-7 7-12a7 7 0 1 0-14 0c0 5 7 12 7 12zm0-14a2 2 0 1 0 .01 0',
  close: 'm6 6 12 12M6 18 18 6',
  chevron: 'm9 5 7 7-7 7',
  search: 'M17 17l4 4m-3-11a7 7 0 1 1-14 0 7 7 0 0 1 14 0',
  lock: 'M5 10h14v11H5zM8 10V7a4 4 0 0 1 8 0v3',
  clock: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0zm-9-5v6l4 2',
  globe: 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0zM3 12h18m-9-9c5 5 5 13 0 18-5-5-5-13 0-18z',
  ext: 'M13 3h8v8m-1-7L10 14M10 5H4v15h15v-6',
  book: 'M3 5c4-2 7-1 9 1 3-2 6-3 9-1v14c-4-2-7-1-9 1-2-2-5-3-9-1zM12 6v14',
};
export const icon = (name) =>
  `<svg class="icon" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round"><path d="${paths[name]}"/></svg>`;
export const esc = (value) =>
  String(value ?? '').replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c],
  );
export const $ = (id) => document.getElementById(id);
export const invoke = (name, args = {}) => window.__TAURI__.core.invoke(name, args);
export const copyText = (text) => invoke('copy_text', { text: String(text) });
export const tool = (action, label, glyph, attrs = '') =>
  `<button type="button" class="icon-btn" data-do="${esc(action)}" title="${esc(tr(label))}" aria-label="${esc(tr(label))}" ${attrs}>${icon(glyph)}</button>`;
export const options = (pairs, value, translate = true) =>
  pairs
    .map(
      ([v, text]) =>
        `<option value="${esc(v)}" ${String(v) === String(value) ? 'selected' : ''}>${esc(translate ? tr(text) : text)}</option>`,
    )
    .join('');
export const foot = (html) => `<div class="dialog-foot">${btn('close', 'Cancel')}${html}</div>`;
export const btn = (action, label, glyph, style = '') =>
  `<button type="button" data-do="${esc(action)}" class="${style}">${glyph ? icon(glyph) : ''}${esc(tr(label))}</button>`;
export const empty = (glyph, title, detail) =>
  `<div class="empty"><span class="emblem">${icon(glyph)}</span><strong>${esc(tr(title))}</strong><p>${esc(tr(detail))}</p></div>`;
export const pill = (text, up = false) =>
  `<span class="pill ${up ? 'good' : ''}"><i></i>${esc(tr(text))}</span>`;
export const typeName = (t) =>
  ({ wifi: 'Wi-Fi', ethernet: 'Ethernet', virtual: 'Virtual', loopback: 'Loopback' })[t] || 'Network';
export const profileMark = () => `<span class="emblem">${icon('layers')}</span>`;
export function notify(message) {
  $('notice').textContent = message;
  clearTimeout(notify.timer);
  notify.timer = setTimeout(() => {
    $('notice').textContent = '';
  }, 4500);
}
export function modal(title, body) {
  const d = $('dialog');
  d.innerHTML = `<div class="dialog-head"><h2 id="dialog-title">${esc(tr(title))}</h2><button class="icon-btn" aria-label="${esc(tr('Close'))}" data-do="close">${icon('close')}</button></div>${body}<p id="dialog-error" role="alert" hidden></p>`;
  if (!d.open) window.__NM_UTILITY__ ? d.show() : d.showModal();
  return d;
}
