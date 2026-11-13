//! Transpilers that convert other shells/languages into IronShell Shell source.
//!
//! Currently includes:
//! - Bash → IronShell transpiler for a useful subset
//! - Zsh → IronShell transpiler with Zsh-specific features
//! - PowerShell → IronShell transpiler for a useful subset
//! - Agentic → token-minimized syntax for AI agent consumption
//!
//! Re-exported as:
//! - `iron_shell::transpile::bash::transpile_bash_to_ae`
//! - `iron_shell::transpile::zsh::transpile_zsh_to_ae`
//! - `iron_shell::transpile::powershell::transpile_powershell_to_ae`
//! - `iron_shell::transpile::agentic::transpile_agentic_to_ae`

pub mod agentic;
pub mod bash;
pub mod powershell;
pub mod zsh;

/// A construct a transpiler could not translate and handed to `sh(...)`
/// instead: what it was, and the 1-based source line it started on. A caller
/// that does not allow `sh` refuses these by name before anything runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    pub line: usize,
    pub construct: String,
}

/// What a mapped command does to the output: a `Value` command produces a
/// value that a shell would print, a `Quiet` one is run for its effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Value,
    Quiet,
}
