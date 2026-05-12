use vvva_permissions::PermissionState;

pub struct Runtime {
    pub permissions: PermissionState,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            permissions: PermissionState::new(),
        }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        // Core event loop stub
        Ok(())
    }
}
