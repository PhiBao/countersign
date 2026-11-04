//! Deterministic verification of a proposed branch.
//!
//! No model is involved in this path. The verifier reproduces the reported
//! failure on the base revision, runs the whole suite on the proposed branch,
//! refuses branches that touch the tests, and records exactly what it ran.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;
use crate::ledger::Verdict;
use crate::process::{run, CommandOutput};

pub struct Outcome {
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub commit_sha: String,
    pub changed_files: Vec<String>,
    pub base_output: String,
    pub suite_output: String,
    pub duration_ms: i64,
}

struct Worktrees {
    base: PathBuf,
    branch: PathBuf,
    repo: PathBuf,
}

impl Worktrees {
    async fn create(repo: &Path, branch: &str, base_ref: &str, id: &str) -> Result<Self> {
        let root = repo.join(".worktrees").join(format!("verify-{id}"));
        let base = root.join("base");
        let branch_dir = root.join("branch");
        let base_string = base.to_string_lossy().to_string();
        let branch_string = branch_dir.to_string_lossy().to_string();

        let add_base = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "--detach".to_string(),
                base_string.clone(),
                base_ref.to_string(),
            ],
            repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !add_base.success {
            bail!("git worktree add (base) failed: {}", add_base.tail(800));
        }

        let add_branch = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "--detach".to_string(),
                branch_string,
                branch.to_string(),
            ],
            repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !add_branch.success {
            let _ = cleanup(repo, &base_string).await;
            bail!("git worktree add (branch) failed: {}", add_branch.tail(800));
        }

        Ok(Self {
            base,
            branch: branch_dir,
            repo: repo.to_path_buf(),
        })
    }

    async fn remove(&self) {
        let _ = run(
            "git",
            &[
                "worktree".to_string(),
                "remove".to_string(),
                "--force".to_string(),
                self.base.to_string_lossy().to_string(),
            ],
            &self.repo,
            &[],
            Duration::from_secs(60),
        )
        .await;
        let _ = run(
            "git",
            &[
                "worktree".to_string(),
                "remove".to_string(),
                "--force".to_string(),
                self.branch.to_string_lossy().to_string(),
            ],
            &self.repo,
            &[],
            Duration::from_secs(60),
        )
        .await;
    }
}

async fn cleanup(repo: &Path, worktree: &str) -> Result<()> {
    let _ = run(
        "git",
        &[
            "worktree".to_string(),
            "remove".to_string(),
            "--force".to_string(),
            worktree.to_string(),
        ],
        repo,
        &[],
        Duration::from_secs(60),
    )
    .await;
    Ok(())
}

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() < 120
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

async fn git(repo: &Path, args: &[&str]) -> Result<CommandOutput> {
    let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    run("git", &args, repo, &[], Duration::from_secs(60)).await
}

/// Verify `branch` against the repository's base revision.
pub async fn verify(config: &Config, branch: &str) -> Result<Outcome> {
    let started = std::time::Instant::now();
    if !valid_branch(branch) {
        bail!("`{branch}` is not a valid branch name");
    }
    if branch == config.base_ref {
        bail!("refusing to verify the base branch against itself");
    }

    let head = git(&config.repo, &["rev-parse", "--verify", branch]).await?;
    if !head.success {
        bail!("I can't find a local branch named `{branch}`");
    }
    let commit_sha = head.combined.trim().to_string();

    let diff = git(
        &config.repo,
        &[
            "diff",
            "--name-only",
            &format!("{}...{}", config.base_ref, branch),
        ],
    )
    .await?;
    if !diff.success {
        bail!("could not diff `{branch}` against `{}`", config.base_ref);
    }
    let changed_files: Vec<String> = diff
        .combined
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();

    if changed_files.is_empty() {
        bail!("`{branch}` has no changes against `{}`", config.base_ref);
    }

    // Separation of duties starts with scope: the proposer may change the
    // implementation, never the tests that judge it, and never anything
    // outside the fixture.
    let denied: Vec<&String> = changed_files
        .iter()
        .filter(|path| path.starts_with("demo/orders/tests/"))
        .collect();
    let outside: Vec<&String> = changed_files
        .iter()
        .filter(|path| !path.starts_with(&config.allowed_prefix))
        .collect();

    if !denied.is_empty() || !outside.is_empty() {
        let mut reason = String::from("Scope check failed. ");
        if !denied.is_empty() {
            reason.push_str(&format!(
                "The branch edits the tests that judge it: {}. ",
                denied
                    .iter()
                    .map(|path| format!("`{path}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !outside.is_empty() {
            reason.push_str(&format!(
                "It also touches files outside `{}`: {}.",
                config.allowed_prefix,
                outside
                    .iter()
                    .map(|path| format!("`{path}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        return Ok(Outcome {
            verdict: Verdict::Fail,
            reason: Some(reason),
            commit_sha,
            changed_files,
            base_output: String::new(),
            suite_output: String::new(),
            duration_ms: started.elapsed().as_millis() as i64,
        });
    }

    let id = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let worktrees = Worktrees::create(&config.repo, branch, &config.base_ref, id).await?;

    let target_dir = config.repo.join(".countersign/target");
    let envs: Vec<(&str, String)> = vec![(
        "CARGO_TARGET_DIR",
        target_dir.to_string_lossy().to_string(),
    )];

    let repro_args = vec![
        "test".to_string(),
        "-p".to_string(),
        config.test_package.clone(),
        "--test".to_string(),
        config.repro_test_file.clone(),
        config.repro_test.clone(),
        "--".to_string(),
        "--exact".to_string(),
    ];
    let base = run(
        "cargo",
        &repro_args,
        &worktrees.base,
        &envs,
        Duration::from_secs(240),
    )
    .await?;

    let suite_args = vec![
        "test".to_string(),
        "-p".to_string(),
        config.test_package.clone(),
    ];
    let suite = run(
        "cargo",
        &suite_args,
        &worktrees.branch,
        &envs,
        Duration::from_secs(240),
    )
    .await?;

    worktrees.remove().await;

    let duration_ms = started.elapsed().as_millis() as i64;

    if base.success {
        return Ok(Outcome {
            verdict: Verdict::Fail,
            reason: Some(format!(
                "The reported failure does not reproduce on `{}`: `{}` already passes. Refusing to certify a fix for a bug that isn't there.",
                config.base_ref, config.repro_test
            )),
            commit_sha,
            changed_files,
            base_output: base.tail(1200),
            suite_output: String::new(),
            duration_ms,
        });
    }

    if !suite.success {
        let failing = suite
            .combined
            .lines()
            .find(|line| line.contains("FAILED") || line.starts_with("test result: FAILED"))
            .or_else(|| suite.combined.lines().find(|line| line.contains("panicked")))
            .unwrap_or("the suite exited non-zero");
        return Ok(Outcome {
            verdict: Verdict::Fail,
            reason: Some(format!(
                "The suite still fails on the branch: {}",
                failing.trim()
            )),
            commit_sha,
            changed_files,
            base_output: base.tail(1200),
            suite_output: suite.tail(2400),
            duration_ms,
        });
    }

    Ok(Outcome {
        verdict: Verdict::Pass,
        reason: None,
        commit_sha,
        changed_files,
        base_output: base.tail(1200),
        suite_output: suite.tail(2400),
        duration_ms,
    })
}

pub fn receipt_id() -> String {
    format!(
        "R-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..6].to_uppercase()
    )
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}
