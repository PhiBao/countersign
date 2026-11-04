#!/usr/bin/env python3
"""Assemble the Countersign demo video from the recorded clips.

Inputs (produced by scripts/record-demo.mjs and edge-tts):
  /tmp/countersign-video/*.webm, card-intro.png, card-outro.png, vo-*.mp3

Output:
  /tmp/countersign-video/countersign-demo.mp4
"""

import json
import pathlib
import subprocess

ROOT = pathlib.Path("/tmp/countersign-video")
FONT = "/tmp/space-grotesk.ttf"
W, H = 1920, 1080
FPS = 30

# name, audio file, target duration padding, caption (or None), clips
SECTIONS = [
    {
        "name": "intro",
        "audio": "vo-intro.mp3",
        "pad": 0.5,
        "card": "card-intro.png",
        "caption": None,
    },
    {
        "name": "fail",
        "audio": "vo-fail.mp3",
        "pad": 0.4,
        "caption": "A passing regression test is not proof.",
        "clips": [("01-request-fail.webm", 3.2, 4.5), ("02-fail-receipt.webm", 4.0, 8.3)],
    },
    {
        "name": "revise",
        "audio": "vo-revise.mp3",
        "pad": 0.4,
        "caption": "The verifier hands the failure back.",
        "clips": [("03-revise.webm", 3.0, 6.0), ("04-proposal.webm", 3.5, 4.0)],
    },
    {
        "name": "pass",
        "audio": "vo-pass.mp3",
        "pad": 0.4,
        "caption": "Attempt two. Chained to the first receipt.",
        "clips": [("04-proposal.webm", 6.0, 2.0), ("05-pass.webm", 4.0, 5.0)],
    },
    {
        "name": "sign",
        "audio": "vo-sign.mp3",
        "pad": 0.4,
        "caption": "An approver signs the verified commit.",
        "clips": [("06-countersign.webm", 3.0, 4.0), ("07-landed.webm", 4.0, 5.0)],
    },
    {
        "name": "ledger",
        "audio": "vo-ledger.mp3",
        "pad": 0.5,
        "caption": "Every receipt is evidence.",
        "clips": [("08-ledger.webm", 3.5, 8.0)],
    },
    {
        "name": "outro",
        "audio": "vo-outro.mp3",
        "pad": 0.8,
        "card": "card-outro.png",
        "caption": None,
    },
]



def run(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"{' '.join(args)}\n{result.stderr[-2000:]}")
    return result


def duration(path):
    result = run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            str(path),
        ]
    )
    return float(result.stdout.strip())


def cut_clip(source, start, length, target):
    run(
        [
            "ffmpeg",
            "-y",
            "-ss",
            str(start),
            "-t",
            str(length),
            "-i",
            str(ROOT / source),
            "-vf",
            "crop=1920:1040:0:40,pad=1920:1080:0:20:color=0xf4f1e8,fps=30,format=yuv420p",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "18",
            str(target),
        ]
    )


def caption_path(name):
    return ROOT / f"caption-{name}.png"


segments = []
for section in SECTIONS:
    audio = ROOT / section["audio"]
    target_duration = duration(audio) + section["pad"]
    out = ROOT / f"section-{section['name']}.mp4"

    if "card" in section:
        run(
            [
                "ffmpeg",
                "-y",
                "-loop",
                "1",
                "-i",
                str(ROOT / section["card"]),
                "-i",
                str(audio),
                "-vf",
                f"fps={FPS},format=yuv420p",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-ar",
                "48000",
                "-t",
                f"{target_duration:.3f}",
                str(out),
            ]
        )
    else:
        parts = []
        for index, (source, start, length) in enumerate(section["clips"]):
            part = ROOT / f"part-{section['name']}-{index}.mp4"
            cut_clip(source, start, length, part)
            parts.append(part)

        concat_list = ROOT / f"concat-{section['name']}.txt"
        concat_list.write_text(
            "\n".join(f"file '{part}'" for part in parts) + "\n"
        )
        joined = ROOT / f"joined-{section['name']}.mp4"
        run(
            [
                "ffmpeg",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                str(concat_list),
                "-c",
                "copy",
                str(joined),
            ]
        )

        caption = caption_path(section["name"])
        has_caption = section["caption"] and caption.exists()
        if has_caption:
            filter_complex = (
                f"[0:v]tpad=stop_mode=clone:stop_duration=6,"
                f"trim=duration={target_duration:.3f},setpts=PTS-STARTPTS[v0];"
                f"[2:v]format=rgba[c];[v0][c]overlay=0:0[v];"
                f"[1:a]aresample=48000,apad[a]"
            )
        else:
            filter_complex = (
                f"[0:v]tpad=stop_mode=clone:stop_duration=6,"
                f"trim=duration={target_duration:.3f},setpts=PTS-STARTPTS[v];"
                f"[1:a]aresample=48000,apad[a]"
            )
        args = [
            "ffmpeg",
            "-y",
            "-i",
            str(joined),
            "-i",
            str(audio),
        ]
        if has_caption:
            args.extend(["-i", str(caption)])
        args.extend(
            [
                "-filter_complex",
                filter_complex,
                "-map",
                "[v]",
                "-map",
                "[a]",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-t",
                f"{target_duration:.3f}",
                str(out),
            ]
        )
        run(args)

    segments.append(out)
    print(f"{section['name']}: {target_duration:.2f}s")

concat_list = ROOT / "concat-final.txt"
concat_list.write_text("\n".join(f"file '{segment}'" for segment in segments) + "\n")
joined = ROOT / "joined-final.mp4"
run(
    [
        "ffmpeg",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        str(concat_list),
        "-c",
        "copy",
        str(joined),
    ]
)

total = duration(joined)
final = ROOT / "countersign-demo.mp4"
run(
    [
        "ffmpeg",
        "-y",
        "-i",
        str(joined),
        "-vf",
        f"fade=t=in:st=0:d=0.5,fade=t=out:st={max(total - 0.7, 0):.3f}:d=0.7",
        "-af",
        "loudnorm=I=-16:TP=-1.5:LRA=11",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        str(final),
    ]
)

print(json.dumps({"file": str(final), "duration": round(duration(final), 2)}, indent=2))
