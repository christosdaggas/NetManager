const paths = {
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
export const btn = (action, label, glyph, style = '') =>
  `<button type="button" data-do="${esc(action)}" class="${style}">${glyph ? icon(glyph) : ''}${esc(label)}</button>`;
export const empty = (glyph, title, detail) =>
  `<div class="empty"><span class="emblem">${icon(glyph)}</span><strong>${esc(title)}</strong><p>${esc(detail)}</p></div>`;
export const pill = (text, up = false) =>
  `<span class="pill ${up ? 'good' : ''}"><i></i>${esc(text)}</span>`;
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
  d.innerHTML = `<div class="dialog-head"><h2 id="dialog-title">${esc(title)}</h2><button class="icon-btn" aria-label="Close" data-do="close">${icon('close')}</button></div>${body}<p id="dialog-error" role="alert" hidden></p>`;
  if (!d.open) d.showModal();
  return d;
}
