// Arm A/B runner: scripted replay through `belt mcp`.
// Arm A: one process per call. Arm B (--arm B): kept `webdriver --repl` child.
//
// Usage: bun bench/arm-a.ts [--base http://localhost:4173] [--belt target/release/belt]
//                           [--include-slow] [--runs 3] [--out bench/results/arm-a.json]
//
// Token counts are ESTIMATES (characters / 4). They are not API token counts.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { tasks } from "./tasks";

const arg = (name: string, fallback: string): string => {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1]! : fallback;
};

const base = arg("--base", "http://localhost:4173");
const beltBin = arg("--belt", "target/release/belt");
const runs = Number(arg("--runs", "3"));
const arm = arg("--arm", "A");
const armB = arm !== "A"; // B and D use the kept child
const out = arg("--out", `bench/results/arm-${arm.toLowerCase()}.json`);
const includeSlow = process.argv.includes("--include-slow");

const est = (chars: number): number => Math.ceil(chars / 4);

type McpReply = { result?: { content?: { text?: string }[]; isError?: boolean; tools?: unknown[] } };

const proc = Bun.spawn([beltBin, "mcp"], {
  stdin: "pipe", stdout: "pipe", stderr: "inherit",
  // Arm A forces one process per call; arm B uses the kept webdriver child.
  env: armB ? process.env : { ...process.env, BELT_MCP_ONESHOT: "1" },
});
const decoder = new TextDecoder();
const reader = proc.stdout.getReader();
let buffered = "";
let nextId = 1;

const readLine = async (): Promise<string> => {
  for (;;) {
    const nl = buffered.indexOf("\n");
    if (nl >= 0) {
      const line = buffered.slice(0, nl);
      buffered = buffered.slice(nl + 1);
      return line;
    }
    const { value, done } = await reader.read();
    if (done) throw new Error("belt mcp closed its stdout");
    buffered += decoder.decode(value);
  }
};

const send = (msg: object): void => {
  proc.stdin.write(`${JSON.stringify(msg)}\n`);
  proc.stdin.flush();
};

const rpc = async (method: string, params?: object): Promise<{ raw: string; reply: McpReply }> => {
  send({ jsonrpc: "2.0", id: nextId++, method, params });
  const raw = await readLine();
  return { raw, reply: JSON.parse(raw) as McpReply };
};

await rpc("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "bench", version: "0" } });
send({ jsonrpc: "2.0", method: "notifications/initialized" });

const list = await rpc("tools/list");
const schemaChars = list.raw.length;

type CallResult = { ms: number; callChars: number; resultChars: number; isError: boolean; text: string };

const callTool = async (args: string[]): Promise<CallResult> => {
  const params = { name: "webdriver", arguments: { args } };
  const start = performance.now();
  const { reply } = await rpc("tools/call", params);
  const ms = performance.now() - start;
  const text = reply.result?.content?.map((c) => c.text ?? "").join("") ?? "";
  return {
    ms,
    callChars: JSON.stringify(params).length,
    resultChars: text.length,
    isError: reply.result?.isError ?? true,
    text,
  };
};

const median = (xs: number[]): number => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)] ?? 0;

type TaskSummary = {
  id: string;
  name: string;
  success: boolean;
  calls: number;
  callTokensEst: number;
  resultTokensEst: number;
  totalTokensEst: number;
  medianMs: number;
  runsMs: number[];
  note: string;
};

const summaries: TaskSummary[] = [];

for (const task of tasks) {
  if (task.slow && !includeSlow) {
    summaries.push({
      id: task.id, name: task.name, success: false, calls: 0, callTokensEst: 0,
      resultTokensEst: 0, totalTokensEst: 0, medianMs: 0, runsMs: [], note: "skipped (slow; use --include-slow)",
    });
    continue;
  }
  const runsMs: number[] = [];
  let last: CallResult[] = [];
  for (let r = 0; r < runs; r++) {
    // Arm B drops the leading `-t <dur>`: flags force a one-shot process.
    const calls = (arm === "D" ? (task.armD ?? task.armA) : task.armA)(base).map((a) => (armB && a[0] === "-t" ? a.slice(2) : a));
    const results: CallResult[] = [];
    for (const args of calls) results.push(await callTool(args));
    runsMs.push(results.reduce((sum, c) => sum + c.ms, 0));
    last = results;
  }
  const joined = last.map((c) => c.text).join("\n");
  const callChars = last.reduce((s, c) => s + c.callChars, 0);
  const resultChars = last.reduce((s, c) => s + c.resultChars, 0);
  const success = joined.includes(task.expect);
  summaries.push({
    id: task.id,
    name: task.name,
    success,
    calls: last.length,
    callTokensEst: est(callChars),
    resultTokensEst: est(resultChars),
    totalTokensEst: est(callChars + resultChars),
    medianMs: Math.round(median(runsMs)),
    runsMs: runsMs.map(Math.round),
    note: success ? "" : `missing "${task.expect}" in: ${joined.slice(0, 120).replace(/\n/g, " ")}`,
  });
}

proc.stdin.end();
proc.kill();

const report = {
  arm,
  date: new Date().toISOString(),
  tokenCounts: "estimate (chars/4), not API counts",
  toolListTokensEst: est(schemaChars),
  runsPerTask: runs,
  tasks: summaries,
};

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${JSON.stringify(report, null, 2)}\n`);

console.log(`tool list: ~${report.toolListTokensEst} tokens (est)`);
console.log("task".padEnd(12), "ok".padEnd(4), "calls", "call~tok", "result~tok", "total~tok", "median ms");
for (const s of summaries) {
  console.log(
    s.id.padEnd(12), (s.success ? "yes" : "NO").padEnd(4), String(s.calls).padEnd(5),
    String(s.callTokensEst).padEnd(8), String(s.resultTokensEst).padEnd(10),
    String(s.totalTokensEst).padEnd(9), String(s.medianMs), s.note,
  );
}
console.log(`wrote ${out}`);
