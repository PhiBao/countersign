#!/usr/bin/env python3
"""Render the blog markdown into the Countersign site shell."""

import pathlib
import sys

import markdown

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs/blog/building-countersign.md"
TARGET = ROOT / "docs/blog/building-countersign.html"

SHELL = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>No agent ships alone · Countersign</title>
  <meta name="description" content="Building a countersignature for AI work on Switch: a Rust verifier, an existing coding agent, and one human decision." />
  <link rel="icon" type="image/svg+xml" href="../favicon.svg" />
  <link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=Space+Grotesk:wght@400;500;600&display=swap" rel="stylesheet" />
  <style>
    :root {{
      --paper: #f4f1e8;
      --ink: #141312;
      --muted: #6f695d;
      --rule: #d9d2c2;
      --accent: #d9481c;
      --mono: "IBM Plex Mono", ui-monospace, monospace;
      --sans: "Space Grotesk", system-ui, sans-serif;
    }}
    * {{ box-sizing: border-box; }}
    html {{ background: var(--paper); }}
    body {{
      margin: 0;
      background: var(--paper);
      color: var(--ink);
      font-family: var(--sans);
      font-size: 19px;
      line-height: 1.68;
      -webkit-font-smoothing: antialiased;
    }}
    .top {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 1rem;
      padding: 22px clamp(20px, 5vw, 64px);
      border-bottom: 1px solid var(--rule);
    }}
    .brand {{ display: flex; align-items: center; gap: 12px; text-decoration: none; color: var(--ink); }}
    .brand img {{ width: 34px; height: 34px; }}
    .brand strong {{ font-size: 20px; font-weight: 600; letter-spacing: -0.03em; }}
    .top a.back {{ font-family: var(--mono); font-size: 11px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--muted); text-decoration: none; }}
    .top a.back:hover {{ color: var(--accent); }}
    article {{ max-width: 760px; margin: 0 auto; padding: clamp(40px, 7vw, 84px) 24px 80px; }}
    .kicker {{ font-family: var(--mono); font-size: 11px; letter-spacing: 0.18em; text-transform: uppercase; color: var(--accent); margin: 0 0 18px; }}
    h1 {{ font-size: clamp(38px, 5vw, 56px); line-height: 1.07; letter-spacing: -0.045em; font-weight: 600; margin: 0 0 22px; }}
    h2 {{ font-size: clamp(24px, 3vw, 30px); line-height: 1.2; letter-spacing: -0.03em; font-weight: 600; margin: 56px 0 18px; }}
    h3 {{ font-size: 21px; letter-spacing: -0.02em; margin: 38px 0 12px; }}
    p {{ margin: 0 0 22px; }}
    em {{ color: var(--muted); }}
    strong {{ font-weight: 600; }}
    a {{ color: var(--accent); text-decoration-thickness: 1px; text-underline-offset: 3px; }}
    blockquote {{
      margin: 34px 0;
      padding: 6px 0 6px 22px;
      border-left: 4px solid var(--accent);
      font-size: 22px;
      line-height: 1.45;
      letter-spacing: -0.02em;
    }}
    blockquote p {{ margin: 0; }}
    code {{ font-family: var(--mono); font-size: 0.82em; background: #ece7da; padding: 0.15em 0.35em; }}
    pre {{ background: var(--ink); color: #ece7da; padding: 20px 22px; overflow-x: auto; margin: 26px 0; }}
    pre code {{ background: none; padding: 0; font-size: 0.78em; line-height: 1.65; color: inherit; }}
    hr {{ border: 0; border-top: 1px solid var(--rule); margin: 48px 0; }}
    ul, ol {{ padding-left: 1.2em; margin: 0 0 24px; }}
    li {{ margin: 0 0 10px; }}
    footer {{ border-top: 1px solid var(--rule); padding: 26px clamp(20px, 5vw, 64px); display: flex; justify-content: space-between; gap: 16px; font-family: var(--mono); font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--muted); }}
    footer a {{ color: var(--muted); text-decoration: none; }}
    footer a:hover {{ color: var(--accent); }}
    @media (max-width: 640px) {{
      body {{ font-size: 17px; }}
      .top a.back span {{ display: none; }}
      footer {{ flex-direction: column; }}
    }}
    @media print {{
      .top, footer {{ display: none; }}
      article {{ max-width: none; padding: 0; }}
      a {{ color: inherit; }}
    }}
  </style>
</head>
<body>
  <header class="top">
    <a class="brand" href="../">
      <img src="../favicon.svg" alt="" />
      <strong>Countersign</strong>
    </a>
    <a class="back" href="../">← <span>Back to the project</span></a>
  </header>
  <article>
    <p class="kicker">Build note · Flip the Switch Hackathon</p>
    {body}
  </article>
  <footer>
    <span>Countersign · No agent ships alone</span>
    <a href="https://github.com/PhiBao/countersign">github.com/PhiBao/countersign</a>
  </footer>
</body>
</html>
"""


def main() -> int:
    text = SOURCE.read_text()
    html = markdown.markdown(
        text,
        extensions=["fenced_code", "tables", "smarty"],
    )
    # The first markdown H1 is the article title; keep it but drop the
    # standalone kicker duplication by leaving the shell kicker in place.
    TARGET.write_text(SHELL.format(body=html))
    print(f"wrote {TARGET}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
