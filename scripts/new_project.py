#!/usr/bin/env python3
"""Creates a new circuit-model plugin project from the template.

Usage: new_project.py <project-dir> --name "Plugin Name" [--vendor "Your Name"]
"""

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

TEMPLATE = Path(__file__).resolve().parent.parent / "template"
IGNORE = shutil.ignore_patterns("target", "validation", ".venv", "__pycache__", "*.wav")


def slug(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-") or "circuit"


def vst3_id(name: str) -> str:
    """16 ASCII bytes derived from the name, padded with digits."""
    letters = re.sub(r"[^A-Za-z0-9]", "", name)[:14] or "Circuit"
    return (letters + "0123456789abcdef")[:16]


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("dir", type=Path)
    p.add_argument("--name", required=True, help="plugin name shown in DAWs")
    p.add_argument("--vendor", default="Analog to VST")
    args = p.parse_args()

    if args.dir.exists() and any(args.dir.iterdir()):
        print(f"{args.dir} exists and is not empty", file=sys.stderr)
        return 1
    shutil.copytree(TEMPLATE, args.dir, ignore=IGNORE, dirs_exist_ok=True)

    model = args.dir / "dsp/src/model.rs"
    text = model.read_text()
    for pattern, value in [
        (r'pub const NAME: &str = ".*";', f'pub const NAME: &str = "{args.name}";'),
        (r'pub const VENDOR: &str = ".*";', f'pub const VENDOR: &str = "{args.vendor}";'),
        (r'pub const CLAP_ID: &str = ".*";', f'pub const CLAP_ID: &str = "local.analog-to-vst.{slug(args.name)}";'),
        (r'pub const VST3_ID: \[u8; 16\] = \*b".*";', f'pub const VST3_ID: [u8; 16] = *b"{vst3_id(args.name)}";'),
    ]:
        text = re.sub(pattern, value.replace("\\", "\\\\"), text)
    model.write_text(text)

    bundler = args.dir / "bundler.toml"
    bundler.write_text(re.sub(r'name = ".*"', f'name = "{args.name}"', bundler.read_text()))

    subprocess.run(["git", "init", "-q"], cwd=args.dir, check=False)
    print(f"Created {args.dir}. The example circuit is in dsp/src/model.rs and reference/circuit.cir.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
