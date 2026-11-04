//! The fixer: an existing coding agent (Codex CLI) driven from the room.
//!
//! It proposes work on its own branch in a disposable worktree. It cannot
//! land anything: that is the verifier's and a person's job.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::Config;
use crate::process::run;
use crate::switch::{strip_mention, Handler, IncomingMessage, SwitchClient};

pub struct Fixer {
    config: Config,
    client: Arc<SwitchClient>,
    busy: tokio::sync::Mutex<()>,
    announced: AtomicBool,
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

    fn prompt(&self, report: &str, branch: &str) -> String {
        format!(
            r#"You are the fixer in a team room called "checkout-incidents". You are working in the Rust crate `demo/orders` inside a disposable git worktree on branch `{branch}`.

A teammate reported:
---
{report}
---

Rules:
- Find the root cause, then fix it in `src/`. Do not modify tests. Do not modify anything outside `demo/orders`.
- The regression test for this report is:
    cargo test -p orders-demo --test checkout half_cent_charges_round_up
- Run `cargo test -p orders-demo` until the whole suite passes.
- Do not commit, push, or open a pull request. The harness does that.
- End with a short summary: root cause, the change, and the test result.
"#
        )
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
        let worktree = self.config.repo.join(".worktrees").join(format!("fix-{id}"));
        let worktree_string = worktree.to_string_lossy().to_string();

        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "On it. I'll work on `{branch}` in a separate worktree and hand it to `@verifier` when the suite is green.\n\nI can't land it myself — that's the point."
                ),
            )
            .await?;

        let added = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.clone(),
                worktree_string.clone(),
                self.config.base_ref.clone(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !added.success {
            self.client
                .post_message(
                    &message.room_id,
                    &format!("I couldn't start a worktree: {}", added.tail(600)),
                )
                .await?;
            return Ok(());
        }

        let crate_dir = worktree.join("demo/orders");
        let summary_path = self.config.repo.join(".countersign").join(format!("{id}.summary.txt"));
        let prompt = self.prompt(report, &branch);
        info!("running Codex for branch {branch}");

        let codex = run(
            "codex",
            &[
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
            &self.config.repo,
            &[],
            Duration::from_secs(600),
        )
        .await;

        let codex = match codex {
            Ok(output) => output,
            Err(error) => {
                let _ = cleanup(&self.config.repo, &worktree_string).await;
                let _ = run(
                    "git",
                    &["branch".to_string(), "-D".to_string(), branch.clone()],
                    &self.config.repo,
                    &[],
                    Duration::from_secs(30),
                )
                .await;
                self.client
                    .post_message(&message.room_id, &format!("Codex failed to run: {error:#}"))
                    .await?;
                return Ok(());
            }
        };

        if !codex.success {
            let excerpt = codex.tail(900);
            let _ = cleanup(&self.config.repo, &worktree_string).await;
            let _ = run(
                "git",
                &["branch".to_string(), "-D".to_string(), branch.clone()],
                &self.config.repo,
                &[],
                Duration::from_secs(30),
            )
            .await;
            self.client
                .post_message(
                    &message.room_id,
                    &format!("The fix run failed; no branch was proposed.\n\n```text\n{excerpt}\n```"),
                )
                .await?;
            return Ok(());
        }

        let status = run(
            "git",
            &[
                "status".to_string(),
                "--porcelain".to_string(),
                "--".to_string(),
                "demo/orders".to_string(),
            ],
            &worktree,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if status.combined.trim().is_empty() {
            let _ = cleanup(&self.config.repo, &worktree_string).await;
            let _ = run(
                "git",
                &["branch".to_string(), "-D".to_string(), branch.clone()],
                &self.config.repo,
                &[],
                Duration::from_secs(30),
            )
            .await;
            self.client
                .post_message(
                    &message.room_id,
                    "I couldn't find a change to propose. Nothing was committed.",
                )
                .await?;
            return Ok(());
        }

        let commit_message = format!("fix: {slug}\n\nProposed by countersign-fixer; not yet verified.");
        let added_files = run(
            "git",
            &["add".to_string(), "demo/orders".to_string()],
            &worktree,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !added_files.success {
            bail!("git add failed: {}", added_files.tail(500));
        }
        let committed = run(
            "git",
            &["commit".to_string(), "-m".to_string(), commit_message],
            &worktree,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !committed.success {
            bail!("git commit failed: {}", committed.tail(700));
        }

        let sha = run(
            "git",
            &["rev-parse".to_string(), "HEAD".to_string()],
            &worktree,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        let sha = sha.combined.trim().to_string();
        let files = run(
            "git",
            &[
                "diff".to_string(),
                "--name-only".to_string(),
                format!("{}...HEAD", self.config.base_ref),
            ],
            &worktree,
            &[],
            Duration::from_secs(30),
        )
        .await?;

        let summary = std::fs::read_to_string(&summary_path)
            .unwrap_or_else(|_| codex.tail(900))
            .trim()
            .to_string();
        let _ = cleanup(&self.config.repo, &worktree_string).await;

        let message_out = format!(
            "**Proposed fix · `{branch}`**\nCommit `{}` · files: {}\n\n{}\n\n`@verifier` please run the independent check.",
            &sha[..sha.len().min(10)],
            files
                .combined
                .lines()
                .map(|line| format!("`{}`", line.trim()))
                .collect::<Vec<_>>()
                .join(", "),
            truncate(&summary, 1600)
        );
        self.client
            .post_message(&message.room_id, &message_out)
            .await?;
        let _ = std::fs::remove_file(summary_path);
        Ok(())
    }
}

#[async_trait]
impl Handler for Fixer {
    async fn on_ready(&self, rooms: Vec<String>) -> Result<()> {
        let Some(room) = rooms.first().cloned() else {
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
        let body = strip_mention(&message.body);
        let instructions = body
            .trim()
            .strip_prefix("fix")
            .map(str::trim)
            .unwrap_or(body.trim())
            .to_string();

        if instructions.eq_ignore_ascii_case("help") || instructions.is_empty() {
            self.client
                .post_message(
                    &message.room_id,
                    "Send a report in the room and mention me, or run `@fixer fix <what is wrong>`. I work on a branch and hand it to `@verifier`.",
                )
                .await?;
            return Ok(());
        }

        self.handle_fix(&message, &instructions).await
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
