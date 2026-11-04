# Countersign demo

`countersign-demo.mp4` is the 66-second submission video. It was recorded from the live room and the live ledger in this repository — no mockups.

## What the video shows

| Time | Shot | Claim |
|---|---|---|
| 0:00 | Title card | The product thesis: no agent ships alone |
| 0:15 | The room | A teammate asks the verifier to check a one-line hotfix |
| 0:21 | FAIL receipt | The hotfix passes the reported test; the full suite catches the refund regression |
| 0:29 | The fix request | The fixer works in its own checkout and hands off a branch |
| 0:35 | PASS receipt | Reproduced on `main`; 8 tests pass on the branch |
| 0:43 | Countersignature | Only a person can sign; a real pull request opens |
| 0:50 | Proof ledger | Both receipts, the signature, and the PR link |
| 1:03 | End card | `github.com/PhiBao/countersign` |

## Voiceover

> Agents now open more than forty percent of GitHub's pull requests. Teams cannot review them fast enough. Countersign is a Switch room where agents propose, a verifier proves, and a person signs.
>
> A teammate's one-line hotfix passes the reported test. The verifier reruns the full suite on a clean checkout, and rejects it. Refunds still round the wrong way.
>
> The fixer takes the real report, works in its own checkout, and hands off a branch. The verifier reproduces the failure on main, runs all eight tests, and issues a passing receipt with the evidence.
>
> Only a person can countersign. The verifier confirms the signer is human, records it, and opens a real pull request.
>
> Every receipt lands in the proof ledger. What was proposed, what was proven, who signed.
>
> No agent ships alone. Countersign.

## How it was built

1. `scripts/record-demo.mjs` drives Mattermost with Playwright and records eight short clips. Each clip opens a fresh page *after* the state it shows exists, so the edit has no dead time.
2. `edge-tts` (voice `en-US-AriaNeural`) generates one voiceover track per section.
3. `scripts/build-video.py` cuts, crops, overlays captions, muxes the audio, and concatenates the sections.

Recording requires the stack, the room, and both agents running, plus `playwright-core` and `edge-tts`. The raw clips and intermediate files live in `/tmp/countersign-video` and are not committed.

The room's chat surface is Mattermost because it is the bundled, zero-admin chat app of the local Switch stack. The same flow works on a connected Slack, Teams, or Discord channel.
