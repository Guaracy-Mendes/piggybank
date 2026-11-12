#!/usr/bin/env python3
"""Validate, render and resume the implementation backlog without changing task status."""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
BACKLOG = ROOT / "docs/implementation/backlog.json"
BOARD = ROOT / "docs/implementation/WORK_PACKAGES.md"
ID_PATTERN = r"(?:FR-[A-Z]+|BR|SEC|NFR|AT|SRC|DEC|KPI|ASM|RISK)-\d{2}"
STAGES = {
    "S0": "Reproducible foundation",
    "S1": "Core and customer lifecycle",
    "S2": "Money, records and R1 channels",
    "S3": "R1 engineering qualification",
    "S4": "Provider integration and controlled R1",
    "S5": "R2 capability expansion",
    "S6": "R3 optimization",
}


def load() -> dict:
    return json.loads(BACKLOG.read_text())


def next_package(data: dict) -> dict | None:
    packages = data["work_packages"]
    active = sorted((p for p in packages if p["status"] == "in_progress"), key=lambda p: p["id"])
    if active:
        return active[0]
    completed = {p["id"] for p in packages if p["status"] == "done"}
    ready = [p for p in packages if p["status"] == "planned" and set(p["depends_on"]) <= completed]
    return min(ready, key=lambda p: (p["stage"], p["id"])) if ready else None


def validate(data: dict) -> None:
    errors: list[str] = []
    prd = (ROOT / "docs/PRD.md").read_text()
    architecture = (ROOT / "docs/ARCHITECTURE.md").read_text()
    known = set(re.findall(rf"^\| ({ID_PATTERN}) \|", prd + "\n" + architecture, re.M))
    required = {item for item in known if item.startswith(("FR-", "BR-", "SEC-", "NFR-", "KPI-"))}
    packages = data["work_packages"]
    owners = data["requirement_owners"]
    ids = [p["id"] for p in packages]
    by_id = {p["id"]: p for p in packages}
    if data.get("schema_version") != 1:
        errors.append("Unsupported backlog schema version.")
    if len(set(ids)) != len(ids):
        errors.append("Duplicate work-package ID.")
    if set(owners) != required:
        errors.append(f"Requirement ownership mismatch: {sorted(set(owners) ^ required)}")
    for requirement, owner in owners.items():
        if owner not in by_id:
            errors.append(f"{requirement}: unknown owner {owner}")
    assigned: Counter[str] = Counter()
    scenarios: set[str] = set()
    release_order = {f"R{i}": i for i in range(4)}
    fr_releases: dict[str, str] = {}
    for line in prd.splitlines():
        match = re.match(r"^\| (FR-[A-Z]+-\d{2}) \|.*?\| P[012] / (R[0-3]) \|", line)
        if match:
            fr_releases[match[1]] = match[2]
    for package in packages:
        ident = package["id"]
        if not re.fullmatch(r"WP-\d{2}", ident):
            errors.append(f"Invalid work-package ID: {ident}")
        if package["status"] not in {"planned", "in_progress", "blocked", "done"}:
            errors.append(f"{ident}: invalid status")
        if package["release"] not in release_order or package["stage"] not in STAGES:
            errors.append(f"{ident}: invalid release/stage")
        if package["size"] not in {"S", "M", "L", "XL"}:
            errors.append(f"{ident}: invalid relative size")
        for key in ("implementation_steps", "planned_artifacts", "definition_of_done", "verification", "acceptance_scenarios"):
            if not package[key]:
                errors.append(f"{ident}: missing {key}")
        references = package["requirements"] + package["acceptance_scenarios"]
        references += package["activation_decisions"] + package["required_external_inputs"]
        for reference in references:
            if reference not in known:
                errors.append(f"{ident}: undefined baseline reference {reference}")
        for scenario in package["acceptance_scenarios"]:
            if not scenario.startswith("AT-"):
                errors.append(f"{ident}: invalid acceptance reference {scenario}")
        scenarios.update(package["acceptance_scenarios"])
        for requirement in package["requirements"]:
            assigned[requirement] += 1
            if owners.get(requirement) != ident:
                errors.append(f"{ident}: inconsistent requirement owner for {requirement}")
            if requirement in fr_releases and release_order[package["release"]] > release_order[fr_releases[requirement]]:
                errors.append(f"{ident}: {requirement} allocated later than its PRD release")
        for field in ("depends_on", "activation_dependencies"):
            for dependency in package[field]:
                if dependency not in by_id or dependency == ident:
                    errors.append(f"{ident}: invalid {field} dependency {dependency}")
        if package["status"] in {"in_progress", "done"}:
            unmet = [d for d in package["depends_on"] if by_id.get(d, {}).get("status") != "done"]
            if unmet:
                errors.append(f"{ident}: started/completed before prerequisites {unmet}")
        if package["status"] == "blocked" and not package["blocker"].strip():
            errors.append(f"{ident}: blocked status requires a specific blocker")
        if package["status"] == "done":
            if not package["completion_notes"].strip() or not package["evidence"]:
                errors.append(f"{ident}: done requires completion notes and actual evidence")
            if package["blocker"].strip():
                errors.append(f"{ident}: done cannot retain an unresolved blocker")
        for evidence in package["evidence"]:
            for key in ("timestamp", "environment", "command", "result", "artifact"):
                if not evidence.get(key):
                    errors.append(f"{ident}: incomplete evidence field {key}")
            try:
                timestamp = datetime.fromisoformat(evidence.get("timestamp", "").replace("Z", "+00:00"))
                if timestamp.tzinfo is None:
                    errors.append(f"{ident}: evidence timestamp needs a timezone")
            except ValueError:
                errors.append(f"{ident}: invalid evidence timestamp")
    if set(assigned) != required or any(count != 1 for count in assigned.values()):
        errors.append("Each baseline requirement/rule/objective must have exactly one primary package.")
    expected_scenarios = {item for item in known if item.startswith("AT-")}
    if scenarios != expected_scenarios:
        errors.append(f"Acceptance coverage mismatch: {sorted(scenarios ^ expected_scenarios)}")

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(ident: str) -> None:
        if ident in visiting:
            errors.append(f"Dependency cycle at {ident}")
            return
        if ident in visited or ident not in by_id:
            return
        visiting.add(ident)
        for dependency in by_id[ident]["depends_on"] + by_id[ident]["activation_dependencies"]:
            visit(dependency)
        visiting.remove(ident)
        visited.add(ident)

    for ident in ids:
        visit(ident)
    candidate = next_package(data)
    expected_next = candidate["id"] if candidate else None
    if data.get("next_work_package") != expected_next:
        errors.append(f"next_work_package must be {expected_next!r} for the current state")
    if errors:
        raise SystemExit("\n".join(errors))


def render(data: dict) -> str:
    packages = data["work_packages"]
    counts = Counter(p["status"] for p in packages)
    lines = [
        "# Digital Bank — Implementation Work Packages", "",
        f"**Baseline:** {data['baseline_date']}. **Source of truth:** [backlog.json](backlog.json). Generated by `make plan`; edit the JSON and regenerate this board.", "",
        "Read the [implementation plan](../IMPLEMENTATION_PLAN.md), [technical baseline](../decisions/0001-local-implementation-baseline.md), [PRD](../PRD.md) and [architecture](../ARCHITECTURE.md) before coding. Dependencies determine execution order; numeric identifiers alone do not.", "",
        f"**Status:** {len(packages)} packages; {counts['planned']} planned, {counts['in_progress']} in progress, {counts['blocked']} blocked, {counts['done']} done. **Next:** {data['next_work_package'] or 'no ready package'}.", "",
        "Requirements below are primary accountability assignments, with supporting packages linked through dependencies and acceptance coverage. There are 77 functional requirements plus 47 business/security/quality/outcome items. Planned coverage is not evidence of implementation.", "",
        "Sizes S/M/L/XL express relative complexity, not elapsed time. Activation decisions apply to real enablement; fixture-based local work may proceed unless a package explicitly requires external inputs. R2/R3 production enablement additionally requires the R1 release gate.", "",
        "## Package index", "",
        "| Package | Stage / release | Size | Status | Implementation dependencies |",
        "| --- | --- | --- | --- | --- |",
    ]
    for p in packages:
        lines.append(f"| {p['id']} — {p['title']} | {p['stage']} / {p['release']} | {p['size']} | {p['status']} | {', '.join(p['depends_on']) or 'None'} |")
    lines += ["", "## Primary requirement allocation", "", "| Baseline item | Primary work package |", "| --- | --- |"]
    for requirement, owner in data["requirement_owners"].items():
        lines.append(f"| {requirement} | {owner} |")
    for p in packages:
        lines += ["", f"## {p['id']} — {p['title']}", "",
                  f"**Stage:** {p['stage']} — {STAGES[p['stage']]}. **Release:** {p['release']}. **Size:** {p['size']}. **Status:** {p['status']}.", "",
                  f"**Implementation dependencies:** {', '.join(p['depends_on']) or 'None'}.", "",
                  f"**Primary baseline items:** {', '.join(p['requirements']) or 'Enabling infrastructure; supports dependent requirements'}.", "",
                  f"**Acceptance scenarios:** {', '.join(p['acceptance_scenarios'])}.", "",
                  f"**Execution mode:** `{p['execution_mode']}`. **Activation decisions:** {', '.join(p['activation_decisions']) or 'No external activation decision specific to this package'}."]
        if p["activation_dependencies"]:
            lines += ["", f"**Production activation dependencies:** {', '.join(p['activation_dependencies'])}."]
        if p["required_external_inputs"]:
            lines += ["", f"**External inputs required for package completion:** {', '.join(p['required_external_inputs'])}."]
        lines += ["", "Implementation steps:", ""]
        lines.extend(f"{i}. {step}" for i, step in enumerate(p["implementation_steps"], 1))
        lines += ["", "Planned artifacts (created during implementation):", ""]
        lines.extend(f"- `{artifact}`" for artifact in p["planned_artifacts"])
        lines += ["", "Completion conditions:", ""]
        lines.extend(f"- {condition}" for condition in p["definition_of_done"])
        lines += ["", "Verification required:", ""]
        lines.extend(f"- {check}" for check in p["verification"])
        if p["blocker"]:
            lines += ["", f"**Blocker:** {p['blocker']}"]
        if p["completion_notes"]:
            lines += ["", f"**Completion notes:** {p['completion_notes']}"]
        lines += ["", f"**Evidence entries recorded:** {len(p['evidence'])}."]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "render", "next"))
    args = parser.parse_args()
    data = load()
    validate(data)
    content = render(data)
    if args.action == "render":
        BOARD.write_text(content)
        print(f"Wrote {BOARD.relative_to(ROOT)} from {len(data['work_packages'])} packages.")
    elif args.action == "check":
        if not BOARD.exists() or BOARD.read_text() != content:
            raise SystemExit("Implementation board is stale. Run make plan.")
        print(f"Implementation plan passed: {len(data['work_packages'])} packages, "
              f"{len(data['requirement_owners'])} assigned baseline items, 30 acceptance scenarios, acyclic dependencies.")
    else:
        package = next_package(data)
        if package is None:
            print("No ready package. Inspect completed work and specific recorded blockers.")
            return
        print(f"{package['id']} — {package['title']}")
        print(f"Status: {package['status']}; stage: {package['stage']}; release: {package['release']}")
        for number, step in enumerate(package["implementation_steps"], 1):
            print(f"{number}. {step}")
        if package["required_external_inputs"]:
            print("Required external inputs: " + ", ".join(package["required_external_inputs"]))
        print("Completion conditions:")
        for condition in package["definition_of_done"]:
            print(f"- {condition}")


if __name__ == "__main__":
    main()
