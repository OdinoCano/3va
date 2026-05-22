# 06 - VULNERABILITY AUDIT (OSV)

## 6.1 Overview

`3va audit` detects known vulnerabilities in installed dependencies by querying the [Open Source Vulnerabilities API (OSV)](https://osv.dev). OSV aggregates data from NVD, GitHub Advisory Database (GHSA), RustSec, PyPI Advisory, and others, so a single query covers multiple authoritative sources.

The auditor operates in two sequential phases:

| Phase | Module | What it detects |
|------|--------|-------------|
| 1 | `MalwareScanner` | Malware patterns in code extracted from `node_modules/` |
| 2 | `auditor::run_audit` | Known CVEs, GHSAs, and advisories for each `package@version` |

---

## 6.2 OSV Auditor Architecture

### 6.2.1 Data flow

```
3va-lock.json
     │
     ▼
List of (name, version)
     │
     ├── Cache hit (~/.cache/3va/audit/) ──► result in memory
     │
     └── Cache miss
              │
              ▼
     OSV Batch API (POST /v1/querybatch)
     up to 100 packages per request
              │
              ▼
     Save to cache + result in memory
              │
              ▼
     Parse CVSS v3 severity / GHSA tag
              │
              ▼
     AuditReport { findings, critical_count, high_count, ... }
```

### 6.2.2 Architecture choice: API + cache vs local database

**API-first with per-package cache** was chosen over downloading the full OSV database (~600 MB compressed) for the following reasons:

- Data is always up to date with no manual steps.
- No daemon or scheduler required to keep the DB updated.
- Only information about actually installed packages is downloaded.
- Per-`package@version` cache is granular: a new install only fetches what is new.

---

## 6.3 OSV API: Batch Query

**Endpoint:** `POST https://api.osv.dev/v1/querybatch`

**Request:**
```json
{
  "queries": [
    {
      "version": "4.17.20",
      "package": { "name": "lodash", "ecosystem": "npm" }
    },
    {
      "version": "1.7.9",
      "package": { "name": "axios", "ecosystem": "npm" }
    }
  ]
}
```

**Response:**
```json
{
  "results": [
    {
      "vulns": [
        {
          "id": "GHSA-35jh-r3h4-6jhm",
          "summary": "Prototype Pollution in lodash",
          "severity": [
            { "type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:H/A:N" }
          ],
          "affected": [{
            "ranges": [{ "type": "SEMVER", "events": [{"introduced":"0"},{"fixed":"4.17.21"}] }],
            "database_specific": { "severity": "HIGH" }
          }],
          "references": [
            { "type": "ADVISORY", "url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm" }
          ]
        }
      ]
    },
    { "vulns": [] }
  ]
}
```

The `results` array has the same length and order as `queries`, enabling O(1) result correlation.

---

## 6.4 Local Cache

### 6.4.1 Location

```
~/.cache/3va/audit/
```

### 6.4.2 Entry format

File: `<pkg_sanitized>@<version>.json`

- Scoped packages are sanitized: `@scope/name` → `scope__name@1.0.0.json`

```json
{
  "fetched_at_unix": 1716235200,
  "vulns": [ ... ]
}
```

### 6.4.3 TTL and Refresh

| Situation | Behavior |
|-----------|---------------|
| Entry < 24 h old | Used directly (0 requests to OSV) |
| Entry ≥ 24 h | Auto re-fetch in background of the command |
| `--update-cache` passed | TTL ignored, all packages re-fetched |
| Network unavailable and cache exists | Stale cache used with user warning |
| Network unavailable and no cache | Package omitted from analysis (visible warning) |

The command **never fails with an error** due to connectivity issues.

---

## 6.5 Severity Calculation

Severity is determined in order of preference:

1. **CVSS v3.1 vector** — base score calculated per full NVD formula.
2. **CVSS v2 numeric score** — for older advisories.
3. **`database_specific.severity`** — GitHub Advisory string tag (`CRITICAL`, `HIGH`, `MODERATE`, `LOW`).
4. **`affected[].database_specific.severity`** — same field at the affected package level.

### CVSS v3 Thresholds

| Score | Severity |
|-------|-----------|
| 9.0 – 10.0 | **CRITICAL** |
| 7.0 – 8.9  | **HIGH** |
| 4.0 – 6.9  | **MEDIUM** |
| 0.1 – 3.9  | **LOW** |
| 0.0        | UNKNOWN |

---

## 6.6 Network Error and Rate Limiting Handling

```
request → HTTP 429 → wait 5s → retry once
                                         │
                              ┌──────────┴──────────┐
                           success                failure
                              │                      │
                         save cache           stale cache
                                                (with warning)
```

- Single automatic retry after HTTP 429 (rate limit).
- All network errors (timeout, DNS, TLS) are recoverable: stale cache is used.
- Errors are reported as warnings, never as fatal errors.

---

## 6.7 Privacy

Only the following is sent to the OSV API:
- Package name
- Exact version
- Ecosystem (`"npm"`)

**Not sent:** file paths, code content, project name, environment variables, or any other system metadata.

---

## 6.8 CI/CD Usage

```yaml
# GitHub Actions — block merge if HIGH/CRITICAL
- name: Security audit
  run: 3va audit --deny
```

```bash
# Local pipeline
3va audit --deny && echo "OK" || exit 1
```

The `--deny` flag causes the command to exit with code ≠ 0 if and only if at least one CRITICAL or HIGH vulnerability is found. MEDIUM and LOW vulnerabilities produce a warning but do not block the pipeline.

---

## 6.9 Relationship with the Malware Scanner

The two phases are complementary, not redundant:

| | Malware Scanner (Phase 1) | OSV Auditor (Phase 2) |
|---|---|---|
| **Source of truth** | Heuristics + custom patterns | Public OSV database |
| **What it detects** | Unreported malicious code, obfuscation, exfiltration | Known and published CVEs and advisories |
| **Requires network** | No | Yes (with offline cache) |
| **False positives** | Possible (heuristic) | Low (authoritative data) |
| **Coverage** | 0-day and novel malware | Cataloged vulnerabilities |
