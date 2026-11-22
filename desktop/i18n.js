let catalog = Object.create(null);
export const tr = (text) => (Object.hasOwn(catalog, text) ? catalog[text] : text);
export async function loadCatalog() {
  const language = navigator.language || 'en';
  catalog = await window.__TAURI__.core.invoke('catalog', { language });
  document.documentElement.lang = Object.keys(catalog).length ? language : 'en';
}
// Only application-owned labels are marked. Names, file paths, diagnostic
// output, and editable profile values must never be passed through gettext.
export function localize(root) {
  for (const el of root.querySelectorAll('[data-i18n]')) {
    for (const node of el.childNodes)
      if (node.nodeType === Node.TEXT_NODE) {
        const key = node.data.trim();
        if (key && Object.hasOwn(catalog, key)) node.data = node.data.replace(key, catalog[key]);
      }
  }
}
