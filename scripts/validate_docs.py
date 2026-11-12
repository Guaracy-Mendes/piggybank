#!/usr/bin/env python3
"""Validate local document links, defined IDs, source coverage and acceptance mapping."""
from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import urllib.parse
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
ID_PATTERN = r"(?:FR-[A-Z]+|BR|SEC|NFR|AT|SRC|DEC|KPI|ASM|RISK)-\d{2}"


def without_code(text: str) -> str:
    return re.sub(r"```[^\n]*\n.*?```", "", text, flags=re.S)


def anchors(text: str) -> set[str]:
    result: set[str] = set()
    counts: Counter[str] = Counter()
    for heading in re.findall(r"^#{1,6}\s+(.+)$", without_code(text), flags=re.M):
        clean = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
        count = counts[clean]
        counts[clean] += 1
        result.add(f"{clean}-{count}" if count else clean)
    return result


def main() -> None:
    errors: list[str] = []
    markdown = [ROOT / "README.md"] + sorted((ROOT / "docs").rglob("*.md"))
    texts = {path: path.read_text() for path in markdown}
    prd = texts[ROOT / "docs/PRD.md"]
    architecture = texts[ROOT / "docs/ARCHITECTURE.md"]
    definitions = re.findall(rf"^\| ({ID_PATTERN}) \|", prd + "\n" + architecture, re.M)
    for ident, count in Counter(definitions).items():
        if count != 1:
            errors.append(f"Duplicate definition: {ident}")
    known = set(definitions)
    source_ids = {ident for ident in known if ident.startswith("SRC-")}
    trace = texts[ROOT / "docs/TRACEABILITY.md"]
    mapped_sources = set(re.findall(r"^\| (SRC-\d{2}) \|", trace, re.M))
    if source_ids != mapped_sources:
        errors.append(f"Source coverage mismatch: {sorted(source_ids ^ mapped_sources)}")
    functional = {ident for ident in known if ident.startswith("FR-")}
    acceptance = prd.split("## 16. Acceptance and test strategy", 1)[1].split("## 17.", 1)[0]
    accepted = set(re.findall(r"FR-[A-Z]+-\d{2}", acceptance))
    if functional != accepted:
        errors.append(f"Acceptance reference mismatch: {sorted(functional ^ accepted)}")

    link_count = 0
    for path, content in texts.items():
        for target in re.findall(r"!?\[[^\]]*\]\(([^)]+)\)", without_code(content)):
            target = target.strip("<>")
            parsed = urllib.parse.urlsplit(target)
            if parsed.scheme or parsed.netloc:
                continue
            link_count += 1
            destination = (path.parent / urllib.parse.unquote(parsed.path)).resolve() if parsed.path else path
            if not destination.exists():
                errors.append(f"{path.relative_to(ROOT)}: missing {target}")
            elif parsed.fragment and destination.suffix == ".md":
                anchor = urllib.parse.unquote(parsed.fragment)
                if anchor not in anchors(destination.read_text()):
                    errors.append(f"{path.relative_to(ROOT)}: missing anchor {target}")

    sources = sorted((ROOT / "docs/diagrams").glob("*/*.puml"))
    for path in markdown + sources:
        content = path.read_text()
        for ident in sorted(set(re.findall(ID_PATTERN, content)) - known):
            errors.append(f"{path.relative_to(ROOT)}: undefined ID {ident}")
    for source in sources:
        content = source.read_text()
        if content.count("@startuml") != 1 or content.count("@enduml") != 1:
            errors.append(f"{source.name}: expected one complete diagram")
        if re.search(r"!include(?:url)?\s+https?://", content):
            errors.append(f"{source.name}: remote include breaks offline rendering")
        rendered = ROOT / "docs/diagrams/rendered" / f"{source.stem}.svg"
        if not rendered.exists():
            errors.append(f"{source.name}: missing SVG preview")
        else:
            try:
                if not ET.parse(rendered).getroot().tag.endswith("svg"):
                    errors.append(f"{rendered.name}: not SVG")
            except ET.ParseError as exc:
                errors.append(f"{rendered.name}: invalid XML: {exc}")
    if errors:
        raise SystemExit("\n".join(errors))
    scenarios = len({ident for ident in known if ident.startswith("AT-")})
    print(f"Documentation passed: {len(markdown)} Markdown files, {link_count} local links, "
          f"{len(source_ids)} source capabilities, {len(functional)} functional requirements, "
          f"{scenarios} acceptance scenarios, {len(sources)} SVG previews.")


if __name__ == "__main__":
    main()
