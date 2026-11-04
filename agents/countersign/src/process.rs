use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub combined: String,
}

impl CommandOutput {
    pub fn tail(&self, limit: usize) -> String {
        let text = self.combined.trim();
        if text.len() <= limit {
            return text.to_string();
        }
        let cut = text.len() - limit;
        // Do not split a UTF-8 character.
        let start = text
            .char_indices()
            .find(|(index, _)| *index >= cut)
            .map(|(index, _)| index)
            .unwrap_or(text.len());
        format!("…{}", &text[start..])
    }
}

fn remove_agent_secrets(command: &mut Command) {
    // Child processes never need the credentials this process uses to talk to
    // Switch, and branch code must not be able to read them.
    for (key, _) in std::env::vars() {
        if key.starts_with("SWITCH_") || key.starts_with("COUNTERSIGN_") {
            command.env_remove(&key);
        }
    }
}

fn scrub_environment(command: &mut Command) {
    command.env_clear();
    for key in [
        "PATH",
        "HOME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "TMPDIR",
        "TMP",
        "LANG",
        "LC_ALL",
        "TERM",
    ] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
}

pub async fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
) -> Result<CommandOutput> {
    run_with(program, args, cwd, envs, timeout, false).await
}

/// Run a command with a minimal environment. Used for the verifier's test
/// runs, where the code being executed is untrusted.
pub async fn run_scrubbed(
    program: &str,
    args: &[String],
    cwd: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
) -> Result<CommandOutput> {
    run_with(program, args, cwd, envs, timeout, true).await
}

async fn run_with(
    program: &str,
    args: &[String],
    cwd: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
    scrubbed: bool,
) -> Result<CommandOutput> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if scrubbed {
        scrub_environment(&mut command);
    } else {
        remove_agent_secrets(&mut command);
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    let child = command.output();
    let output = tokio::time::timeout(timeout, child)
        .await
        .with_context(|| format!("{program} timed out after {timeout:?}"))?
        .with_context(|| format!("run {program} in {}", cwd.display()))?;

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.stderr.is_empty() {
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(CommandOutput {
        success: output.status.success(),
        combined,
    })
}
