//! The fixer: an existing coding agent (OpenCode or Codex) driven from the
//! room.
//!
//! It proposes work on its own branch in its own checkout. It cannot land
//! anything: that is the verifier's and a person's job. A separate clone is
//! used rather than a worktree because coding agents resolve project roots in
//! their own ways; a clone has no shared git directory to fall back into.

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::{Config, FixerProvider};
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
            r#"You are the fixer in a team room called "checkout-incidents". You are working in the Rust crate `demo/orders` of a disposable checkout on branch `{branch}`.

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
- End your reply with a line `SUMMARY:` followed by two or three sentences: root cause, the change, and the test result.
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
        let checkout = std::env::temp_dir().join(format!("countersign-fix-{id}"));

        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "On it. I'll work on `{branch}` in my own checkout and hand it to the verifier when the suite is green.\n\nI can't land it myself — that's the point."
                ),
            )
            .await?;

        // A plain copy, with no .git: coding agents resolve project roots in
        // their own ways, and one that follows a git directory can wander back
        // into the operator's checkout. Nothing the agent does here can touch
        // the repository until the harness commits the result.
        std::fs::create_dir_all(&checkout)?;
        for entry in ["Cargo.toml", "Cargo.lock"] {
            let source = self.config.repo.join(entry);
            if source.exists() {
                std::fs::copy(&source, checkout.join(entry))?;
            }
        }
        for dir in ["demo", "agents"] {
            let source = self.config.repo.join(dir);
            if source.exists() {
                let copied = run(
                    "cp",
                    &[
                        "-a".to_string(),
                        source.to_string_lossy().to_string(),
                        checkout.to_string_lossy().to_string(),
                    ],
                    &self.config.repo,
                    &[],
                    Duration::from_secs(60),
                )
                .await?;
                if !copied.success {
                    let _ = std::fs::remove_dir_all(&checkout);
                    self.client
                        .post_message(
                            &message.room_id,
                            &format!("I couldn't create my checkout: {}", copied.tail(600)),
                        )
                        .await?;
                    return Ok(());
                }
            }
        }

        let crate_dir = checkout.join("demo/orders");
        let summary_path = self
            .config
            .repo
            .join(".countersign")
            .join(format!("{id}.summary.txt"));
        let prompt = self.prompt(report, &branch);
        let provider = self.config.fixer_provider;
        info!("running {provider:?} for branch {branch}");

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

        let agent_envs: Vec<(&str, String)> = vec![(
            "PWD",
            crate_dir.to_string_lossy().to_string(),
        )];
        let agent = run(program, &args, &crate_dir, &agent_envs, Duration::from_secs(600)).await;

        let agent = match agent {
            Ok(output) => output,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&checkout);
                self.client
                    .post_message(&message.room_id, &format!("The coding agent failed to run: {error:#}"))
                    .await?;
                return Ok(());
            }
        };
        info!("agent output tail: {}", agent.tail(600));

        if !agent.success {
            let excerpt = agent.tail(900);
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    &format!("The fix run failed; no branch was proposed.\n\n```text\n{excerpt}\n```"),
                )
                .await?;
            return Ok(());
        }

        let status = run(
            "diff",
            &[
                "-rq".to_string(),
                "--exclude".to_string(),
                "target".to_string(),
                "--exclude".to_string(),
                ".opencode".to_string(),
                "--exclude".to_string(),
                ".codex".to_string(),
                self.config
                    .repo
                    .join("demo/orders")
                    .to_string_lossy()
                    .to_string(),
                crate_dir.to_string_lossy().to_string(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if status.success {
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    "I couldn't find a change to propose. Nothing was committed.",
                )
                .await?;
            return Ok(());
        }

        // Publish the agent's work through a real git worktree. The agent
        // never had access to this one.
        let wt = self
            .config
            .repo
            .join(".worktrees")
            .join(format!("commit-{id}"));
        let wt_string = wt.to_string_lossy().to_string();
        let added_branch = run(
            "git",
            &[
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.clone(),
                wt_string.clone(),
                self.config.base_ref.clone(),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !added_branch.success {
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    &format!("I couldn't create the branch: {}", added_branch.tail(600)),
                )
                .await?;
            return Ok(());
        }

        // Replace the whole crate with the agent's version so additions and
        // deletions both survive, then drop anything that is not source.
        let target_crate = wt.join("demo/orders");
        let _ = std::fs::remove_dir_all(&target_crate);
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
                format!("{}/", crate_dir.to_string_lossy()),
                format!("{}/", target_crate.to_string_lossy()),
            ],
            &self.config.repo,
            &[],
            Duration::from_secs(60),
        )
        .await?;
        if !synced.success {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            let _ = std::fs::remove_dir_all(&checkout);
            self.client
                .post_message(
                    &message.room_id,
                    &format!("I couldn't assemble the branch: {}", synced.tail(600)),
                )
                .await?;
            return Ok(());
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
            let _ = std::fs::remove_dir_all(&checkout);
            bail!("git add failed: {}", added_files.tail(500));
        }
        let commit_message = format!("fix: {slug}\n\nProposed by countersign-fixer; not yet verified.");
        let committed = run(
            "git",
            &["commit".to_string(), "-m".to_string(), commit_message],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        if !committed.success {
            let _ = cleanup_worktree(&self.config.repo, &wt_string).await;
            let _ = std::fs::remove_dir_all(&checkout);
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
        let sha = sha.combined.trim().to_string();
        let files = run(
            "git",
            &[
                "diff".to_string(),
                "--name-only".to_string(),
                format!("{}...HEAD", self.config.base_ref),
            ],
            &wt,
            &[],
            Duration::from_secs(30),
        )
        .await?;
        let _ = cleanup_worktree(&self.config.repo, &wt_string).await;

        let raw_summary = std::fs::read_to_string(&summary_path)
            .unwrap_or_else(|_| extract_text(&agent.combined));
        let summary = raw_summary
            .split("SUMMARY:")
            .last()
            .unwrap_or(&raw_summary)
            .trim()
            .to_string();
        let _ = std::fs::remove_dir_all(&checkout);
        let _ = std::fs::remove_file(summary_path);

        let message_out = format!(
            "**Proposed fix · `{branch}`**\nCommit `{}` · files: {}\n\n{}",
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
        // The handoff is a separate, explicit message so the room shows who
        // asked whom, and so the verifier receives a clean command.
        self.client
            .post_message(&message.room_id, &format!("@verifier verify {branch}"))
            .await?;
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

