# No Agent Ships Alone

**Your coding agent grades its own homework. I built a room where it can’t.**

*[Watch the 78-second demo](VIDEO_URL) · [Live proof ledger](https://countersign-proof.kiter0211.workers.dev/) · [GitHub](https://github.com/PhiBao/countersign)*

---

A teammate pushed a one-line fix for a refund rounding bug. It passed the test they were given. The suite was green. The summary said the change was safe.

It was still wrong. The refund rounded in the wrong direction.

That is the shape of the problem I keep running into with coding agents. The agent that writes the change is also the one reporting whether it works. It runs the tests in a terminal only it can see, summarizes the result, and asks a person to believe the summary. Most of the time we do, because the alternative is redoing the work ourselves.

We have made generation fast and verification manual. That is backwards.

---

## The review bottleneck is not a tooling problem

The numbers are now hard to ignore. In September 2026, agents started more than 40% of the pull requests on GitHub — roughly 110 million of them. Under high AI adoption, PRs are 51% larger, median review time is up 441%, and 31% more changes merge with no review at all.

The bottleneck is no longer writing code. It is trusting it.

The usual answers do not fix the underlying issue. A coding agent in chat can generate a change and report its own test run. An AI reviewer can comment on a pull request. An observability dashboard can show what agents are doing. None of them give the team an independent proof that a specific change works, and none of them put the decision in the same place as the evidence.

---

## The idea: separation of duties

The fix is not a better agent. It is a different shape of collaboration.

In any serious change-control process, the person who proposes a change is not the person who approves it. Agents should work the same way:

> The proposer cannot verify. The verifier cannot write. Only an approver can countersign. Only a passing receipt can be countersigned.

I built Countersign around those four sentences. It is a room in your team chat, built on Switch, with three roles:

- a **fixer** — an existing coding agent that proposes a branch in its own checkout;
- a **verifier** — deterministic Rust that reproduces the failure and runs the proof;
- an **approver** — a person who signs the exact verified commit.

The important part is that the verifier is a separate process with different capabilities and different credentials. It cannot edit the code it checks. It does not share a session with the fixer. It is not a prompt telling the agent to be careful.

![Countersign sequence](https://phibao.github.io/countersign/diagrams/sequence.png)

*The loop: propose, prove, countersign, land.*

---

## What the loop looks like

A person reports the problem. The fixer proposes a branch. The verifier does the same thing every time, with no model in the loop:

1. Create a clean worktree at the base revision.
2. Run the reported regression test. It **must fail** — otherwise the verifier refuses to certify a fix for a bug that is not there.
3. Check out the proposed branch in the same worktree.
4. Run the **full** suite in a scrubbed environment. It must pass.
5. Refuse any branch that edits its own tests or touches files outside the allowed paths.
6. Issue a receipt with the commit SHA, base SHA, changed files, diffstat, full diff, exact command output, and a verdict.

The first version of this loop was one-directional: the verifier said yes or no, and a human had to re-ask the fixer after a failure. That is half a workflow. A failure should become a work order.

So a failed receipt now goes back to the fixer with the exact output:

```text
@fixer revise R-C3285C — the receipt has the failing tests
and the exact output. Attempt 2/2.
```

The fixer starts from the rejected branch, fixes what the proof caught, and proposes a retry. The second receipt is chained to the first:

```text
attempt 2 · supersedes R-C3285C
```

That chain is the part I care about most. It shows what changed between attempts, why the first one failed, and who decided the second one was ready.

---

## The signature has to mean something

A receipt is only useful if the landing step trusts it. So the countersignature is not a reaction emoji.

An approver replies:

```text
@verifier countersign R-0176E1
```

The verifier checks that the sender is a human in the room and one of the configured approvers. It re-checks that the branch still points at the commit it verified and that the base has not moved. Then it pushes **that commit** — not whatever the branch name points at now — and opens a real pull request with the receipt in the body.

If the branch moved, the signature is refused. If the base moved, the receipt is stale and the change is verified again. The agent never had the authority to land its own work.

---

## Why Switch

I did not want to build another chat app or another agent framework. I wanted the room itself to be the control plane. Switch gives a workflow three things that are hard to fake:

- **Roles that outlive a particular agent.** `@verifier` is a job, not a process. An exclusive role means two verifiers cannot issue contradictory receipts.
- **Addressing policy.** The room decides who can instruct an agent. The verifier is not reachable by a random channel member.
- **A shared log plus room documents.** Every receipt is posted to the room and also written as a room-scoped document, discoverable with `!list-documents`. The reasoning is still there next month when the same package ships another release.

The verifier is a Rust client that speaks the Switch agent protocol directly: WebSocket events in, HTTP operations out. It calls `connect_to_room`, `assume_role`, `post_message`, `list_participants`, and `create_room_document`. It is not a wrapper around the desktop app, and it is not a bot that happens to sit in a channel.

The demo uses Mattermost because Switch bundles it and it needs no workspace admin. The same room bridges to Slack, Teams, Discord, or Telegram.

---

## Three things that were harder than expected

**Coding agents resolve project roots in their own ways.** I first gave the fixer a git worktree. It followed the worktree’s git directory back into my checkout and edited `main`. The fix was to give it a plain copy with no `.git` at all, outside the repository, and to publish the result through a worktree the agent never sees. A second layer: the child process inherited `PWD`, so even with the right working directory it resolved the old project root. Set both.

**Cargo can quietly reuse a stale test binary.** The verifier created two worktrees and shared one target directory. The branch run executed the base worktree’s test binary, so a failing branch looked like it failed for the wrong reason. One worktree, checkout the base then the branch, fixed it. A verification product that lies is worse than no verification product.

**A headless browser does not always receive chat updates.** The recorder needed the room to be in a known state, so each clip opens a fresh page after the state exists. That also made the edit better: no dead time waiting for an agent.

---

## What is real, and what is not

The room, the roles, the addressing policy, the protocol client, the worktree, the test runs, the receipts, the room documents, and the pull request are real. The verifier has no model in the proof path.

The bug in the demo is deliberately seeded. It has to be, or the demo is not reproducible. The failed branch is a real branch that passes the reported test and fails the invariants. The verifier catches it for the right reason.

The limitations are real too. The human and approver checks resolve the sender by room participant name, because Switch exposes client IDs and participant IDs in different namespaces; production needs a bridge-identity mapping and signed receipts. Fork PRs are refused because landing would push to the wrong remote. The fixer is confined to a disposable copy but is not OS-sandboxed. The viewer is a read-only proof surface, not a multi-user dashboard.

---

## Try it

The repository has the whole loop, a one-command local stack, the seeded bug, and the proof ledger:

**https://github.com/PhiBao/countersign**

- [Project page](https://phibao.github.io/countersign/)
- [Live proof ledger](https://countersign-proof.kiter0211.workers.dev/)
- [78-second demo](VIDEO_URL)

If you take one idea from this, make it this: the valuable agent is not the one that writes the code. It is the one that can prove the code is safe to land — and the person who decides that it is.

No agent ships alone.

---

*Built for the Flip the Switch Hackathon (WeMakeDevs × DevTools Academy, powered by Switch AI), Track 01 — Best Business Workflow.*
