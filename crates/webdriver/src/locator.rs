//! Locator resolution helpers injected into the page.
//!
//! Supports three spec forms so scripts can target elements the way a person
//! describes them rather than by brittle CSS alone:
//!
//! - `text=Some label` — innermost element whose text contains the substring
//! - `role=button:Save` — implicit/explicit ARIA role plus accessible name
//! - `sel=<css>` or a bare CSS selector
//!
//! The prelude is idempotent and re-evaluated with every script, because a
//! navigation wipes the page's JS context.

/// JS prelude defining `window.__wd`. Prepended to any script using a locator.
pub const PRELUDE: &str = r#"window.__wd = window.__wd || {
  visible(el) {
    if (!el) return false;
    if (!el.getClientRects().length) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && parseFloat(s.opacity) > 0;
  },
  name(el) {
    const v = el.getAttribute('aria-label') || el.getAttribute('title') ||
      el.getAttribute('alt') || el.value || el.textContent || '';
    return String(v).trim().toLowerCase();
  },
  role(el) {
    const explicit = el.getAttribute('role');
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    if (tag === 'a') return el.hasAttribute('href') ? 'link' : null;
    if (tag === 'input') {
      const t = (el.type || 'text').toLowerCase();
      if (t === 'submit' || t === 'button' || t === 'reset') return 'button';
      if (t === 'checkbox' || t === 'radio') return t;
      if (t === 'search') return 'searchbox';
      return 'textbox';
    }
    return {
      button: 'button', select: 'combobox', textarea: 'textbox', img: 'img',
      h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading',
      h6: 'heading', nav: 'navigation', table: 'table', ul: 'list', ol: 'list',
      li: 'listitem', form: 'form', dialog: 'dialog', main: 'main',
      header: 'banner', footer: 'contentinfo', summary: 'button'
    }[tag] || null;
  },
  // `data-wd-overlay` marks the recording cursor and captions, never targets.
  find(spec) {
    if (spec.startsWith('text=')) {
      const want = spec.slice(5).trim().toLowerCase();
      const hits = Array.prototype.filter.call(
        document.querySelectorAll('body *:not([data-wd-overlay])'),
        (e) => (e.textContent || '').trim().toLowerCase().includes(want)
      );
      // Innermost match wins, so a wrapper div never shadows its own button.
      return hits.filter((e) => !hits.some((o) => o !== e && e.contains(o)))[0] || null;
    }
    if (spec.startsWith('role=')) {
      const rest = spec.slice(5);
      const sep = rest.indexOf(':');
      const role = (sep < 0 ? rest : rest.slice(0, sep)).trim();
      const want = sep < 0 ? null : rest.slice(sep + 1).trim().toLowerCase();
      return Array.prototype.find.call(
        document.querySelectorAll('body *:not([data-wd-overlay])'),
        (e) => window.__wd.role(e) === role && (want === null || window.__wd.name(e).includes(want))
      ) || null;
    }
    return document.querySelector(spec.startsWith('sel=') ? spec.slice(4) : spec);
  },
  require(spec) {
    const el = window.__wd.find(spec);
    if (!el) throw new Error('no element matched locator: ' + spec);
    return el;
  }
};
"#;

/// Prepend the prelude to `body` so `window.__wd` is guaranteed to exist.
pub fn with_prelude(body: &str) -> String {
    format!("{PRELUDE}\n{body}")
}
