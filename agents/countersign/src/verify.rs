//! Deterministic verification of a proposed branch.
//!
//! No model is involved in this path. The verifier reproduces the reported
//! failure on the base revision, runs the whole suite on the proposed branch,
//! refuses branches that touch the tests, and records exactly what it ran.
//!
//! One worktree is used for both runs so cargo sees one path and cannot reuse
//! a stale test binary from the other revision.

use anyhow::{bail, Result};
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

struct Worktree {
    dir: PathBuf,
    repo: PathBuf,
}

impl Worktree {
    async fn create(repo: &Path, base_ref: &str, id: &str) -> Result<Self> {
        let dir = repo.join(".worktrees").join(format!("verify-{id}"));
        let added = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "--detach".to_string(),
                dir.to_string_lossy().to_string(),
                base_ref.to_string(),
            ],
            repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !added.success {
            bail!("git worktree add failed: {}", added.tail(800));
        }
        Ok(Self {
            dir,
            repo: repo.to_path_buf(),
        })
    }

    async fn checkout(&self, branch: &str) -> Result<()> {
        let checked = run(
            "git",
            &[
                "checkout".to_string(),
                "--detach".to_string(),
                branch.to_string(),
            ],
            &self.dir,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !checked.success {
            bail!("git checkout {branch} failed: {}", checked.tail(600));
        }
        Ok(())
    }

    async fn remove(&self) {
        let _ = run(
            "git",
            &[
                "worktree".to_string(),
                "remove".to_string(),
                "--force".to_string(),
                self.dir.to_string_lossy().to_string(),
            ],
            &self.repo,
            &[],
            Duration::from_secs(60),
        )
        .await;
    }
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
    let worktree = Worktree::create(&config.repo, &config.base_ref, id).await?;

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
    let base = match run(
        "cargo",
        &repro_args,
        &worktree.dir,
        &envs,
        Duration::from_secs(240),
    )
    .await
    {
        Ok(output) => output,
        Err(error) => {
            worktree.remove().await;
            return Err(error);
        }
    };

    if let Err(error) = worktree.checkout(branch).await {
        worktree.remove().await;
        return Err(error);
    }

    let suite_args = vec![
        "test".to_string(),
        "-p".to_string(),
        config.test_package.clone(),
    ];
    let suite = match run(
        "cargo",
        &suite_args,
        &worktree.dir,
        &envs,
        Duration::from_secs(240),
    )
    .await
    {
        Ok(output) => output,
        Err(error) => {
            worktree.remove().await;
            return Err(error);
        }
    };

    worktree.remove().await;
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
            base_output: cap(&base.combined, 20_000),
            suite_output: String::new(),
            duration_ms,
        });
    }

    if !suite.success {
        let failing: Vec<String> = suite
            .combined
            .lines()
            .filter(|line| {
                line.trim_start().starts_with("test ") && line.trim_end().ends_with("FAILED")
            })
            .map(|line| {
                line.trim()
                    .trim_end_matches("FAILED")
                    .trim()
                    .trim_end_matches("...")
                    .trim()
                    .trim_start_matches("test ")
                    .to_string()
            })
            .collect();
        let detail = if failing.is_empty() {
            "the suite exited non-zero".to_string()
        } else {
            failing
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Ok(Outcome {
            verdict: Verdict::Fail,
            reason: Some(format!(
                "The suite still fails on the branch: {detail}."
            )),
            commit_sha,
            changed_files,
            base_output: cap(&base.combined, 20_000),
            suite_output: cap(&suite.combined, 20_000),
            duration_ms,
        });
    }

    Ok(Outcome {
        verdict: Verdict::Pass,
        reason: None,
        commit_sha,
        changed_files,
        base_output: cap(&base.combined, 20_000),
        suite_output: cap(&suite.combined, 20_000),
        duration_ms,
    })
}

fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let start = text
        .char_indices()
        .find(|(index, _)| *index >= text.len() - limit)
        .map(|(index, _)| index)
        .unwrap_or(0);
    text[start..].to_string()
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

#[cfg(test)]
mod tests {
    use super::valid_branch;

    #[test]
    fn accepts_normal_branch_names() {
        assert!(valid_branch("fix/half-cent-1a2b3c"));
        assert!(valid_branch("hotfix/naive-rounding"));
    }

    #[test]
    fn rejects_dangerous_names() {
        assert!(!valid_branch(""));
        assert!(!valid_branch("-rf"));
        assert!(!valid_branch("a..b"));
        assert!(!valid_branch("fix branch"));
        assert!(!valid_branch("fix;rm"));
    }
}
