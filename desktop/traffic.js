import { $, icon } from './ui.js';

let timer,
  epoch = 0,
  last,
  points = [];
const rate = (n) => {
  if (n == null) return '—';
  const i = n < 1024 ? 0 : n < 1024 ** 2 ? 1 : 2;
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${['B', 'KiB', 'MiB'][i]}/s`;
};
export function chartCard() {
  return `<section class="card traffic"><div class="card-head"><div class="section-title">${icon('pulse')}<h2>Traffic monitor</h2></div><span class="tag">60 SEC</span></div>
    <div class="rates"><div><small><i></i>Receive</small><b id="rx">—</b></div><div><small><i></i>Transmit</small><b id="tx">—</b></div></div>
    <div class="plot"><svg viewBox="0 0 600 80" preserveAspectRatio="none" aria-hidden="true"><path id="area" fill="var(--chart-fill)"/><path id="in-line" class="trace"/><path id="out-line" class="trace out"/><circle id="rx-point" r="0" fill="var(--chart)"/><circle id="tx-point" r="0" fill="var(--chart-out)"/></svg></div>
    <div class="scale"><span id="history-span">−60 s</span><span id="sample-state">Collecting samples</span><span>Now</span></div></section>`;
}
export function stop() {
  clearTimeout(timer);
  ++epoch;
  last = null;
  points = [];
}
export function sample(invoke, device, interval) {
  stop();
  const generation = epoch;
  const tick = async () => {
    if (generation !== epoch || !$('rx') || document.hidden) return;
    try {
      const counters = device && (await invoke('traffic_counters'));
      if (generation !== epoch) return;
      const c = counters && counters.find((c) => c.interface === device);
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
      const y = (n) => (77 - (74 * n) / max).toFixed(1);
      const line = (key) => points.map((p, i) => `${i ? 'L' : 'M'}${x(p)} ${y(p[key])}`).join(' ');
      $('rx').textContent = rate(recent?.rx);
      $('tx').textContent = rate(recent?.tx);
      $('in-line').setAttribute('d', line('rx'));
      $('out-line').setAttribute('d', line('tx'));
      $('area').setAttribute('d', first ? `${line('rx')} L598 80 L${x(first)} 80 Z` : '');
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
          ? `Sampling · ${interval / 1000} s`
          : 'Collecting samples';
    } catch {
      if (generation === epoch) $('sample-state').textContent = 'Counters unavailable';
    }
    if (generation === epoch) timer = setTimeout(tick, interval);
  };
  tick();
}
