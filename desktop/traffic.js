import { $, icon } from './ui.js';

let timer,
  epoch = 0,
  last,
  resume,
  points = [];
function rate(n) {
  if (n == null) return ['—', ''];
  const i = Math.min(3, Math.floor(Math.log(Math.max(1, n)) / Math.log(1024)));
  return [(n / 1024 ** i).toFixed(i ? 1 : 0), ['B/s', 'KiB/s', 'MiB/s', 'GiB/s'][i]];
}
export function chartCard() {
  const metric = (
    key,
    caption,
    glyph,
  ) => `<div class="metric ${key}"><small>${icon(glyph)}${caption}</small>
    <div class="reading"><b id="${key}">—</b><span id="${key}-unit"></span></div></div>`;
  return `<section class="card traffic"><div class="card-head"><h2>${icon('pulse')} Network Activity</h2>
    <div class="sampling"><small id="sample-state">Collecting samples</small><button type="button" class="tool" id="pause-traffic" data-do="pause-traffic" aria-label="Pause traffic sampling" title="Pause traffic sampling" disabled>${icon('pause')}</button></div></div>
    <div class="rates">${metric('rx', 'Download', 'down')}${metric('tx', 'Upload', 'up')}</div>
    <div class="graph"><div class="y-axis" aria-hidden="true"><span id="scale-high">—</span><span id="scale-mid">—</span><span>0</span></div>
    <div class="plot"><svg viewBox="0 0 600 100" preserveAspectRatio="none" aria-hidden="true">
      <defs><linearGradient id="under-line" x1="0" x2="0" y1="0" y2="1" gradientUnits="objectBoundingBox">
      <stop offset="0%" stop-color="var(--chart)" stop-opacity=".24"/><stop offset="100%" stop-color="var(--chart)" stop-opacity=".025"/></linearGradient></defs>
      <path id="area" fill="url(#under-line)"/><path id="in-line" class="trace"/><path id="out-line" class="trace out"/>
      <circle id="rx-point" r="0" fill="var(--chart)"/><circle id="tx-point" r="0" fill="var(--chart-out)"/></svg></div>
    <div class="x-axis"><span id="history-span">−60 s</span><span>Now</span></div></div></section>`;
}
export function stop() {
  clearTimeout(timer);
  ++epoch;
  last = null;
  points = [];
  resume = null;
}
export function togglePause() {
  if (!resume) return;
  const restart = resume;
  const button = $('pause-traffic');
  if (button.dataset.paused === 'true') {
    restart();
    return;
  }
  clearTimeout(timer);
  ++epoch;
  button.dataset.paused = 'true';
  button.title = 'Resume traffic sampling';
  button.setAttribute('aria-label', button.title);
  button.innerHTML = icon('play');
  $('sample-state').textContent = 'Paused';
}
export function sample(invoke, device, interval) {
  stop();
  resume = () => sample(invoke, device, interval);
  const button = $('pause-traffic');
  button.disabled = !device;
  button.dataset.paused = 'false';
  button.title = 'Pause traffic sampling';
  button.setAttribute('aria-label', button.title);
  button.innerHTML = icon('pause');
  const generation = epoch;
  const tick = async () => {
    if (generation !== epoch || !$('rx') || document.hidden) return;
    try {
      const counters = device && (await invoke('traffic_counters'));
      if (generation !== epoch) return;
      const c =
        counters &&
        (device === '*'
          ? counters
              .filter((c) => c.interface !== 'lo')
              .reduce((sum, c) => ({ rx: sum.rx + c.rx, tx: sum.tx + c.tx }), { rx: 0, tx: 0 })
          : counters.find((c) => c.interface === device));
      const now = performance.now();
      if (c && last && c.rx >= last.rx && c.tx >= last.tx) {
        const dt = (now - last.t) / 1000;
        points.push({ t: now, rx: (c.rx - last.rx) / dt, tx: (c.tx - last.tx) / dt });
      } else points = [];
      last = c ? { ...c, t: now } : null;
      points = points.filter((p) => now - p.t <= 60000);
      const max = Math.max(1, ...points.flatMap((p) => [p.rx, p.tx]));
      const first = points[0],
        recent = points.at(-1);
      const span = Math.max(interval, first ? now - first.t : 60000);
      const x = (p) => (598 - (596 * (now - p.t)) / span).toFixed(1);
      const y = (n) => (98 - (96 * n) / max).toFixed(1);
      const line = (key) => points.map((p, i) => `${i ? 'L' : 'M'}${x(p)} ${y(p[key])}`).join(' ');
      for (const k of ['rx', 'tx']) {
        const [value, unit] = rate(recent?.[k]);
        $(k).textContent = value;
        $(k + '-unit').textContent = unit;
      }
      $('scale-high').textContent = recent ? rate(max).join(' ') : '—';
      $('scale-mid').textContent = recent ? rate(max / 2).join(' ') : '—';
      $('in-line').setAttribute('d', line('rx'));
      $('out-line').setAttribute('d', line('tx'));
      $('area').setAttribute('d', first ? `${line('rx')} L598 100 L${x(first)} 100 Z` : '');
      for (const k of ['rx', 'tx']) {
        const point = $(k + '-point');
        point.setAttribute('cx', 598);
        point.setAttribute('cy', y(recent?.[k] || 0));
        point.setAttribute('r', recent ? 2.5 : 0);
      }
      $('history-span').textContent = `−${Math.round(span / 1000)} s`;
      $('sample-state').textContent = !c
        ? 'No interface counters'
        : recent
          ? `Live · ${interval / 1000} s`
          : 'Collecting samples';
    } catch {
      if (generation === epoch) $('sample-state').textContent = 'Counters unavailable';
    }
    if (generation === epoch) timer = setTimeout(tick, interval);
  };
  tick();
}
