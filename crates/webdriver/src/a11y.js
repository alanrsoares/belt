(() => {
  const out = [];
  const sel = (e) => e.id ? `#${e.id}` : e.tagName.toLowerCase() + (e.name ? `[name=${e.name}]` : '');
  const text = (e) => (e.getAttribute('aria-label') || e.getAttribute('aria-labelledby') || e.getAttribute('title') || e.textContent || '').trim();
  document.querySelectorAll('img:not([alt])').forEach((e) => out.push(`img-alt ${sel(e)}`));
  document.querySelectorAll('input:not([type=hidden]):not([type=submit]):not([type=button]), select, textarea').forEach((e) => {
    const labelled = e.labels?.length || e.getAttribute('aria-label') || e.getAttribute('aria-labelledby') || e.title;
    if (!labelled) out.push(`input-label ${sel(e)}`);
  });
  document.querySelectorAll('button, a[href], [role=button]').forEach((e) => {
    if (!text(e) && !e.querySelector('img[alt]:not([alt=""])')) out.push(`name-missing ${sel(e)}`);
  });
  const seen = new Set();
  document.querySelectorAll('[id]').forEach((e) => {
    if (seen.has(e.id)) out.push(`duplicate-id #${e.id}`);
    seen.add(e.id);
  });
  if (!document.documentElement.lang) out.push('html-lang <html>');
  return out.join('\n');
})()
