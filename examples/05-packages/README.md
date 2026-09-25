# 05 — Using npm Packages

Install packages with `3va install` and use them in your code.

## Setup

```bash
3va install --allow-net=registry.npmjs.org
```

## Run

```bash
3va run app.js --allow-read=./node_modules
```

## What it demonstrates

- Installing packages with `3va install` (post-install scripts are **never** executed)
- Package permissions declared in `package.json` under the `"3va"` key
- Using `is-odd` and `ms` — two popular npm packages
- Scoped `allow-read` for `node_modules`

## Real-world note: 3va's malware scanner

`3va install` scans every package with a built-in malware scanner before it lands in `node_modules`. The scanner errs on the side of blocking, so a legitimate package can occasionally be flagged (a false positive). To override one for a package you trust:

```bash
# Audit first to see the findings
3va audit

# Then install with a yes override (only for trusted packages)
3va install <package> --allow-net=registry.npmjs.org --yes
```
