# Countersign demo

`countersign-demo.mp4` is the 78-second submission video. It was recorded from the live room and the live ledger in this repository — no mockups.

The cut was produced with [`open-video-demo`](https://github.com/PhiBao/open-video-demo): animated HTML cards, Playwright recording of the real Mattermost room and the proof ledger, edge-tts narration, burned subtitles, and an `.srt` sidecar for uploads. The source story and branding are in [`source/story.json`](source/story.json) and [`source/video.json`](source/video.json).

## What the video shows

| Time | Shot | Claim |
|---|---|---|
| 0:00 | Title card | Coding agents are fast; review is not |
| 0:12 | The room | A teammate asks the verifier to check a one-line refund fix |
| 0:19 | FAIL receipt | It passes the reported test; the full suite catches the refund regression |
| 0:31 | The hand-back | The verifier sends the failure to the fixer as a work order |
| 0:38 | The retry | The fixer starts from the rejected branch and proposes attempt two |
| 0:44 | PASS receipt | Eight tests pass; the receipt chains to the failed one |
| 0:54 | Countersignature | An approver signs the verified commit; a real PR opens |
| 1:04 | Proof ledger | Both receipts, the diff, and the signature |
| 1:13 | End card | `github.com/PhiBao/countersign` |

## Voiceover

> Coding agents can open a pull request in minutes. Review still takes a person. And the agent that wrote the change is usually the one telling you it works.
>
> Countersign is a real team room where that changes. A teammate tried a one-line refund fix. It passes the test they were given. The verifier runs the whole suite on a clean checkout, and rejects it. The refund still rounds the wrong way.
>
> The verifier does not just say no. It sends the failure back to the fixer with the exact output. The fixer starts from the rejected branch and fixes what the proof caught.
>
> The second attempt passes every test. The new receipt is chained to the first one, so you can see what changed between them.
>
> An approver looks at the diff, signs the exact commit, and a real pull request opens. The agent never had the authority to land it.
>
> Every receipt lives in the proof ledger, and in the room itself. What was proposed, what was proven, who signed.
>
> No agent ships alone. Countersign.

## Rebuilding

The story/config are ordinary `open-video-demo` project files. From an `open-video-demo` checkout:

```bash
cp docs/demo/source/story.json      videos/countersign-demo/script/story.json
cp docs/demo/source/video.json      videos/countersign-demo/config/video.json

# The room needs an authenticated Mattermost session and the preview banner hidden.
DEMO_STORAGE_STATE=/path/to/mm-storage.json \
DEMO_INIT_SCRIPT=/path/to/countersign-demo-init.js \
./scripts/run.sh --project countersign-demo
```

`DEMO_STORAGE_STATE` is a Playwright storage-state JSON with the `MMAUTHTOKEN` and `MMUSERID` cookies plus `__landingPageSeen__` in localStorage. The init script hides the Mattermost preview banner and sets the cursor accent to Countersign orange.

The chat surface is Mattermost because it is the bundled, zero-admin chat app of the local Switch stack. The same room works over a connected Slack, Teams, or Discord channel; the video uses Mattermost so a judge can reproduce it with no workspace admin.
