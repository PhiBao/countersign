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
    pub allowed_prefix: String,
    pub test_package: String,
    pub repro_test: String,
    pub repro_test_file: String,
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

        Ok(Self {
            base_url: env::var("SWITCH_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into()),
            agent_id: env::var("SWITCH_AGENT_ID").context("SWITCH_AGENT_ID is required")?,
            api_key: env::var("SWITCH_AGENT_KEY").context("SWITCH_AGENT_KEY is required")?,
            role,
            repo,
            ledger_path,
            role_name: env::var("COUNTERSIGN_ROLE_NAME").ok(),
            room_id: env::var("COUNTERSIGN_ROOM_ID").ok().filter(|id| !id.is_empty()),
            allowed_prefix: env::var("COUNTERSIGN_ALLOWED_PREFIX")
                .unwrap_or_else(|_| "demo/orders/src".into()),
            test_package: env::var("COUNTERSIGN_TEST_PACKAGE")
                .unwrap_or_else(|_| "orders-demo".into()),
            repro_test: env::var("COUNTERSIGN_REPRO_TEST")
                .unwrap_or_else(|_| "half_cent_charges_round_up".into()),
            repro_test_file: env::var("COUNTERSIGN_REPRO_TEST_FILE")
                .unwrap_or_else(|_| "checkout".into()),
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
}
