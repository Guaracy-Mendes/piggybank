# Audiences and journey-partner acceptance (WP-38)

`journeys.py` plays the journey partner against the real local issuer, API and worker on a fresh fixture database:
`make test-journeys`. The partner is the single local simulator role `SIM_JOURNEY` (user direction 2026-11-18: build
one local template). Real journey partners need selection (DEC-05, DEC-06, DEC-09).

| Result key | What is proven |
| --- | --- |
| `consented_audience_build` | A `savings_starter` audience includes only parties whose current marketing preference is on. |
| `service_outreach_not_exported` | A `dormant_check_in` (service outreach) audience cannot be exported to a partner (403, recorded). |
| `export_pseudonymous_signed` | A fetch with a wrong signature returns 401. A signed fetch returns `m_…` tokens only, with no account or party identifiers, and a signed response. |
| `partner_advice_only_boundary` | A recommendation becomes an advisory insight. A duplicate replays identical signed bytes; reusing an id with other content returns 409; a bad signature returns 401. The following are rejected and recorded: approval or entitlement fields, unlisted recommendation codes, foreign action routes and markup. Core entries, holds, role grants and applications are unchanged. |
| `withdrawal_hides_and_withholds` | The customer sees the partner advice with a "why". After withdrawing marketing, suggestions are hidden, delivery withholds the token, and new advice is `SUPPRESSED`. The insight records remain as evidence. |
| `outcome_report_denominators_watermarks` | The outcome report states the build denominators, the members who withdrew since, the accepted partner suggestions, the dataset watermarks and a completeness status. |

The database test `crates/data/tests/cases/audiences.rs` adds role denial, dismissal ownership, the `LATE_DATA` flag and
the full report figures.
