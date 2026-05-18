use clap::{Parser, Subcommand};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use std::path::PathBuf;

pub mod accessibility;

#[derive(Parser)]
#[command(name = "3va")]
#[command(author = "Satoshi")]
#[command(version = "0.1.0")]
#[command(about = "Modern, secure-by-default, WASM-first JS/TS runtime", long_about = None)]
struct Cli {
    /// Activa el modo de accesibilidad para lectores Braille/Screen readers (desactiva color y animaciones)
    #[arg(global = true, long = "accessible", help = "Enable screen-reader/braille accessible output (disables colors and animations)")]
    accessible: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a JavaScript or TypeScript file
    Run {
        /// The file to run
        file: PathBuf,

        /// Allow read access to specified paths
        #[arg(long = "allow-read")]
        allow_read: Option<Vec<PathBuf>>,

        /// Allow network access to specified hosts
        #[arg(long = "allow-net")]
        allow_net: Option<Vec<String>>,

        /// Allow write access to specified paths
        #[arg(long = "allow-write")]
        allow_write: Option<Vec<PathBuf>>,

        /// Allow environment variable access
        #[arg(long = "allow-env")]
        allow_env: bool,

        /// Allow spawning child processes
        #[arg(long = "allow-child-process")]
        allow_child_process: bool,
    },
    /// Install dependencies from 3va registry
    Install {
        /// The package to install
        package: Option<String>,
        
        /// Allow network access to specified hosts for the installed package
        #[arg(long = "allow-net")]
        allow_net: Option<Vec<String>>,
    },
    /// Development server
    Dev,
    /// Bundle the application
    Bundle,
    /// Audit dependencies
    Audit,
    /// Check runtime health
    Doctor,
    /// Enter an isolated interactive sandbox
    Sandbox,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    
    let is_accessible = accessibility::is_accessible_mode(cli.accessible);

    // Initialize tracing with ANSI colors conditionally
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_ansi(!is_accessible)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    if is_accessible {
        info!("Accessible mode enabled: colors and complex terminal animations are turned off.");
    }

    match &cli.command {
        Commands::Run { file, allow_read, allow_net, allow_write, allow_env, allow_child_process } => {
            info!("Running {:?} (Sandboxed)", file);
            let mut permissions = vvva_permissions::PermissionState::new();
            if let Some(reads) = allow_read {
                for path in reads {
                    permissions.grant(vvva_permissions::Capability::FileRead(path.clone()));
                }
            }
            if let Some(writes) = allow_write {
                for path in writes {
                    permissions.grant(vvva_permissions::Capability::FileWrite(path.clone()));
                }
            }
            if let Some(nets) = allow_net {
                for host in nets {
                    permissions.grant(vvva_permissions::Capability::Network(host.clone()));
                }
            }
            if *allow_env {
                permissions.grant(vvva_permissions::Capability::EnvAccess);
            }
            if *allow_child_process {
                permissions.grant(vvva_permissions::Capability::SpawnProcess);
            }

            let _engine = vvva_js::JsEngine::new(&permissions)?;
            // TODO: In a real implementation we shouldn't clone the whole state or we'd share it differently,
            // but for now we'll move a clone into Runtime
            let _runtime = vvva_core::Runtime::new(permissions);
            info!("3va Runtime initialized securely.");
            // Execute file...
        }
        Commands::Install { package, allow_net: _ } => {
            if let Some(pkg) = package {
                info!("Installing package '{}'", pkg);
                vvva_pm::install_package(pkg).await?;
            } else {
                info!("Installing dependencies from manifest...");
                info!("Note: Post-install scripts are DISABLED by default for security.");
            }
        }
        Commands::Dev => info!("Starting dev server..."),
        Commands::Bundle => info!("Bundling application..."),
        Commands::Audit => info!("Auditing dependencies..."),
        Commands::Doctor => info!("Checking 3va health..."),
        Commands::Sandbox => info!("Entering interactive sandbox..."),
    }

    Ok(())
}
