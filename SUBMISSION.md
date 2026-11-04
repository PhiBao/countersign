# Submission — Countersign

**Hackathon:** Flip the Switch (WeMakeDevs × DevTools Academy, powered by Switch AI)
**Track:** 01 — Best Business Workflow
**Window:** November 4–7, 2026

---

## One line

A Switch room where AI agents propose changes, an independent verifier proves or refutes them with reproducible evidence, and a named person countersigns before anything lands.

## Links

| | |
|---|---|
| Repository | https://github.com/PhiBao/countersign |
| Demo video (66s) | [docs/demo/countersign-demo.mp4](docs/demo/countersign-demo.mp4) · https://phibao.github.io/countersign/demo/countersign-demo.mp4 |
| Project page | https://phibao.github.io/countersign/ |
| Build note (Best Blog) | https://phibao.github.io/countersign/blog/building-countersign.html |
| Live proof | https://github.com/PhiBao/countersign/pull/4 |

## The two-minute path for a judge

1. Watch the 66-second video. It shows a real Mattermost room, a real FAIL receipt, a real PASS receipt, a real countersignature, and a real GitHub pull request.
2. Read the top of the README for the problem and the mechanism.
3. Open the proof ledger screenshot / run `http://127.0.0.1:4090` after setup.
4. If you want to reproduce: `./scripts/init-env.sh`, `docker compose up -d`, `./scripts/bootstrap.sh`, `./scripts/start-agents.sh`, then post the two messages from the README.

## How this uses Switch (the judging criterion)

Switch is not an integration detail; it is the control plane.

- **Rooms** hold the report, the proposal, the receipt, the signature, and the reasoning after the fact.
- **Roles** (`@fixer`, `@verifier`) make the job addressable rather than a particular agent. The verifier role is exclusive, so two verifiers cannot issue contradictory receipts.
- **Addressing policy** decides who may instruct an agent; the bootstrap opens both agents to room members.
- **Room instructions** carry the rules every agent reads on join: propose, never land; verify, never write; a person signs.
- **The agent protocol** is implemented directly in Rust: WebSocket events, `pong` + cursor, `connect_to_room`, `assume_role`, `post_message`, `list_participants`.

Remove Switch and the product has no room, no roles, no policy, no shared record, and no identity to attach a signature to.

## What is real

- Self-hosted Switch 0.31.0 with bundled Mattermost.
- A Rust verifier with **zero model calls in the proof path**: clean worktree, reproduce the failure on `main`, full suite on the branch, scope and test-tampering checks, receipt.
- A fixer driven by an existing coding agent (OpenCode), isolated in a disposable copy of the repository.
- Human-only countersignature, checked against the room's participants.
- A real GitHub pull request with the receipt in the body.
- A proof ledger served by the same Rust process.

The demo bug is deliberately seeded; the FAIL branch, tests, receipts, PR, and signature are real.

## Judging criteria

| Criterion | Evidence |
|---|---|
| **Use of Switch** | The room is the control plane; removing Switch removes roles, addressing, the shared log, and the identity behind a countersignature |
| **Technical execution** | Protocol-level Rust agent, deterministic verification, scope enforcement, one-command stack, tests, reproducible demo |
| **Creativity and impact** | A real team problem (agent output outpacing review) with a structural mechanism — separation of duties — and a credible path beyond the demo |
| **Presentation** | A 66-second demo with a FAIL that is visible, a PASS that is earned, a signature that is a person's, and a public build note |

## Known limitations (stated, not hidden)

- One room and one repository per deployment; not multi-tenant.
- The human check resolves by room participant name because Switch exposes client ids and participant ids in different namespaces; production should map the bridge identity to a Switch user and sign receipts with a key.
- The fixer executes model-chosen commands on the operator's machine; it is confined to a disposable copy but not OS-sandboxed.
- The viewer and ledger API are loopback-only and unauthenticated — a proof surface, not a dashboard.
- `COUNTERSIGN_PR_MODE=local` produces a real merge on `countersign/landed`, not a PR; this is for offline reproduction.

## Built during the window

The Switch room, roles, room instructions, agent registrations, verifier, fixer harness, ledger, viewer, diagrams, and demo were built for this submission. OpenCode is an existing coding agent, as the brief allows.
