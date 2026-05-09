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
use clap::{ArgAction, CommandFactory, Parser, Subcommand, error::ErrorKind};
use clap_complete::{Shell, generate};
use mohaus_core::{
    BuildOptions, ProjectConfig, PythonInfo, SdistOptions, VERBOSITY_ENV, Verbosity, build_sdist,
    build_wheel, ensure_editable_built_with_verbosity,
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
    /// Increase diagnostic output. Repeat for more detail, e.g. -vvv.
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true)]
    verbose: u8,

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

        /// Forward extra args to the installer after mohaus' editable args.
        #[arg(last = true)]
        passthrough: Vec<String>,
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

    /// Add a dependency to pyproject.toml. Defaults to a Python package via
    /// `uv add`. With `--mojo` adds an entry to `tool.mohaus.mojo-include-paths`.
    Add {
        /// Package specifier. Python form: `name`, `name==1.2`, `name @ url`,
        /// or a local `./path`. Mojo form: a local include path, `owner/repo`,
        /// `github:owner/repo`, or a git URL.
        spec: String,

        /// Treat the spec as a Mojo package. Git specs are cloned into vendor/.
        #[arg(long)]
        mojo: bool,

        /// Pin into `[project.optional-dependencies] <extra>` (Python only).
        #[arg(long)]
        extra: Option<String>,

        /// Pin into `[dependency-groups] <group>` via `uv add --group` (Python only).
        #[arg(long)]
        group: Option<String>,

        /// Pin as a build-system requirement (`requires` in `[build-system]`).
        /// Useful for mojo nightly pins.
        #[arg(long)]
        build_system: bool,

        /// Forward extra args to `uv add` after the spec (Python only).
        #[arg(last = true)]
        passthrough: Vec<String>,
    },
}

fn run(cli: Cli) -> Result<()> {
    let verbosity = Verbosity::new(cli.verbose);
    match cli.command {
        CommandKind::Init { name, path } => init(name, path, verbosity),
        CommandKind::New { name } => new_project(name, verbosity),
        CommandKind::Completions { shell } => completions(shell),
        CommandKind::Build { release, out } => build(release, out, verbosity),
        CommandKind::Develop {
            no_build_isolation,
            passthrough,
        } => develop(no_build_isolation, passthrough, verbosity),
        CommandKind::Sdist { out } => sdist(out, verbosity),
        CommandKind::Watch { interval_ms } => watch(interval_ms, verbosity),
        CommandKind::Add {
            spec,
            mojo,
            extra,
            group,
            build_system,
            passthrough,
        } => add(
            spec,
            mojo,
            extra,
            group,
            build_system,
            passthrough,
            verbosity,
        ),
    }
}

fn init(name: Option<String>, path: Option<PathBuf>, verbosity: Verbosity) -> Result<()> {
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
    log(verbosity, 1, || {
        format!("scaffolding project {name} into {}", destination.display())
    });
    scaffold_project(&ScaffoldOptions { name, destination })?;
    Ok(())
}

fn new_project(name: String, verbosity: Verbosity) -> Result<()> {
    let cwd = env::current_dir().context("could not read current directory")?;
    log(verbosity, 1, || {
        format!(
            "scaffolding project {name} into {}",
            cwd.join(&name).display()
        )
    });
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

fn build(release: bool, out: PathBuf, verbosity: Verbosity) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    log(verbosity, 1, || {
        format!("building wheel from {}", project_dir.display())
    });
    let python = PythonInfo::detect()?;
    let wheel = build_wheel(&BuildOptions {
        project_dir,
        out_dir: out,
        python,
        release,
        verbosity,
        metadata_dir: None,
    })?;
    println!("{}", wheel.display());
    Ok(())
}

fn develop(no_build_isolation: bool, passthrough: Vec<String>, verbosity: Verbosity) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let config = ProjectConfig::load(&project_dir)?;
    let disable_isolation = no_build_isolation || should_disable_isolation(&config);
    log(verbosity, 1, || {
        format!(
            "installing editable {} from {}",
            config.package.as_str(),
            project_dir.display()
        )
    });
    log(verbosity, 2, || {
        format!("build isolation disabled: {disable_isolation}")
    });
    run_editable_install(
        disable_isolation,
        editable_mojo_requirement(&config),
        passthrough,
        verbosity,
    )
}

fn sdist(out: PathBuf, verbosity: Verbosity) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    log(verbosity, 1, || {
        format!(
            "building source distribution from {}",
            project_dir.display()
        )
    });
    let archive = build_sdist(&SdistOptions {
        project_dir,
        out_dir: out,
    })?;
    println!("{}", archive.display());
    Ok(())
}

fn add(
    spec: String,
    mojo: bool,
    extra: Option<String>,
    group: Option<String>,
    build_system: bool,
    passthrough: Vec<String>,
    verbosity: Verbosity,
) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    if mojo {
        if extra.is_some() || group.is_some() || !passthrough.is_empty() {
            return Err(anyhow!(
                "--mojo is incompatible with --extra, --group, or trailing uv args"
            ));
        }
        return add_mojo_dependency(&project_dir, &spec, build_system, verbosity);
    }
    add_python_dependency(
        &project_dir,
        &spec,
        extra,
        group,
        build_system,
        passthrough,
        verbosity,
    )
}

fn add_python_dependency(
    project_dir: &Path,
    spec: &str,
    extra: Option<String>,
    group: Option<String>,
    build_system: bool,
    passthrough: Vec<String>,
    verbosity: Verbosity,
) -> Result<()> {
    if build_system {
        let pyproject = project_dir.join("pyproject.toml");
        log(verbosity, 1, || {
            format!(
                "adding build-system requirement {spec} to {}",
                pyproject.display()
            )
        });
        mohaus_core::pyproject_edit::add_build_system_requirement(&pyproject, spec)?;
        return Ok(());
    }
    let uv = mohaus_core::toolchain::find_program_in_path("uv")
        .ok_or_else(|| anyhow!("`uv` is not on PATH; install uv to use `mohaus add`"))?;
    let mut args = verbosity.flag_args();
    args.push(OsString::from("add"));
    if let Some(value) = extra {
        args.push(OsString::from("--optional"));
        args.push(OsString::from(value));
    }
    if let Some(value) = group {
        args.push(OsString::from("--group"));
        args.push(OsString::from(value));
    }
    args.push(OsString::from(spec));
    for value in passthrough {
        args.push(OsString::from(value));
    }
    run_status(uv, args, verbosity)
}

fn add_mojo_dependency(
    project_dir: &Path,
    spec: &str,
    build_system: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let pyproject = project_dir.join("pyproject.toml");
    if !pyproject.is_file() {
        return Err(anyhow!("no pyproject.toml at {}", pyproject.display()));
    }
    if build_system {
        log(verbosity, 1, || {
            format!(
                "adding build-system requirement {spec} to {}",
                pyproject.display()
            )
        });
        mohaus_core::pyproject_edit::add_build_system_requirement(&pyproject, spec)?;
        return Ok(());
    }
    let resolved = mohaus_core::pyproject_edit::resolve_mojo_dependency(project_dir, spec)?;
    if let mohaus_core::pyproject_edit::ResolvedMojoDependency::Git {
        url, checkout_dir, ..
    } = &resolved
    {
        ensure_git_mojo_checkout(url, checkout_dir, verbosity)?;
    }
    let include_path = resolved.include_path();
    log(verbosity, 1, || {
        format!(
            "adding Mojo include path {} to {}",
            include_path,
            pyproject.display()
        )
    });
    mohaus_core::pyproject_edit::add_mojo_include_path(&pyproject, include_path)?;
    println!("added mojo include path: {include_path}");
    Ok(())
}

fn ensure_git_mojo_checkout(url: &str, checkout_dir: &Path, verbosity: Verbosity) -> Result<()> {
    if checkout_dir.exists() {
        if checkout_dir.is_dir() {
            log(verbosity, 1, || {
                format!(
                    "using existing Mojo git checkout at {}",
                    checkout_dir.display()
                )
            });
            return Ok(());
        }
        return Err(anyhow!(
            "cannot clone Mojo git dependency into {}; path exists and is not a directory",
            checkout_dir.display()
        ));
    }

    let parent = checkout_dir
        .parent()
        .ok_or_else(|| anyhow!("cannot determine parent for {}", checkout_dir.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let git = mohaus_core::toolchain::find_program_in_path("git").ok_or_else(|| {
        anyhow!("`git` is not on PATH; install git to add Mojo dependencies from git")
    })?;
    log(verbosity, 1, || {
        format!(
            "cloning Mojo git dependency {url} into {}",
            checkout_dir.display()
        )
    });
    let args = vec![
        OsString::from("clone"),
        OsString::from("--depth"),
        OsString::from("1"),
        OsString::from(url),
        checkout_dir.as_os_str().to_os_string(),
    ];
    let result = run_status(git, args, verbosity);
    if result.is_err() {
        let _ = fs::remove_dir_all(checkout_dir);
    }
    result.with_context(|| {
        format!(
            "failed to clone Mojo git dependency {url} into {}",
            checkout_dir.display()
        )
    })
}

fn watch(interval_ms: u64, verbosity: Verbosity) -> Result<()> {
    let project_dir = env::current_dir().context("could not read current directory")?;
    let python = PythonInfo::detect()?;
    let config = ProjectConfig::load(&project_dir)?;
    let interval = Duration::from_millis(interval_ms);
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(interval, tx)
        .map_err(|error| anyhow!("could not create filesystem watcher: {error}"))?;

    let watch_roots = watch_roots(&project_dir, &config);
    log(verbosity, 2, || {
        format!("watch roots: {}", format_paths(&watch_roots))
    });
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
    ensure_editable_built_with_verbosity(&project_dir, &python, verbosity)?;
    eprintln!(
        "mohaus watch: ready ({} roots, debounce {interval_ms}ms)",
        watch_roots.len()
    );
    while let Ok(event) = rx.recv() {
        match event {
            Ok(events) if relevant_events(&events) => {
                if let Err(error) =
                    ensure_editable_built_with_verbosity(&project_dir, &python, verbosity)
                {
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
    let Some(version) = config.mojo_version.as_ref() else {
        return false;
    };
    let value = version.as_str();
    value.contains("dev") || value.contains("nightly")
}

fn editable_mojo_requirement(config: &ProjectConfig) -> Option<OsString> {
    let version = config.mojo_version.as_ref()?.as_str();
    if version.contains("dev") || version.contains("nightly") {
        return None;
    }
    Some(OsString::from(format!("mojo=={version}")))
}

fn run_editable_install(
    no_build_isolation: bool,
    mojo_requirement: Option<OsString>,
    passthrough: Vec<String>,
    verbosity: Verbosity,
) -> Result<()> {
    let self_wheelhouse = self_wheelhouse()?;
    if let Some(wheelhouse) = &self_wheelhouse {
        log(verbosity, 1, || {
            format!("using mohaus self wheelhouse {}", wheelhouse.path.display())
        });
    }
    if let Some(uv) = mohaus_core::toolchain::find_program_in_path("uv") {
        let args = uv_pip_install_args(
            verbosity,
            no_build_isolation,
            self_wheelhouse.as_ref().map(SelfWheelhouse::arg),
            self_wheelhouse.as_ref().and_then(SelfWheelhouse::wheel_arg),
            mojo_requirement.clone(),
            &passthrough,
        );
        return run_status(uv, args, verbosity);
    }

    let python = mohaus_core::toolchain::find_program_in_path("python")
        .or_else(|| mohaus_core::toolchain::find_program_in_path("python3"))
        .ok_or_else(|| anyhow!("could not find uv, python, or python3 on PATH"))?;
    let args = python_pip_install_args(
        verbosity,
        no_build_isolation,
        self_wheelhouse.as_ref().map(SelfWheelhouse::arg),
        self_wheelhouse.as_ref().and_then(SelfWheelhouse::wheel_arg),
        mojo_requirement,
        &passthrough,
    );
    run_status(python, args, verbosity)
}

fn uv_pip_install_args(
    verbosity: Verbosity,
    no_build_isolation: bool,
    self_find_links: Option<OsString>,
    self_wheel: Option<OsString>,
    mojo_requirement: Option<OsString>,
    passthrough: &[String],
) -> Vec<OsString> {
    let mut args = verbosity.flag_args();
    args.push(OsString::from("pip"));
    args.extend(editable_install_args(
        no_build_isolation,
        self_find_links,
        self_wheel,
        mojo_requirement,
        passthrough,
    ));
    args
}

fn python_pip_install_args(
    verbosity: Verbosity,
    no_build_isolation: bool,
    self_find_links: Option<OsString>,
    self_wheel: Option<OsString>,
    mojo_requirement: Option<OsString>,
    passthrough: &[String],
) -> Vec<OsString> {
    let mut args = vec![OsString::from("-m"), OsString::from("pip")];
    args.extend(verbosity.flag_args());
    args.extend(editable_install_args(
        no_build_isolation,
        self_find_links,
        self_wheel,
        mojo_requirement,
        passthrough,
    ));
    args
}

fn editable_install_args(
    no_build_isolation: bool,
    self_find_links: Option<OsString>,
    self_wheel: Option<OsString>,
    mojo_requirement: Option<OsString>,
    passthrough: &[String],
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
    for value in passthrough {
        args.push(OsString::from(value.as_str()));
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

fn run_status(program: PathBuf, args: Vec<OsString>, verbosity: Verbosity) -> Result<()> {
    let mut command = Command::new(&program);
    command.args(&args);
    if verbosity.is_enabled() {
        command.env(VERBOSITY_ENV, verbosity.env_value());
    }
    log(verbosity, 1, || {
        format!("running {}", format_command(&program, &args))
    });
    if verbosity.is_enabled() {
        log(verbosity, 2, || {
            format!("setting child {VERBOSITY_ENV}={}", verbosity.count())
        });
    }
    let status = command
        .status()
        .with_context(|| format!("failed to run {}", program.display()))?;
    if status.success() {
        log(verbosity, 2, || {
            format!("{} exited with {status}", program.display())
        });
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

fn log(verbosity: Verbosity, level: u8, message: impl FnOnce() -> String) {
    if verbosity.at_least(level) {
        eprintln!("mohaus: {}", message());
    }
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "<none>".to_string();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_command(program: &Path, args: &[OsString]) -> String {
    std::iter::once(format_arg(program.as_os_str()))
        .chain(args.iter().map(|arg| format_arg(arg.as_os_str())))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_arg(arg: &std::ffi::OsStr) -> String {
    let value = arg.to_string_lossy();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '=' | ':'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use clap::{CommandFactory, Parser};
    use clap_complete::{Shell, generate};
    use mohaus_core::Verbosity;
    use tempfile::TempDir;

    #[test]
    fn version_exits_successfully() {
        assert!(crate::run_from(["mohaus", "--version"]).is_ok());
    }

    #[test]
    fn init_accepts_explicit_destination_path() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let destination = root.path().join("workspace").join("monpy");

        crate::init(
            Some("monpy".to_string()),
            Some(destination.clone()),
            Verbosity::default(),
        )?;

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
    fn add_mojo_flag_routes_to_include_paths() -> anyhow::Result<()> {
        let cli = crate::Cli::try_parse_from(["mohaus", "add", "--mojo", "vendor/some_pkg"])?;
        match cli.command {
            crate::CommandKind::Add {
                spec,
                mojo,
                extra,
                group,
                build_system,
                passthrough,
            } => {
                assert_eq!(spec, "vendor/some_pkg");
                assert!(mojo);
                assert!(!build_system);
                assert!(extra.is_none());
                assert!(group.is_none());
                assert!(passthrough.is_empty());
            }
            other => panic!("expected Add, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn add_python_default_passes_through_uv_args() -> anyhow::Result<()> {
        let cli =
            crate::Cli::try_parse_from(["mohaus", "add", "numpy>=1", "--", "--prerelease=allow"])?;
        match cli.command {
            crate::CommandKind::Add {
                spec,
                mojo,
                passthrough,
                ..
            } => {
                assert_eq!(spec, "numpy>=1");
                assert!(!mojo);
                assert_eq!(passthrough, vec!["--prerelease=allow".to_string()]);
            }
            other => panic!("expected Add, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn develop_passes_through_installer_args_after_separator() -> anyhow::Result<()> {
        let cli = crate::Cli::try_parse_from([
            "mohaus",
            "develop",
            "--no-build-isolation",
            "--",
            "--refresh-package",
            "mohaus",
        ])?;
        match cli.command {
            crate::CommandKind::Develop {
                no_build_isolation,
                passthrough,
            } => {
                assert!(no_build_isolation);
                assert_eq!(
                    passthrough,
                    vec!["--refresh-package".to_string(), "mohaus".to_string()]
                );
            }
            other => panic!("expected Develop, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn verbose_counter_parses_before_subcommand() -> anyhow::Result<()> {
        let cli = crate::Cli::try_parse_from(["mohaus", "-vvv", "build"])?;

        assert_eq!(cli.verbose, 3);
        assert!(matches!(cli.command, crate::CommandKind::Build { .. }));
        Ok(())
    }

    #[test]
    fn verbose_counter_parses_after_subcommand() -> anyhow::Result<()> {
        let cli = crate::Cli::try_parse_from(["mohaus", "develop", "-vv"])?;

        assert_eq!(cli.verbose, 2);
        assert!(matches!(cli.command, crate::CommandKind::Develop { .. }));
        Ok(())
    }

    #[test]
    fn add_mojo_dependency_appends_to_pyproject() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let project = root.path();
        fs::create_dir_all(project.join("vendor/some_pkg"))?;
        fs::write(
            project.join("pyproject.toml"),
            "[build-system]\n\
             requires = [\"mohaus>=0.1,<0.2\"]\n\
             build-backend = \"mohaus.backend\"\n\n\
             [project]\n\
             name = \"demo\"\n\
             version = \"0.1.0\"\n\n\
             [tool.mohaus]\n\
             mojo-include-paths = []\n",
        )?;
        crate::add_mojo_dependency(project, "vendor/some_pkg", false, Verbosity::default())?;
        let updated = fs::read_to_string(project.join("pyproject.toml"))?;
        assert!(updated.contains("\"vendor/some_pkg\","));
        Ok(())
    }

    #[test]
    fn add_mojo_git_dependency_uses_existing_vendor_checkout() -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let project = root.path();
        fs::create_dir_all(project.join("vendor/NuMojo"))?;
        fs::write(
            project.join("pyproject.toml"),
            "[build-system]\n\
             requires = [\"mohaus>=0.1,<0.2\"]\n\
             build-backend = \"mohaus.backend\"\n\n\
             [project]\n\
             name = \"demo\"\n\
             version = \"0.1.0\"\n\n\
             [tool.mohaus]\n\
             mojo-include-paths = []\n",
        )?;

        crate::add_mojo_dependency(
            project,
            "github:Mojo-Numerics-and-Algorithms-group/NuMojo",
            false,
            Verbosity::default(),
        )?;

        let updated = fs::read_to_string(project.join("pyproject.toml"))?;
        assert!(updated.contains("\"vendor/NuMojo\","));
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
            &[],
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
    fn uv_pip_install_args_forward_repeated_verbose_flags_before_pip() {
        let args = crate::uv_pip_install_args(Verbosity::new(3), false, None, None, None, &[]);

        assert_eq!(
            args,
            ["-v", "-v", "-v", "pip", "install", "-e", "."].map(OsString::from)
        );
    }

    #[test]
    fn python_pip_install_args_forward_repeated_verbose_flags_before_install() {
        let args = crate::python_pip_install_args(Verbosity::new(2), true, None, None, None, &[]);

        assert_eq!(
            args,
            [
                "-m",
                "pip",
                "-v",
                "-v",
                "install",
                "-e",
                ".",
                "--no-build-isolation"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn editable_install_args_keep_no_build_isolation_escape_hatch() {
        let args = crate::editable_install_args(true, None, None, None, &[]);

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
            &[],
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
            &[],
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
    fn editable_install_args_append_passthrough_after_owned_args() {
        let passthrough = vec![
            "--reinstall".to_string(),
            "--refresh-package".to_string(),
            "mohaus".to_string(),
        ];
        let args = crate::editable_install_args(
            true,
            Some(OsString::from("/tmp/mohaus-wheelhouse")),
            None,
            None,
            &passthrough,
        );

        assert_eq!(
            args,
            [
                "install",
                "-e",
                ".",
                "--find-links",
                "/tmp/mohaus-wheelhouse",
                "--no-build-isolation",
                "--reinstall",
                "--refresh-package",
                "mohaus",
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
