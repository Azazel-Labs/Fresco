"""Generate a line-coverage badge from cargo-llvm-cov's JSON summary."""

import argparse
import json
from pathlib import Path


def render_badge(report):
    data = report["data"]
    if len(data) != 1:
        raise ValueError("expected one workspace coverage summary")
    lines = data[0]["totals"]["lines"]
    count, covered = lines["count"], lines["covered"]
    if type(count) is not int or type(covered) is not int:
        raise ValueError("coverage line counts must be integers")
    if count <= 0 or not 0 <= covered <= count:
        raise ValueError("coverage requires a nonempty, valid line count")
    percent = 100 * covered / count
    value = f"{percent:.1f}%"
    color = "#4c1" if percent >= 80 else "#dfb317" if percent >= 60 else "#e05d44"
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="132" height="20"
  role="img" aria-label="Rust lines: {value}">
  <title>Rust line coverage: {value} ({covered} of {count} lines)</title>
  <clipPath id="badge"><rect width="132" height="20" rx="3"/></clipPath>
  <g clip-path="url(#badge)">
    <path fill="#555" d="M0 0h76v20H0z"/>
    <path fill="{color}" d="M76 0h56v20H76z"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,DejaVu Sans,sans-serif" font-size="11">
    <text x="38" y="14">Rust lines</text>
    <text x="104" y="14">{value}</text>
  </g>
</svg>
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("summary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    badge = render_badge(json.loads(args.summary.read_text(encoding="utf-8")))
    args.output.write_text(badge, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
