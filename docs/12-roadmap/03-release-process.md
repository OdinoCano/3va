# 03 - RELEASE PROCESS

## 3.1 Versioning

3va follows Semantic Versioning (SemVer).

## 3.2 Release Types

| Type | Description | Example |
|------|-------------|---------|
| Major | Incompatible changes | 1.0.0 → 2.0.0 |
| Minor | Compatible features | 1.0.0 → 1.1.0 |
| Patch | Bug fixes | 1.0.0 → 1.0.1 |

## 3.3 Channels

| Channel | Description | Frequency |
|-------|-------------|------------|
| stable | Production | Every 2 months |
| beta | Testing | Every 2 weeks |
| alpha | Development | Weekly |

## 3.4 Process

### 3.4.1 Release Checklist

```
1. Feature freeze (2 weeks before)
2. Bug fix freeze (1 week before)
3. RC build
4. Testing (1 week)
5. Release notes
6. Publication
7. Announcement
```

### 3.4.2 Tags

```bash
# Alpha
git tag v1.0.0-alpha.1

# Beta
git tag v1.0.0-beta.1

# RC
git tag v1.0.0-rc.1

# Stable
git tag v1.0.0
```

## 3.5 Distribution

| Channel | Destination |
|-------|---------|
| Homebrew | brew install 3va |
| npm | npm i -g 3va |
| Docker | docker pull 3va/3va |
| Binaries | GitHub releases |

## 3.6 Changelog

```markdown
# Changelog v1.0.0 (2027-03-15)

## Breaking
- Removed deprecated `3va.crypto` API

## Features
- Added post-quantum crypto support
- Implemented malware scanner

## Fixes
- Fixed memory leak in module cache

## Security
- Patched CVE-2026-XXXX
```

---

*Release process compliant with maintainers guide.*
