//! Landing a countersigned change: a real pull request, or a local merge when
//! the deployment has no GitHub credentials.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::time::Duration;

use crate::config::{Config, PrMode};
use crate::ledger::Receipt;
use crate::process::run;

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

pub async fn land(config: &Config, receipt: &Receipt, countersigner: &str) -> Result<String> {
    match config.pr_mode {
        PrMode::Gh => land_pr(config, receipt, countersigner).await,
        PrMode::Local => land_local(config, receipt, countersigner).await,
    }
}

async fn land_pr(config: &Config, receipt: &Receipt, countersigner: &str) -> Result<String> {
    let repo = config
        .github_repo
        .as_deref()
        .context("COUNTERSIGN_GITHUB_REPO is required when COUNTERSIGN_PR_MODE=gh")?;

    let remote = run(
        "git",
        &["remote".to_string(), "get-url".to_string(), "origin".to_string()],
        &config.repo,
        &[],
        Duration::from_secs(30),
    )
    .await?;
    if !remote.success {
        let add = run(
            "git",
            &[
                "remote".to_string(),
                "add".to_string(),
                "origin".to_string(),
                format!("https://github.com/{repo}.git"),
            ],
            &config.repo,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !add.success {
            bail!("could not add origin: {}", add.tail(500));
        }
    }

    let push = run(
        "git",
        &[
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            receipt.branch.clone(),
        ],
        &config.repo,
        &[],
        Duration::from_secs(120),
    )
    .await?;
    if !push.success {
        bail!("push failed: {}", push.tail(800));
    }

    let body = pr_body(receipt, countersigner);
    let title = format!(
        "fix: {} (verified + countersigned)",
        receipt
            .branch
            .split('/')
            .last()
            .unwrap_or("fix")
            .replace('-', " ")
    );
    let created = run(
        "gh",
        &[
            "pr".to_string(),
            "create".to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--base".to_string(),
            config.base_ref.clone(),
            "--head".to_string(),
            receipt.branch.clone(),
            "--title".to_string(),
            title,
            "--body".to_string(),
            body,
        ],
        &config.repo,
        &[],
        Duration::from_secs(120),
    )
    .await?;
    if !created.success {
        bail!("gh pr create failed: {}", created.tail(800));
    }
    let url = created
        .combined
        .lines()
        .rev()
        .find(|line| line.starts_with("https://"))
        .map(str::trim)
        .unwrap_or("(pull request created)")
        .to_string();
    Ok(url)
}

async fn land_local(config: &Config, receipt: &Receipt, countersigner: &str) -> Result<String> {
    let id = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let root = config.repo.join(".worktrees").join(format!("land-{id}"));
    let root_string = root.to_string_lossy().to_string();
    let landed = "countersign/landed";

    // The landed branch is a proof artifact for the last countersigned change,
    // not an integration branch: reset it to the base revision so a second
    // landing cannot conflict with the first.
    let args = vec![
        "worktree".to_string(),
        "add".to_string(),
        "-B".to_string(),
        landed.to_string(),
        root_string.clone(),
        config.base_ref.clone(),
    ];
    let added = run("git", &args, &config.repo, &[], Duration::from_secs(60)).await?;
    if !added.success {
        bail!("could not open the landing worktree: {}", added.tail(600));
    }

    let message = format!(
        "countersign: land {} ({})\n\nVerified by countersign-verifier; countersigned by {}.",
        receipt.id, receipt.branch, countersigner
    );
    let merged = run(
        "git",
        &[
            "merge".to_string(),
            "--no-ff".to_string(),
            receipt.branch.clone(),
            "-m".to_string(),
            message,
        ],
        &root,
        &[],
        Duration::from_secs(60),
    )
    .await?;
    if !merged.success {
        let _ = cleanup(&config.repo, &root_string).await;
        bail!("merge failed: {}", merged.tail(800));
    }

    let sha = run(
        "git",
        &["rev-parse".to_string(), "HEAD".to_string()],
        &root,
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let _ = cleanup(&config.repo, &root_string).await;

    Ok(format!(
        "`{landed}` at `{}`",
        short(sha.combined.trim())
    ))
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

fn pr_body(receipt: &Receipt, countersigner: &str) -> String {
    format!(
        "## Countersigned change\n\n\
         This pull request was proposed by an agent, independently verified, and \
         countersigned by **{countersigner}** in the team room.\n\n\
         | | |\n|---|---|\n\
         | Receipt | `{id}` |\n\
         | Verdict | **{verdict}** |\n\
         | Branch | `{branch}` |\n\
         | Commit | `{sha}` |\n\
         | Changed files | {files} |\n\
         | Countersigned by | {countersigner} |\n\n\
         ### What the verifier ran\n\n\
         The reported failure was reproduced on `main`, then the full suite was run \
         on the branch in a clean worktree. No model sits in the verification path, \
         and a branch that edits its own tests is rejected.\n\n\
         ```text\n{suite}\n```\n\n\
         ---\n\
         Generated by [Countersign](https://github.com/PhiBao/countersign) — no agent ships alone.\n",
        id = receipt.id,
        verdict = receipt.verdict.as_str(),
        branch = receipt.branch,
        sha = receipt.commit_sha,
        files = receipt
            .changed_files
            .iter()
            .map(|file| format!("`{file}`"))
            .collect::<Vec<_>>()
            .join(", "),
        countersigner = countersigner,
        suite = receipt.suite_output.trim(),
    )
}
