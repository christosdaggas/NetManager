import { esc, btn, modal } from './ui.js';
const topics = [
  [
    'About Network Manager',
    'Save the network and system configuration for a place as a profile. Switch between home, work, and travel without re-entering settings.',
  ],
  [
    'Dashboard',
    'Check your connection, IPv4 address, gateway, and DNS servers in the network panel. Refresh or create a profile from the page header. Below the live traffic chart you can inspect your interfaces; the Active Profile card lets you switch profiles and check the NetworkManager service.',
  ],
  [
    'Network Scanner',
    'Enter a subnet, an IPv4 range, or a single address. Fast discovery uses ping and the neighbour table. Thorough discovery also checks common services. Filter or sort results, inspect a device, copy details, send Wake-on-LAN, or export CSV.',
  ],
  [
    'Port Scanner',
    'Enter the address or name of a device. Choose common ports, ports 1–1024, all ports, or a custom range. Scan Ports on a discovered device opens the same tool. Results show open TCP ports, service names, and response times.',
  ],
  [
    'Profiles',
    'Create profiles from a template or a blank configuration. Apply, edit, duplicate, and delete from the profile list. Import or export JSON backups from the toolbar.',
  ],
  [
    'Logs',
    'Review application events and profile activations. Search by text, filter by level, clear the history, or export the visible log.',
  ],
  [
    'Settings',
    'Configure appearance, automatic switching, scheduling, the connection watchdog, sandboxing, encrypted profiles, and shortcuts. Open Preferences from the main menu for startup and tray options.',
  ],
  [
    'Profile Features',
    'Configure DHCP or static IPv4, DNS, adapter state, Wi-Fi, VPN, proxy, scripts, and programs. Imported profiles retain additional system actions and rules. Review actions before activation.',
  ],
  [
    'Tips',
    'Give profiles descriptive names. Keep a default configuration and export important profiles for backup. Use Ctrl+P to switch profiles or inspect the logs if a change fails.',
  ],
];
export function helpPage() {
  return `<div class="help-actions">${btn('shortcuts', 'Keyboard Shortcuts', 'command')}${btn('diagnostics', 'Network Diagnostics', 'pulse')}${btn('copy-network', 'Copy Network Info', 'copy')}</div><div class="topic-grid">${topics.map(([h, p]) => `<section class="help-topic"><h2 data-i18n>${esc(h)}</h2><p data-i18n>${esc(p)}</p></section>`).join('')}</div>`;
}
export function shortcuts() {
  modal(
    'Keyboard Shortcuts',
    `<dl class="link-facts">${[
      ['New profile', 'Ctrl N'],
      ['Switch profile', 'Ctrl P'],
      ['Toggle sidebar', 'Ctrl B'],
      ['Network diagnostics', 'Ctrl D'],
      ['Copy network info', 'Ctrl Shift C'],
      ['Refresh', 'Ctrl R / F5'],
      ['Dashboard through Help', 'Ctrl 1–7'],
      ['Help', 'F1'],
      ['Main menu', 'F10'],
      ['Shortcuts', 'Ctrl ?'],
      ['Quit', 'Ctrl Q'],
    ]
      .map(([l, k]) => `<div><dt>${l}</dt><dd><kbd>${k}</kbd></dd></div>`)
      .join('')}</dl>`,
  );
}
