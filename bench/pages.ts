// Deterministic fixture pages for the webdriver benchmark.
// Each page is generated in code: no timers, no random content.

const shell = (title: string, body: string, script = ""): string =>
  `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head><body>${body}${
    script ? `<script>${script}</script>` : ""
  }</body></html>`;

const range = (n: number): number[] => Array.from({ length: n }, (_, i) => i + 1);

const article = (): string =>
  shell(
    "Article",
    `<header><nav>${range(10)
      .map((i) => `<a href="/article#s${i}">Section ${i}</a>`)
      .join(" ")}</nav></header>
<main><article><h1>A long article</h1>${range(40)
      .map(
        (i) =>
          `<p>Paragraph ${i}. The quick brown fox jumps over the lazy dog while the benchmark counts tokens in a fixed, repeatable way.</p>`,
      )
      .join("")}<p>ARTICLE-END-MARKER</p></article></main>
<footer>${range(10)
      .map((i) => `<a href="/article#f${i}">Footer ${i}</a>`)
      .join(" ")}</footer>`,
  );

const login = (): string =>
  shell(
    "Login",
    `<form id="f"><label>User <input id="user" name="user"></label>
<label>Password <input id="pass" name="pass" type="password"></label>
<button id="submit" type="submit">Sign in</button></form>`,
    `document.getElementById("f").addEventListener("submit", (e) => {
  e.preventDefault();
  document.title = "Dashboard";
  document.body.innerHTML = '<main class="dashboard"><h1>Welcome, admin</h1></main>';
});`,
  );

const table = (): string =>
  shell(
    "Table",
    `<table id="t"><thead><tr><th>ID</th><th>Name</th><th>Qty</th><th>Price</th></tr></thead><tbody>${range(
      30,
    )
      .map(
        (i) =>
          `<tr><td>row-${i}</td><td>Item ${i}</td><td>${i * 3}</td><td>${(i * 1.25).toFixed(2)}</td></tr>`,
      )
      .join("")}</tbody></table>`,
  );

const form = (): string =>
  shell(
    "Form",
    `<form id="f"><label>Country <select id="country" name="country">
<option value="">Choose</option><option value="US">US</option><option value="CA">Canada</option><option value="NZ">New Zealand</option></select></label>
<label><input id="terms" type="checkbox" name="terms"> Accept terms</label>
<fieldset><legend>Plan</legend>
<label><input type="radio" name="plan" value="free"> Free</label>
<label><input type="radio" name="plan" value="pro"> Pro</label></fieldset>
<button id="submit" type="submit">Save</button></form><p id="out"></p>`,
    `document.getElementById("f").addEventListener("submit", (e) => {
  e.preventDefault();
  const d = new FormData(e.target);
  document.getElementById("out").textContent =
    "saved: country=" + d.get("country") + " terms=" + (d.get("terms") ? "on" : "off") + " plan=" + d.get("plan");
});`,
  );

const nav300 = (): string =>
  shell(
    "Nav 300",
    `<header><nav>${range(300)
      .map((i) => `<a href="/nav300#l${i}">Link ${i}</a>`)
      .join("\n")}</nav></header>
<main><button id="target" type="button">Run</button><p id="out"></p></main>`,
    `document.getElementById("target").addEventListener("click", () => {
  document.getElementById("out").textContent = "clicked";
});`,
  );

const dialog = (): string =>
  shell(
    "Dialog",
    `<button id="alert" type="button">Show alert</button><button id="confirm" type="button">Ask</button><button id="prompt" type="button">Name</button><p id="out"></p><p id="res"></p>`,
    `document.getElementById("alert").addEventListener("click", () => {
  alert("hello");
  document.getElementById("out").textContent = "after-alert";
});
document.getElementById("confirm").addEventListener("click", () => {
  document.getElementById("res").textContent = "confirm=" + confirm("sure?");
});
document.getElementById("prompt").addEventListener("click", () => {
  document.getElementById("res").textContent = "prompt=" + prompt("name?", "dflt");
});`,
  );

const late = (): string =>
  shell(
    "Late",
    `<button id="go" type="button">Go</button><p id="out"></p>`,
    `document.getElementById("go").addEventListener("click", () => {
  fetch("/slow").then(() => { document.getElementById("out").textContent = "loaded"; });
});`,
  );

const flow = (step: number): string =>
  step < 4
    ? shell(
        `Flow ${step}`,
        `<main><h1>Step ${step} of 4</h1><a href="/flow/${step + 1}">Next</a></main>`,
      )
    : shell("Flow 4", `<main><h1>Step 4 of 4</h1><p id="value">flow-final</p></main>`);

const scroll = (): string =>
  shell(
    "Scroll",
    `<ul id="list">${range(20)
      .map((i) => `<li style="height:60px">item-${i}</li>`)
      .join("")}</ul>`,
    `let count = 20;
window.addEventListener("scroll", () => {
  if (window.innerHeight + window.scrollY >= document.body.scrollHeight - 5 && count < 80) {
    const list = document.getElementById("list");
    for (let i = 0; i < 20; i++) {
      count++;
      const li = document.createElement("li");
      li.style.height = "60px";
      li.textContent = "item-" + count;
      list.appendChild(li);
    }
  }
});`,
  );

export const pages: Record<string, () => string> = {
  "/article": article,
  "/login": login,
  "/table": table,
  "/form": form,
  "/nav300": nav300,
  "/dialog": dialog,
  "/late": late,
  "/flow/1": () => flow(1),
  "/flow/2": () => flow(2),
  "/flow/3": () => flow(3),
  "/flow/4": () => flow(4),
  "/scroll": scroll,
};
