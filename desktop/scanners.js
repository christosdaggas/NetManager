import { $, esc, icon, tool, btn, pill, empty, options, modal, invoke, notify, copyText } from './ui.js';

const device = () => ({
  target: '',
  mode: 'fast',
  custom: '',
  rows: [],
  progress: 0,
  total: 0,
  status: 'Ready to scan',
  running: false,
  query: '',
  filter: 'all',
  sort: 'ip',
  offset: 0,
  address: '',
  widths: {},
});
const models = { scanner: device(), ports: { ...device(), mode: 'common', sort: 'port' } };
const types = {
  scanner: [
    ['fast', 'Fast'],
    ['thorough', 'Thorough'],
  ],
  ports: [
    ['common', 'Common ports'],
    ['known', 'Ports 1–1024'],
    ['all', 'All ports'],
    ['custom', 'Custom'],
  ],
};
const ipKey = (ip) =>
  String(ip)
    .split('.')
    .reduce((v, n) => v * 256 + Number(n), 0);
const latency = (n) => (n == null ? '—' : `${n.toFixed(1)} ms`);
const text = (v) => v ?? '—';
const csv = (rows) =>
  rows
    .map((r) =>
      r
        .map(
          (v) =>
            `"${String(v ?? '')
              .replace(/^[=+@\t\r-]/, "'$&")
              .replaceAll('"', '""')}"`,
        )
        .join(','),
    )
    .join('\r\n');
function filtered(kind) {
  const s = models[kind],
    q = s.query.toLowerCase();
  return s.rows
    .filter(
      (r) =>
        (kind === 'ports' || s.filter === 'all' || (s.filter === 'online') === r.online) &&
        Object.values(r).some((v) =>
          String(v ?? '')
            .toLowerCase()
            .includes(q),
        ),
    )
    .sort((a, b) => {
      if (s.sort === 'ip') return ipKey(a.ip) - ipKey(b.ip);
      if (s.sort === 'online') return Number(b.online) - Number(a.online) || ipKey(a.ip) - ipKey(b.ip);
      if (['latency', 'port'].includes(s.sort)) return (a[s.sort] ?? Infinity) - (b[s.sort] ?? Infinity);
      return String(a[s.sort] ?? '').localeCompare(String(b[s.sort] ?? ''), undefined, { numeric: true });
    });
}
function results(kind) {
  const s = models[kind],
    lan = kind === 'scanner',
    list = filtered(kind);
  if (!s.total && !s.running)
    return (
      empty(
        lan ? 'scan' : 'server',
        lan ? 'Scan Your Network' : 'Scan a Device’s Ports',
        lan
          ? 'Enter an IPv4 range or detect your local network.'
          : 'Enter an address or host name and choose the ports to check.',
      ) + `<div class="empty-action">${btn('scan-start', 'Start Scan', 'play', 'primary')}</div>`
    );
  if (!list.length)
    return empty(
      'search',
      s.running
        ? 'Scanning…'
        : s.rows.length
          ? 'No Matching Results'
          : s.failed
            ? 'Device Not Found'
            : lan
              ? 'No devices found'
              : s.summary?.closed
                ? 'No Open Ports'
                : 'No Answer',
      s.status,
    );
  s.offset = Math.min(s.offset, Math.floor((list.length - 1) / 100) * 100);
  const cols = lan
    ? [
        ['ip', 'IP Address'],
        ['hostname', 'Device Name'],
        ['mac', 'MAC Address'],
        ['vendor', 'Vendor'],
        ['latency', 'Ping'],
        ['ports', 'Services'],
      ]
    : [
        ['port', 'Port'],
        ['service', 'Service'],
        ['latency', 'Response'],
      ];
  return `<div class="table-wrap" tabindex="0" role="region" aria-label="Scan results"><table><colgroup>${cols.map(([k]) => `<col data-column="${k}" ${s.widths[k] ? `width="${s.widths[k]}"` : ''}>`).join('')}<col></colgroup><thead><tr>${cols.map(([k, label]) => `<th scope="col"><button data-sort="${k}">${label}${s.sort === k ? ' ↑' : ''}</button><span class="column-grip" tabindex="0" role="separator" aria-orientation="vertical" aria-label="Resize ${label}" aria-valuemin="64" aria-valuemax="800" aria-valuenow="${s.widths[k] || 120}" data-resize="${k}"></span></th>`).join('')}<th scope="col">Actions</th></tr></thead><tbody>${list
    .slice(s.offset, s.offset + 100)
    .map(
      (r) =>
        `<tr data-device="${esc(r.ip ?? r.port)}">${cols
          .map(([k], i) => {
            const v =
              k === 'latency' ? latency(r[k]) : k === 'ports' ? r.ports.join(', ') || '—' : text(r[k]);
            return i === 0
              ? `<th scope="row">${lan ? `<button class="link" data-do="scan-details">${esc(v)}</button>${pill(r.online ? 'Online' : 'Offline', r.online)}` : esc(v)}</th>`
              : `<td>${esc(v)}</td>`;
          })
          .join(
            '',
          )}<td>${tool('scan-copy', 'Copy address', 'copy')}${lan ? tool('scan-device-ports', 'Scan Ports', 'server') : ''}</td></tr>`,
    )
    .join(
      '',
    )}</tbody></table></div>${list.length > 100 ? `<div class="pager">${btn('scan-prev', 'Previous')} ${s.offset + 1}–${Math.min(s.offset + 100, list.length)} of ${list.length} ${btn('scan-next', 'Next')}</div>` : ''}`;
}
// Like GtkColumnView, remember a manually sized column while sorting/filtering.
// Keyboard users can change the width without performing a drag.
let resizing = false;
function setWidth(grip, px) {
  const kind = $('scan-page')?.dataset.kind;
  if (!kind) return;
  const width = Math.min(800, Math.max(64, Math.round(px)));
  models[kind].widths[grip.dataset.resize] = width;
  document.querySelector(`col[data-column="${grip.dataset.resize}"]`)?.setAttribute('width', String(width));
  grip.setAttribute('aria-valuenow', String(width));
}
document.addEventListener('pointerdown', (e) => {
  const g = e.target.closest('[data-resize]');
  if (!g || e.button !== 0) return;
  const origin = e.clientX,
    width = g.parentElement.getBoundingClientRect().width;
  g.setPointerCapture(e.pointerId);
  resizing = true;
  const move = (ev) => setWidth(g, width + ev.clientX - origin);
  const end = () => {
    resizing = false;
    g.removeEventListener('pointermove', move);
  };
  g.addEventListener('pointermove', move);
  g.addEventListener('lostpointercapture', end, { once: true });
});
document.addEventListener('keydown', (e) => {
  const g = e.target.closest('[data-resize]');
  if (!g || !['ArrowLeft', 'ArrowRight'].includes(e.key)) return;
  e.preventDefault();
  setWidth(g, g.parentElement.getBoundingClientRect().width + (e.key === 'ArrowLeft' ? -16 : 16));
});
function paint(kind) {
  if ($('scan-page')?.dataset.kind !== kind) return;
  const s = models[kind];
  const oldTable = $('scan-results').querySelector('.table-wrap');
  const x = oldTable?.scrollLeft || 0,
    y = oldTable?.scrollTop || 0;
  if (!resizing) {
    $('scan-results').innerHTML = results(kind);
    $('scan-results').querySelector('.table-wrap')?.scrollTo(x, y);
  }
  $('scan-status').textContent = s.status;
  $('scan-progress').hidden = !s.running;
  $('scan-progress').max = s.total || 1;
  $('scan-progress').value = s.progress;
  $('scan-count').textContent = s.total
    ? `${kind === 'scanner' ? s.rows.filter((r) => r.online).length + ' online' : s.rows.length + ' open'} · ${s.progress} of ${s.total} scanned`
    : 'No scan yet';
  document.querySelectorAll('[data-do="scan-export"]').forEach((b) => (b.disabled = !s.rows.length));
}
export function scannerPage(kind) {
  const s = models[kind],
    lan = kind === 'scanner';
  return `<div id="scan-page" data-kind="${kind}"><form id="scan-form" class="scan-controls">
    <label>${lan ? 'IP range or subnet' : 'Host'}<input id="scan-target" value="${esc(s.target)}" placeholder="${lan ? '192.168.1.0/24' : 'IP address or host name'}" ${s.running ? 'disabled' : ''}></label>
    ${lan ? tool('scan-detect', 'Detect local network', 'route', s.running ? 'disabled' : '') : ''}
    <label>${lan ? 'Mode' : 'Ports'}<select id="scan-mode" ${s.running ? 'disabled' : ''}>${options(types[kind], s.mode)}</select></label>
    ${!lan && s.mode === 'custom' ? `<label>Custom ports<input id="scan-custom" value="${esc(s.custom)}" placeholder="22, 80, 8000-8100" ${s.running ? 'disabled' : ''}></label>` : ''}
    ${s.running ? btn('scan-stop', 'Stop', null) : '<button class="primary" type="submit">Scan</button>'}</form>
    <div class="scan-status"><span id="scan-status">${esc(s.status)}</span><progress id="scan-progress" ${s.running ? '' : 'hidden'}></progress></div>
    <div class="filters" ${s.total ? '' : 'hidden'}><span id="scan-count"></span><label><span class="sr">Search results</span><input id="scan-query" type="search" value="${esc(s.query)}" placeholder="${lan ? 'Search devices' : 'Search ports'}"></label>
    ${
      lan
        ? `<select id="scan-filter" aria-label="Filter devices">${options(
            [
              ['all', 'All'],
              ['online', 'Online'],
              ['offline', 'Offline'],
            ],
            s.filter,
          )}</select>`
        : ''
    }
    <select id="scan-sort" aria-label="Sort results">${options(
      lan
        ? [
            ['ip', 'IP'],
            ['hostname', 'Name'],
            ['online', 'Status'],
            ['mac', 'MAC'],
            ['vendor', 'Vendor'],
            ['latency', 'Ping'],
            ['ports', 'Services'],
          ]
        : [
            ['port', 'Port'],
            ['service', 'Service'],
            ['latency', 'Response'],
          ],
      s.sort,
    )}</select></div>
    <div id="scan-results"></div></div>`;
}
export function mountScanner(kind) {
  paint(kind);
}
function redraw(kind) {
  if ($('scan-page')?.dataset.kind === kind) {
    $('view').innerHTML = scannerPage(kind);
    paint(kind);
  }
}
export async function inspectDevice(ip, navigate) {
  if (models.ports.running) throw Error('A port scan is already running.');
  models.ports.target = ip;
  navigate('ports');
  await start('ports');
}
export function deviceDetails(row) {
  modal(
    'Device Details',
    `<dl class="link-facts">${Object.entries({
      'IP Address': row.ip,
      Hostname: row.hostname,
      'MAC Address': row.mac,
      Vendor: row.vendor,
      Status: row.online ? 'Online' : 'Offline',
      Ping: latency(row.latency),
      'Open Services': row.ports.join(', ') || 'None',
    })
      .map(
        ([k, v]) =>
          `<div><dt>${k}</dt><dd>${esc(text(v))} ${v != null ? tool('scan-copy-field', 'Copy ' + k, 'copy', `data-value="${esc(v)}"`) : ''}</dd></div>`,
      )
      .join(
        '',
      )}</dl><div class="dialog-foot">${btn('scan-copy-details', 'Copy Details', 'copy')}${row.mac && !row.online ? btn('scan-wake', 'Wake on LAN') : ''}${btn('scan-device-ports', 'Scan Ports', 'server', 'primary')}</div>`,
  );
}
export function scannerInput(el) {
  const k = $('scan-page')?.dataset.kind;
  if (!k) return;
  const s = models[k];
  const field = {
    'scan-target': 'target',
    'scan-mode': 'mode',
    'scan-custom': 'custom',
    'scan-query': 'query',
    'scan-filter': 'filter',
    'scan-sort': 'sort',
  }[el.id];
  if (!field) return;
  s[field] = el.value;
  s.offset = 0;
  if (field === 'mode') redraw(k);
  else if (['query', 'filter', 'sort'].includes(field)) paint(k);
}
async function start(kind) {
  const s = models[kind];
  if (s.running) return;
  const total = await invoke('scan_start', {
    kind,
    target: s.target.trim(),
    mode: s.mode,
    custom: s.custom,
  });
  Object.assign(s, {
    rows: [],
    progress: 0,
    total,
    running: true,
    status: 'Scanning…',
    offset: 0,
    failed: false,
    address: '',
    summary: null,
    stopping: false,
  });
  redraw(kind);
  const began = performance.now();
  const collect = async () => {
    try {
      const e = await invoke('scan_events', { kind });
      s.rows.push(...e.rows);
      s.progress = Math.max(s.progress, e.progress);
      s.address = e.address || s.address;
      s.failed ||= e.failed;
      s.summary = e.summary || s.summary;
      if (e.done) {
        s.running = false;
        s.status = s.failed
          ? 'Could not resolve host name.'
          : s.stopping
            ? 'Scan stopped.'
            : `Scan complete in ${((performance.now() - began) / 1000).toFixed(1)} s`;
        redraw(kind);
        return;
      }
      s.status = s.stopping ? 'Stopping…' : `Scanning… ${s.progress} of ${s.total}`;
      paint(kind);
      setTimeout(collect, 180);
    } catch (e) {
      s.running = false;
      s.status = String(e);
      redraw(kind);
    }
  };
  await collect();
}
export async function scanAction(action, kind, el, navigate) {
  if (action === 'scan-copy-field') {
    await copyText(el.dataset.value);
    notify('Copied to clipboard.');
    return;
  }
  const s = models[kind];
  const id = el?.closest('[data-device]')?.dataset.device || $('dialog')?.dataset.device;
  const row =
    window.__NM_UTILITY__?.kind === 'device'
      ? window.__NM_UTILITY__.data
      : s.rows.find((r) => String(r.ip ?? r.port) === id);
  if (action === 'scan-start') {
    await start(kind);
    return;
  }
  if (action === 'scan-stop') {
    s.stopping = true;
    await invoke('scan_stop', { kind });
    return;
  }
  if (action === 'scan-detect') {
    s.target = await invoke('detect_range');
    redraw(kind);
    return;
  }
  if (action === 'scan-export') {
    const data =
      kind === 'scanner'
        ? [
            ['IP', 'Hostname', 'MAC', 'Vendor', 'Online', 'Ping (ms)', 'Open ports'],
            ...filtered(kind).map((r) => [
              r.ip,
              r.hostname,
              r.mac,
              r.vendor,
              r.online,
              r.latency,
              r.ports.join(';'),
            ]),
          ]
        : [
            ['Host', 'Address', 'Port', 'Service', 'Response (ms)'],
            ...filtered(kind).map((r) => [s.target, s.address, r.port, r.service, r.latency]),
          ];
    if (await invoke('export_file', { kind, contents: csv(data) })) notify('Scan results exported.');
    return;
  }
  if (action === 'scan-prev' || action === 'scan-next') {
    s.offset = Math.max(0, s.offset + (action === 'scan-next' ? 100 : -100));
    paint(kind);
    return;
  }
  if (action === 'scan-sort') {
    s.sort = el.dataset.sort;
    s.offset = 0;
    redraw(kind);
    return;
  }
  if (!row) return;
  if (action === 'scan-details') {
    await invoke('open_utility', { kind: 'device', data: row });
  } else if (action === 'scan-wake') {
    await invoke('wake_device', { mac: row.mac });
    notify('Wake-on-LAN packet sent.');
  } else if (action === 'scan-device-ports') {
    if (window.__NM_UTILITY__) {
      await invoke('inspect_device', { ip: row.ip });
      return window.__TAURI__.window.getCurrentWindow().close();
    }
    await inspectDevice(row.ip, navigate);
  } else if (action === 'scan-copy' || action === 'scan-copy-details') {
    await copyText(
      action.endsWith('details')
        ? Object.entries(row)
            .map(([k, v]) => `${k}: ${text(v)}`)
            .join('\n')
        : row.ip || `${s.address || s.target}:${row.port}`,
    );
    notify('Copied to clipboard.');
  }
}
export function scannerDefaults(range, gateway) {
  if (!models.scanner.target) models.scanner.target = range || '';
  if (!models.ports.target) models.ports.target = gateway || '';
}
