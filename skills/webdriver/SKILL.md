---
name: webdriver
description: "Zero-dependency browser automation CLI for viewport/full-page/element screenshots, PDF export, DOM dumps, accessibility snapshots, console errors, and multi-hop persistent session testing. Use when taking web screenshots, driving headless browser actions, testing local web servers, or inspecting web UIs."
---

# `webdriver`

Headless Chromium/Chrome/Brave automation over the Chrome DevTools Protocol. One binary, no dependencies.

Install: `curl -fsSL https://raw.githubusercontent.com/alanrsoares/belt/main/install.sh | sh -s -- webdriver`

## Usage

```bash
webdriver [OPTIONS] <URL> [VERBS...]      # one run, verbs execute in order
webdriver [OPTIONS] -f script.wd          # verbs from a file, one per line
webdriver [OPTIONS] --repl                # one command per stdin line, browser stays open
```

Script files and `--repl` input skip blank lines and lines that start with `#` or `//`. Quote arguments with spaces: `fill "#q" "two words"`.

## Output

One line per step: `ok`, `ok <payload>`, `warn <detail>` or `err <message>`. Colour is off when stdout is not a TTY. The exit code is 1 if any step failed.

- `warn` is not a failure. Examples: a near-blank screenshot, `a11y` findings, `find` with too many matches.
- A failed step stops the run. Add `--keep-going` to continue.
- `-q` prints only steps with a payload. `-v` adds step numbers and timings.

## Locators

Any verb that takes a selector accepts a locator:

| Locator | Meaning |
| :--- | :--- |
| `text=Some label` | Innermost element that contains the text |
| `role=button:Save` | ARIA role and accessible name |
| `sel=<css>` or bare CSS | CSS selector |
| `ref=e7` | Handle from the last `snapshot` or `find` |

Actions retry until the element is present and visible, up to `--timeout`. Do not add `wait-for` before every `click`. Clicks and typing use real browser input events, so React `onChange` fires.

## Verbs

- `goto <url>` — Navigate. The first URL argument is an implicit `goto`.
- `reload` — Reload the current page.
- `viewport <w> <h>` — Set the window size (`1440 900` or `1440x900`).
- `wait-for <loc> [timeout]` — Wait until the element is visible and painted.
- `wait-for-url <substr> [timeout]` — Wait until the URL contains the text (redirects).
- `wait-for-hydration [quiet]` — Wait until the DOM stops changing (default `500ms`).
- `wait <duration>` — Fixed sleep (`500ms`, `2s`, `1m`).
- `click <loc>` — Click the element.
- `hover <loc>` — Real mouse move, so CSS `:hover` applies.
- `press <key>` — Key chord: `Enter`, `Escape`, `Meta+O`.
- `type <loc> <text>` — Append text.
- `fill <loc> <text>` — Clear, then type.
- `eval <js_expr>` — Evaluate JavaScript and print the JSON result.
- `url` — Print the current URL.
- `title` — Print the page title.
- `find <query>` — List interactive nodes whose name contains the text or whose role matches. The refs work with `click`.
- `text [css]` — Visible text of `main`/`body` or a CSS match. Cheaper than `html`.
- `dialog accept [text]|dismiss` — Answer later JS dialogs (default: accept). The setting stays.
- `a11y` — Basic accessibility findings (alt, labels, names, ids, lang). Not a WCAG audit.
- `console [--errors|--full|--clear]` — Buffered console errors, repeats collapsed.
- `snapshot [--all]` — Accessibility tree as `eN role "name"` lines.
- `frame <substr> | --top` — Aim later commands at an iframe (URL or name), or back at the top page.
- `screenshot [path] [--full-page] [--selector <loc>]` — Capture a PNG.
- `pdf [path]` — Print the page to PDF.
- `html [path]` — Dump the current HTML to a file or stdout.
- `close` — End the run (useful in `--repl`).

## Options

- `-s, --session <name>` — Persistent profile: cookies (including HttpOnly), `localStorage`, `sessionStorage`, IndexedDB.
- `--user-data-dir <dir>` — Custom browser profile directory.
- `--list-sessions` — List stored session profiles and their sizes.
- `--clear-session <name>` — Delete a stored session profile.
- `-b, --browser <path>` — Browser binary (Chromium, Chrome, Brave).
- `--browser-arg <flag>` — Extra browser flag, repeatable. Running as root already adds `--no-sandbox` and `--disable-dev-shm-usage`.
- `-t, --timeout <duration>` — Default step timeout (default `30s`).
- `-f, --file <path>` — Read a script file instead of CLI verbs.
- `--repl, -` — Stream commands from stdin, one per line.
- `--keep-going` — Continue after a failed step.
- `--end-marker <text>` — In `--repl`, print the text on its own line after each command.
- `--headed` — Show the browser window.
- `-q, --quiet` — Print only steps that carry a payload.
- `-v, --verbose` — Step numbers and timings.
- `-h, --help` — Print help.
- `-V, --version` — Print the version.

## Recipes

Screenshots and exports:

```bash
webdriver https://example.com screenshot out.png
webdriver https://example.com viewport 1440 900 screenshot out.png --full-page
webdriver https://example.com screenshot card.png --selector "role=article:Pricing"
webdriver https://example.com pdf doc.pdf
```

Inspect a page without a screenshot. `snapshot` and `text` are cheaper than `html`:

```bash
webdriver http://localhost:3000 wait-for-hydration snapshot
webdriver http://localhost:3000 find Save click ref=e7 console --errors
```

Multi-hop login with a persistent session:

```bash
# Hop 1: log in and store the session
webdriver --session app http://localhost:3000/login \
  fill "#email" "admin@example.com" \
  fill "#password" "secret123" \
  click "role=button:Sign in" \
  wait-for-url /dashboard

# Hop 2: open a protected route with the stored session
webdriver --session app http://localhost:3000/admin/settings screenshot settings.png

webdriver --clear-session app
```

Interactive drive. The browser launches once, and each stdin line runs when it arrives:

```bash
webdriver --repl --end-marker '---' <<'SCRIPT'
goto http://localhost:3000
wait-for-hydration
snapshot
click "text=Get started"
url
SCRIPT
```

Sessions live under `~/.local/share/belt/webdriver/sessions/<name>/`.
