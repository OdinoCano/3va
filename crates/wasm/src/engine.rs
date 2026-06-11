use std::path::Path;
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

pub struct WasmEngine {
    engine: Engine,
    permissions: Arc<PermissionState>,
}

struct MyState {
    wasi: WasiP1Ctx,
}

impl WasmEngine {
    pub fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            permissions,
        })
    }

    pub async fn eval_file_with_args(&self, path: &Path, args: &[String]) -> anyhow::Result<()> {
        let mut linker: Linker<MyState> = Linker::new(&self.engine);
        preview1::add_to_linker_async(&mut linker, |s: &mut MyState| &mut s.wasi)?;

        // Support for parsing WebAssembly Text Format (.wat)
        let bytes = if path.extension().and_then(|e| e.to_str()) == Some("wat") {
            wat::parse_file(path)?
        } else {
            std::fs::read(path)?
        };

        let module = Module::new(&self.engine, &bytes)?;

        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();

        let mut cmd_args = vec![path.to_string_lossy().to_string()];
        cmd_args.extend(args.iter().cloned());
        builder.args(&cmd_args);

        if self.permissions.check(&Capability::EnvAccess) {
            builder.inherit_env();
        } else {
            // On Windows env var names are case-insensitive (the OS stores
            // `Path` while scripts grant `PATH`), so match granted names
            // case-insensitively there.
            #[cfg(windows)]
            let granted_upper: Vec<String> = self
                .permissions
                .list_granted()
                .iter()
                .filter_map(|c| match c {
                    Capability::EnvVar(n) => Some(n.to_uppercase()),
                    _ => None,
                })
                .collect();
            for (k, v) in std::env::vars() {
                #[cfg(windows)]
                let allowed = self.permissions.check(&Capability::EnvVar(k.clone()))
                    || granted_upper.contains(&k.to_uppercase());
                #[cfg(not(windows))]
                let allowed = self.permissions.check(&Capability::EnvVar(k.clone()));
                if allowed {
                    builder.env(&k, &v);
                }
            }
        }

        let granted = self.permissions.list_granted();
        for cap in granted {
            match cap {
                Capability::FileRead(p) => {
                    if p.is_dir() {
                        let _ = builder.preopened_dir(
                            &p,
                            p.to_string_lossy().as_ref(),
                            DirPerms::READ,
                            FilePerms::READ,
                        );
                    } else if let Some(parent) = p.parent() {
                        let _ = builder.preopened_dir(
                            parent,
                            parent.to_string_lossy().as_ref(),
                            DirPerms::READ,
                            FilePerms::READ,
                        );
                    }
                }
                Capability::FileWrite(p) => {
                    if p.is_dir() {
                        let _ = builder.preopened_dir(
                            &p,
                            p.to_string_lossy().as_ref(),
                            DirPerms::all(),
                            FilePerms::all(),
                        );
                    } else if let Some(parent) = p.parent() {
                        let _ = builder.preopened_dir(
                            parent,
                            parent.to_string_lossy().as_ref(),
                            DirPerms::all(),
                            FilePerms::all(),
                        );
                    }
                }
                _ => {}
            }
        }

        let mut store = Store::new(
            &self.engine,
            MyState {
                wasi: builder.build_p1(),
            },
        );

        let instance = linker.instantiate_async(&mut store, &module).await?;

        // Attempt to run the standard WASI command entrypoint "_start"
        if let Ok(func) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            func.call_async(&mut store, ()).await?;
        } else {
            anyhow::bail!(
                "No '_start' function found in WebAssembly module. Only WASI Command modules are supported."
            );
        }

        Ok(())
    }
}
