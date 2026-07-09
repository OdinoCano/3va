# 01 - GENERAL SYSTEM ARCHITECTURE

## 1.1 Architecture Vision

The architecture of 3va follows a modular design based on Rust crates, where each component fulfills a specific responsibility and communicates through well-defined interfaces. The system is designed following principles of security by default and separation of privileges.

```
┌─────────────────────────────────────────────────────────────────┐
│                         3va CLI                                 │
│                    (vvva_cli - Entrypoint)                       │
└─────────────────────────────────────────────────────────────────┘
                              │
         ┌─────────────────────┼─────────────────────┐
         │                     │                     │
         ▼                     ▼                     ▼
 ┌───────────────┐    ┌───────────────┐    ┌───────────────┐
 │   vvva_core   │    │   vvva_pm     │    │  vvva_bundler │
 │   (Runtime)   │    │  (Package     │    │   (Bundler)   │
 │               │    │   Manager)    │    │               │
 └───────────────┘    └───────────────┘    └───────────────┘
         │                     │                     │
         └─────────────────────┼─────────────────────┘
                               │
                               ▼
                     ┌───────────────┐
                     │ vvva_permissions
                     │ (Permissions) │
                     └───────────────┘
                               │
                               ▼
                     ┌───────────────┐
                     │   vvva_js     │
                     │ (JS Engine)   │
                     └───────────────┘
```

## 1.2 Design Principles

### 1.2.1 Security by Default
All components operate under the principle of "deny by default". No process has access to system resources without an explicit Capability granted by the user.

### 1.2.2 Minimum Privilege
Each component has exactly the permissions necessary to fulfill its function. The JavaScript runtime has no direct access to the filesystem; any operation must go through the permission verifier.

### 1.2.3 Defense in Depth
Multiple security layers protect the system:
- Layer 1: CLI (argument validation)
- Layer 2: Permissions (capability verification)
- Layer 3: Sandbox (process isolation)
- Layer 4: Audit (operation logging)

### 1.2.4 Modularity
Each crate is independent and can be replaced or updated without affecting other components. The integration with V8, for example, is abstracted to allow future changes.

## 1.3 Component Architecture

### 1.3.1 Presentation Layer (CLI)
The CLI acts as the single entry point for all operations. It parses user arguments, builds the permission context, and delegates to the appropriate component.

**Responsibilities:**
- Argument parsing and validation
- PermissionState construction
- Command routing
- Output formatting

**Interfaces:**
- Command line interface (user)
- Event interface (core, pm, bundler)

### 1.3.2 Execution Layer (Core)
The core manages the runtime lifecycle, including the event loop, process management, and asynchronous task coordination.

**Responsibilities:**
- Main event loop
- Task scheduling
- Memory management
- Component coordination

**Interfaces:**
- Async task API
- Process management API

### 1.3.3 Security Layer (Permissions)
The permission system implements the capability model, verifying each operation against the list of granted permissions.

**Responsibilities:**
- Capability storage
- Permission verification
- Pattern matching
- Operation auditing

**Interfaces:**
- Permission verification API
- Audit API

### 1.3.4 JavaScript Execution Layer (JS)
The JavaScript engine executes user code in an isolated environment with access to standard web APIs.

**Responsibilities:**
- JS/TS code execution
- Module management
- Polyfill implementation
- Context isolation

**Interfaces:**
- Code evaluation API
- Module API
- Globals API

### 1.3.5 Package Layer (PM)
The package manager handles downloading, verification, and installation of dependencies.

**Responsibilities:**
- Dependency resolution
- Package downloading
- Signature verification
- Installation sandboxing

**Interfaces:**
- Installation API
- Resolution API
- Lockfile API

## 1.4 Deployment Model

3va is deployed as a single binary containing all integrated components. This simplifies distribution and reduces the attack surface.

```
┌─────────────────────────────────────────┐
│              3va Binary                 │
├─────────────────────────────────────────┤
│ CLI    │ Core │ Perms │ JS │ PM │ Bundler│
└─────────────────────────────────────────┘
```

---

*Document conforming to ISO/IEC/IEEE 42010 and systems architecture.*
