# Countersign demo

`countersign-demo.mp4` is the 66-second submission video. It was recorded from the live room and the live ledger in this repository — no mockups.

## What the video shows

| Time | Shot | Claim |
|---|---|---|
| 0:00 | Title card | The product thesis: no agent ships alone |
| 0:15 | The room | A teammate asks the verifier to check a one-line hotfix |
| 0:21 | FAIL receipt | The hotfix passes the reported test; the full suite catches the refund regression |
| 0:29 | The hand-back | The verifier sends the failure to the fixer as a work order |
| 0:36 | The retry | The fixer starts from the rejected branch and proposes attempt two |
| 0:42 | PASS receipt | Eight tests pass; the receipt chains to the failed one |
| 0:48 | Countersignature | An approver signs the verified commit; a real PR opens |
| 0:55 | Proof ledger | Both receipts, the diff, and the signature |
| 1:03 | End card | `github.com/PhiBao/countersign` |

## Voiceover

> Agents now open more than forty percent of GitHub's pull requests. Teams cannot review them fast enough. Countersign is a Switch room where agents propose, a verifier proves, and an approver signs.
>
> A teammate's one-line hotfix passes the reported test. The verifier reruns the full suite on a clean checkout, and rejects it. Refunds still round the wrong way.
>
> The verifier hands the failure back to the fixer with the exact output. The fixer starts from the rejected branch and fixes what the receipt caught.
>
> Attempt two passes all eight tests, and the new receipt chains to the first one.
>
> An approver sees the diff, countersigns the verified commit, and a real pull request opens.
>
> Both receipts live in the proof ledger, and in the room itself. What was proposed, what was proven, who signed.
>
> No agent ships alone. Countersign.

## How it was built

1. `scripts/record-demo.mjs` drives Mattermost with Playwright and records eight short clips. Each clip opens a fresh page *after* the state it shows exists, so the edit has no dead time.
2. `edge-tts` (voice `en-US-AriaNeural`) generates one voiceover track per section.
3. `scripts/build-video.py` cuts, crops, overlays captions, muxes the audio, and concatenates the sections.

Recording requires the stack, the room, and both agents running, plus `playwright-core` (`npm i playwright-core` in a scratch directory) and `edge-tts`. The raw clips and intermediate files live in `/tmp/countersign-video` and are not committed.

The room's chat surface is Mattermost because it is the bundled, zero-admin chat app of the local Switch stack. The same flow works on a connected Slack, Teams, or Discord channel.
