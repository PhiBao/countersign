use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub combined: String,
    pub duration: Duration,
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

pub async fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
) -> Result<CommandOutput> {
    let started = Instant::now();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in envs {
        command.env(key, value);
    }
    let child = command
        .output();
    let output = tokio::time::timeout(timeout, child)
        .await
        .with_context(|| format!("{program} timed out after {:?}", timeout))?
        .with_context(|| format!("run {program} in {}", cwd.display()))?;

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.stderr.is_empty() {
        combined.push_str("\n");
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(CommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        combined,
        duration: started.elapsed(),
    })
}
