#!/usr/bin/env python3
"""Check/render authored PlantUML using a checksum-pinned official distribution."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.2026.9"
SHA256 = "aa4a1d06f8911c4b842beff4828dd955be431692b631a312d04c0e9fab9581de"
URL = f"https://github.com/plantuml/plantuml/releases/download/v{VERSION}/plantuml.jar"
CACHE = ROOT / ".cache"
JAR = CACHE / f"plantuml-{VERSION}.jar"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ensure_jar() -> None:
    CACHE.mkdir(exist_ok=True)
    if JAR.exists():
        if digest(JAR) != SHA256:
            raise SystemExit(f"Checksum mismatch: {JAR}; remove it and retry.")
        return
    print(f"Downloading official PlantUML {VERSION} (first use only).", flush=True)
    with tempfile.NamedTemporaryFile(dir=CACHE, suffix=".download", delete=False) as tmp:
        temporary = Path(tmp.name)
    try:
        with urllib.request.urlopen(URL, timeout=60) as response, temporary.open("wb") as target:
            shutil.copyfileobj(response, target)
        if digest(temporary) != SHA256:
            raise SystemExit("Downloaded PlantUML checksum mismatch; refusing to execute it.")
        temporary.replace(JAR)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Check syntax without rendering")
    args = parser.parse_args()
    if not shutil.which("java"):
        raise SystemExit("Java 17+ is required.")
    if not args.check and not shutil.which("dot"):
        raise SystemExit("Graphviz dot is required to render this diagram set.")
    ensure_jar()
    sources = sorted((ROOT / "docs/diagrams/c4").glob("*.puml"))
    sources += sorted((ROOT / "docs/diagrams/uml").glob("*.puml"))
    implementation = sorted((ROOT / "docs/implementation").glob("*.puml"))
    implementation += sorted((ROOT / "docs/deployment").rglob("*.puml"))
    if not sources or len({p.stem for p in sources + implementation}) != len(sources + implementation):
        raise SystemExit("Missing diagram sources or colliding output basenames.")
    base = ["java", "-Djava.awt.headless=true", "-Xmx1024m", "-jar", str(JAR)]
    if args.check:
        subprocess.run(base + ["--check-syntax", "--stop-on-error"] + [str(p) for p in sources + implementation],
                       cwd=ROOT, check=True)
        print(f"PlantUML syntax passed: {len(sources + implementation)} diagrams (version {VERSION}).")
        return
    output = ROOT / "docs/diagrams/rendered"
    output.mkdir(parents=True, exist_ok=True)
    # Render into a staging directory so failed runs cannot replace valid previews.
    with tempfile.TemporaryDirectory(prefix="diagrams-", dir=CACHE) as staging_dir:
        staging = Path(staging_dir)
        subprocess.run(base + ["--svg", "--check-before-run", "--disable-metadata", "--threads", "2",
                               "--output-dir", str(staging)] + [str(p) for p in sources + implementation], cwd=ROOT, check=True)
        for source in sources + implementation:
            rendered = staging / f"{source.stem}.svg"
            if not rendered.is_file():
                raise SystemExit(f"Missing rendered artifact: {rendered.name}")
            tree = ET.parse(rendered)
            if not tree.getroot().tag.endswith("svg"):
                raise SystemExit(f"Invalid SVG root: {rendered.name}")
            svg_text = rendered.read_text()
            if "Syntax Error?" in svg_text or "An error has occurred" in svg_text:
                raise SystemExit(f"PlantUML error image: {rendered.name}")
        for source in sources + implementation:
            name = f"{source.stem}.svg"
            (staging / name).replace((output if source in sources else source.parent) / name)
    print(f"Rendered and XML-validated {len(sources + implementation)} SVG diagrams in {output.relative_to(ROOT)}.")


if __name__ == "__main__":
    main()
