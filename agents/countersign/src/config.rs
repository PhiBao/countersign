use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Fixer,
    Verifier,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Fixer => "fixer",
            Role::Verifier => "verifier",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub agent_id: String,
    pub api_key: String,
    pub role: Role,
    pub repo: PathBuf,
    pub ledger_path: PathBuf,
    pub role_name: Option<String>,
    pub room_id: Option<String>,
    pub viewer_url: Option<String>,
    /// File prefixes the proposer may touch.
    pub allowed_paths: Vec<String>,
    /// File prefixes that can never change, whatever else is allowed.
    pub denied_paths: Vec<String>,
    /// The full suite, as argv (no shell).
    pub test_command: Vec<String>,
    /// The reported failing test, as argv (no shell).
    pub repro_command: Vec<String>,
    /// People allowed to countersign. Empty means any human in the room.
    pub approvers: Vec<String>,
    /// How many agent attempts a single change may take before escalation.
    pub max_attempts: u8,
    pub pr_mode: PrMode,
    pub github_repo: Option<String>,
    pub base_ref: String,
    pub fixer_provider: FixerProvider,
    pub fixer_model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixerProvider {
    OpenCode,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrMode {
    /// Land the change as a real pull request with `gh`.
    Gh,
    /// Land the change by merging into `countersign/landed` locally.
    Local,
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// Commands are argv, not shell strings. Whitespace separates arguments; a
/// future version can switch to a structured config without changing callers.
fn split_command(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn command_or(value: Result<String, env::VarError>, default: &[&str]) -> Vec<String> {
    match value {
        Ok(command) if !command.trim().is_empty() => split_command(&command),
        _ => default.iter().map(|part| part.to_string()).collect(),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let role = match env::var("COUNTERSIGN_ROLE").as_deref() {
            Ok("fixer") => Role::Fixer,
            Ok("verifier") => Role::Verifier,
            other => anyhow::bail!(
                "COUNTERSIGN_ROLE must be 'fixer' or 'verifier' (got {other:?})"
            ),
        };

        let repo = match env::var("COUNTERSIGN_REPO") {
            Ok(path) => PathBuf::from(path),
            Err(_) => env::current_dir().context("resolve working directory")?,
        };
        let repo = repo
            .canonicalize()
            .with_context(|| format!("resolve repository path {}", repo.display()))?;

        let ledger_path = match env::var("COUNTERSIGN_LEDGER") {
            Ok(path) => PathBuf::from(path),
            Err(_) => repo.join(".countersign/ledger.sqlite"),
        };

        let pr_mode = match env::var("COUNTERSIGN_PR_MODE").as_deref() {
            Ok("gh") => PrMode::Gh,
            Ok("local") | Err(_) => PrMode::Local,
            other => anyhow::bail!("COUNTERSIGN_PR_MODE must be 'gh' or 'local' (got {other:?})"),
        };

        let package = env::var("COUNTERSIGN_TEST_PACKAGE").unwrap_or_else(|_| "orders-demo".into());
        let repro_file =
            env::var("COUNTERSIGN_REPRO_TEST_FILE").unwrap_or_else(|_| "checkout".into());
        let repro_test = env::var("COUNTERSIGN_REPRO_TEST")
            .unwrap_or_else(|_| "half_cent_charges_round_up".into());

        let default_test = vec!["cargo", "test", "-p", &package];
        let default_repro = vec![
            "cargo",
            "test",
            "-p",
            &package,
            "--test",
            &repro_file,
            &repro_test,
            "--",
            "--exact",
        ];

        let test_command = command_or(env::var("COUNTERSIGN_TEST_COMMAND"), &default_test);
        let repro_command = command_or(env::var("COUNTERSIGN_REPRO_COMMAND"), &default_repro);
        if test_command.is_empty() || repro_command.is_empty() {
            anyhow::bail!("test and repro commands must not be empty");
        }

        let allowed_paths = env::var("COUNTERSIGN_ALLOWED_PATHS")
            .map(|value| split_csv(&value))
            .unwrap_or_else(|_| {
                vec![env::var("COUNTERSIGN_ALLOWED_PREFIX")
                    .unwrap_or_else(|_| "demo/orders/src".into())]
            });
        let denied_paths = env::var("COUNTERSIGN_DENIED_PATHS")
            .map(|value| split_csv(&value))
            .unwrap_or_else(|_| vec!["demo/orders/tests".into()]);
        if allowed_paths.is_empty() {
            anyhow::bail!("COUNTERSIGN_ALLOWED_PATHS must not be empty");
        }

        let approvers = env::var("COUNTERSIGN_APPROVERS")
            .map(|value| split_csv(&value))
            .unwrap_or_default();

        let max_attempts = env::var("COUNTERSIGN_MAX_ATTEMPTS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2)
            .clamp(1, 5);

        Ok(Self {
            base_url: env::var("SWITCH_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into()),
            agent_id: env::var("SWITCH_AGENT_ID").context("SWITCH_AGENT_ID is required")?,
            api_key: env::var("SWITCH_AGENT_KEY").context("SWITCH_AGENT_KEY is required")?,
            role,
            repo,
            ledger_path,
            role_name: env::var("COUNTERSIGN_ROLE_NAME").ok(),
            room_id: env::var("COUNTERSIGN_ROOM_ID").ok().filter(|id| !id.is_empty()),
            viewer_url: env::var("COUNTERSIGN_VIEWER_URL").ok().filter(|url| !url.is_empty()),
            allowed_paths,
            denied_paths,
            test_command,
            repro_command,
            approvers,
            max_attempts,
            pr_mode,
            github_repo: env::var("COUNTERSIGN_GITHUB_REPO").ok(),
            base_ref: env::var("COUNTERSIGN_BASE_REF").unwrap_or_else(|_| "main".into()),
            fixer_provider: match env::var("COUNTERSIGN_FIXER_PROVIDER").as_deref() {
                Ok("codex") => FixerProvider::Codex,
                Ok("opencode") | Err(_) => FixerProvider::OpenCode,
                other => anyhow::bail!(
                    "COUNTERSIGN_FIXER_PROVIDER must be 'opencode' or 'codex' (got {other:?})"
                ),
            },
            fixer_model: env::var("COUNTERSIGN_FIXER_MODEL")
                .unwrap_or_else(|_| "opencode-go/gpt-5.6-luna".into()),
        })
    }

    pub fn is_approver(&self, name: &str) -> bool {
        self.approvers.is_empty()
            || self
                .approvers
                .iter()
                .any(|approver| approver.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::{split_command, split_csv};

    #[test]
    fn splits_commands_without_a_shell() {
        assert_eq!(
            split_command("cargo test -p orders-demo"),
            vec!["cargo", "test", "-p", "orders-demo"]
        );
    }

    #[test]
    fn splits_csv_and_ignores_empty_entries() {
        assert_eq!(split_csv("Ada, , Grace"), vec!["Ada", "Grace"]);
    }
}
