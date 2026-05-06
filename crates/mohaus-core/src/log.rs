use std::env;
use std::ffi::OsString;

/// Environment key used to carry CLI verbosity into backend subprocesses.
pub const VERBOSITY_ENV: &str = "MOHAUS_VERBOSITY";

/// Explicit verbosity level for human-facing diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Verbosity(u8);

impl Verbosity {
    /// Construct a verbosity level from repeated `-v` flags.
    pub const fn new(count: u8) -> Self {
        Self(count)
    }

    /// Read verbosity propagated through the process environment.
    pub fn from_env() -> Self {
        env::var_os(VERBOSITY_ENV)
            .and_then(|value| value.to_str().and_then(|text| text.parse::<u8>().ok()))
            .map_or_else(Self::default, Self::new)
    }

    /// Raw counter value.
    pub const fn count(self) -> u8 {
        self.0
    }

    /// Whether any verbosity was requested.
    pub const fn is_enabled(self) -> bool {
        self.0 > 0
    }

    /// Whether this verbosity includes a diagnostic level.
    pub const fn at_least(self, level: u8) -> bool {
        self.0 >= level
    }

    /// Value to write into [`VERBOSITY_ENV`] for child backend processes.
    pub fn env_value(self) -> OsString {
        OsString::from(self.0.to_string())
    }

    /// Repeated `-v` flags suitable for tools with uv/pip-style verbosity.
    pub fn flag_args(self) -> Vec<OsString> {
        (0..self.0).map(|_| OsString::from("-v")).collect()
    }
}

pub(crate) fn debug(verbosity: Verbosity, level: u8, message: impl FnOnce() -> String) {
    if verbosity.at_least(level) {
        eprintln!("mohaus: {}", message());
    }
}
