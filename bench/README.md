# Webdriver benchmark

Design: `.scratch/webdriver-benchmark.md`. Status: arm A (scripted replay) only.

```sh
cargo build --release -p belt -p webdriver
bun bench/serve.ts            # fixtures on http://localhost:4173
bun bench/arm-a.ts            # in another shell; writes bench/results/arm-a.json
bun bench/arm-a.ts --include-slow   # also runs the dialog task (can block ~60 s)
```

* Token counts are **estimates** (characters / 4), not API counts.
* Pages are generated in `pages.ts`. Tasks are listed in `tasks.ts`.
* `results/` is git-ignored.
