//! Deterministic verification of a proposed branch.
//!
//! No model is involved in this path. The verifier reproduces the reported
//! failure on the base revision, runs the whole suite on the proposed branch,
//! refuses branches that touch the tests, and records exactly what it ran.
//!
//! One worktree is used for both runs so cargo sees one path and cannot reuse
//! a stale test binary from the other revision.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;
use crate::ledger::Verdict;
use crate::process::{run, run_scrubbed, CommandOutput};

pub struct Outcome {
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub commit_sha: String,
    pub base_sha: String,
    pub changed_files: Vec<String>,
    pub base_output: String,
    pub suite_output: String,
    pub diffstat: String,
    pub diff: String,
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

async fn run_argv(
    argv: &[String],
    cwd: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
) -> Result<CommandOutput> {
    let (program, args) = argv.split_first().context("command must not be empty")?;
    run_scrubbed(program, args, cwd, envs, timeout).await
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

    let base = git(&config.repo, &["rev-parse", &config.base_ref]).await?;
    if !base.success {
        bail!("I can't resolve the base revision `{}`", config.base_ref);
    }
    let base_sha = base.combined.trim().to_string();

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
    // outside the allowed paths.
    let denied: Vec<&String> = changed_files
        .iter()
        .filter(|path| config.denied_paths.iter().any(|prefix| path.starts_with(prefix)))
        .collect();
    let outside: Vec<&String> = changed_files
        .iter()
        .filter(|path| !config.allowed_paths.iter().any(|prefix| path.starts_with(prefix)))
        .collect();

    if !denied.is_empty() || !outside.is_empty() {
        let mut reason = String::from("Scope check failed. ");
        if !denied.is_empty() {
            reason.push_str(&format!(
                "The branch edits files it must not touch: {}. ",
                denied
                    .iter()
                    .map(|path| format!("`{path}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !outside.is_empty() {
            reason.push_str(&format!(
                "It also touches files outside the allowed paths ({}): {}.",
                config.allowed_paths.join(", "),
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
            base_sha,
            changed_files,
            base_output: String::new(),
            suite_output: String::new(),
            diffstat: String::new(),
            diff: String::new(),
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

    let repro = match run_argv(
        &config.repro_command,
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

    let suite = match run_argv(
        &config.test_command,
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

    let diffstat = git(
        &config.repo,
        &[
            "diff",
            "--shortstat",
            &format!("{}...{}", config.base_ref, branch),
        ],
    )
    .await
    .map(|output| output.combined.trim().to_string())
    .unwrap_or_default();
    let full_diff = git(
        &config.repo,
        &[
            "diff",
            "--no-color",
            &format!("{}...{}", config.base_ref, branch),
        ],
    )
    .await
    .map(|output| cap(&output.combined, 30_000))
    .unwrap_or_default();

    let duration_ms = started.elapsed().as_millis() as i64;

    if repro.success {
        return Ok(Outcome {
            verdict: Verdict::Fail,
            reason: Some(format!(
                "The reported failure does not reproduce on `{}`: the reproduction already passes. Refusing to certify a fix for a bug that isn't there.",
                config.base_ref
            )),
            commit_sha,
            base_sha,
            changed_files,
            base_output: cap(&repro.combined, 20_000),
            suite_output: String::new(),
            diffstat,
            diff: full_diff,
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
            reason: Some(format!("The suite still fails on the branch: {detail}.")),
            commit_sha,
            base_sha,
            changed_files,
            base_output: cap(&repro.combined, 20_000),
            suite_output: cap(&suite.combined, 20_000),
            diffstat,
            diff: full_diff,
            duration_ms,
        });
    }

    Ok(Outcome {
        verdict: Verdict::Pass,
        reason: None,
        commit_sha,
        base_sha,
        changed_files,
        base_output: cap(&repro.combined, 20_000),
        suite_output: cap(&suite.combined, 20_000),
        diffstat,
        diff: full_diff,
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
    use super::{valid_branch, verify};
    use crate::config::{Config, FixerProvider, PrMode, Role};
    use crate::ledger::Verdict;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "countersign-verify-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        write(&dir.join("demo/orders/src/lib.rs"), "pub fn total() -> i64 { 1 }\n");
        write(
            &dir.join("demo/orders/tests/checkout.rs"),
            "#[test] fn money() { assert_eq!(1, 1); }\n",
        );
        write(&dir.join("README.md"), "fixture\n");
        git(&dir, &["add", "-A"]);
        git(
            &dir,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "base",
            ],
        );
        dir
    }

    fn branch(repo: &Path, name: &str, path: &str, contents: &str) {
        git(repo, &["checkout", "-q", "-b", name, "main"]);
        write(&repo.join(path), contents);
        git(repo, &["add", "-A"]);
        git(
            repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "change",
            ],
        );
        git(repo, &["checkout", "-q", "main"]);
    }

    fn runner() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "countersign-runner-{}.sh",
            uuid::Uuid::new_v4()
        ));
        write(
            &path,
            "#!/usr/bin/env bash\nprintf '%s ran\\n' \"$1\"\nexit \"${2:-0}\"\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        path
    }

    fn config(repo: &Path, runner: &Path, repro_code: &str, suite_code: &str) -> Config {
        Config {
            base_url: "http://127.0.0.1:8000".into(),
            agent_id: "verifier".into(),
            api_key: "key".into(),
            role: Role::Verifier,
            repo: repo.to_path_buf(),
            ledger_path: repo.join("ledger.sqlite"),
            role_name: Some("verifier".into()),
            room_id: None,
            viewer_url: None,
            workdir: ".".into(),
            allowed_paths: vec!["demo/orders/src".into()],
            denied_paths: vec!["demo/orders/tests".into()],
            test_command: vec![
                runner.to_string_lossy().to_string(),
                "suite".into(),
                suite_code.into(),
            ],
            repro_command: vec![
                runner.to_string_lossy().to_string(),
                "repro".into(),
                repro_code.into(),
            ],
            approvers: vec![],
            max_attempts: 2,
            pr_mode: PrMode::Local,
            github_repo: None,
            base_ref: "main".into(),
            fixer_provider: FixerProvider::OpenCode,
            fixer_model: "test".into(),
        }
    }

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

    #[tokio::test]
    async fn a_clean_fix_passes_with_evidence() {
        let dir = repo();
        let runner = runner();
        branch(&dir, "fix/ok", "demo/orders/src/lib.rs", "pub fn total() -> i64 { 2 }\n");
        let outcome = verify(&config(&dir, &runner, "1", "0"), "fix/ok")
            .await
            .expect("verify");
        assert_eq!(outcome.verdict, Verdict::Pass, "reason={:?} base={:?} suite={:?}", outcome.reason, outcome.base_output, outcome.suite_output);
        assert!(outcome.base_output.contains("repro ran"), "base={:?}", outcome.base_output);
        assert!(outcome.suite_output.contains("suite ran"), "suite={:?}", outcome.suite_output);
        assert!(outcome.diff.contains("+pub fn total"));
        assert!(!outcome.base_sha.is_empty());
        assert!(outcome.diffstat.contains("1 file changed"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn editing_the_tests_is_rejected_before_running_them() {
        let dir = repo();
        let runner = runner();
        branch(
            &dir,
            "fix/tests",
            "demo/orders/tests/checkout.rs",
            "#[test] fn money() {}\n",
        );
        let outcome = verify(&config(&dir, &runner, "1", "0"), "fix/tests")
            .await
            .expect("verify");
        assert_eq!(outcome.verdict, Verdict::Fail);
        assert!(outcome.reason.unwrap().contains("must not touch"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_change_outside_the_allowed_paths_is_rejected() {
        let dir = repo();
        let runner = runner();
        branch(&dir, "fix/readme", "README.md", "tampered\n");
        let outcome = verify(&config(&dir, &runner, "1", "0"), "fix/readme")
            .await
            .expect("verify");
        assert_eq!(outcome.verdict, Verdict::Fail);
        assert!(outcome.reason.unwrap().contains("outside the allowed paths"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_bug_that_does_not_reproduce_is_not_certified() {
        let dir = repo();
        let runner = runner();
        branch(&dir, "fix/ok", "demo/orders/src/lib.rs", "pub fn total() -> i64 { 2 }\n");
        let outcome = verify(&config(&dir, &runner, "0", "0"), "fix/ok")
            .await
            .expect("verify");
        assert_eq!(outcome.verdict, Verdict::Fail);
        assert!(outcome.reason.as_deref().unwrap_or("").contains("does not reproduce"), "reason={:?} base={:?}", outcome.reason, outcome.base_output);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_failing_suite_is_a_failed_receipt() {
        let dir = repo();
        let runner = runner();
        branch(&dir, "fix/ok", "demo/orders/src/lib.rs", "pub fn total() -> i64 { 2 }\n");
        let outcome = verify(&config(&dir, &runner, "1", "1"), "fix/ok")
            .await
            .expect("verify");
        assert_eq!(outcome.verdict, Verdict::Fail);
        assert!(outcome.reason.as_deref().unwrap_or("").contains("still fails"), "reason={:?} suite={:?}", outcome.reason, outcome.suite_output);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_missing_branch_is_an_error() {
        let dir = repo();
        let runner = runner();
        assert!(verify(&config(&dir, &runner, "1", "0"), "fix/missing")
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
