# 03 - PROCESO DE RELEASE

## 3.1 Versionado

3va sigue Semantic Versioning (SemVer).

## 3.2 Tipos de Release

| Tipo | Descripcion | Ejemplo |
|------|-------------|---------|
| Major | Cambios incompatibles | 1.0.0 → 2.0.0 |
| Minor | Features compatibles | 1.0.0 → 1.1.0 |
| Patch | Bug fixes | 1.0.0 → 1.0.1 |

## 3.3 Canales

| Canal | Descripcion | Frecuencia |
|-------|-------------|------------|
| stable | Producción | Cada 2 meses |
| beta | Testing | Cada 2 semanas |
| alpha | Desarrollo | Weekly |

## 3.4 Proceso

### 3.4.1 Release Checklist

```
1. Feature freeze (2 semanas antes)
2. Bug fix freeze (1 semana antes)
3. RC build
4. Testing (1 semana)
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

## 3.5 Distribución

| Canal | Destino |
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

*Release process conforme a maintainers guide.*