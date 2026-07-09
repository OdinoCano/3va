# 04 - SCRIPT EXECUTION

## 4.1 Main Flow: Script Execution
### 4.1.1 Flow Diagram
┌──────────────┐
│    User      │
│ invokes CLI  │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Argument  │ ──► Argument validation
│   Parsing   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Build     │ ──► PermissionState construction
│ Permissions │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Initialize │ ──► Runtime + JsEngine creation
│   Runtime   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Load Module  │ ──► Check FileRead permission
│   (require) │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Check       │ ──► Matching against PermissionState
│  Permission  │
└──────┬───────┘
       │
   ┌───┴───┐
   │ALLOW  │DENY
   ▼       ▼
┌─────┐ ┌─────┐
│Execute│ │Error│
│ Code │ │403  │
└─────┘ └─────┘
   │
   ▼
┌──────────────┐
│   Output     │
│  to stdout   │
└─────────────┘
### 4.1.2 Step Description
1. 
Argument Parsing: CLI validates and normalizes arguments
2. 
Build Permissions: Builds permission state from flags
3. 
Initialize Runtime: Creates the event loop and JS engine
4. 
Load Module: Reads the file and checks read permissions
5. 
Check Permission: Compares the operation against granted capabilities
6. 
Execute Code: Executes in V8 with polyfills
7. 
Output: Writes result to stdout/stderr
## 4.2 Flow: Package Installation
### 4.2.1 Flow Diagram
┌──────────────┐
│  3va install │
│   <package>  │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Check Net   │ ──► --allow-net permission for registry
│  Permission  │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Query      │ ──► Registry API (npm/yarn)
│   Registry   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Resolve     │ ──► Dependency tree
│ Dependencies │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Verify      │ ──► Checksum, digital signature
│  Signature   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Security    │ ──► Malware/vulnerability analysis
│  Scan        │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Download    │ ──► Local cache + hash verification
│  Packages    │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Install     │ ──► Copy to node_modules (WITHOUT post-install)
│  Sandbox    │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Generate    │ ──► Lockfile with exact versions
│  Lockfile   │
└─────────────┘
## 4.3 Flow: Permission Verification
### 4.3.1 Verification Algorithm
fn verify_permission(
    operation: &Operation,
    resources: &[Resource],
    permissions: &PermissionState
) -> Result<Decision> {
    
    // 1. Check if it is an explicitly denied operation
    if permissions.is_denied(operation) {
        return Ok(Denied::Explicit);
    }
    
    // 2. Find capability that allows the operation
    for cap in &permissions.granted {
        if matches_capability(operation, resources, cap) {
            return Ok(Allowed(cap.clone()));
        }
    }
    
    // 3. If no match, deny (deny-by-default)
    Ok(Denied::NoCapability)
}
### 4.3.2 Decision Matrix
Operation	Resource	Capability Match
fs.read	/app/config	FileRead(/app/config)
fs.read	/etc/passwd	FileRead(/app/config)
net.connect	api.example.com	Network(*.example.com)
net.connect	evil.com	Network(*.example.com)
spawn	ls	SpawnProcess
spawn	(none)	(none)
## 4.4 Data Flow: Modules
### 4.4.1 ESM Module Resolution
File.ts
    │
    ▼
┌─────────────────┐
│   Parse         │ ──► Detect imports/exports
│   imports       │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Resolve        │ ──► node_modules, path resolution
│  Path           │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Check          │ ──► FileRead permission
│  Permission     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Load          │ ──► Read file, transpile TS►JS
│   Module        │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Execute       │ ──► Add to module graph
│   in context    │
└─────────────────┘
### 4.4.2 Module Graph Format
pub struct ModuleGraph {
    pub modules: HashMap<ModuleId, Module>,
    pub edges: HashMap<ModuleId, Vec<ModuleId>>,
}
Data flows conforming to IEEE 1012 and process modeling.
