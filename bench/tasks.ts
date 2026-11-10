// Task set for the webdriver benchmark.
// `armA` is the best sequence of `webdriver` calls available TODAY, as one
// `belt mcp` tool call each (one process per call). Later arms add their own.

export type Task = {
  id: string;
  name: string;
  /** Each inner array is the argv of one `webdriver` call. */
  armA: (base: string) => string[][];
  /** Calls for arm D (new verbs); falls back to `armA`. */
  armD?: (base: string) => string[][];
  /** Text that must appear in the joined results for the task to count as done. */
  expect: string;
  /** Skipped unless `--include-slow` is passed (can block for a long time). */
  slow?: boolean;
};

const T = ["-t", "10s"];

export const tasks: Task[] = [
  {
    id: "t1-article",
    name: "Read the main text of a long article",
    // No `text` verb yet: `html` is the only full-content read.
    armA: (b) => [[...T, `${b}/article`, "html"]],
    armD: (b) => [[`${b}/article`, "text"]],
    expect: "ARTICLE-END-MARKER",
  },
  {
    id: "t2-login",
    name: "Log in, confirm the dashboard loaded",
    armA: (b) => [
      [
        ...T,
        `${b}/login`,
        "fill",
        "#user",
        "admin",
        "fill",
        "#pass",
        "secret",
        "click",
        "#submit",
        "wait-for",
        ".dashboard",
        "title",
      ],
    ],
    expect: "Dashboard",
  },
  {
    id: "t3-table",
    name: "Extract a 30-row table",
    // No structured read verb: `eval` with a JS expression.
    armA: (b) => [
      [
        ...T,
        `${b}/table`,
        "eval",
        "JSON.stringify([...document.querySelectorAll('#t tbody tr')].map(r=>[...r.cells].map(c=>c.innerText)))",
      ],
    ],
    expect: "row-30",
  },
  {
    id: "t4-form",
    name: "Fill a form with select, checkbox and radio",
    // No select/check verbs: `eval` sets the values.
    armA: (b) => [
      [...T, `${b}/form`, "snapshot"],
      [
        ...T,
        `${b}/form`,
        "eval",
        "(()=>{const s=document.querySelector('#country');s.value='US';s.dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#terms').click();document.querySelector('input[value=pro]').click();return 'set'})()",
        "click",
        "#submit",
        "eval",
        "document.querySelector('#out').innerText",
      ],
    ],
    expect: "saved: country=US terms=on plan=pro",
  },
  {
    id: "t5-nav300",
    name: "Find and click 'Link 250' in a 300-link nav bar",
    // The snapshot cap is 200 nodes, so 'Link 250' should be missing from it.
    armA: (b) => [
      [...T, `${b}/nav300`, "snapshot"],
      [...T, `${b}/nav300`, "click", "role=link:Link 250", "url"],
    ],
    expect: "#l250",
  },
  {
    id: "t6-dialog",
    name: "Click a button that opens an alert, then continue",
    // No dialog handling: an open alert blocks CDP evaluate (up to 60 s).
    armA: (b) => [
      [...T, `${b}/dialog`, "click", "#alert", "eval", "document.querySelector('#out').innerText"],
    ],
    expect: "after-alert",
    slow: true,
  },
  {
    id: "t7-flow",
    name: "Multi-page flow: 3 clicks, check the final value",
    armA: (b) => [
      [
        ...T,
        `${b}/flow/1`,
        "click",
        "text=Next",
        "click",
        "text=Next",
        "click",
        "text=Next",
        "eval",
        "document.querySelector('#value').innerText",
      ],
    ],
    expect: "flow-final",
  },
  {
    id: "t8-scroll",
    name: "Infinite scroll: load 3 more batches, read the last item",
    // No scroll verb: `eval` scrolls, `wait` lets the page append items.
    armA: (b) => {
      const down = ["eval", "window.scrollTo(0,document.body.scrollHeight)", "wait", "300ms"];
      return [
        [
          ...T,
          `${b}/scroll`,
          ...down,
          ...down,
          ...down,
          "eval",
          "document.querySelector('#list li:last-child').innerText",
        ],
      ];
    },
    expect: "item-80",
  },
];
