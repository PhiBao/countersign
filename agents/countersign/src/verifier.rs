//! The verifier: independent, deterministic, and the only role that can issue
//! a receipt a person may countersign.

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use crate::config::{Config, PrMode};
use crate::land;
use crate::ledger::{Ledger, Receipt, Verdict};
use crate::process::run;
use crate::switch::{strip_mention, Handler, IncomingMessage, SwitchClient};
use crate::verify::{self, receipt_id};

pub struct Verifier {
    config: Config,
    client: Arc<SwitchClient>,
    ledger: Ledger,
    busy: tokio::sync::Mutex<()>,
    announced: AtomicBool,
}

impl Verifier {
    pub fn new(config: Config, client: Arc<SwitchClient>, ledger: Ledger) -> Result<Self> {
        Ok(Self {
            config,
            client,
            ledger,
            busy: tokio::sync::Mutex::new(()),
            announced: AtomicBool::new(false),
        })
    }

    async fn handle_verify(
        &self,
        message: &IncomingMessage,
        branch: &str,
        supersedes: Option<&str>,
    ) -> Result<()> {
        if branch.is_empty() {
            self.client
                .post_message(
                    &message.room_id,
                    "Which branch? `@verifier verify <branch>`",
                )
                .await?;
            return Ok(());
        }
        let _guard = match self.busy.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                self.client
                    .post_message(
                        &message.room_id,
                        "One verification at a time — I'm still working on the previous receipt.",
                    )
                    .await?;
                return Ok(());
            }
        };

        let (attempt, supersedes) = self.attempt_for(branch, supersedes);

        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "Verifying `{branch}` on a clean worktree.\n\nNo model sits in this path: I reproduce the reported failure on `{}`, run the full suite on the branch, and refuse any branch that edits its own tests.",
                    self.config.base_ref
                ),
            )
            .await?;

        let outcome = match verify::verify(&self.config, branch).await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I can't verify that: {error:#}"),
                    )
                    .await?;
                return Ok(());
            }
        };

        let id = receipt_id();
        let receipt = Receipt {
            id: id.clone(),
            room_id: message.room_id.clone(),
            branch: branch.to_string(),
            commit_sha: outcome.commit_sha.clone(),
            base_sha: outcome.base_sha.clone(),
            verdict: outcome.verdict,
            reason: outcome.reason.clone(),
            changed_files: outcome.changed_files.clone(),
            base_output: outcome.base_output.clone(),
            suite_output: outcome.suite_output.clone(),
            diffstat: outcome.diffstat.clone(),
            diff: outcome.diff.clone(),
            duration_ms: outcome.duration_ms,
            attempt,
            supersedes: supersedes.clone(),
            created_at: verify::now(),
            countersigned_by: None,
            countersigned_at: None,
            landed_url: None,
            pr_state: None,
        };
        self.ledger.insert(&receipt)?;

        let text = format_receipt(
            &receipt,
            self.config.viewer_url.as_deref(),
            !self.config.approvers.is_empty(),
            &self.config.base_ref,
        );
        self.client.post_message(&message.room_id, &text).await?;

        if receipt.verdict == Verdict::Fail {
            self.escalate_or_revise(&receipt).await?;
        }
        Ok(())
    }

    fn attempt_for(&self, branch: &str, supersedes: Option<&str>) -> (u8, Option<String>) {
        if let Some(parent_id) = supersedes {
            if let Ok(Some(parent)) = self.ledger.get(parent_id) {
                return (
                    parent.attempt.saturating_add(1).min(9),
                    Some(parent.id),
                );
            }
        }
        (if branch.starts_with("retry/") { 2 } else { 1 }, None)
    }

    /// On a first failure, hand the receipt back to the fixer as a work order.
    /// On the last allowed attempt, stop and escalate to a person.
    async fn escalate_or_revise(&self, receipt: &Receipt) -> Result<()> {
        if receipt.verdict == Verdict::Pass {
            return Ok(());
        }
        if receipt.attempt < self.config.max_attempts {
            self.client
                .post_message(
                    &receipt.room_id,
                    &format!(
                        "@fixer revise {} — the receipt has the failing tests and the exact output. Attempt {}/{}.",
                        receipt.id,
                        receipt.attempt + 1,
                        self.config.max_attempts
                    ),
                )
                .await?;
        } else {
            self.client
                .post_message(
                    &receipt.room_id,
                    &format!(
                        "Attempt {}/{} failed. I'm not starting another agent loop; a person should decide what happens next.",
                        receipt.attempt, self.config.max_attempts
                    ),
                )
                .await?;
        }
        Ok(())
    }

    async fn handle_countersign(&self, message: &IncomingMessage, id: &str) -> Result<()> {
        let id = id.trim().to_uppercase();
        let Some(receipt) = self.ledger.get(&id)? else {
            self.client
                .post_message(
                    &message.room_id,
                    &format!("I have no receipt `{id}`. Try `@verifier ledger`."),
                )
                .await?;
            return Ok(());
        };

        if receipt.verdict == Verdict::Fail {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{id}` failed verification. There is nothing to countersign — the change has to come back green first."
                    ),
                )
                .await?;
            return Ok(());
        }

        if let Some(by) = &receipt.countersigned_by {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{id}` was already countersigned by {by}. A receipt is countersigned once."
                    ),
                )
                .await?;
            return Ok(());
        }

        // Only a person can countersign. Resolve who the sender is with the
        // room's own participant list rather than parsing the opaque id.
        let participants = match self.client.list_participants().await {
            Ok(participants) => participants,
            Err(error) => {
                warn!("could not list participants: {error:#}");
                Vec::new()
            }
        };
        // `sender` is the room's client id for the sender, while
        // `list_participants` reports Switch participant ids; they are
        // different namespaces. Resolve by name instead, and fail closed when
        // the name is ambiguous or belongs to an agent.
        let matches: Vec<&serde_json::Value> = participants
            .iter()
            .filter(|participant| {
                participant.get("name").and_then(|value| value.as_str())
                    == Some(message.sender_name.as_str())
            })
            .collect();
        let kind = if matches.len() == 1 {
            matches[0].get("type").and_then(|value| value.as_str())
        } else {
            None
        };

        if kind != Some("user") {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "I can't countersign that from `{}`. A countersignature has to come from a person in this room, not from an agent.",
                        message.sender_name
                    ),
                )
                .await?;
            return Ok(());
        }

        if !self.config.is_approver(&message.sender_name) {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{}` is not an approver for this room. Approvers: {}.",
                        message.sender_name,
                        self.config.approvers.join(", ")
                    ),
                )
                .await?;
            return Ok(());
        }

        // The receipt is a claim about a specific commit on a specific base.
        // Re-check both before any irreversible action.
        let head = match rev(&self.config.repo, &receipt.branch).await {
            Ok(sha) => sha,
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!("I can't resolve `{}`: {error:#}", receipt.branch),
                    )
                    .await?;
                return Ok(());
            }
        };
        if head != receipt.commit_sha {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{}` no longer points at the verified commit. The receipt is for `{}`, the branch is at `{}`. Run `@verifier verify {}` again.",
                        receipt.branch,
                        short(&receipt.commit_sha),
                        short(&head),
                        receipt.branch
                    ),
                )
                .await?;
            return Ok(());
        }
        let base = match rev(&self.config.repo, &self.config.base_ref).await {
            Ok(sha) => sha,
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!(
                            "I can't resolve the base `{}`: {error:#}",
                            self.config.base_ref
                        ),
                    )
                    .await?;
                return Ok(());
            }
        };
        if !receipt.base_sha.is_empty() && base != receipt.base_sha {
            self.client
                .post_message(
                    &message.room_id,
                    &format!(
                        "`{}` moved since this receipt was issued. Run `@verifier verify {}` again so the proof is against the current base.",
                        self.config.base_ref, receipt.branch
                    ),
                )
                .await?;
            return Ok(());
        }

        self.client
            .post_message(
                &message.room_id,
                &format!(
                    "Countersigned by **{}**. Landing `{}` at `{}`.",
                    message.sender_name,
                    receipt.branch,
                    short(&receipt.commit_sha)
                ),
            )
            .await?;

        match land::land(&self.config, &receipt, &message.sender_name).await {
            Ok(where_it_landed) => {
                self.ledger
                    .countersign(&id, &message.sender_name, Some(&where_it_landed))?;
                let state = match self.config.pr_mode {
                    PrMode::Gh => "OPEN",
                    PrMode::Local => "LOCAL",
                };
                self.ledger.set_pr_state(&id, state)?;
                self.client
                    .post_message(
                        &message.room_id,
                        &format!(
                            "**LANDED · `{id}`**\n\n{where_it_landed}\n\nCountersigned by **{}** · commit `{}`.\n\nThe room keeps the whole record: who proposed, what was verified, who signed.",
                            message.sender_name,
                            short(&receipt.commit_sha)
                        ),
                    )
                    .await?;
            }
            Err(error) => {
                self.client
                    .post_message(
                        &message.room_id,
                        &format!(
                            "I could not land it: {error:#}\n\nNothing was merged, and the receipt is still unsigned. Fix the landing path, then countersign again."
                        ),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_ledger(&self, message: &IncomingMessage) -> Result<()> {
        let receipts = self.ledger.recent(8)?;
        if receipts.is_empty() {
            self.client
                .post_message(&message.room_id, "No receipts yet.")
                .await?;
            return Ok(());
        }
        let mut text = String::from("**Receipt ledger**\n");
        for receipt in receipts {
            let mark = match receipt.verdict {
                Verdict::Pass => "✅ PASS",
                Verdict::Fail => "❌ FAIL",
            };
            text.push_str(&format!(
                "- `{}` {} · attempt {} · `{}` · {}\n",
                receipt.id,
                mark,
                receipt.attempt,
                receipt.branch,
                match &receipt.countersigned_by {
                    Some(by) => format!(
                        "countersigned by {by}{}",
                        receipt
                            .landed_url
                            .as_ref()
                            .map(|url| format!(" → {url}"))
                            .unwrap_or_default()
                    ),
                    None => "not countersigned".to_string(),
                }
            ));
        }
        text.push_str("\nEvery entry is backed by a receipt in the ledger; the room log is the permanent record.");
        self.client.post_message(&message.room_id, &text).await?;
        Ok(())
    }

    async fn help(&self, room: &str) -> Result<()> {
        self.client
            .post_message(
                room,
                "**Verifier** — I check changes; I never write them.\n\n\
                 `@verifier verify <branch> [supersedes <receipt>]` — reproduce the report on the base, run the suite on the branch, issue a receipt.\n\
                 `@verifier countersign <receipt>` — an approver signs a passing receipt; the verified commit lands.\n\
                 `@verifier ledger` — recent receipts.\n\n\
                 Agents cannot countersign.",
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Handler for Verifier {
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
                    "Verifier online. I check changes and issue receipts; I never write the code I check.\n\n`@verifier verify <branch>` · `@verifier countersign <receipt>` · `@verifier ledger`",
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

        if let Some(rest) = lower.strip_prefix("countersign") {
            let offset = body.len() - rest.len();
            let id = body[offset..].trim();
            return self.handle_countersign(&message, id).await;
        }
        if lower.starts_with("verify") {
            let mut parts = body.split_whitespace();
            parts.next(); // "verify"
            let branch = parts.next().unwrap_or("").trim_matches('`');
            let mut supersedes = None;
            if let Some(token) = parts.next() {
                if token.eq_ignore_ascii_case("supersedes") {
                    supersedes = parts.next().map(str::to_string);
                }
            }
            return self
                .handle_verify(&message, branch, supersedes.as_deref())
                .await;
        }
        if lower.starts_with("ledger") {
            return self.handle_ledger(&message).await;
        }
        if lower.starts_with("help") || lower.is_empty() {
            return self.help(&message.room_id).await;
        }

        // Anything else addressed to the verifier is room conversation, not a
        // command. Stay quiet rather than adding noise to the room.
        Ok(())
    }
}

async fn rev(repo: &Path, reference: &str) -> Result<String> {
    let output = run(
        "git",
        &[
            "rev-parse".to_string(),
            "--verify".to_string(),
            reference.to_string(),
        ],
        repo,
        &[],
        Duration::from_secs(30),
    )
    .await?;
    if !output.success {
        bail!("git rev-parse {reference} failed");
    }
    Ok(output.combined.trim().to_string())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

fn format_receipt(
    receipt: &Receipt,
    viewer_url: Option<&str>,
    restricted: bool,
    base_ref: &str,
) -> String {
    let short_sha = short(&receipt.commit_sha);
    let seconds = receipt.duration_ms as f64 / 1000.0;
    let files = receipt
        .changed_files
        .iter()
        .map(|file| format!("`{file}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let ledger = viewer_url
        .map(|url| format!("\n\nFull evidence and diff: {url}"))
        .unwrap_or_default();
    let chain = match (&receipt.supersedes, receipt.attempt) {
        (Some(parent), attempt) => format!(" · attempt {attempt} · supersedes `{parent}`"),
        (None, 1) => String::new(),
        (None, attempt) => format!(" · attempt {attempt}"),
    };
    let diffstat = if receipt.diffstat.is_empty() {
        String::new()
    } else {
        format!(" · {}", receipt.diffstat)
    };

    match receipt.verdict {
        Verdict::Pass => {
            let summary = suite_summary(&receipt.suite_output);
            let sign_line = if restricted {
                format!("An approver can land it: **`@verifier countersign {id}`**. Agents cannot countersign.", id = receipt.id)
            } else {
                format!("A person can land it: **`@verifier countersign {id}`**. Agents cannot countersign.", id = receipt.id)
            };
            format!(
                "**✅ PASS · `{id}`** · `{branch}`{chain}\n\
                 Commit `{short_sha}` · {files}{diffstat} · {seconds:.1}s\n\n\
                 Reproduced the reported failure on `{base}`; the branch passed the full suite: **{summary}**.\n\n\
                 {sign_line}{ledger}",
                id = receipt.id,
                branch = receipt.branch,
                chain = chain,
                short_sha = short_sha,
                files = files,
                diffstat = diffstat,
                seconds = seconds,
                base = base_ref,
                summary = summary,
                sign_line = sign_line,
                ledger = ledger,
            )
        }
        Verdict::Fail => format!(
            "**❌ FAIL · `{id}`** · `{branch}`{chain}\n\
             Commit `{short_sha}` · {files}{diffstat} · {seconds:.1}s\n\n\
             {reason}\n\n\
             ```text\n{evidence}\n```\n\n\
             Nothing can land from a failed receipt.{ledger}",
            id = receipt.id,
            branch = receipt.branch,
            chain = chain,
            short_sha = short_sha,
            files = files,
            diffstat = diffstat,
            seconds = seconds,
            reason = receipt
                .reason
                .clone()
                .unwrap_or_else(|| "Verification failed.".to_string()),
            evidence = truncate(
                if receipt.suite_output.trim().is_empty() {
                    &receipt.base_output
                } else {
                    &receipt.suite_output
                },
                700,
            ),
            ledger = ledger,
        ),
    }
}

/// Sum the `test result:` lines across every test binary in the run.
fn suite_summary(output: &str) -> String {
    let mut passed = 0u64;
    let mut failed = 0u64;
    let mut seen = false;
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("test result:") else {
            continue;
        };
        seen = true;
        for part in rest.split(';') {
            if let Some(value) = count_before(part, "passed") {
                passed += value;
            }
            if let Some(value) = count_before(part, "failed") {
                failed += value;
            }
        }
    }
    if !seen {
        "the suite passed".to_string()
    } else {
        format!("{passed} passed; {failed} failed")
    }
}

/// `" ok. 8 passed"` + `"passed"` -> `8`.
fn count_before(part: &str, word: &str) -> Option<u64> {
    let index = part.find(word)?;
    part[..index].split_whitespace().last()?.parse().ok()
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
    use super::{count_before, count_before as _, suite_summary};

    #[test]
    fn counts_a_single_test_result_line() {
        assert_eq!(count_before(" ok. 8 passed", "passed"), Some(8));
        assert_eq!(count_before(" 0 failed", "failed"), Some(0));
    }

    #[test]
    fn sums_every_test_binary() {
        let output = "\
running 2 tests
test result: ok. 2 passed; 0 failed; 0 ignored

running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored

running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored
";
        assert_eq!(suite_summary(output), "8 passed; 0 failed");
    }

    #[test]
    fn reports_failures_across_binaries() {
        let output = "test result: ok. 2 passed; 0 failed\ntest result: FAILED. 1 passed; 2 failed";
        assert_eq!(suite_summary(output), "3 passed; 2 failed");
    }
}
