# Governance

3va is a single-maintainer open source project. This document describes how
decisions are made, who holds which role, and who can reach the project's
sensitive resources.

## Decision model

3va follows a *benevolent dictator* model: the project lead makes final
decisions on the roadmap, design, releases, and which contributions are
merged. Proposals and discussion happen in public, in GitHub issues and pull
requests, and anyone can take part. When the project gains more maintainers,
decisions will move to lazy consensus among them, with the project lead
breaking ties.

## Roles and responsibilities

| Role | Who | Responsibilities |
|------|-----|------------------|
| Project lead / maintainer | Edgar Cano ([@OdinoCano](https://github.com/OdinoCano)) | Sets the roadmap; reviews and merges pull requests; cuts releases; triages and fixes security reports within the [SLA](SECURITY.md#response-times-sla); administers the repository, CI secrets, and package registries; enforces the [Code of Conduct](CODE_OF_CONDUCT.md). |
| Contributor | Anyone | Opens issues and pull requests that follow [CONTRIBUTING.md](CONTRIBUTING.md), including a DCO sign-off on every commit. |
| Security reporter | Anyone | Reports vulnerabilities privately as described in [SECURITY.md](SECURITY.md). |

## Access to sensitive resources

Only the people listed here can reach these resources:

| Resource | Access |
|----------|--------|
| GitHub repository (admin), branch rules, private security advisories | Edgar Cano |
| GitHub Actions secrets and environments (`crates-io`, `npm`, Chocolatey, Snapcraft, winget) | Edgar Cano |
| crates.io, npm (`@edge_166`), Docker Hub (`edge166`), Chocolatey, Snapcraft, winget | Edgar Cano |
| Documentation site hosting (Vercel) | Edgar Cano |

Every account with access uses phishing-resistant two-factor authentication (a passkey).

## Becoming a maintainer

Nobody is granted write access or access to any resource above without a
review first. To become a maintainer, a contributor needs:

1. At least 3 merged, non-trivial pull requests over at least 3 months.
2. Approval from the project lead, after reviewing their contribution history
   and how they have interacted with the community.
3. Two-factor authentication enabled on every account involved.

New maintainers start with the lowest privilege that lets them do the job
(repository *write* for reviewing and merging). Registry and secret access is
granted separately, only when needed. Access is removed when someone steps
down or has been inactive for 12 months.

## Continuity

The project currently has a single maintainer, so its bus factor is 1. If
the maintainer becomes unavailable, nobody can release or merge today. The
project lead is actively looking for a second maintainer. This section will
name that person, and the credentials they hold, once they have been appointed.
