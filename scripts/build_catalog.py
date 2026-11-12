#!/usr/bin/env python3
"""Keep the Markdown PlantUML gallery synchronized with canonical .puml sources."""
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIAGRAMS = ROOT / "docs/diagrams"
CATALOG = DIAGRAMS / "README.md"

VIEWS = [
    ("c4/01-context", "C4 L1 — System context", "System scope, people and external providers; J-01–J-08."),
    ("c4/02-containers", "C4 L2 — Containers and data ownership", "Proposed deployables, owned stores, principal calls and events; all capability groups."),
    ("c4/03-onboarding-components", "C4 L3 — Journey process components", "Acquisition, onboarding, opening, servicing and durable orchestration; FR-ACQ, FR-ONB, FR-ACC."),
    ("c4/04-core-components", "C4 L3 — Core components", "Accounts/parties, products, holds, journal and atomic financial commit; FR-CORE."),
    ("c4/05-payments-components", "C4 L3 — Payment components", "Approvals, execution, adapters, evidence and recovery; FR-PAY, FR-EXT."),
    ("c4/06-controls-components", "C4 L3 — Control components", "Separate entitlements, identity, screening, risk, fraud, cases and audit; FR-CTRL."),
    ("c4/07-payment-code", "C4 L4 — Payment code design", "Proposed UML classes inside the L3 Payment lifecycle component; no existing implementation is implied."),
    ("c4/08-payment-dynamic", "C4 dynamic — Payment flow", "Numbered successful outgoing payment interactions; failures are detailed in UML sequences."),
    ("c4/09-deployment", "C4 deployment — Production and recovery", "Provider-neutral instance placement, trust zones, protection and recovery; NFR-06/07."),
    ("uml/01-use-cases", "UML — Use cases", "Actors, primary jobs, required controls and optional manual review; full target."),
    ("uml/02-domain-model", "UML — Domain model", "Business entities, multiplicities, financial invariants and owning domains; PRD section 9."),
    ("uml/03-onboarding-sequence", "UML — Onboarding sequence", "Pending checks, duplicate evidence, manual review and stable party creation; J-02."),
    ("uml/04-account-opening-activity", "UML — Account opening activity", "Terms/eligibility gates, idempotency, restricted partial creation and activation; J-03."),
    ("uml/05-payment-sequence", "UML — Payment sequence", "Durable acceptance, approvals, reservation, timeout, settlement and booking recovery; J-05."),
    ("uml/06-reconciliation-sequence", "UML — Reconciliation sequence", "Unknown outcomes, evidence matching, correction, GL uniqueness and break ownership; J-06/07."),
    ("uml/07-payment-state", "UML — Payment state machine", "Canonical outgoing states, safe cancellation and linked returns; BR-04–10."),
    ("uml/08-application-state", "UML — Application state machine", "Draft, checks, information, review, approval, decline and expiry; J-01/02."),
    ("uml/09-account-state", "UML — Account state machine", "Activation, restrictions, closure and retained records; FR-ACC-01/06."),
    ("uml/10-exception-activity", "UML — Exception activity", "Case ownership, incomplete evidence, independent approval and immutable corrections; J-06."),
    ("uml/11-event-components", "UML — Event integration components", "Outbox/inbox, projection, dead-letter replay, analytics and purpose boundaries; FR-DATA, FR-OPS-03."),
    ("uml/12-authorization-sequence", "UML — Authorization sequence", "Identity, partner consent, resource entitlement, action-bound proof and authoritative core checks; FR-IAM."),
    ("uml/13-document-sequence", "UML — Document sequence", "Terms acceptance, consistent statements, retrieval authorization and superseding versions; FR-DOC."),
]


def build() -> str:
    lines = [
        "# Digital Bank — C4 and UML Diagram Catalog", "",
        "**Status:** Proposed design, 2026-10-22. All 22 views derive from the supplied capability image plus explicitly proposed requirements. The C4 L4 view is a design-level class proposal, not generated source code.", "",
        "Read with the [PRD](../PRD.md), [architecture](../ARCHITECTURE.md) and [traceability](../TRACEABILITY.md). Editable `.puml` files are canonical; this gallery embeds their complete PlantUML notation and rendered SVG previews. Expand a source block or open the linked file to inspect details.", "",
        "C4 containers represent deployable applications/data stores. L3 components remain inside one container. The source image's L2 badges are independent of C4 zoom levels. Arrows are directed and labeled; actors/providers are outside the owned system boundary unless explicitly shown otherwise.", "",
        "## Rendering", "",
        "Run `make docs` from the repository root to render, synchronize and validate everything. `make diagrams` renders SVG and refreshes this catalog; `make check` checks the catalog, local Markdown references, requirement/scenario coverage and PlantUML syntax. Prerequisites: Python 3.10+, Java 17+, Graphviz `dot`, and `make`.", "",
        "The renderer uses official PlantUML **1.2026.9**, SHA-256 `aa4a1d06f8911c4b842beff4828dd955be431692b631a312d04c0e9fab9581de`, cached under `.cache/`. First use requires network access to its [official release](https://github.com/plantuml/plantuml/releases/tag/v1.2026.9); cached execution needs no network. C4 notation comes from the library bundled in that pinned JAR, with `!include <C4/...>` rather than remote includes.", "",
        "The shared [theme](theme.puml) is resolved relative to each source. Render canonical files through the provided command so includes remain correct. The fenced blocks below are exact source listings for reading; when copying to another directory, preserve the relative theme path or adjust it. Preview SVGs can be opened independently and do not need a PlantUML extension.", "",
        "Notation references: [official C4 views](https://c4model.com/diagrams.html), [C4-PlantUML](https://github.com/plantuml-stdlib/C4-PlantUML), [PlantUML command line](https://plantuml.com/command-line).", "",
        "## Index", "",
        "| View | Editable source | Rendered SVG |",
        "| --- | --- | --- |",
    ]
    for stem, title, _ in VIEWS:
        basename = Path(stem).name
        lines.append(f"| {title} | [PlantUML]({stem}.puml) | [SVG](rendered/{basename}.svg) |")
    for stem, title, description in VIEWS:
        source = (DIAGRAMS / f"{stem}.puml").read_text().rstrip()
        basename = Path(stem).name
        lines += ["", f"## {title}", "", description, "",
                  f"[Editable PlantUML]({stem}.puml) · [Open SVG](rendered/{basename}.svg)", "",
                  f"![{title}](rendered/{basename}.svg)", "",
                  "<details>", "<summary>Complete PlantUML source</summary>", "",
                  "```plantuml", source, "```", "", "</details>"]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected_sources = {str(Path(stem + ".puml")) for stem, _, _ in VIEWS}
    actual_sources = {str(p.relative_to(DIAGRAMS)) for p in DIAGRAMS.glob("*/*.puml")}
    if expected_sources != actual_sources:
        raise SystemExit(f"Catalog/source mismatch: {expected_sources ^ actual_sources}")
    content = build()
    if args.check:
        if not CATALOG.exists() or CATALOG.read_text() != content:
            raise SystemExit("Diagram catalog is stale. Run make catalog.")
        print(f"Catalog synchronized: {len(VIEWS)} complete PlantUML source blocks.")
    else:
        CATALOG.write_text(content)
        print(f"Wrote {CATALOG.relative_to(ROOT)} with {len(VIEWS)} views.")


if __name__ == "__main__":
    main()
