//! The fixer: an existing coding agent (OpenCode or Codex) driven from the
//! room.
//!
//! It proposes work on its own branch in its own checkout. It cannot land
//! anything: that is the verifier's and a person's job. The agent works in a
//! plain copy with no `.git`, and the harness publishes the result through a
//! worktree the agent never sees.

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::{Config, FixerProvider};
use crate::ledger::Ledger;
use crate::process::run;
use crate::switch::{strip_mention, Handler, IncomingMessage, SwitchClient};

pub struct Fixer {
    config: Config,
    client: Arc<SwitchClient>,
    busy: tokio::sync::Mutex<()>,
    announced: AtomicBool,
}

struct Proposal {
    branch: String,
    sha: String,
    files: Vec<String>,
}

impl Fixer {
    pub fn new(config: Config, client: Arc<SwitchClient>) -> Self {
        Self {
            config,
            client,
            busy: tokio::sync::Mutex::new(()),
            announced: AtomicBool::new(false),
        }
    }

    fn fix_prompt(&self, report: &str, branch: &str) -> String {
        format!(
            r#"You are the fixer in a team room called "checkout-incidents". You are working in the Rust crate `demo/orders` of a disposable checkout on branch `{branch}`.

A teammate reported:
---
{report}
---

Rules:
- Find the root cause, then fix it in `src/`. Do not modify tests. Do not modify anything outside `demo/orders`.
- Run the full test suite and make it pass. The reported failing test is part of it.
- Do not commit, push, or open a pull request. The harness does that.
- End your reply with a line `SUMMARY:` followed by two or three sentences: root cause, the change, and the test result.
"#
        )
    }

    fn revise_prompt(&self, receipt: &crate::ledger::Receipt, branch: &str) -> String {
        format!(
            r#"You are the fixer in a team room called "checkout-incidents". You are working in the Rust crate `demo/orders` of a disposable checkout on branch `{branch}`.

An independent verifier rejected a previous proposal. Its receipt:

Reason: {reason}

Failing output:
---
{output}
---

Rules:
- Start from the current branch. The previous attempt is already applied.
- Fix the failure. Do not weaken, delete, or edit the tests. Do not modify anything outside `demo/orders`.
- Run the full test suite until it passes.
- Do not commit, push, or open a pull request. The harness does that.
- End your reply with a line `SUMMARY:` followed by two or three sentences: what the verifier caught, what you changed, and the test result.
"#,
            branch = branch,
            reason = receipt
                .reason
                .clone()
                .unwrap_or_else(|| "The suite failed.".to_string()),
            output = tail(&receipt.suite_output, 6000),
        )
    }

    /// Extract a plain copy of `reference` with no git directory, so the agent
    /// cannot follow a git root back into the operator's checkout.
    async fn checkout_at(&self, reference: &str, id: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("countersign-fix-{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let archive = std::env::temp_dir().join(format!("countersign-fix-{id}.tar"));
        let archived = run(
            "git",
            &[
                "archive".to_string(),
                "--format=tar".to_string(),
                format!("--output={}", archive.to_string_lossy()),
                reference.to_string(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(120),
        )
        .await?;
        if !archived.success {
            let _ = std::fs::remove_dir_all(&dir);
            bail!("git archive {reference} failed: {}", archived.tail(600));
        }
        let extracted = run(
            "tar",
            &[
                "-xf".to_string(),
                archive.to_string_lossy().to_string(),
                "-C".to_string(),
                dir.to_string_lossy().to_string(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(120),
        )
        .await?;
        let _ = std::fs::remove_file(&archive);
        if !extracted.success {
            let _ = std::fs::remove_dir_all(&dir);
            bail!("tar failed: {}", extracted.tail(600));
        }
        Ok(dir)
    }

    async fn run_agent(&self, checkout: &Path, prompt: String) -> Result<crate::process::CommandOutput> {
        let crate_dir = checkout.join("demo/orders");
        let summary_path = self
            .config
            .repo
            .join(".countersign")
            .join(format!("{}.summary.txt", uuid::Uuid::new_v4()));
        let provider = self.config.fixer_provider;
        info!("running {provider:?} in {}", crate_dir.display());

        let (program, args) = match provider {
            FixerProvider::OpenCode => (
                "opencode",
                vec![
                    "run".to_string(),
                    "--standalone".to_string(),
                    "--auto".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    "--model".to_string(),
                    self.config.fixer_model.clone(),
                    prompt,
                ],
            ),
            FixerProvider::Codex => (
                "codex",
                vec![
                    "exec".to_string(),
                    "--dangerously-bypass-approvals-and-sandbox".to_string(),
                    "--skip-git-repo-check".to_string(),
                    "--color".to_string(),
                    "never".to_string(),
                    "-C".to_string(),
                    crate_dir.to_string_lossy().to_string(),
                    "-o".to_string(),
                    summary_path.to_string_lossy().to_string(),
                    prompt,
                ],
            ),
        };
        let agent_envs: Vec<(&str, String)> =
            vec![("PWD", crate_dir.to_string_lossy().to_string())];
        let output = run(program, &args, &crate_dir, &agent_envs, Duration::from_secs(600)).await;
        let _ = std::fs::remove_file(summary_path);
        output
    }

    /// Publish the agent's crate through a git worktree. Returns `None` when
    /// the agent changed nothing.
    async fn publish(
        &self,
        base_ref: &str,
        branch: &str,
        checkout: &Path,
        summary: &str,
        commit_message: &str,
    ) -> Result<Option<Proposal>> {
        let id = &uuid::Uuid::new_v4().simple().to_string()[..6];
        let wt = self
            .config
            .repo
            .join(".worktrees")
            .join(format!("commit-{id}"));
        let wt_string = wt.to_string_lossy().to_string();
        let added = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.to_string(),
                wt_string.clone(),
                base_ref.to_string(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !added.success {
            bail!("could not open a branch worktree: {}", added.tail(600));
        }

        let target = wt.join("demo/orders");
        let _ = std::fs::remove_dir_all(&target);
        let synced = run(
            "rsync",
            &[
                "-a".to_string(),
                "--exclude".to_string(),
                "target".to_string(),
                "--exclude".to_string(),
                ".opencode".to_string(),
                "--exclude".to_string(),
                ".codex".to_string(),
                format!("{}/", checkout.join("demo/orders").to_string_lossy()),
                format!("{}/", target.to_string_lossy()),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !synced.success {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            bail!("could not assemble the branch: {}", synced.tail(600));
        }

        let status = run(
            "git",
            &[
                "status".to_string(),
                "--porcelain".to_string(),
                "--".to_string(),
                "demo/orders".to_string(),
            ],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if status.combined.trim().is_empty() {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            return Ok(None);
        }

        let added_files = run(
            "git",
            &["add".to_string(), "demo/orders".to_string()],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !added_files.success {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            bail!("git add failed: {}", added_files.tail(500));
        }
        let committed = run(
            "git",
            &[
                "commit".to_string(),
                "-m".to_string(),
                format!("{commit_message}\n\nProposed by countersign-fixer; not yet verified.\n\n{summary}"),
            ],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !committed.success {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            bail!("git commit failed: {}", committed.tail(700));
        }

        let sha = run(
            "git",
            &["rev-parse".to_string(), "HEAD".to_string()],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        let files = run(
            "git",
            &[
                "diff".to_string(),
                "--name-only".to_string(),
                format!("{base_ref}...HEAD"),
            ],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        let _ = cleanup_worktree(&self.config.repo, &wt_string).await;

        Ok(Some(Proposal {
            branch: branch.to_string(),
            sha: sha.combined.trim().to_string(),
            files: files
                .combined
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        }))
    }

    async fn handle_fix(&self, message: &IncomingMessage, report: &str) -> Result<()> {
        let _guard = match self.busy.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                self.client
                    .post_message(
                        &message.room_id,
                        "One change at a time — I'm still working on the previous one.",
                    )
                    .await?;
                return Ok(());
            }
        };

        let id = &uuid::Uuid::new_v4().simple().to_string()[..6];
        let slug = slugify(report);
        let branch = format!("fix/{slug}-{id}");

        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "On it. I'll work on `{branch}` in my own checkout and hand it to the verifier when the suite is green.\n\nI can't land it myself — that's the point."
                ),
            )
            .await?;

        let checkout = match self.checkout_at(&self.config.base_ref, id).await {
            Ok(checkout) => checkout,
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I couldn't create my checkout: {error:#}"),
                    )
                    .await?;
                return Ok(());
            }
        };

        let prompt = self.fix_prompt(report, &branch);
        let output = match self.run_agent(&checkout, prompt).await {
            Ok(output) => output,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&checkout);
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("The coding agent failed to run: {error:#}"),
                    )
                    .await?;
                return Ok(());
            }
        };
        if !output.success {
            let excerpt = output.tail(900);
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    &format!("The fix run failed; no branch was proposed.\n\n```text\n{excerpt}\n```"),
                )
                .await?;
            return Ok(());
        }

        let summary = summary_of(&output);
        let published = self
            .publish(
                &self.config.base_ref,
                &branch,
                &checkout,
                &summary,
                &format!("fix: {slug}"),
            )
            .await;
        let _ = std::fs::remove_dir_all(&checkout);

        match published {
            Ok(Some(proposal)) => {
                self.post_proposal(message, &proposal, &summary, None).await?;
            }
            Ok(None) => {
                self.client
                    .post_message(
                        &message.room_id,
                        "I couldn't find a change to propose. Nothing was committed.",
                    )
                    .await?;
            }
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I couldn't publish the proposal: {error:#}"),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_revise(&self, message: &IncomingMessage, receipt_id: &str) -> Result<()> {
        let _guard = match self.busy.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                self.client
                    .post_message(
                        &message.room_id,
                        "One change at a time — I'm still working on the previous one.",
                    )
                    .await?;
                return Ok(());
            }
        };

        let ledger = Ledger::open(&self.config.ledger_path)?;
        let Some(receipt) = ledger.get(receipt_id)? else {
            self.client
                .post_message(
                    &message.room_id,
                    &format!("I can't find a receipt `{receipt_id}` to revise."),
                )
                .await?;
            return Ok(());
        };
        if receipt.verdict == crate::ledger::Verdict::Pass {
            self.client
                .post_message(
                    &message.room_id,
                    &format!("`{receipt_id}` already passed. There is nothing to revise."),
                )
                .await?;
            return Ok(());
        }
        if receipt.attempt >= self.config.max_attempts {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{receipt_id}` is at attempt {}/{}. I'm not starting another loop; a person should decide.",
                        receipt.attempt, self.config.max_attempts
                    ),
                )
                .await?;
            return Ok(());
        }

        let id = &uuid::Uuid::new_v4().simple().to_string()[..6];
        let slug = slugify(&receipt.branch.replace('/', " "));
        let branch = format!("retry/{slug}-{id}");
        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "Revising from `{}`: attempt {}/{}. I'll start from `{}` and hand the new branch back to the verifier.",
                    receipt.id,
                    receipt.attempt + 1,
                    self.config.max_attempts,
                    receipt.branch
                ),
            )
            .await?;

        let checkout = match self.checkout_at(&receipt.branch, id).await {
            Ok(checkout) => checkout,
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I couldn't check out the failed branch: {error:#}"),
                    )
                    .await?;
                return Ok(());
            }
        };

        let prompt = self.revise_prompt(&receipt, &branch);
        let output = match self.run_agent(&checkout, prompt).await {
            Ok(output) => output,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&checkout);
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("The coding agent failed to run: {error:#}"),
                    )
                    .await?;
                return Ok(());
            }
        };
        if !output.success {
            let excerpt = output.tail(900);
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    &format!("The revision failed; no branch was proposed.\n\n```text\n{excerpt}\n```"),
                )
                .await?;
            return Ok(());
        }

        let summary = summary_of(&output);
        let published = self
            .publish(
                &receipt.branch,
                &branch,
                &checkout,
                &summary,
                &format!("fix: revise {}", receipt.id),
            )
            .await;
        let _ = std::fs::remove_dir_all(&checkout);

        match published {
            Ok(Some(proposal)) => {
                self.post_proposal(message, &proposal, &summary, Some(&receipt.id))
                    .await?;
            }
            Ok(None) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!(
                            "I couldn't find a change beyond `{}`. The revision is empty.",
                            receipt.branch
                        ),
                    )
                    .await?;
            }
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I couldn't publish the revision: {error:#}"),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn post_proposal(
        &self,
        message: &IncomingMessage,
        proposal: &Proposal,
        summary: &str,
        supersedes: Option<&str>,
    ) -> Result<()> {
        let chain = supersedes
            .map(|parent| format!("\nSupersedes `{parent}`."))
            .unwrap_or_default();
        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "**Proposed fix · `{}`**\nCommit `{}` · files: {}{}\n\n{}",
                    proposal.branch,
                    &proposal.sha[..proposal.sha.len().min(10)],
                    proposal
                        .files
                        .iter()
                        .map(|file| format!("`{file}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    chain,
                    truncate(summary, 1600)
                ),
            )
            .await?;
        let handoff = match supersedes {
            Some(parent) => format!(
                "@verifier verify {} supersedes {}",
                proposal.branch, parent
            ),
            None => format!("@verifier verify {}", proposal.branch),
        };
        self.client.post_message(&message.room_id, &handoff).await?;
        Ok(())
    }
}

#[async_trait]
impl Handler for Fixer {
    async fn on_ready(&self, rooms: Vec<String>) -> Result<()> {
        let Some(room) = self
            .config
            .room_id
            .clone()
            .or_else(|| rooms.first().cloned())
        else {
            return Ok(());
        };
        *self.client.room_id.lock().await = Some(room.clone());
        self.client.connect_room(&room).await?;
        if let Some(role) = &self.config.role_name {
            if let Err(error) = self.client.assume_role(role).await {
                warn!("could not assume role {role}: {error:#}");
            }
        }
        if !self.announced.swap(true, Ordering::Relaxed) {
            self.client
                .post_message(
                    &room,
                    "Fixer online. Send me a report and I'll propose a branch — I can't verify or land it myself.\n\n`@fixer fix <what is wrong>`",
                )
                .await?;
        }
        Ok(())
    }

    async fn on_message(&self, message: IncomingMessage) -> Result<()> {
        if let Some(expected) = &self.config.room_id {
            if &message.room_id != expected {
                return Ok(());
            }
        }
        let body = strip_mention(&message.body);
        let lower = body.to_lowercase();

        if lower.starts_with("revise") {
            let mut parts = body.split_whitespace();
            parts.next();
            let id = parts.next().unwrap_or("").trim_matches('`');
            if id.is_empty() {
                self.client
                    .post_message(
                        &message.room_id,
                        "Which receipt? `@fixer revise <receipt>`",
                    )
                    .await?;
                return Ok(());
            }
            return self.handle_revise(&message, id).await;
        }

        let Some(rest) = body.trim().strip_prefix("fix") else {
            self.client
                .post_message(
                    &message.room_id,
                    "Mention me with a report to act on: `@fixer fix <what is wrong>`. I propose a branch and hand it to `@verifier`; I can't verify or land it myself.",
                )
                .await?;
            return Ok(());
        };
        let instructions = rest.trim().trim_start_matches(':').trim().to_string();
        if instructions.is_empty() {
            self.client
                .post_message(
                    &message.room_id,
                    "What should I fix? `@fixer fix <what is wrong>`",
                )
                .await?;
            return Ok(());
        }
        self.handle_fix(&message, &instructions).await
    }
}

async fn cleanup_worktree(repo: &Path, worktree: &str) -> Result<()> {
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

fn summary_of(output: &crate::process::CommandOutput) -> String {
    let raw = extract_text(&output.combined);
    raw.split("SUMMARY:")
        .last()
        .unwrap_or(&raw)
        .trim()
        .to_string()
}

fn slugify(text: &str) -> String {
    let mut words = Vec::new();
    for word in text.split_whitespace() {
        let cleaned: String = word
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if cleaned.is_empty() || cleaned.len() < 2 {
            continue;
        }
        words.push(cleaned);
        if words.len() == 3 {
            break;
        }
    }
    let slug = words.join("-");
    if slug.is_empty() {
        "checkout-incident".to_string()
    } else {
        slug.chars().take(32).collect()
    }
}

/// Pull the assistant's text out of OpenCode's JSONL event stream.
fn extract_text(output: &str) -> String {
    let mut chunks = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|value| value.as_str()) != Some("text") {
            continue;
        }
        if let Some(text) = value
            .get("part")
            .and_then(|part| part.get("text"))
            .and_then(|text| text.as_str())
        {
            chunks.push(text.to_string());
        }
    }
    if chunks.is_empty() {
        output.to_string()
    } else {
        chunks.join("\n\n")
    }
}

fn tail(text: &str, limit: usize) -> String {
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

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let start = text
        .char_indices()
        .find(|(index, _)| *index >= text.len() - limit)
        .map(|(index, _)| index)
        .unwrap_or(0);
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs_are_branch_safe() {
        assert_eq!(slugify("refunds are still off!"), "refunds-are-still");
        assert_eq!(slugify("!!!"), "checkout-incident");
    }
}
