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

Some popular packages (e.g. `dayjs`, `uuid`) trigger 3va's built-in malware scanner because their `package.json` or source contain words like "RCE" or "environment variable" in security documentation or test scripts. This is a **false positive**, but it demonstrates that 3va's supply-chain security is active and aggressive — when in doubt, it blocks. To override a false positive for a trusted package:

```bash
# Audit first to see the findings
3va audit

# Then install with a yes override (only for trusted packages)
3va install dayjs --allow-net=registry.npmjs.org --yes
```
