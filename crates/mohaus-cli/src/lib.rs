//! Command-line interface for mohaus.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
use clap_complete::{Shell, generate};
use mohaus_core::{
    BuildOptions, ProjectConfig, PythonInfo, SdistOptions, build_sdist, build_wheel,
    ensure_editable_built,
};
use mohaus_scaffold::{ScaffoldOptions, scaffold_project};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

const SELF_FIND_LINKS_ENV: &str = "MOHAUS_SELF_FIND_LINKS";
const SELF_WHEEL_ENV: &str = "MOHAUS_SELF_WHEEL";

#[derive(Debug)]
struct SelfWheelhouse {
    path: PathBuf,
    wheel: Option<PathBuf>,
    cleanup: bool,
}

/// Run the mohaus CLI from an argv iterator.
///
/// # Errors
///
/// Returns an error when argument parsing or command execution fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.print().context("failed to print CLI help")?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    run(cli)
}

#[derive(Debug, Parser)]
#[command(
    name = "mohaus",
    version,
    about = "Build mixed Python and Mojo packages"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    /// Scaffold a project in the current directory, in <name>/, or at [path].
    Init {
        /// Project name. If omitted, the current directory name is used.
        name: Option<String>,

        /// Destination directory. If omitted with a name, defaults to ./<name>.
        path: Option<PathBuf>,
    },

    /// Scaffold a project in a new directory.
    New {
        /// Project name and destination directory.
        name: String,
    },

    /// Generate shell completions.
    #[command(visible_alias = "completion")]
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },

    /// Build a wheel for the active host.
    Build {
        /// Build with release intent. Currently forwarded to the build context.
        #[arg(long)]
        release: bool,

        /// Output directory.
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },

    /// Build and install an editable package.
    Develop {
        /// Force non-isolated install, useful for nightly/local Mojo.
        #[arg(long)]
        no_build_isolation: bool,
    },

    /// Build a source distribution.
    Sdist {
        /// Output directory.
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },

    /// Keep editable Mojo extensions warm.
    Watch {
        /// Debounce interval in milliseconds.
        #[arg(long, default_value_t = 250)]
        interval_ms: u64,
    },
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        CommandKind::Init { name, path } => init(name, path),
        CommandKind::New { name } => new_project(name),
        CommandKind::Completions { shell } => completions(shell),
        CommandKind::Build { release, out } => build(release, out),
        CommandKind::Develop { no_build_isolation } => develop(no_build_isolation),
        CommandKind::Sdist { out } => sdist(out),
        CommandKind::Watch { interval_ms } => watch(interval_ms),
    }
}

fn init(name: Option<String>, path: Option<PathBuf>) -> Result<()> {
    let cwd = env::current_dir().context("could not read current directory")?;
    let (name, destination) = match (name, path) {
        (Some(name), Some(destination)) => (name, destination),
        (Some(name), None) => {
            let destination = cwd.join(&name);
            (name, destination)
        }
        (None, None) => {
            let name = cwd
                .file_name()
                .ok_or_else(|| anyhow!("current directory has no project-like name"))?
                .to_os_string()
                .into_string()
                .map_err(os_string_error)?;
            (name, cwd)
        }
        (None, Some(destination)) => {
            let name = destination
                .file_name()
                .ok_or_else(|| anyhow!("destination has no project-like name"))?
                .to_os_string()
                .into_string()
                .map_err(os_string_error)?;
            (name, destination)
        }
    };
    scaffold_project(&ScaffoldOptions { name, destination })?;
    Ok(())
}

fn new_project(name: String) -> Result<()> {
    let cwd = env::current_dir().context("could not read current directory")?;
    scaffold_project(&ScaffoldOptions {
        destination: cwd.join(&name),
        name,
    })?;
    Ok(())
}

fn completions(shell: Shell) -> Result<()> {
    let mut command = Cli::command();
    generate(shell, &mut command, "mohaus", &mut io::stdout());
    Ok(())
}

fn build(release: bool, out: PathBuf) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let python = PythonInfo::detect()?;
    let wheel = build_wheel(&BuildOptions {
        project_dir,
        out_dir: out,
        python,
        release,
        metadata_dir: None,
    })?;
    println!("{}", wheel.display());
    Ok(())
}

fn develop(no_build_isolation: bool) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let config = ProjectConfig::load(&project_dir)?;
    let disable_isolation = no_build_isolation || should_disable_isolation(&config);
    run_editable_install(disable_isolation, editable_mojo_requirement(&config))
}

fn sdist(out: PathBuf) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let archive = build_sdist(&SdistOptions {
        project_dir,
        out_dir: out,
    })?;
    println!("{}", archive.display());
    Ok(())
}

fn watch(interval_ms: u64) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let python = PythonInfo::detect()?;
    let config = ProjectConfig::load(&project_dir)?;
    let interval = Duration::from_millis(interval_ms);
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(interval, tx)
        .map_err(|error| anyhow!("could not create filesystem watcher: {error}"))?;

    let watch_roots = watch_roots(&project_dir, &config);
    for root in &watch_roots {
        if !root.is_dir() {
            continue;
        }
        debouncer
            .watcher()
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| anyhow!("could not watch {}: {error}", root.display()))?;
    }

    eprintln!("mohaus watch: building once before tracking changes");
    ensure_editable_built(&project_dir, &python)?;
    eprintln!(
        "mohaus watch: ready ({} roots, debounce {interval_ms}ms)",
        watch_roots.len()
    );
    while let Ok(event) = rx.recv() {
        match event {
            Ok(events) if relevant_events(&events) => {
                if let Err(error) = ensure_editable_built(&project_dir, &python) {
                    eprintln!("mohaus watch: rebuild failed: {error}");
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("mohaus watch: watcher error: {error}");
            }
        }
    }
    Ok(())
}

fn watch_roots(project_dir: &Path, config: &ProjectConfig) -> Vec<PathBuf> {
    let mut roots = vec![config.mojo_source_root(), config.python_source_root()];
    for include in &config.mojo_include_paths {
        roots.push(project_dir.join(include));
    }
    roots
}

fn relevant_events(events: &[notify_debouncer_mini::DebouncedEvent]) -> bool {
    events.iter().any(|event| {
        event
            .path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| matches!(ext, "mojo" | "🔥" | "mojopkg" | "py"))
    })
}

fn should_disable_isolation(config: &ProjectConfig) -> bool {
    if env::var_os("MOHAUS_MOJO").is_some() {
        return true;
    }
    let version = config.mojo_version.as_str();
    version.contains("dev") || version.contains("nightly")
}

fn editable_mojo_requirement(config: &ProjectConfig) -> Option<OsString> {
    let version = config.mojo_version.as_str();
    if version.contains("dev") || version.contains("nightly") {
        return None;
    }
    Some(OsString::from(format!("mojo=={version}")))
}

fn run_editable_install(
    no_build_isolation: bool,
    mojo_requirement: Option<OsString>,
) -> Result<()> {
    let self_wheelhouse = self_wheelhouse()?;
    if let Some(uv) = mohaus_core::toolchain::find_program_in_path("uv") {
        let mut args = vec![OsString::from("pip")];
        args.extend(editable_install_args(
            no_build_isolation,
            self_wheelhouse.as_ref().map(SelfWheelhouse::arg),
            self_wheelhouse.as_ref().and_then(SelfWheelhouse::wheel_arg),
            mojo_requirement.clone(),
        ));
        return run_status(uv, args);
    }

    let python = mohaus_core::toolchain::find_program_in_path("python")
        .or_else(|| mohaus_core::toolchain::find_program_in_path("python3"))
        .ok_or_else(|| anyhow!("could not find uv, python, or python3 on PATH"))?;
    let mut args = vec![OsString::from("-m"), OsString::from("pip")];
    args.extend(editable_install_args(
        no_build_isolation,
        self_wheelhouse.as_ref().map(SelfWheelhouse::arg),
        self_wheelhouse.as_ref().and_then(SelfWheelhouse::wheel_arg),
        mojo_requirement,
    ));
    run_status(python, args)
}

fn editable_install_args(
    no_build_isolation: bool,
    self_find_links: Option<OsString>,
    self_wheel: Option<OsString>,
    mojo_requirement: Option<OsString>,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("install")];
    if let Some(wheel) = self_wheel {
        args.push(wheel);
    }
    if let Some(requirement) = mojo_requirement {
        args.push(requirement);
    }
    args.push(OsString::from("-e"));
    args.push(OsString::from("."));
    if let Some(find_links) = self_find_links {
        args.push(OsString::from("--find-links"));
        args.push(find_links);
    }
    if no_build_isolation {
        args.push(OsString::from("--no-build-isolation"));
    }
    args
}

fn self_wheelhouse() -> Result<Option<SelfWheelhouse>> {
    if let Some(value) = env::var_os(SELF_WHEEL_ENV).filter(|value| !value.is_empty()) {
        return Ok(Some(SelfWheelhouse::from_wheel(PathBuf::from(value))?));
    }
    let Some(value) = env::var_os(SELF_FIND_LINKS_ENV).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(SelfWheelhouse {
        path: PathBuf::from(value),
        wheel: None,
        cleanup: false,
    }))
}

impl SelfWheelhouse {
    fn from_wheel(wheel: PathBuf) -> Result<Self> {
        if !wheel.is_file() {
            return Err(anyhow!(
                "{} points at a missing wheel: {}",
                SELF_WHEEL_ENV,
                wheel.display()
            ));
        }
        let file_name = wheel
            .file_name()
            .ok_or_else(|| anyhow!("{} has no file name: {}", SELF_WHEEL_ENV, wheel.display()))?;
        let path = env::temp_dir().join(format!(
            "mohaus-self-wheelhouse-{}-{}",
            std::process::id(),
            monotonicish_nanos()
        ));
        fs::create_dir_all(&path)
            .with_context(|| format!("could not create {}", path.display()))?;
        let wheel_path = path.join(file_name);
        fs::copy(&wheel, &wheel_path).with_context(|| {
            format!(
                "could not copy self wheel {} into {}",
                wheel.display(),
                path.display()
            )
        })?;
        Ok(Self {
            path,
            wheel: Some(wheel_path),
            cleanup: true,
        })
    }

    fn arg(&self) -> OsString {
        self.path.as_os_str().to_os_string()
    }

    fn wheel_arg(&self) -> Option<OsString> {
        self.wheel
            .as_ref()
            .map(|wheel| wheel.as_os_str().to_os_string())
    }
}

impl Drop for SelfWheelhouse {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn monotonicish_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn run_status(program: PathBuf, args: Vec<OsString>) -> Result<()> {
    let status = Command::new(&program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {}", program.display()))?;
    if status.success() {
        return Ok(());
    }
    Err(anyhow!("{} exited with {status}", program.display()))
}

fn os_string_error(error: OsString) -> anyhow::Error {
    let printable = os_string_lossy(error);
    anyhow!("could not convert path component `{printable}` to UTF-8")
}

fn os_string_lossy(value: OsString) -> String {
    value.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use clap::{CommandFactory, Parser};
    use clap_complete::{Shell, generate};
    use tempfile::TempDir;

    #[test]
    fn version_exits_successfully() {
        assert!(crate::run_from(["mohaus", "--version"]).is_ok());
    }

    #[test]
    fn init_accepts_explicit_destination_path() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let destination = root.path().join("workspace").join("monpy");

        crate::init(Some("monpy".to_string()), Some(destination.clone()))?;

        assert!(destination.join("src").join("lib.mojo").is_file());
        assert!(
            destination
                .join("python")
                .join("monpy")
                .join("py.typed")
                .is_file()
        );
        let pyproject = fs::read_to_string(destination.join("pyproject.toml"))?;
        assert!(pyproject.contains("name = \"monpy\""));
        Ok(())
    }

    #[test]
    fn completion_alias_parses() -> anyhow::Result<()> {
        let cli = crate::Cli::try_parse_from(["mohaus", "completion", "zsh"])?;
        assert!(matches!(
            cli.command,
            crate::CommandKind::Completions { shell: Shell::Zsh }
        ));
        Ok(())
    }

    #[test]
    fn completion_script_includes_mohaus_commands() -> anyhow::Result<()> {
        let mut command = crate::Cli::command();
        let mut buffer = Vec::new();
        generate(Shell::Bash, &mut command, "mohaus", &mut buffer);

        let script = String::from_utf8(buffer)?;
        assert!(script.contains("init"));
        assert!(script.contains("develop"));
        Ok(())
    }

    #[test]
    fn editable_install_args_include_self_find_links() {
        let args = crate::editable_install_args(
            false,
            Some(OsString::from("/tmp/mohaus-wheelhouse")),
            None,
            None,
        );

        assert_eq!(
            args,
            [
                "install",
                "-e",
                ".",
                "--find-links",
                "/tmp/mohaus-wheelhouse"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn editable_install_args_keep_no_build_isolation_escape_hatch() {
        let args = crate::editable_install_args(true, None, None, None);

        assert_eq!(
            args,
            ["install", "-e", ".", "--no-build-isolation"].map(OsString::from)
        );
    }

    #[test]
    fn editable_install_args_install_self_wheel_as_root_requirement() {
        let args = crate::editable_install_args(
            false,
            Some(OsString::from("/tmp/mohaus-wheelhouse")),
            Some(OsString::from("/tmp/mohaus-wheelhouse/mohaus-0.1.0.whl")),
            None,
        );

        assert_eq!(
            args,
            [
                "install",
                "/tmp/mohaus-wheelhouse/mohaus-0.1.0.whl",
                "-e",
                ".",
                "--find-links",
                "/tmp/mohaus-wheelhouse",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn editable_install_args_install_stable_mojo_as_root_requirement() {
        let args = crate::editable_install_args(
            false,
            Some(OsString::from("/tmp/mohaus-wheelhouse")),
            Some(OsString::from("/tmp/mohaus-wheelhouse/mohaus-0.1.0.whl")),
            Some(OsString::from("mojo==0.26.2.0")),
        );

        assert_eq!(
            args,
            [
                "install",
                "/tmp/mohaus-wheelhouse/mohaus-0.1.0.whl",
                "mojo==0.26.2.0",
                "-e",
                ".",
                "--find-links",
                "/tmp/mohaus-wheelhouse",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn self_wheelhouse_contains_only_the_exact_wheel() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let wheel_name = "mohaus-0.1.0-cp311-abi3-macosx_11_0_arm64.whl";
        let wheel = root.path().join(wheel_name);
        fs::write(&wheel, "")?;

        let wheelhouse = crate::SelfWheelhouse::from_wheel(wheel.clone())?;
        let entries = fs::read_dir(&wheelhouse.path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;

        assert_eq!(entries, vec![wheelhouse.path.join(wheel_name)]);
        assert_eq!(wheelhouse.wheel, Some(wheelhouse.path.join(wheel_name)));
        assert!(
            !wheelhouse
                .path
                .join("mohaus-0.1.0-cp311-abi3-macosx_14_0_arm64.whl")
                .exists()
        );
        Ok(())
    }
}
