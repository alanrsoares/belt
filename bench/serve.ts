// Static fixture server for the webdriver benchmark.
// Usage: bun bench/serve.ts [port]   (default 4173)
import { pages } from "./pages";

const port = Number(process.argv[2] ?? 4173);

Bun.serve({
  port,
  fetch(req) {
    const page = pages[new URL(req.url).pathname];
    return page
      ? new Response(page(), { headers: { "content-type": "text/html; charset=utf-8" } })
      : new Response("not found", { status: 404 });
  },
});

console.log(`bench fixtures on http://localhost:${port}`);
