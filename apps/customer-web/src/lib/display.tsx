import { Badge } from "@piggybank/web-core";
const labels: Record<string, string> = {
  DRAFT: "In progress",
  SUBMITTING: "Submitted",
  CONTROLS_PENDING: "Checks in progress",
  MANUAL_REVIEW: "With our team",
  INFORMATION_REQUIRED: "Information needed",
  DECLINED: "Application declined",
  ACTIVE: "Account ready",
  EXPIRED: "Expired",
  OPEN: "Open",
  PROPOSED: "Under review",
  CLOSED: "Closed",
};

export function StateBadge({ state }: { state: string }) {
  return (
    <Badge
      tone={
        state === "ACTIVE" || state === "CLOSED"
          ? "good"
          : state === "DECLINED"
            ? "danger"
            : state === "DRAFT"
              ? "neutral"
              : "attention"
      }
    >
      {labels[state] ?? state.toLowerCase().replaceAll("_", " ")}
    </Badge>
  );
}

export function PageTitle({
  eyebrow,
  title,
  description,
}: {
  eyebrow: string;
  title: string;
  description: string;
}) {
  return (
    <div className="page-heading">
      <span className="eyebrow">{eyebrow}</span>
      <h1>{title}</h1>
      <p className="muted">{description}</p>
    </div>
  );
}
