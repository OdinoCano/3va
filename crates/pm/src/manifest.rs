use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageManifest {
    pub package: PackageInfo,
    pub permissions: PackagePermissions,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PackagePermissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub process: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_manifest_parsing() {
        let json = r#"{
            "package": {
                "name": "lodash",
                "version": "4.17.21"
            },
            "permissions": {
                "network": true
            }
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(manifest.package.name, "lodash");
        assert_eq!(manifest.package.version.as_deref(), Some("4.17.21"));

        // Provided by JSON
        assert!(manifest.permissions.network);

        // Defaults correctly assigned
        assert!(!manifest.permissions.filesystem);
        assert!(!manifest.permissions.process);
    }

    #[test]
    fn test_package_permissions_default() {
        let json = r#"{
            "package": {
                "name": "chalk"
            },
            "permissions": {}
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        // All security permissions should be false by default if empty struct provided
        assert!(!manifest.permissions.network);
        assert!(!manifest.permissions.filesystem);
        assert!(!manifest.permissions.process);
    }
}
