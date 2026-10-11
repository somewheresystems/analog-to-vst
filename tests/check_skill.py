"""Checks SKILL.md against the Hermes skill authoring rules that can be verified mechanically."""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
text = (ROOT / "SKILL.md").read_text()
errors = []

if not text.startswith("---\n"):
    errors.append("frontmatter must start on the first line")
front, _, body = text[4:].partition("\n---\n")
fields = dict(re.findall(r"^(\w+):\s*(.*)$", front, re.M))
for key in ["name", "description", "version", "author", "license", "platforms"]:
    if key not in fields:
        errors.append(f"missing frontmatter field {key}")
name, description = fields.get("name", ""), fields.get("description", "").strip('"')
if not re.fullmatch(r"[a-z0-9-]{1,64}", name):
    errors.append(f"bad name {name!r}")
if len(description) > 60 or not description.endswith("."):
    errors.append(f"description must be at most 60 chars and end with a period ({len(description)})")
if not body.strip():
    errors.append("empty body")
for section in ["## When to Use", "## Procedure", "## Pitfalls", "## Verification"]:
    if section not in body:
        errors.append(f"missing section {section}")
for ref in re.findall(r"`(references/[\w.-]+|scripts/[\w.-]+)`", body):
    if not (ROOT / ref).exists():
        errors.append(f"SKILL.md points at missing {ref}")
if re.search(r"/Users/|/home/", text):
    errors.append("machine-local path in SKILL.md")

print("\n".join(errors) or "SKILL.md ok")
sys.exit(1 if errors else 0)
