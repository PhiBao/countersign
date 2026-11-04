# No agent ships alone: building a countersignature for AI work on Switch

*How a Rust verifier, an existing coding agent, and one human decision became a Switch room where agent work has to be proven before it lands.*

---

Coding agents are now a normal part of a team's output. In September 2026, agents started more than 40% of the pull requests on GitHub — roughly 110 million of them. Review did not scale with generation: under high AI adoption, PRs are larger, review time is up sharply, and more changes merge with no review at all.

The bottleneck is no longer writing code. It is trusting it.

Most agent-in-chat setups make this worse in a subtle way: the agent that wrote the change is also the one reporting whether it works. It runs the tests in a terminal only it can see, summarizes the result, and asks a person to believe the summary. The agent grades its own homework, and the room has no way to tell.

[Countersign](https://github.com/PhiBao/countersign) is my answer to that, built for the Flip the Switch Hackathon. The rule is structural:

> **The proposer cannot verify. The verifier cannot write. Only a person can countersign. Only a passing receipt can be countersigned.**

This post is about why that shape matters, how Switch made it possible, and the sharp edges I hit while building it — so the next person building a Switch workflow can skip them.

---

## Why Switch, and not another bot

A Slack bot can post a message. It cannot give you a **role** that outlives the person holding it, an **addressing policy** that decides who may ask whom, or a shared log that every participant reads from. Those are the primitives Countersign needs, and they are exactly what Switch provides:

- **A room** is the channel plus a durable log and a shared briefing.
- **Roles** are jobs, not agents. `@verifier` reaches whoever holds the verifier job, and an exclusive role means only one agent can hold it at a time.
- **Addressing policy** decides who may instruct an agent — the owner, the owner's agents, or anyone in the room.
- **The agent protocol** lets any agent join: events down a WebSocket, operations up over HTTP.

That last point mattered most. I did not want a desktop app in the loop. I wanted the verifier to be a process I could read, test, and reproduce. So Countersign's verifier is a Rust client that speaks the protocol directly.

The strongest test of whether Switch is load-bearing: remove it and the product disappears. No rooms, no roles, no addressing policy, no shared record to attach a receipt to, no identity for a countersignature. It is not a bot bolted onto a channel; the room is the control plane.

---

## The shape of the workflow

One room, two agents, one person:

```
Person        reports the failing refund total
Fixer         proposes a branch in its own checkout
Verifier      reproduces the failure on main, runs the full suite on the branch,
              refuses branches that edit their own tests, issues a receipt
Person        countersigns the passing receipt
Verifier      opens the pull request (or merges locally)
```

The fixer is **OpenCode**, an existing coding agent, driven in a disposable copy of the repository. The verifier is **deterministic Rust** — no model calls in the proof path.

They genuinely cannot be one agent. They have different capabilities, different credentials, and different failure modes. The fixer is creative and untrusted; the verifier is narrow and trusted. Collapsing them is what every "agent reports its own tests" workflow does, and it is the thing that makes the report worthless.

### The receipt

Every verification produces a receipt:

- the branch and commit SHA,
- the changed files,
- the exact command output from reproducing the failure on `main`,
- the exact output from the full suite on the branch,
- a verdict and a duration.

The receipt is posted to the room and stored in a SQLite ledger. The room is the permanent record; the ledger is its index.

A person replies `@verifier countersign R-XXXXXX`. The verifier resolves the sender through the room's participant list and requires a **human**. An agent's countersignature is refused. Only then does it push the branch and open a pull request with the receipt in the body. If landing fails, the receipt stays unsigned.

### The checks that make it a gate

The verifier is deliberately boring. Before it runs anything, it rejects a branch that touches its own tests (`demo/orders/tests/**`) or anything outside the fixture. Then:

1. Create a clean worktree at `main`.
2. Run the reported regression test. It **must fail** — otherwise the verifier refuses to certify a fix for a bug that is not there.
3. Check out the branch in the same worktree.
4. Run the **full** suite. It must pass.

A passing regression test is not proof. A branch that edits the test is not a fix. A fix for a bug that does not reproduce is not a fix. Each of those is a real failure mode, and each one produces a FAIL receipt rather than a green check.

---

## Building the Switch client in Rust

Switch's agent protocol is small enough to implement directly: one WebSocket for events, HTTP for operations.

Two details are easy to miss:

1. **The server sends application-level `ping` frames, not WebSocket protocol pings.** The client must answer `{"type":"pong","cursor":n}` within the heartbeat TTL (6 seconds), or the connection is swept.
2. **An `all`-scope connection covers rooms without claiming them.** `connection_state.rooms` can be empty even though the agent is a member of several rooms. I had to call `list_rooms` and then `connect_to_room` explicitly. The first version of the client sat there connected and silent, which is a very confusing failure.

The operations I use are `connect_to_room`, `assume_role`, `post_message`, and `list_participants`. I read the operation list at startup rather than hard-coding it, which is what the protocol asks for.

Registration is the usual one-time exchange: a registration key creates the agent, returns an API key once, and `overwrite: true` rotates it on re-runs. The bootstrap script is idempotent, because a demo you cannot reset is a demo you cannot trust.

Roles are the part I would keep in any Switch project. `@fixer` and `@verifier` are jobs, and the room's instructions travel with the job. The verifier holds an **exclusive** role, so two verifiers cannot issue contradictory receipts. That is a policy guarantee, not a prompt.

---

## Three sharp edges worth knowing

### 1. Coding agents resolve project roots in their own ways

The fixer runs OpenCode in a checkout of the repository. I first used a git worktree. OpenCode followed the worktree's git directory back to the operator's checkout and edited `main` — exactly what the design is supposed to prevent.

The fix was to give the agent a **plain copy with no `.git` at all**, outside the repository, and to have the harness commit the result through a worktree the agent never sees. The agent gets a disposable workspace; the repository stays untouched until a human signs.

There was a second layer: the child process inherited `PWD` from the harness, so even with `current_dir` set, the agent resolved the old project root. Setting `PWD` explicitly fixed it. If you drive a coding agent from a service, set both the working directory and `PWD`.

### 2. Cargo can reuse a stale test binary between worktrees

The verifier created two worktrees (main and branch) and shared one `CARGO_TARGET_DIR`. The branch run silently executed the base worktree's test binary, so a failing branch looked like it failed for the wrong reason.

Using **one worktree and checking out the base, then the branch**, keeps the path stable and forces cargo to rebuild the crate. If you use two worktrees, give each its own target directory. This is the kind of bug that makes a verification product lie, so it is worth testing explicitly.

### 3. Mattermost does not live-update in a headless browser

The demo recorder drives the Mattermost web client with Playwright. The channel loaded, but new messages never appeared: the websocket did not connect in that context. The fix for the recording harness was to record each state **after** it exists, opening a fresh page per clip. That also made the edit better: no dead time waiting for an agent.

This is a harness problem, not a Switch problem — but if you script a demo, plan for it.

---

## What is real, and what is not

Everything in the proof path is real:

- Switch rooms, roles, addressing, and the log run on a self-hosted Switch 0.31.0 stack with bundled Mattermost.
- The verifier is a real protocol client, not the Console.
- The reproduction and test runs are real `git` and `cargo` invocations in a clean worktree.
- The countersignature is checked against the room's participants and refused for agents.
- The pull request is a real GitHub PR with the receipt in the body.

The demo's bug is deliberately seeded: a half-cent rounding error in a checkout fixture. That is what makes the demo reproducible. The FAIL branch (`hotfix/naive-rounding`) is a real one-line fix that passes the reported test and fails the invariants. The verifier catches it for the right reason.

The one honest limitation: the human check resolves the sender by room participant name, because Switch exposes client ids and participant ids in different namespaces. A production build should map the bridge identity to a Switch user id and sign receipts with a key. That is the next step, not a hidden assumption.

---

## Advice for the next Switch workflow

- **Use roles, not names.** A job survives the agent being replaced. `@reviewer` is a better interface than `@alice-reviewer-2`.
- **Write room instructions once.** They are the briefing every agent reads on join. Do not repeat team conventions in every agent prompt.
- **Set addressing policy deliberately.** The default is owner-only. A colleague who cannot get an answer usually has not been admitted, not found a bug.
- **Pin the server version.** `SWITCH_VERSION=0.31.0`. The protocol and Console are versioned separately.
- **Use the bundled Mattermost for the demo.** No workspace admin, no OAuth app, no public URL. Connect Slack or Teams later.
- **Do not build a chatbot and call it a workflow.** The judges' own guide says it: if one agent answering questions is the whole product, a webhook could have done it. Find the second job that has to happen somewhere else.

---

## Try it

The repository contains the whole loop, a one-command local stack, the demo fixture, and the proof ledger:

**https://github.com/PhiBao/countersign**

The demo is 66 seconds: a hotfix is rejected, the real fix passes, a person signs, and a pull request opens. The ledger keeps both receipts.

If you take one idea from this: **the valuable agent is not the one that writes the code. It is the one that can prove the code is safe to land — and the person who decides that it is.**

No agent ships alone.

---

*Built for the Flip the Switch Hackathon (WeMakeDevs × DevTools Academy, powered by Switch AI), Track 01 — Best Business Workflow.*
