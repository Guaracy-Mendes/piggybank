import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type Page,
} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
const customer = "http://127.0.0.1:4300",
  staff = "http://127.0.0.1:4301";
async function login(browser: Browser, user: string, workforce = false) {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(workforce ? staff : customer);
  await page
    .getByRole(workforce ? "link" : "button", {
      name: workforce ? /Sign in to operations/ : /Sign in/,
    })
    .click();
  await page.locator("#username").fill(user);
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL((workforce ? staff : customer) + "/**");
  return { context, page };
}
async function send(
  context: BrowserContext,
  path: string,
  data?: unknown,
  method = "POST",
  key?: string,
  workforce = false,
) {
  const trust = workforce ? "workforce" : "customer";
  const csrf = (await context.cookies()).find(
    (c) => c.name === "pb_csrf_" + trust,
  )!.value;
  return context.request.fetch((workforce ? staff : customer) + path, {
    method,
    data,
    headers: {
      Origin: workforce ? staff : customer,
      "X-CSRF-Token": csrf,
      ...(key ? { "Idempotency-Key": key } : {}),
    },
  });
}
async function open(context: BrowserContext, label: string) {
  const products = await (
    await context.request.get(customer + "/api/v1/products")
  ).json();
  const p = products[0];
  const created = await send(
    context,
    "/api/v1/applications",
    { productVersionId: p.id, marketing: false },
    "POST",
    crypto.randomUUID(),
  );
  expect(created.status()).toBe(201);
  let app = await created.json();
  const path = "/api/v1/applications/" + app.id;
  const profile = await send(
    context,
    path + "/profile",
    {
      version: app.version,
      profile: {
        fullName: "Synthetic " + label,
        dateOfBirth: "1990-01-01",
        email: label.toLowerCase() + "@example.invalid",
        residency: "SYNTHETIC",
        consentVersion: "synthetic-consent-v1",
        marketing: false,
      },
    },
    "PUT",
  );
  expect(profile.status()).toBe(200);
  expect(
    (
      await send(context, path + "/evidence", {
        content: "Synthetic payment browser identity evidence.",
      })
    ).status(),
  ).toBe(200);
  const terms = await (
    await context.request.get(customer + `/api/v1/products/${p.id}/terms`)
  ).json();
  expect(
    (await send(context, path + "/terms", { termsId: terms.id })).status(),
  ).toBe(200);
  expect(
    (
      await send(
        context,
        path + "/submit",
        undefined,
        "POST",
        crypto.randomUUID(),
      )
    ).status(),
  ).toBe(202);
  await expect
    .poll(
      async () => {
        app = await (await context.request.get(customer + path)).json();
        return app.state;
      },
      { timeout: 30000 },
    )
    .toBe("ACTIVE");
  return app.accountId as string;
}
async function accessible(page: Page) {
  const result = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(
    result.violations.map((v) => ({
      id: v.id,
      nodes: v.nodes.map((n) => n.target),
    })),
  ).toEqual([]);
}

test("Finance preserves the original GL batch through quarantined acknowledgment and audited recovery", async ({
  browser,
}) => {
  const alice = await login(browser, "alice");
  const reviewer = await login(browser, "reviewer", true);
  const security = await login(browser, "security", true);
  const approver = await login(browser, "approver", true);
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  reviewer.page.on("console", (e) => {
    if (e.type() === "error") errors.push(e.text());
  });
  const account = crypto.randomUUID();
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  function sql(statement: string) {
    return execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        manifest.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  }
  const operation = crypto.randomUUID();
  sql(
    `INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('${account}','CUSTOMER','ACTIVE',false)`,
  );
  // Disposable fixture only: seed a balanced Core command and provider fault atomically,
  // before discovery can send the source to GL. This SQL is never a product action.
  sql(
    `DO $$ DECLARE r record; BEGIN SELECT * INTO r FROM core.fund_synthetic('${operation}','${account}','4f1a780b-3e31-48ef-a294-000000000003',99007); INSERT INTO simulator.gl_scenarios VALUES(r.batch_id,'CONFLICT'); END $$;`,
  );
  const source = sql(
    `SELECT id FROM core.journal_batches WHERE operation_id='${operation}'`,
  );
  expect(source).toMatch(/^[a-f0-9-]{36}$/);
  let id = "";
  await expect
    .poll(
      async () => {
        const exports = await (
          await reviewer.context.request.get(
            staff + "/api/v1/staff/finance/exports",
          )
        ).json();
        const item = exports.find(
          (e: { sourceBatchId: string }) => e.sourceBatchId === source,
        );
        if (item) id = item.id;
        return item?.state;
      },
      { timeout: 30000 },
    )
    .toBe("REVIEW_REQUIRED");
  expect(
    (
      await alice.context.request.get(
        customer + "/api/v1/staff/finance/exports",
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await security.context.request.get(
        staff + "/api/v1/staff/finance/exports",
      )
    ).status(),
  ).toBe(403);
  await reviewer.page.goto(staff + "/#/finance/" + id);
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Account for every batch.",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    reviewer.page.getByText("No authenticated booking receipt", {
      exact: true,
    }),
  ).toBeVisible();
  const initial = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/exports/" + id,
    )
  ).json();
  expect(initial.export.batch.debitTotal).toBe("99007");
  expect(initial.export.batch.creditTotal).toBe("99007");
  expect(initial.export.receipt).toBeNull();
  const balance = sql(
    `SELECT booked::text FROM core.ledger_accounts WHERE id='${account}'`,
  );
  const updated = await reviewer.context.request.post(
    "http://127.0.0.1:4182/admin/gl-scenarios",
    {
      headers: {
        "X-Finance-Key": "synthetic-finance-authentication-key-only-2026",
      },
      data: { sourceBatchId: source, mode: "ACCEPT" },
    },
  );
  expect(updated.status()).toBe(204);
  await reviewer.page
    .getByLabel("Finance recovery reason", { exact: true })
    .fill("Original provider receipt is now available for inquiry.");
  await reviewer.page
    .getByRole("button", { name: "Recover original GL batch", exact: true })
    .click();
  await expect
    .poll(
      async () => {
        return (
          await (
            await reviewer.context.request.get(
              staff + "/api/v1/staff/finance/exports/" + id,
            )
          ).json()
        ).export.state;
      },
      { timeout: 30000 },
    )
    .toBe("BOOKED");
  await reviewer.page
    .getByRole("button", { name: "Refresh Finance", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("SIMGL-" + source.replaceAll("-", ""), {
      exact: true,
    }),
  ).toBeVisible();
  const final = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/exports/" + id,
    )
  ).json();
  expect(final.export.batch).toEqual(initial.export.batch);
  expect(final.export.retryGeneration).toBe(1);
  expect(final.recoveryRequests).toHaveLength(1);
  expect(final.recoveryRequests[0].operationId).toBeTruthy();
  expect(
    sql(
      `SELECT count(*) FROM simulator.gl_batches WHERE source_batch_id='${source}'`,
    ),
  ).toBe("1");
  expect(
    sql(
      `SELECT count(*) FROM core.journal_batches WHERE operation_id='${operation}'`,
    ),
  ).toBe("1");
  expect(
    sql(`SELECT booked::text FROM core.ledger_accounts WHERE id='${account}'`),
  ).toBe(balance);
  await expect(reviewer.page.getByTestId("liquidity-available")).toHaveText(
    "Unavailable",
  );
  const firstReportRequest = reviewer.page.waitForRequest(
    (r) =>
      r.method() === "POST" &&
      r.url().endsWith("/api/v1/staff/finance/reports"),
  );
  await reviewer.page
    .getByRole("button", { name: "Save Finance snapshot", exact: true })
    .click();
  await reviewer.page.waitForURL(/#\/finance\/reports\/[a-f0-9-]{36}$/);
  const originalReportId = reviewer.page.url().split("/").at(-1)!;
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Saved snapshot " + originalReportId.slice(-8).toUpperCase(),
      exact: true,
    }),
  ).toBeVisible();
  const originalReport = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/reports/" + originalReportId,
    )
  ).json();
  expect(originalReport.report.glCompleteness).toBe("COMPLETE");
  expect(originalReport.report.completeness).toBe("PARTIAL");
  expect(originalReport.report.liquidity.status).toBe("MISSING");
  expect(originalReport.report.coreDebitTotal).toBe("99007");
  const accepted = await firstReportRequest;
  const reportKey = await accepted.headerValue("Idempotency-Key");
  expect(reportKey).toBeTruthy();
  const replayResponse = await send(
    reviewer.context,
    "/api/v1/staff/finance/reports",
    accepted.postDataJSON(),
    "POST",
    reportKey!,
    true,
  );
  expect(replayResponse.status()).toBe(201);
  expect((await replayResponse.json()).digest).toBe(originalReport.digest);
  const changedRange = {
    ...accepted.postDataJSON(),
    fromDate: new Date(
      Date.parse(accepted.postDataJSON().fromDate + "T00:00:00Z") - 86400000,
    )
      .toISOString()
      .slice(0, 10),
  };
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/reports",
        changedRange,
        "POST",
        reportKey!,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    originalReport.issues.some(
      (i: { code: string; caseId: string | null }) =>
        i.code === "LIQUIDITY_MISSING" && i.caseId,
    ),
  ).toBeTruthy();
  const cash = {
    evidenceId: crypto.randomUUID(),
    provider: "SYNTHETIC_LIQUIDITY",
    scope: "SYNTHETIC_TST_CLEARING",
    currency: { code: "TST", scale: 2 },
    version: 1,
    asOf: new Date().toISOString(),
    validUntil: new Date(Date.now() + 60000).toISOString(),
    settled: "9007199254741993",
    available: "9007199254740993",
    reserved: "1000",
    expectedIncoming: "4321",
    expectedOutgoing: "2345",
  };
  // Set expiry relative to the same as-of instant to preserve the protocol's 60 second bound.
  cash.validUntil = new Date(Date.parse(cash.asOf) + 60000).toISOString();
  const feed = await reviewer.context.request.post(
    "http://127.0.0.1:4182/admin/liquidity-positions",
    {
      headers: {
        "X-Liquidity-Key": "synthetic-local-liquidity-authentication-key-2026",
      },
      data: cash,
    },
  );
  expect(feed.status()).toBe(201);
  await reviewer.page
    .getByRole("button", { name: "Refresh reports and liquidity", exact: true })
    .click();
  await expect(reviewer.page.getByTestId("liquidity-available")).toHaveText(
    "TST 90,071,992,547,409.93",
  );
  await reviewer.page
    .getByRole("button", { name: "Save Finance snapshot", exact: true })
    .click();
  await reviewer.page.waitForURL(
    (url) =>
      url.hash.startsWith("#/finance/reports/") &&
      !url.hash.endsWith(originalReportId),
  );
  const completeReportId = reviewer.page.url().split("/").at(-1)!;
  const completeReport = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/reports/" + completeReportId,
    )
  ).json();
  expect(completeReport.report.completeness).toBe("COMPLETE");
  expect(completeReport.report.matchedCount).toBe(1);
  expect(completeReport.report.liquidity.available).toBe("9007199254740993");
  const lineage = await (
    await reviewer.context.request.get(
      staff +
        "/api/v1/staff/finance/reports/" +
        completeReportId +
        "/items?after=0&limit=1",
    )
  ).json();
  expect(lineage.items[0].source.journal.id).toBe(source);
  expect(lineage.items[0].receipt.providerJournalReference).toBe(
    "SIMGL-" + source.replaceAll("-", ""),
  );
  const replay = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/reports/" + originalReportId,
    )
  ).json();
  expect(replay.digest).toBe(originalReport.digest);
  expect(replay.report.liquidity.status).toBe("MISSING");
  expect(
    (
      await security.context.request.get(
        staff + "/api/v1/staff/finance/reports/" + completeReportId,
      )
    ).status(),
  ).toBe(403);
  await reviewer.page
    .getByLabel("Original snapshot", { exact: true })
    .selectOption(originalReportId);
  await reviewer.page
    .getByLabel("Replacement snapshot", { exact: true })
    .selectOption(completeReportId);
  await reviewer.page
    .getByLabel("Report revision reason", { exact: true })
    .fill(
      "Replace the missing cash observation with independently authenticated evidence.",
    );
  await reviewer.page
    .getByRole("button", { name: "Propose report revision", exact: true })
    .click();
  let reportRevision: any;
  await expect
    .poll(
      async () => {
        const list = await (
          await reviewer.context.request.get(
            staff + "/api/v1/staff/finance/report-revisions",
          )
        ).json();
        reportRevision = list.find(
          (r: any) =>
            r.details.originalReportId === originalReportId &&
            r.details.replacementReportId === completeReportId,
        );
        return reportRevision?.state;
      },
      { timeout: 30000 },
    )
    .toBe("PENDING");
  const revisionPath =
    "/api/v1/staff/finance/report-revisions/" + reportRevision.id;
  expect(
    (
      await send(
        reviewer.context,
        revisionPath + "/approval",
        { detailsDigest: reportRevision.detailsDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await send(
        reviewer.context,
        revisionPath + "/publication",
        { detailsDigest: reportRevision.detailsDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  await approver.page.goto(staff + "/#/finance/reports/" + completeReportId);
  const approvalRow = approver.page.getByTestId(
    "report-revision-" + reportRevision.id,
  );
  await expect(
    approvalRow.getByRole("button", {
      name: "Approve report revision",
      exact: true,
    }),
  ).toBeDisabled();
  await approvalRow
    .getByLabel("I reviewed both frozen artifacts and the bound reason.", {
      exact: true,
    })
    .check();
  await approvalRow
    .getByRole("button", { name: "Approve report revision", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (
          await (
            await reviewer.context.request.get(staff + revisionPath)
          ).json()
        ).state,
    )
    .toBe("APPROVED");
  await reviewer.page
    .getByRole("button", { name: "Refresh report revisions", exact: true })
    .click();
  const publicationRow = reviewer.page.getByTestId(
    "report-revision-" + reportRevision.id,
  );
  await publicationRow
    .getByRole("button", { name: "Publish approved revision", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (
          await (
            await reviewer.context.request.get(staff + revisionPath)
          ).json()
        ).state,
    )
    .toBe("PUBLISHED");
  const published = await (
    await reviewer.context.request.get(staff + revisionPath)
  ).json();
  expect(published.checkerPerson).not.toBe(published.makerPerson);
  const publicationReplay = await send(
    reviewer.context,
    revisionPath + "/publication",
    { detailsDigest: reportRevision.detailsDigest },
    "POST",
    undefined,
    true,
  );
  expect(publicationReplay.status()).toBe(200);
  expect((await publicationReplay.json()).publishedAt).toBe(
    published.publishedAt,
  );
  const revisedOriginal = await (
    await reviewer.context.request.get(
      staff + "/api/v1/staff/finance/reports/" + originalReportId,
    )
  ).json();
  expect(revisedOriginal.digest).toBe(originalReport.digest);
  expect(revisedOriginal.revisions[0].replacementReportId).toBe(
    completeReportId,
  );
  const stale = {
    ...cash,
    evidenceId: crypto.randomUUID(),
    version: 2,
    asOf: new Date(Date.now() - 120000).toISOString(),
    validUntil: new Date(Date.now() - 60000).toISOString(),
  };
  stale.validUntil = new Date(Date.parse(stale.asOf) + 60000).toISOString();
  expect(
    (
      await reviewer.context.request.post(
        "http://127.0.0.1:4182/admin/liquidity-positions",
        {
          headers: {
            "X-Liquidity-Key":
              "synthetic-local-liquidity-authentication-key-2026",
          },
          data: stale,
        },
      )
    ).status(),
  ).toBe(201);
  await reviewer.page
    .getByRole("button", { name: "Refresh reports and liquidity", exact: true })
    .click();
  await expect(reviewer.page.getByTestId("liquidity-available")).toHaveText(
    "Unavailable",
  );
  await reviewer.page.reload();
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Saved snapshot " + completeReportId.slice(-8).toUpperCase(),
      exact: true,
    }),
  ).toBeVisible();
  expect(
    sql(
      `SELECT count(*) FROM core.journal_batches WHERE operation_id='${operation}'`,
    ),
  ).toBe("1");
  expect(
    sql(`SELECT booked::text FROM core.ledger_accounts WHERE id='${account}'`),
  ).toBe(balance);
  await accessible(reviewer.page);
  expect(errors).toEqual([]);
  await reviewer.page.screenshot({
    path: ".local/finance-report-workbench.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await reviewer.page.screenshot({
    path: ".local/finance-report-mobile.png",
    fullPage: true,
  });
  await Promise.all([
    alice.context.close(),
    reviewer.context.close(),
    security.context.close(),
    approver.context.close(),
  ]);
});
test("real issuer approval executes one internal payment and exposes the same booked reference after resume", async ({
  browser,
}) => {
  const alice = await login(browser, "alice");
  const bob = await login(browser, "bob");
  const reviewer = await login(browser, "reviewer", true);
  const from = await open(alice.context, "Payer"),
    to = await open(bob.context, "Recipient");
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), from, "100000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
      stdio: "ignore",
    },
  );
  await alice.page.reload();
  await bob.page.reload();
  await bob.page.goto(customer + "/#/payments");
  await bob.page
    .getByRole("button", { name: "Show receiving reference", exact: true })
    .click();
  const reference = await bob.page
    .locator(".receiving-reference code")
    .innerText();
  expect(reference).toMatch(/^PB[A-F0-9]{32}$/);
  await alice.page.goto(customer + "/#/payments");
  await alice.page
    .getByLabel("Recipient label", { exact: true })
    .fill("Synthetic recipient");
  await alice.page
    .getByLabel("Shared receiving reference", { exact: true })
    .fill(reference);
  await alice.page
    .getByRole("button", { name: "Add recipient", exact: true })
    .click();
  await expect(
    alice.page.getByText("Recipient saved and verified.", { exact: true }),
  ).toBeVisible();
  await alice.page
    .getByRole("combobox", { name: "From account", exact: true })
    .selectOption(from);
  await alice.page
    .getByRole("textbox", { name: /^Amount · TST minor units/ })
    .fill("10000");
  await accessible(alice.page);
  await alice.page
    .getByRole("button", { name: "Review payment", exact: true })
    .click();
  await expect(
    alice.page.getByRole("heading", {
      name: "Review your payment.",
      exact: true,
    }),
  ).toBeVisible();
  const url = alice.page.url(),
    id = url.split("/").at(-1)!;
  expect(id).toMatch(/^[a-f0-9-]{36}$/);
  const draft = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(draft.total.minorUnits).toBe("10000");
  expect(draft.approvalReady).toBe(false);
  expect(
    (
      await bob.context.request.get(customer + "/api/v1/payments/" + id)
    ).status(),
  ).toBe(404);
  expect(
    (
      await send(alice.context, `/api/v1/payments/${id}/confirmation`, {
        quoteId: draft.quoteId,
      })
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(alice.context, `/api/v1/payments/${id}/confirmation`, {
        quoteId: draft.quoteId,
        amount: { minorUnits: "20000", currency: { code: "TST", scale: 2 } },
      })
    ).status(),
  ).toBe(422);
  expect(
    (
      await send(alice.context, `/api/v1/payments/${id}/confirmation`, {
        quoteId: crypto.randomUUID(),
      })
    ).status(),
  ).toBe(409);
  await expect(
    alice.page.getByText("TST 100.00", { exact: true }).first(),
  ).toBeVisible();
  await accessible(alice.page);
  await alice.page
    .getByRole("button", { name: "Verify this payment", exact: true })
    .click();
  await expect(alice.page.locator("#password")).toBeVisible();
  const username = alice.page.locator("#username");
  if ((await username.isVisible()) && (await username.isEditable()))
    await username.fill("alice");
  await alice.page.locator("#password").fill("Synthetic-Local-123!");
  await alice.page.locator("#kc-login").click();
  await alice.page.waitForURL(customer + "/**");
  await expect(
    alice.page.getByRole("heading", {
      name: "Review your payment.",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    alice.page.getByText(
      "Your verification is current. Confirm this exact payment to continue.",
      { exact: true },
    ),
  ).toBeVisible();
  // Hold an authentic older DRAFT response until after confirmation commits.
  // A slow status request must not erase a newer accepted/settled response and stop polling.
  let releaseOlder!: () => void, olderReady!: () => void;
  const olderSeen = new Promise<void>((resolve) => {
    olderReady = resolve;
  });
  const releaseGate = new Promise<void>((resolve) => {
    releaseOlder = resolve;
  });
  let held = false;
  const statusRoute = "**/api/v1/payments/" + id;
  await alice.page.route(statusRoute, async (route) => {
    if (held || route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    held = true;
    const older = await route.fetch();
    expect(older.status()).toBe(200);
    expect((await older.json()).state).toBe("DRAFT");
    olderReady();
    await releaseGate;
    await route.fulfill({ response: older });
  });
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await olderSeen;
  try {
    const confirmedResponse = alice.page.waitForResponse(
      (r) =>
        r.url().endsWith(`/api/v1/payments/${id}/confirmation`) &&
        r.request().method() === "POST",
    );
    await alice.page
      .getByRole("button", { name: "Confirm payment", exact: true })
      .click();
    const confirmed = await confirmedResponse;
    expect(confirmed.status()).toBe(202);
    const accepted = await confirmed.json();
    expect(accepted.state).not.toBe("DRAFT");
    expect(accepted.version).toBeGreaterThan(draft.version);
  } finally {
    releaseOlder();
  }

  await expect(
    alice.page.getByRole("heading", {
      name: "Payment completed.",
      exact: true,
    }),
  ).toBeVisible({ timeout: 30000 });
  await alice.page.unroute(statusRoute);
  const settled = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(settled.batchId).toMatch(/^[a-f0-9-]{36}$/);
  expect(settled.operationId).toMatch(/^[a-f0-9-]{36}$/);
  expect(
    (
      await send(alice.context, `/api/v1/payments/${id}/confirmation`, {
        quoteId: draft.quoteId,
      })
    ).status(),
  ).toBe(202);
  await alice.page.reload();
  await expect(
    alice.page.getByRole("heading", {
      name: "Payment completed.",
      exact: true,
    }),
  ).toBeVisible();
  const balance = async (context: BrowserContext, account: string) =>
    (
      await (
        await context.request.get(
          customer + `/api/v1/accounts/${account}/balance`,
        )
      ).json()
    ).booked.minorUnits;
  expect(await balance(alice.context, from)).toBe("90000");
  expect(await balance(bob.context, to)).toBe("10000");
  await accessible(alice.page);
  await alice.page.screenshot({
    path: ".local/payment-completed.png",
    fullPage: true,
  });
  await alice.page.setViewportSize({ width: 390, height: 844 });
  expect(
    await alice.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await accessible(alice.page);
  await alice.page.screenshot({
    path: ".local/payment-mobile.png",
    fullPage: true,
  });
  await reviewer.page.goto(staff + "/#/payments/" + id);
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Payment " + id.slice(-8).toUpperCase(),
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    reviewer.page.getByText(settled.batchId, { exact: true }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await alice.page.setViewportSize({ width: 1440, height: 1000 });
  await alice.page
    .getByRole("button", { name: "Sign out", exact: true })
    .click();
  await expect(
    alice.page.getByRole("heading", { name: /Your everyday/ }),
  ).toBeVisible();
  await alice.page.getByRole("button", { name: /Sign in/ }).click();
  await expect(alice.page.locator("#username")).toBeVisible();
  await alice.page.locator("#username").fill("alice");
  await alice.page.locator("#password").fill("Synthetic-Local-123!");
  await alice.page.locator("#kc-login").click();
  await alice.page.waitForURL(customer + "/**");
  await alice.page.goto(url);
  await expect(
    alice.page.getByRole("heading", {
      name: "Payment completed.",
      exact: true,
    }),
  ).toBeVisible();
  expect(
    (
      await (
        await alice.context.request.get(customer + "/api/v1/payments/" + id)
      ).json()
    ).batchId,
  ).toBe(settled.batchId);
  expect(await balance(alice.context, from)).toBe("90000");
  await Promise.all([
    alice.context.close(),
    bob.context.close(),
    reviewer.context.close(),
  ]);
});

test("external payments retain held funds until settlement and expose authenticated incoming funds, linked returns and quarantine", async ({
  browser,
}) => {
  const alice = await login(browser, "alice");
  const reviewer = await login(browser, "reviewer", true);
  const approver = await login(browser, "approver", true);
  const accounts = await (
    await alice.context.request.get(customer + "/api/v1/accounts")
  ).json();
  const from =
    accounts.find((a: { state: string }) => a.state === "ACTIVE")?.id ??
    (await open(alice.context, "ExternalPayer"));
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), from, "100000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
      stdio: "ignore",
    },
  );
  const funded = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  await alice.page.reload();
  await alice.page.goto(customer + "/#/payments");
  await alice.page
    .getByLabel("Recipient label", { exact: true })
    .fill("Synthetic external recipient");
  await alice.page
    .getByRole("combobox", { name: "Payment rail", exact: true })
    .selectOption("SIMULATED");
  await alice.page
    .getByLabel("Shared receiving reference", { exact: true })
    .fill("SIM00000000000000000000000000000001");
  await alice.page
    .getByRole("button", { name: "Add recipient", exact: true })
    .click();
  await expect(
    alice.page.getByText("Recipient saved and verified.", { exact: true }),
  ).toBeVisible();
  await alice.page
    .getByRole("combobox", { name: "From account", exact: true })
    .selectOption(from);
  const beneficiary = (
    await (
      await alice.context.request.get(customer + "/api/v1/beneficiaries")
    ).json()
  ).find((b: { label: string }) => b.label === "Synthetic external recipient");
  await alice.page
    .getByRole("combobox", { name: "Recipient", exact: true })
    .selectOption(beneficiary.id);
  await alice.page
    .getByRole("textbox", { name: /^Amount · TST minor units/ })
    .fill("1234");
  await alice.page
    .getByRole("button", { name: "Review payment", exact: true })
    .click();
  await expect(
    alice.page.getByRole("heading", {
      name: "Review your payment.",
      exact: true,
    }),
  ).toBeVisible();
  const cancelledId = alice.page.url().split("/").at(-1)!;
  await alice.page
    .getByRole("button", { name: "Cancel unconfirmed draft", exact: true })
    .click();
  await expect(
    alice.page.getByText("Cancelled", { exact: true }),
  ).toBeVisible();
  expect(
    (
      await (
        await alice.context.request.get(
          customer + "/api/v1/payments/" + cancelledId,
        )
      ).json()
    ).allowedActions,
  ).toEqual([]);
  await alice.page.goto(customer + "/#/payments");
  await alice.page
    .getByRole("combobox", { name: "From account", exact: true })
    .selectOption(from);
  await alice.page
    .getByRole("combobox", { name: "Recipient", exact: true })
    .selectOption(beneficiary.id);
  await alice.page
    .getByRole("textbox", { name: /^Amount · TST minor units/ })
    .fill("1234");
  await alice.page
    .getByRole("button", { name: "Review payment", exact: true })
    .click();
  await expect(
    alice.page.getByRole("heading", {
      name: "Review your payment.",
      exact: true,
    }),
  ).toBeVisible();
  const id = alice.page.url().split("/").at(-1)!;
  const fault = await alice.context.request.post(
    "http://127.0.0.1:4182/admin/rail-scenarios",
    {
      headers: { "X-Rail-Key": "synthetic-rail-authentication-key-only-2026" },
      data: { instructionId: id, mode: "LOST_RESPONSE" },
    },
  );
  expect(fault.status()).toBe(204);
  await alice.page
    .getByRole("button", { name: "Verify this payment", exact: true })
    .click();
  await expect(alice.page.locator("#password")).toBeVisible();
  const username = alice.page.locator("#username");
  if ((await username.isVisible()) && (await username.isEditable()))
    await username.fill("alice");
  await alice.page.locator("#password").fill("Synthetic-Local-123!");
  await alice.page.locator("#kc-login").click();
  await alice.page.waitForURL(customer + "/**");
  await alice.page
    .getByRole("button", { name: "Confirm payment", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (
          await (
            await alice.context.request.get(customer + "/api/v1/payments/" + id)
          ).json()
        ).state,
      { timeout: 30000 },
    )
    .toBe("SUBMITTED");
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await expect(
    alice.page.getByText("Provider acknowledged · pending", { exact: true }),
  ).toBeVisible();
  await expect(
    alice.page.getByText(/Any reserved funds remain unavailable/),
  ).toBeVisible();
  const balance = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(balance.booked.minorUnits).toBe(funded.booked.minorUnits);
  expect(balance.held.minorUnits).toBe("1234");
  expect(balance.available.minorUnits).toBe(
    (BigInt(funded.booked.minorUnits) - 1234n).toString(),
  );
  const record = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(record.batchId).toBeNull();
  await reviewer.page.goto(staff + "/#/rails");
  await reviewer.page
    .getByRole("button", { name: "Pause new submissions", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("New submissions paused", { exact: true }),
  ).toBeVisible();
  await reviewer.page
    .getByRole("button", { name: "Propose resume", exact: true })
    .click();
  await expect(reviewer.page.getByText(/Resume proposal saved/)).toBeVisible();
  const config = await (
    await reviewer.context.request.get(staff + "/api/v1/staff/rails")
  ).json();
  const resume = config.changes.find(
    (c: { kind: string; actedAt: string | null }) =>
      c.kind === "RESUME" && !c.actedAt,
  );
  // The maker cannot approve their own proposal even if a caller forges a request.
  expect(
    (
      await send(
        reviewer.context,
        `/api/v1/staff/rails/changes/${resume.proposalId}/approval`,
        undefined,
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  await approver.page.goto(staff + "/#/rails");
  await approver.page
    .locator(".case-row")
    .filter({ hasText: resume.proposalId.slice(-8).toUpperCase() })
    .getByRole("button", { name: "Approve rail change", exact: true })
    .click();
  await expect(
    approver.page.getByText("Independent approval recorded.", { exact: true }),
  ).toBeVisible();
  await reviewer.page
    .locator(".case-row")
    .filter({ hasText: resume.proposalId.slice(-8).toUpperCase() })
    .getByRole("button", { name: "Apply approved change", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("New submissions enabled", { exact: true }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/rail-controls.png",
    fullPage: true,
  });
  await alice.page.setViewportSize({ width: 390, height: 844 });
  await accessible(alice.page);
  expect(
    await alice.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await alice.page.screenshot({
    path: ".local/rail-pending-mobile.png",
    fullPage: true,
  });
  const finalResponse = await alice.context.request.post(
    "http://127.0.0.1:4182/admin/rail-events",
    {
      headers: { "X-Rail-Key": "synthetic-rail-authentication-key-only-2026" },
      data: { instructionId: id, status: "SETTLED" },
    },
  );
  expect(finalResponse.status()).toBe(200);
  const packet = await finalResponse.json();
  const forged = { ...packet, signature: "forged" };
  expect(
    (
      await alice.context.request.post(
        "http://127.0.0.1:4180/api/v1/providers/simulated/payments",
        { data: forged },
      )
    ).status(),
  ).toBe(401);
  for (let i = 0; i < 2; i++)
    expect(
      (
        await alice.context.request.post(
          "http://127.0.0.1:4180/api/v1/providers/simulated/payments",
          { data: packet },
        )
      ).status(),
    ).toBe(202);
  await expect
    .poll(
      async () =>
        (
          await (
            await alice.context.request.get(customer + "/api/v1/payments/" + id)
          ).json()
        ).state,
      { timeout: 30000 },
    )
    .toBe("SETTLED");
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await expect(
    alice.page.getByRole("heading", {
      name: "Payment completed.",
      exact: true,
    }),
  ).toBeVisible();
  const completed = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(completed.booked.minorUnits).toBe(
    (BigInt(funded.booked.minorUnits) - 1234n).toString(),
  );
  expect(completed.held.minorUnits).toBe("0");
  const canonical = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(canonical.batchId).toMatch(/^[a-f0-9-]{36}$/);
  await reviewer.page.goto(staff + "/#/payments/" + id);
  await expect(
    reviewer.page
      .getByText("Core journal receipt", { exact: true })
      .locator("..")
      .getByText(canonical.batchId, { exact: true }),
  ).toBeVisible();
  await accessible(alice.page);
  await alice.page.screenshot({
    path: ".local/rail-settled-mobile.png",
    fullPage: true,
  });
  const publishCredit = async (
    eventType: "INBOUND_SETTLED" | "RETURN_SETTLED",
    amount: string,
    reference: string | null = null,
  ) => {
    const body = {
      eventType,
      movementId: crypto.randomUUID(),
      evidenceId: crypto.randomUUID(),
      provider: "SYNTHETIC_RAIL",
      rail: "SIMULATED",
      originalPaymentId: eventType === "RETURN_SETTLED" ? id : null,
      receivingReference: reference,
      amount: { minorUnits: amount, currency: { code: "TST", scale: 2 } },
      businessDate: new Date().toISOString().slice(0, 10),
      occurredAt: new Date().toISOString(),
      version: 1,
      reasonCode: "SYNTHETIC_BROWSER_CREDIT",
    };
    const provider = await alice.context.request.post(
      "http://127.0.0.1:4182/admin/rail-credits",
      {
        headers: {
          "X-Rail-Key": "synthetic-rail-authentication-key-only-2026",
        },
        data: body,
      },
    );
    expect(provider.status()).toBe(200);
    const packet = await provider.json();
    for (let n = 0; n < 2; n++)
      expect(
        (
          await alice.context.request.post(
            "http://127.0.0.1:4180/api/v1/providers/simulated/payments",
            { data: packet },
          )
        ).status(),
      ).toBe(202);
    return body.movementId;
  };
  const waitCredit = async (movement: string, state = "CREDITED") =>
    await expect
      .poll(
        async () =>
          (
            await (
              await alice.context.request.get(
                customer + "/api/v1/payment-credits",
              )
            ).json()
          ).find((c: { id: string }) => c.id === movement)?.state,
        { timeout: 30000 },
      )
      .toBe(state);
  const partial = await publishCredit("RETURN_SETTLED", "600");
  await waitCredit(partial);
  let corrected = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(corrected.returns.status).toBe("PARTIALLY_RETURNED");
  expect(corrected.returns.credited.minorUnits).toBe("600");
  expect(corrected.batchId).toBe(canonical.batchId);
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await expect(
    alice.page.getByText("Partially returned", { exact: true }),
  ).toBeVisible();
  const remainder = await publishCredit("RETURN_SETTLED", "634");
  await waitCredit(remainder);
  corrected = await (
    await alice.context.request.get(customer + "/api/v1/payments/" + id)
  ).json();
  expect(corrected.returns.status).toBe("RETURNED");
  expect(corrected.returns.count).toBe(2);
  expect(corrected.batchId).toBe(canonical.batchId);
  const excessive = await publishCredit("RETURN_SETTLED", "1");
  await waitCredit(excessive, "QUARANTINED");
  const unchanged = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(unchanged.booked.minorUnits).toBe(funded.booked.minorUnits);
  const receiving = await send(
    alice.context,
    `/api/v1/accounts/${from}/receiving-reference`,
  );
  expect(receiving.status()).toBe(200);
  const incoming = await publishCredit(
    "INBOUND_SETTLED",
    "125",
    (await receiving.json()).reference,
  );
  await waitCredit(incoming);
  const credited = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(credited.available.minorUnits).toBe(
    (BigInt(funded.booked.minorUnits) + 125n).toString(),
  );
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await expect(
    alice.page.getByText("Principal returned to your account.", {
      exact: true,
    }),
  ).toBeVisible();
  await accessible(alice.page);
  expect(
    await alice.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await alice.page.screenshot({
    path: ".local/rail-returned-mobile.png",
    fullPage: true,
  });
  await reviewer.page
    .getByRole("button", { name: "Refresh payment inquiry", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Refresh credit inquiry", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Refresh evidence quarantine", exact: true })
    .click();
  await expect(
    reviewer.page
      .locator("section")
      .filter({
        has: reviewer.page.getByRole("heading", {
          name: "Quarantined provider evidence",
          exact: true,
        }),
      })
      .getByText("FINANCIAL CREDIT GUARD", { exact: true }),
  ).toBeVisible();
  await expect(
    reviewer.page
      .getByText("Core journal receipt", { exact: true })
      .locator("..")
      .getByText(canonical.batchId, { exact: true }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/rail-return-workbench.png",
    fullPage: true,
  });
  await Promise.all([
    alice.context.close(),
    reviewer.context.close(),
    approver.context.close(),
  ]);
});

test("owned overdue payments recover through operator inquiry and confirmed cancellation preserves exact balances", async ({
  browser,
}) => {
  const alice = await login(browser, "alice");
  const reviewer = await login(browser, "reviewer", true);
  const staffErrors: string[] = [];
  reviewer.page.on("pageerror", (error) => staffErrors.push(error.message));
  reviewer.page.on("console", (entry) => {
    if (entry.type() === "error") staffErrors.push(entry.text());
  });
  const bob = await login(browser, "bob");
  const accounts = await (
    await alice.context.request.get(customer + "/api/v1/accounts")
  ).json();
  const from =
    accounts.find((a: { state: string }) => a.state === "ACTIVE")?.id ??
    (await open(alice.context, "RecoveryPayer"));
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), from, "10000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
      stdio: "ignore",
    },
  );
  const recipient = await send(
    alice.context,
    "/api/v1/beneficiaries",
    {
      label: "Recovery fixture recipient",
      receivingReference: "SIM00000000000000000000000000000001",
      rail: "SIMULATED",
    },
    "POST",
    crypto.randomUUID(),
  );
  expect(recipient.status()).toBe(201);
  const beneficiary = await recipient.json();
  const confirmExternal = async (amount: string) => {
    const quote = await send(alice.context, "/api/v1/payment-quotes", {
      payerAccountId: from,
      beneficiaryId: beneficiary.id,
      amount: { minorUnits: amount, currency: { code: "TST", scale: 2 } },
      requestedDate: new Date().toISOString().slice(0, 10),
    });
    expect(quote.status()).toBe(201);
    const draft = await send(
      alice.context,
      "/api/v1/payments",
      { quoteId: (await quote.json()).id },
      "POST",
      crypto.randomUUID(),
    );
    expect(draft.status()).toBe(201);
    const payment = await draft.json();
    await alice.page.goto(customer + "/#/payments/" + payment.id);
    await alice.page
      .getByRole("button", { name: "Verify this payment", exact: true })
      .click();
    const username = alice.page.locator("#username");
    if ((await username.isVisible()) && (await username.isEditable()))
      await username.fill("alice");
    await alice.page.locator("#password").fill("Synthetic-Local-123!");
    await alice.page.locator("#kc-login").click();
    await alice.page.waitForURL(customer + "/**");
    await alice.page
      .getByRole("button", { name: "Confirm payment", exact: true })
      .click();
    await expect
      .poll(
        async () => {
          const v = await (
            await alice.context.request.get(
              customer + "/api/v1/payments/" + payment.id,
            )
          ).json();
          return v.state;
        },
        { timeout: 30000 },
      )
      .toBe("SUBMITTED");
    return await (
      await alice.context.request.get(
        customer + "/api/v1/payments/" + payment.id,
      )
    ).json();
  };
  const original = await confirmExternal("901");
  expect(original.id).toMatch(/^[a-f0-9-]{36}$/);
  expect(original.operationId).toMatch(/^[a-f0-9-]{36}$/);
  // Disposable fixture clock and exhausted original workflow; no journal is edited.
  execFileSync(
    "podman",
    [
      "exec",
      "piggybank-postgres-1",
      "psql",
      "-U",
      "dev_admin",
      "-d",
      manifest.database,
      "-v",
      "ON_ERROR_STOP=1",
      "-c",
      `UPDATE core.holds SET created_at=now()-interval '10 minutes' WHERE operation_id='${original.id}'::uuid; UPDATE platform.jobs SET state='DEAD' WHERE operation_id='${original.operationId}'::uuid; UPDATE platform.operations SET state='REVIEW_REQUIRED' WHERE id='${original.operationId}'::uuid;`,
    ],
    { stdio: "ignore" },
  );
  await expect
    .poll(
      async () => {
        const breaks = await (
          await reviewer.context.request.get(
            staff + "/api/v1/staff/payment-reconciliation",
          )
        ).json();
        return breaks.some(
          (b: { paymentId: string; code: string; caseId: string | null }) =>
            b.paymentId === original.id &&
            b.code === "SUBMITTED_OUTCOME_OVERDUE" &&
            b.caseId,
        );
      },
      { timeout: 30000 },
    )
    .toBe(true);
  const held = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(held.held.minorUnits).toBe("901");
  expect(
    (
      await send(
        bob.context,
        `/api/v1/payments/${original.id}/cancellation`,
        undefined,
        "POST",
        crypto.randomUUID(),
      )
    ).status(),
  ).toBe(404);
  expect(
    (
      await bob.context.request.get(
        customer + `/api/v1/payments/${original.id}/recovery-requests`,
      )
    ).status(),
  ).toBe(404);
  await reviewer.page.goto(staff + "/#/payments/" + original.id);
  await reviewer.page
    .getByRole("button", { name: "Refresh reconciliation", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("SUBMITTED OUTCOME OVERDUE", { exact: true }),
  ).toBeVisible();
  const event = await reviewer.context.request.post(
    "http://127.0.0.1:4182/admin/rail-events",
    {
      headers: { "X-Rail-Key": "synthetic-rail-authentication-key-only-2026" },
      data: { instructionId: original.id, status: "SETTLED" },
    },
  );
  expect(event.status()).toBe(200);
  await reviewer.page
    .getByLabel("Recovery reason", { exact: true })
    .fill("Investigate original overdue instruction");
  await reviewer.page
    .getByRole("button", {
      name: "Inquire using original instruction",
      exact: true,
    })
    .click();
  await expect(
    reviewer.page.getByText(/Recovery accepted using the original instruction/),
  ).toBeVisible();
  await expect
    .poll(
      async () =>
        (
          await (
            await alice.context.request.get(
              customer + "/api/v1/payments/" + original.id,
            )
          ).json()
        ).state,
      { timeout: 30000 },
    )
    .toBe("SETTLED");
  const settled = await (
    await alice.context.request.get(
      customer + "/api/v1/payments/" + original.id,
    )
  ).json();
  expect(settled.batchId).toMatch(/^[a-f0-9-]{36}$/);
  await reviewer.page.screenshot({
    path: ".local/recovery-after-inquiry.png",
    fullPage: true,
  });
  expect(staffErrors).toEqual([]);
  await expect(reviewer.page).toHaveURL(staff + "/#/payments/" + original.id);
  await reviewer.page
    .getByLabel("Recovery reason", { exact: true })
    .fill("Cancellation after confirmed settlement");
  await reviewer.page
    .getByRole("button", { name: "Request provider cancellation", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (
          await (
            await reviewer.context.request.get(
              staff + `/api/v1/staff/payments/${original.id}/recovery-requests`,
            )
          ).json()
        ).find((r: { action: string }) => r.action === "CANCEL")?.reasonCode,
      { timeout: 30000 },
    )
    .toBe("CANCELLATION_TOO_LATE_OR_NOT_REQUIRED");
  expect(
    (
      await (
        await alice.context.request.get(
          customer + "/api/v1/payments/" + original.id,
        )
      ).json()
    ).batchId,
  ).toBe(settled.batchId);
  const cancellable = await confirmExternal("777");
  const beforeCancel = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(beforeCancel.held.minorUnits).toBe("777");
  await alice.page.setViewportSize({ width: 390, height: 844 });
  await alice.page
    .getByRole("button", { name: "Request cancellation", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (
          await (
            await alice.context.request.get(
              customer + "/api/v1/payments/" + cancellable.id,
            )
          ).json()
        ).state,
      { timeout: 30000 },
    )
    .toBe("CANCELLED");
  await alice.page
    .getByRole("button", { name: "Refresh payment status", exact: true })
    .click();
  await expect(
    alice.page.getByText("Cancellation is confirmed. No payment was booked.", {
      exact: true,
    }),
  ).toBeVisible();
  const afterCancel = await (
    await alice.context.request.get(
      customer + `/api/v1/accounts/${from}/balance`,
    )
  ).json();
  expect(afterCancel.booked.minorUnits).toBe(beforeCancel.booked.minorUnits);
  expect(afterCancel.held.minorUnits).toBe("0");
  await accessible(alice.page);
  expect(
    await alice.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await alice.page.screenshot({
    path: ".local/payment-cancelled-mobile.png",
    fullPage: true,
  });
  await reviewer.page.goto(staff + "/#/payments/" + original.id);
  await expect
    .poll(
      async () =>
        (
          await (
            await reviewer.context.request.get(
              staff + "/api/v1/staff/payment-reconciliation",
            )
          ).json()
        ).find(
          (b: { paymentId: string; code: string }) =>
            b.paymentId === original.id &&
            b.code === "SUBMITTED_OUTCOME_OVERDUE",
        )?.state,
      { timeout: 30000 },
    )
    .toBe("MATCHED");
  await reviewer.page
    .getByRole("button", { name: "Refresh payment inquiry", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Refresh reconciliation", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("CANCELLATION TOO LATE OR NOT REQUIRED", {
      exact: true,
    }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/payment-recovery-workbench.png",
    fullPage: true,
  });
  await Promise.all([
    alice.context.close(),
    reviewer.context.close(),
    bob.context.close(),
  ]);
});

test("Finance matches independent clearing files and preserves classified payment observations", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true);
  const approver = await login(browser, "approver", true);
  const security = await login(browser, "security", true);
  const alice = await login(browser, "alice");
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        manifest.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  const errors: string[] = [];
  reviewer.page.on("pageerror", (error) => errors.push(error.message));
  await expect
    .poll(
      () =>
        Number(
          sql(
            "SELECT count(*) FROM core.journal_batches j LEFT JOIN finance.gl_exports e ON e.source_batch_id=j.id WHERE e.state IS DISTINCT FROM 'BOOKED'",
          ),
        ),
      { timeout: 30000 },
    )
    .toBe(0);
  const range = {
    fromDate: new Date().toISOString().slice(0, 10),
    toDate: new Date(Date.now() + 86400000).toISOString().slice(0, 10),
  };
  const response = await send(
    reviewer.context,
    "/api/v1/staff/finance/reports",
    range,
    "POST",
    crypto.randomUUID(),
    true,
  );
  expect(response.status()).toBe(201);
  const report = await response.json();
  expect(report.report.glCompleteness).toBe("COMPLETE");
  // Independent clearing fixtures come from the provider's own final events, not bank balances.
  const rows = JSON.parse(
    sql(
      "SELECT coalesce(jsonb_agg(jsonb_build_object('lineId',e.id,'paymentId',i.id,'evidenceId',e.id,'providerVersion',e.version,'principalMinor',e.document->'amount'->>'minorUnits','providerFeeMinor','0','currencyCode',e.document->'amount'->'currency'->>'code','currencyScale',2,'businessDate',e.document->>'settlementDate') ORDER BY i.id),'[]'::jsonb) FROM simulator.rail_instructions i JOIN simulator.rail_events e ON e.id=i.evidence_id WHERE i.state='SETTLED'",
    ),
  );
  // An isolated run also exercises the empty outgoing-period state.
  const file = {
    fileId: crypto.randomUUID(),
    formatVersion: 1,
    provider: "SYNTHETIC_RAIL_SETTLEMENT",
    feePolicy: "SYNTHETIC_PROVIDER_ZERO_FEE_V1",
    range,
    generatedAt: new Date().toISOString(),
    rows,
  };
  const publish = (data: unknown) =>
    reviewer.context.request.post(
      "http://127.0.0.1:4182/admin/settlement-files",
      {
        headers: {
          "X-Finance-Key": "synthetic-finance-authentication-key-only-2026",
        },
        data,
      },
    );
  expect((await publish(file)).status()).toBe(201);
  const moneyState = () =>
    sql(
      "SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'executions',(SELECT count(*) FROM core.payment_executions),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held) ORDER BY id) FROM core.ledger_accounts))",
    );
  const before = moneyState();
  const base = "/api/v1/staff/finance/reconciliations";
  expect(
    (
      await send(
        alice.context,
        base,
        { reportId: report.report.id, fileId: file.fileId },
        "POST",
        crypto.randomUUID(),
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await send(
        security.context,
        base,
        { reportId: report.report.id, fileId: file.fileId },
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(403);
  await reviewer.page.goto(staff + "/#/finance/reports/" + report.report.id);
  const region = reviewer.page.getByRole("region", {
    name: "Payment reconciliation",
    exact: true,
  });
  await region
    .getByLabel("Published settlement file ID", { exact: true })
    .fill(file.fileId);
  const saved = reviewer.page.waitForResponse(
    (r) => r.request().method() === "POST" && r.url().endsWith(base),
  );
  await region
    .getByRole("button", {
      name: "Capture payment reconciliation",
      exact: true,
    })
    .click();
  const accepted = await saved;
  expect(accepted.status()).toBe(201);
  const original = await accepted.json();
  expect(original.reconciliation.completeness).toBe("COMPLETE");
  expect(original.reconciliation.matchedCount).toBeGreaterThanOrEqual(
    rows.length,
  );
  if (rows.length)
    expect(original.reconciliation.matchedCount).toBeGreaterThan(0);
  await expect(
    region.getByRole("heading", { name: /Saved payment reconciliation/ }),
  ).toBeVisible();
  await expect(
    region.getByText(original.digest, { exact: true }),
  ).toBeVisible();
  const key = await accepted.request().headerValue("Idempotency-Key");
  const replay = await send(
    reviewer.context,
    base,
    { reportId: report.report.id, fileId: file.fileId },
    "POST",
    key!,
    true,
  );
  expect(replay.status()).toBe(201);
  expect((await replay.json()).digest).toBe(original.digest);
  await reviewer.page.reload();
  await expect(
    region.getByText(original.digest, { exact: true }),
  ).toBeVisible();
  await approver.page.goto(reviewer.page.url());
  await expect(
    approver.page
      .getByRole("region", { name: "Payment reconciliation", exact: true })
      .getByText(original.digest, { exact: true }),
  ).toBeVisible();
  await expect(
    approver.page.getByRole("button", {
      name: "Capture payment reconciliation",
      exact: true,
    }),
  ).toHaveCount(0);
  expect(
    (
      await send(
        approver.context,
        base,
        { reportId: report.report.id, fileId: file.fileId },
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(403);
  const witness = rows[0] ?? {
    lineId: crypto.randomUUID(),
    paymentId: crypto.randomUUID(),
    evidenceId: crypto.randomUUID(),
    providerVersion: 1,
    principalMinor: "1000",
    providerFeeMinor: "0",
    currencyCode: "TST",
    currencyScale: 2,
    businessDate: range.fromDate,
  };
  const bad = {
    ...file,
    fileId: crypto.randomUUID(),
    generatedAt: new Date().toISOString(),
    rows: [
      ...(rows.length ? rows : [witness]),
      { ...witness, providerFeeMinor: "5" },
    ],
  };
  expect((await publish(bad)).status()).toBe(201);
  await region
    .getByLabel("Published settlement file ID", { exact: true })
    .fill(bad.fileId);
  const captured = reviewer.page.waitForResponse(
    (r) => r.request().method() === "POST" && r.url().endsWith(base),
  );
  await region
    .getByRole("button", {
      name: "Capture payment reconciliation",
      exact: true,
    })
    .click();
  const result = await captured;
  expect(result.status()).toBe(201);
  const difference = await result.json();
  expect(difference.reconciliation.completeness).toBe("EXCEPTIONS");
  await expect(
    region.getByText(/DUPLICATE PROVIDER LINE/).first(),
  ).toBeVisible();
  await expect(
    region.getByText(/PROVIDER FEE UNEXPECTED/).first(),
  ).toBeVisible();
  await expect(
    region.getByRole("link", { name: /Open reconciliation case/ }).first(),
  ).toBeVisible();
  expect(
    (
      await (
        await reviewer.context.request.get(
          staff + base + "/" + original.reconciliation.id,
        )
      ).json()
    ).digest,
  ).toBe(original.digest);
  expect(
    (
      await (
        await reviewer.context.request.get(
          staff + "/api/v1/staff/finance/reports/" + report.report.id,
        )
      ).json()
    ).digest,
  ).toBe(report.digest);
  expect(moneyState()).toBe(before);
  await accessible(reviewer.page);
  await region.screenshot({
    path: ".local/finance-reconciliation-workbench.png",
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await region.screenshot({ path: ".local/finance-reconciliation-mobile.png" });
  expect(errors).toEqual([]);
  await Promise.all([
    reviewer.context.close(),
    approver.context.close(),
    security.context.close(),
    alice.context.close(),
  ]);
});

test("Finance reconciles incoming credits, linked returns and owned suspense through the staff workbench", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true),
    approver = await login(browser, "approver", true),
    security = await login(browser, "security", true),
    alice = await login(browser, "alice");
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        manifest.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  const accounts = await (
    await alice.context.request.get(customer + "/api/v1/accounts")
  ).json();
  const account =
    accounts.find((a: { state: string }) => a.state === "ACTIVE")?.id ??
    (await open(alice.context, "CreditReceiver"));
  const receiving = await send(
    alice.context,
    `/api/v1/accounts/${account}/receiving-reference`,
  );
  expect(receiving.status()).toBe(200);
  const reference = (await receiving.json()).reference;
  const movements: string[] = [];
  for (const [amount, ref] of [
    ["525", reference],
    ["123", "PB" + crypto.randomUUID().replaceAll("-", "").toUpperCase()],
  ]) {
    const event = {
      eventType: "INBOUND_SETTLED",
      movementId: crypto.randomUUID(),
      evidenceId: crypto.randomUUID(),
      provider: "SYNTHETIC_RAIL",
      rail: "SIMULATED",
      originalPaymentId: null,
      receivingReference: ref,
      amount: { minorUnits: amount, currency: { code: "TST", scale: 2 } },
      businessDate: new Date().toISOString().slice(0, 10),
      occurredAt: new Date().toISOString(),
      version: 1,
      reasonCode: "SYNTHETIC_BROWSER_CREDIT",
    };
    const response = await alice.context.request.post(
      "http://127.0.0.1:4182/admin/rail-credits",
      {
        headers: {
          "X-Rail-Key": "synthetic-rail-authentication-key-only-2026",
        },
        data: event,
      },
    );
    expect(response.status()).toBe(200);
    expect(
      (
        await alice.context.request.post(
          "http://127.0.0.1:4180/api/v1/providers/simulated/payments",
          { data: await response.json() },
        )
      ).status(),
    ).toBe(202);
    movements.push(event.movementId);
  }
  await expect
    .poll(
      () =>
        sql(
          `SELECT string_agg(state,',' ORDER BY movement_id) FROM payments.credit_intents WHERE movement_id IN ('${movements[0]}','${movements[1]}')`,
        ),
      { timeout: 30000 },
    )
    .toMatch(
      /CREDITED.*DESTINATION_RESTRICTED|DESTINATION_RESTRICTED.*CREDITED/,
    );
  await expect
    .poll(
      () =>
        Number(
          sql(
            "SELECT count(*) FROM core.journal_batches j LEFT JOIN finance.gl_exports e ON e.source_batch_id=j.id WHERE e.state IS DISTINCT FROM 'BOOKED'",
          ),
        ),
      { timeout: 30000 },
    )
    .toBe(0);
  const range = {
    fromDate: new Date().toISOString().slice(0, 10),
    toDate: new Date(Date.now() + 86400000).toISOString().slice(0, 10),
  };
  const captured = await send(
    reviewer.context,
    "/api/v1/staff/finance/reports",
    range,
    "POST",
    crypto.randomUUID(),
    true,
  );
  expect(captured.status()).toBe(201);
  const report = await captured.json();
  // Independently published provider movements include rejected over-cap return claims from preceding flows.
  const rows = JSON.parse(
    sql(
      "SELECT coalesce(jsonb_agg(jsonb_build_object('lineId',evidence_id,'movementId',movement_id,'eventType',document->>'eventType','originalPaymentId',document->'originalPaymentId','receivingReference',document->'receivingReference','evidenceId',evidence_id,'providerVersion',1,'principalMinor',document->'amount'->>'minorUnits','providerFeeMinor','0','currencyCode','TST','currencyScale',2,'businessDate',document->>'businessDate') ORDER BY movement_id),'[]'::jsonb) FROM simulator.rail_credits",
    ),
  );
  const file = {
    fileId: crypto.randomUUID(),
    formatVersion: 1,
    provider: "SYNTHETIC_RAIL_SETTLEMENT",
    feePolicy: "SYNTHETIC_PROVIDER_ZERO_FEE_V1",
    range,
    generatedAt: new Date().toISOString(),
    rows,
  };
  const publish = (data: unknown) =>
    reviewer.context.request.post(
      "http://127.0.0.1:4182/admin/credit-settlement-files",
      {
        headers: {
          "X-Finance-Key": "synthetic-finance-authentication-key-only-2026",
        },
        data,
      },
    );
  expect((await publish(file)).status()).toBe(201);
  const base = "/api/v1/staff/finance/credit-reconciliations";
  const input = { reportId: report.report.id, fileId: file.fileId };
  for (const [ctx, workforce] of [
    [alice.context, false],
    [security.context, true],
    [approver.context, true],
  ] as const)
    expect(
      (
        await send(ctx, base, input, "POST", crypto.randomUUID(), workforce)
      ).status(),
    ).toBe(403);
  const moneyState = () =>
    sql(
      "SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held) ORDER BY id) FROM core.ledger_accounts))",
    );
  const before = moneyState();
  await reviewer.page.goto(staff + "/#/finance/reports/" + report.report.id);
  const region = reviewer.page.getByRole("region", {
    name: "Incoming and return reconciliation",
    exact: true,
  });
  await region
    .getByLabel("Published credit settlement file ID")
    .fill(file.fileId);
  const created = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture credit reconciliation", exact: true })
    .click();
  const response = await created;
  expect(response.status()).toBe(201);
  const original = await response.json();
  expect(original.reconciliation.matchedCount).toBeGreaterThanOrEqual(1);
  expect(original.reconciliation.suspenseCount).toBeGreaterThanOrEqual(1);
  expect(
    BigInt(original.reconciliation.coreSuspensePrincipalTotal),
  ).toBeGreaterThanOrEqual(123n);
  expect(original.reconciliation.completeness).not.toBe("COMPLETE");
  expect(
    original.issues.some(
      (i: { code: string; caseId: string }) =>
        i.code === "FUNDS_IN_SUSPENSE" && i.caseId,
    ),
  ).toBe(true);
  await expect(
    region.getByRole("heading", { name: /Saved credit reconciliation/ }),
  ).toBeVisible();
  await expect(
    region.getByText("INCOMING_RETURNS_V1", { exact: true }),
  ).toBeVisible();
  const key = crypto.randomUUID();
  const replay = await send(reviewer.context, base, input, "POST", key, true);
  expect(replay.status()).toBe(201);
  const same = await send(reviewer.context, base, input, "POST", key, true);
  expect((await same.json()).digest).toBe((await replay.json()).digest);
  const negative = structuredClone(file);
  negative.fileId = crypto.randomUUID();
  negative.rows[0].providerFeeMinor = "5";
  negative.rows.push(structuredClone(negative.rows[0]));
  expect((await publish(negative)).status()).toBe(201);
  await region
    .getByLabel("Published credit settlement file ID")
    .fill(negative.fileId);
  const differenceResponse = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture credit reconciliation", exact: true })
    .click();
  const difference = await (await differenceResponse).json();
  expect(difference.reconciliation.completeness).toBe("EXCEPTIONS");
  expect(
    difference.issues.some(
      (i: { code: string; caseId: string }) =>
        i.code === "PROVIDER_FEE_UNEXPECTED" && i.caseId,
    ),
  ).toBe(true);
  await expect(
    region
      .getByRole("link", { name: /Open credit reconciliation case/ })
      .first(),
  ).toBeVisible();
  expect(
    (
      await (
        await reviewer.context.request.get(
          staff + base + "/" + original.reconciliation.id,
        )
      ).json()
    ).digest,
  ).toBe(original.digest);
  expect(moneyState()).toBe(before);
  await reviewer.page.reload();
  await expect(
    region.getByRole("heading", { name: /Saved credit reconciliation/ }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/finance-credit-reconciliation-desktop.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await region.screenshot({
    path: ".local/finance-credit-reconciliation-mobile.png",
  });
  await approver.page.goto(reviewer.page.url());
  await expect(
    approver.page
      .getByRole("region", {
        name: "Incoming and return reconciliation",
        exact: true,
      })
      .getByRole("button", {
        name: "Capture credit reconciliation",
        exact: true,
      }),
  ).toHaveCount(0);
  expect(errors).toEqual([]);
  await Promise.all([
    reviewer.context.close(),
    approver.context.close(),
    security.context.close(),
    alice.context.close(),
  ]);
});

test("Finance reconciles closed outgoing and credit file groups with row provenance and cross-file exceptions", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true),
    approver = await login(browser, "approver", true),
    security = await login(browser, "security", true),
    alice = await login(browser, "alice");
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        manifest.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  await expect
    .poll(
      () =>
        Number(
          sql(
            "SELECT count(*) FROM core.journal_batches j LEFT JOIN finance.gl_exports e ON e.source_batch_id=j.id WHERE e.state IS DISTINCT FROM 'BOOKED'",
          ),
        ),
      { timeout: 30000 },
    )
    .toBe(0);
  const range = {
    fromDate: new Date().toISOString().slice(0, 10),
    toDate: new Date(Date.now() + 86400000).toISOString().slice(0, 10),
  };
  const captured = await send(
    reviewer.context,
    "/api/v1/staff/finance/reports",
    range,
    "POST",
    crypto.randomUUID(),
    true,
  );
  expect(captured.status()).toBe(201);
  const report = await captured.json();
  const outgoingRows = JSON.parse(
    sql(
      "SELECT coalesce(jsonb_agg(jsonb_build_object('lineId',e.id,'paymentId',i.id,'evidenceId',e.id,'providerVersion',e.version,'principalMinor',e.document->'amount'->>'minorUnits','providerFeeMinor','0','currencyCode','TST','currencyScale',2,'businessDate',e.document->>'settlementDate') ORDER BY i.id),'[]'::jsonb) FROM simulator.rail_instructions i JOIN simulator.rail_events e ON e.id=i.evidence_id WHERE i.state='SETTLED'",
    ),
  );
  const creditRows = JSON.parse(
    sql(
      "SELECT coalesce(jsonb_agg(jsonb_build_object('lineId',evidence_id,'movementId',movement_id,'eventType',document->>'eventType','originalPaymentId',document->'originalPaymentId','receivingReference',document->'receivingReference','evidenceId',evidence_id,'providerVersion',1,'principalMinor',document->'amount'->>'minorUnits','providerFeeMinor','0','currencyCode','TST','currencyScale',2,'businessDate',document->>'businessDate') ORDER BY movement_id),'[]'::jsonb) FROM simulator.rail_credits",
    ),
  );
  const providerKey = {
    "X-Finance-Key": "synthetic-finance-authentication-key-only-2026",
  };
  const publish = async (kind: "OUTGOING" | "CREDITS", rows: unknown[]) => {
    expect(rows.length).toBeLessThanOrEqual(200);
    const cutoff = new Date().toISOString();
    const middle = Math.ceil(rows.length / 2);
    const files = [rows.slice(0, middle), rows.slice(middle)].map((chunk) => ({
      fileId: crypto.randomUUID(),
      formatVersion: 1,
      provider: "SYNTHETIC_RAIL_SETTLEMENT",
      feePolicy: "SYNTHETIC_PROVIDER_ZERO_FEE_V1",
      range,
      generatedAt: cutoff,
      rows: chunk,
    }));
    for (const f of files)
      expect(
        (
          await reviewer.context.request.post(
            "http://127.0.0.1:4182/admin/" +
              (kind === "OUTGOING"
                ? "settlement-files"
                : "credit-settlement-files"),
            { headers: providerKey, data: f },
          )
        ).status(),
      ).toBe(201);
    const input = {
      setId: crypto.randomUUID(),
      kind,
      range,
      generatedAt: cutoff,
      fileIds: files.map((f) => f.fileId).reverse(),
    };
    const response = await reviewer.context.request.post(
      "http://127.0.0.1:4182/admin/settlement-sets",
      { headers: providerKey, data: input },
    );
    expect(response.status()).toBe(201);
    const sealed = await response.json();
    expect(sealed.rowCount).toBe(rows.length);
    expect(sealed.members).toHaveLength(2);
    expect(
      (
        await reviewer.context.request.post(
          "http://127.0.0.1:4182/admin/settlement-sets",
          {
            headers: providerKey,
            data: { ...input, fileIds: [...input.fileIds].reverse() },
          },
        )
      ).status(),
    ).toBe(200);
    return { input, files };
  };
  const outgoing = await publish("OUTGOING", outgoingRows),
    credit = await publish("CREDITS", creditRows);
  const base = "/api/v1/staff/finance/reconciliation-groups";
  const body = { reportId: report.report.id, setId: outgoing.input.setId };
  for (const [context, workforce] of [
    [alice.context, false],
    [security.context, true],
    [approver.context, true],
  ] as const)
    expect(
      (
        await send(context, base, body, "POST", crypto.randomUUID(), workforce)
      ).status(),
    ).toBe(403);
  const moneyState = () =>
    sql(
      "SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held) ORDER BY id) FROM core.ledger_accounts))",
    );
  const before = moneyState();
  await reviewer.page.goto(staff + "/#/finance/reports/" + report.report.id);
  const region = reviewer.page.getByRole("region", {
    name: "Grouped clearing reconciliation",
    exact: true,
  });
  await region
    .getByLabel("Published settlement set ID")
    .fill(outgoing.input.setId);
  const waiting = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture clearing group", exact: true })
    .click();
  const response = await waiting;
  expect(response.status()).toBe(201);
  const original = await response.json();
  expect(original.reconciliation.kind).toBe("OUTGOING");
  expect(original.reconciliation.completeness).toBe("COMPLETE");
  expect(original.reconciliation.providerRowCount).toBe(outgoingRows.length);
  expect(original.reconciliation.members).toHaveLength(2);
  await expect(
    region.getByRole("heading", { name: /Saved clearing group/ }),
  ).toBeVisible();
  await expect(
    region.getByText("OUTGOING_PAYMENT_GROUP_V1", { exact: true }),
  ).toBeVisible();
  const headers = await response.request().allHeaders();
  const replay = await send(
    reviewer.context,
    base,
    body,
    "POST",
    headers["idempotency-key"],
    true,
  );
  expect(replay.status()).toBe(201);
  expect((await replay.json()).digest).toBe(original.digest);
  const page = await (
    await reviewer.context.request.get(
      staff + base + "/" + original.reconciliation.id + "/items?limit=100",
    )
  ).json();
  expect(
    page.items.reduce(
      (total: number, item: { sourceRows: unknown[] }) =>
        total + item.sourceRows.length,
      0,
    ),
  ).toBe(outgoingRows.length);
  expect(JSON.stringify(page)).not.toContain('"signature"');
  // Cross-file duplicates are independently published, retain both references and cannot change bank money.
  const example = outgoingRows[0] ?? {
    lineId: crypto.randomUUID(),
    paymentId: crypto.randomUUID(),
    evidenceId: crypto.randomUUID(),
    providerVersion: 1,
    principalMinor: "17",
    providerFeeMinor: "0",
    currencyCode: "TST",
    currencyScale: 2,
    businessDate: range.fromDate,
  };
  const negative = await publish("OUTGOING", [
    example,
    { ...example, providerFeeMinor: "5" },
  ]);
  await region
    .getByLabel("Published settlement set ID")
    .fill(negative.input.setId);
  const differing = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture clearing group", exact: true })
    .click();
  const bad = await (await differing).json();
  expect(bad.reconciliation.completeness).toBe("EXCEPTIONS");
  for (const code of [
    "DUPLICATE_PROVIDER_LINE",
    "DUPLICATE_PROVIDER_EVIDENCE",
    "PROVIDER_FEE_UNEXPECTED",
  ])
    expect(
      bad.issues.some(
        (i: { code: string; caseId: string }) => i.code === code && i.caseId,
      ),
    ).toBe(true);
  await expect(
    region
      .getByRole("link", { name: /Open group reconciliation case/ })
      .first(),
  ).toBeVisible();
  expect(
    (
      await (
        await reviewer.context.request.get(
          staff + base + "/" + original.reconciliation.id,
        )
      ).json()
    ).digest,
  ).toBe(original.digest);
  await region
    .getByLabel("Published settlement set ID")
    .fill(credit.input.setId);
  const creditWaiting = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture clearing group", exact: true })
    .click();
  const credits = await (await creditWaiting).json();
  expect(credits.reconciliation.kind).toBe("CREDITS");
  expect(credits.reconciliation.coreBankFeeTotal).toBeNull();
  expect(credits.reconciliation.providerRowCount).toBe(creditRows.length);
  await expect(
    region.getByText("INCOMING_RETURN_GROUP_V1", { exact: true }),
  ).toBeVisible();
  const next = region.getByRole("button", {
    name: "Next group lineage page",
    exact: true,
  });
  if (!(await next.isDisabled())) {
    await next.click();
    await region
      .getByRole("button", { name: "First group lineage page", exact: true })
      .click();
  }
  expect(moneyState()).toBe(before);
  await reviewer.page.reload();
  await expect(
    region.getByRole("heading", { name: /Saved clearing group/ }),
  ).toBeVisible();
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/finance-group-reconciliation-desktop.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await region.screenshot({
    path: ".local/finance-group-reconciliation-mobile.png",
  });
  await approver.page.goto(reviewer.page.url());
  const readOnly = approver.page.getByRole("region", {
    name: "Grouped clearing reconciliation",
    exact: true,
  });
  await expect(
    readOnly.getByRole("heading", { name: /Saved clearing group/ }),
  ).toBeVisible();
  await expect(
    readOnly.getByRole("button", {
      name: "Capture clearing group",
      exact: true,
    }),
  ).toHaveCount(0);
  expect(errors).toEqual([]);
  await Promise.all([
    reviewer.context.close(),
    approver.context.close(),
    security.context.close(),
    alice.context.close(),
  ]);
});

test("Finance independently approves immutable periodic preview policies and calculates exact fees and accruals without posting", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true),
    approver = await login(browser, "approver", true),
    security = await login(browser, "security", true),
    alice = await login(browser, "alice");
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        manifest.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  const native = () =>
    sql(
      "SELECT jsonb_build_object('accounts',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts),'journals',(SELECT count(*) FROM core.journal_batches),'legs',(SELECT count(*) FROM core.journal_entries))",
    );
  const before = native();
  const products = await (
    await alice.context.request.get(customer + "/api/v1/products")
  ).json();
  const now = new Date(),
    from = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1))
      .toISOString()
      .slice(0, 10),
    until = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1))
      .toISOString()
      .slice(0, 10);
  await reviewer.page.goto(staff + "/#/finance/policies");
  await reviewer.page
    .getByLabel("Product version ID", { exact: true })
    .fill(products[0].id);
  await reviewer.page
    .getByLabel("Rule code", { exact: true })
    .fill("BROWSER_FEE_" + crypto.randomUUID().slice(0, 8).toUpperCase());
  await reviewer.page.getByLabel("Effective from", { exact: true }).fill(from);
  await reviewer.page
    .getByLabel("Effective until, exclusive", { exact: true })
    .fill(until);
  await reviewer.page
    .getByLabel("Monthly fee minor units", { exact: true })
    .fill("9007199254740993");
  await reviewer.page
    .getByLabel("Reason for policy", { exact: true })
    .fill(
      "Independently review the exact synthetic monthly fee; keep charges disabled",
    );
  const created = reviewer.page.waitForResponse(
    (r) =>
      r.url().endsWith("/api/v1/staff/finance/periodic-policies") &&
      r.request().method() === "POST",
  );
  await reviewer.page
    .getByRole("button", { name: "Save preview policy request", exact: true })
    .click();
  const response = await created;
  expect(response.status()).toBe(202);
  const fee = await response.json();
  const path = `/api/v1/staff/finance/periodic-policies/${fee.id}`;
  const key = response.request().headers()["idempotency-key"],
    body = response.request().postDataJSON();
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/periodic-policies",
        body,
        "POST",
        key,
        true,
      )
    ).status(),
  ).toBe(202);
  const changed = structuredClone(body);
  changed.policy.calendar.cutoff = "16:00:00";
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/periodic-policies",
        changed,
        "POST",
        key,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(
        reviewer.context,
        path + "/publication",
        { detailsDigest: fee.policyDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(
        reviewer.context,
        path + "/preview",
        { periodStart: from, evaluatedAt: new Date().toISOString() },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  await approver.page.goto(staff + "/#/finance/policies/" + fee.id);
  await expect(
    approver.page.getByRole("button", {
      name: "Save preview policy request",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    approver.page.getByRole("button", {
      name: "Approve preview policy",
      exact: true,
    }),
  ).toBeDisabled();
  await approver.page
    .getByLabel("I reviewed this policy, calendar and digest.", { exact: true })
    .check();
  await approver.page
    .getByRole("button", { name: "Approve preview policy", exact: true })
    .click();
  await expect(
    approver.page.getByText("Independent policy approval recorded.", {
      exact: true,
    }),
  ).toBeVisible();
  await reviewer.page
    .getByRole("button", { name: "Refresh policies", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Publish preview policy", exact: true })
    .click();
  await expect(
    reviewer.page.getByText(
      "Preview policy published. Customer charges remain disabled.",
      { exact: true },
    ),
  ).toBeVisible();
  await reviewer.page
    .getByRole("button", { name: "Calculate preview", exact: true })
    .click();
  const preview = reviewer.page.getByRole("status").filter({
    has: reviewer.page.getByRole("heading", {
      name: "Calculation preview",
      exact: true,
    }),
  });
  await expect(
    preview.getByText("9007199254740993", { exact: true }),
  ).toBeVisible();
  const original = await (
    await reviewer.context.request.get(staff + path)
  ).json();
  expect(original.state).toBe("PUBLISHED");
  expect(original.publication.digest).toBe(fee.policyDigest);
  // A second independently approved policy exercises daily exact arithmetic through the same portal.
  await reviewer.page
    .getByLabel("Rule code", { exact: true })
    .fill("BROWSER_ACCRUAL_" + crypto.randomUUID().slice(0, 8).toUpperCase());
  await reviewer.page
    .getByLabel("Calculation kind", { exact: true })
    .selectOption("DAILY_CREDIT_ACCRUAL");
  await reviewer.page
    .getByLabel("Annual basis points", { exact: true })
    .fill("365");
  const dailyResponse = reviewer.page.waitForResponse(
    (r) =>
      r.url().endsWith("/api/v1/staff/finance/periodic-policies") &&
      r.request().method() === "POST",
  );
  await reviewer.page
    .getByRole("button", { name: "Save preview policy request", exact: true })
    .click();
  const dailyCreated = await dailyResponse;
  expect(dailyCreated.status()).toBe(202);
  const daily = await dailyCreated.json(),
    dailyPath = `/api/v1/staff/finance/periodic-policies/${daily.id}`;
  expect(
    (
      await send(
        approver.context,
        dailyPath + "/approval",
        { detailsDigest: daily.policyDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(200);
  await reviewer.page
    .getByRole("button", { name: "Refresh policies", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Publish preview policy", exact: true })
    .click();
  await expect(
    reviewer.page.getByText(
      "Preview policy published. Customer charges remain disabled.",
      { exact: true },
    ),
  ).toBeVisible();
  const computedResponse = await send(
    reviewer.context,
    dailyPath + "/preview",
    {
      periodStart: from,
      closingBooked: {
        minorUnits: "9007199254740993",
        currency: { code: "TST", scale: 2 },
      },
      evaluatedAt: new Date().toISOString(),
    },
    "POST",
    undefined,
    true,
  );
  expect(computedResponse.status()).toBe(200);
  const computed = await computedResponse.json();
  expect(computed.amount.minorUnits).toBe("900719925474");
  expect(computed.exactNumerator).toBe((9007199254740993n * 365n).toString());
  expect(computed.exactDenominator).toBe("3650000");
  await reviewer.page
    .getByLabel("Closing booked minor units", { exact: true })
    .fill("9007199254740993");
  await reviewer.page
    .getByLabel("Evaluate at UTC", { exact: true })
    .fill(computed.dueAt);
  await reviewer.page
    .getByRole("button", { name: "Calculate preview", exact: true })
    .click();
  await expect(
    preview.getByText("900719925474", { exact: true }),
  ).toBeVisible();
  await expect(preview.getByText("Yes", { exact: true })).toBeVisible();
  await expect(preview.getByText(computed.dueAt, { exact: true })).toHaveCount(
    2,
  );
  const invalid = structuredClone(body);
  invalid.policy.id = crypto.randomUUID();
  invalid.policy.activation = "ENABLED";
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/periodic-policies",
        invalid,
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(422);
  invalid.policy.activation = "PREVIEW_ONLY";
  invalid.policy.calendar.timezone = "America/New_York";
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/periodic-policies",
        invalid,
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(422);
  expect(
    (
      await reviewer.context.request.post(staff + dailyPath + "/preview", {
        data: { periodStart: from, evaluatedAt: new Date().toISOString() },
        headers: { Origin: staff },
      })
    ).status(),
  ).toBe(403);
  expect((await security.context.request.get(staff + dailyPath)).status()).toBe(
    403,
  );
  expect([401, 403]).toContain(
    (await alice.context.request.get(customer + dailyPath)).status(),
  );
  expect(
    (
      await send(
        approver.context,
        dailyPath + "/publication",
        { detailsDigest: daily.policyDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(403);
  expect(
    (await (await reviewer.context.request.get(staff + path)).json())
      .publication.digest,
  ).toBe(original.publication.digest);
  expect(native()).toBe(before);
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/finance-periodic-desktop.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await reviewer.page.screenshot({
    path: ".local/finance-periodic-mobile.png",
    fullPage: true,
  });
  await reviewer.page.reload();
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Review periodic policy",
      exact: true,
    }),
  ).toBeVisible();
  expect(errors).toEqual([]);
  await reviewer.context.close();
  await approver.context.close();
  await security.context.close();
  await alice.context.close();
});

test("Finance executes one independently approved synthetic funding correction and recovers a lost response from native evidence", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true),
    approver = await login(browser, "approver", true),
    security = await login(browser, "security", true),
    alice = await login(browser, "alice");
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  const accounts = await (
    await alice.context.request.get(customer + "/api/v1/accounts")
  ).json();
  const account =
    accounts.find(
      (a: { state: string; debitBlocked: boolean; creditBlocked: boolean }) =>
        a.state === "ACTIVE" && !a.debitBlocked && !a.creditBlocked,
    )?.id ?? (await open(alice.context, "Payer"));
  const balance = async () =>
    await (
      await alice.context.request.get(
        customer + `/api/v1/accounts/${account}/balance`,
      )
    ).json();
  const before = await balance();
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(fixture.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        fixture.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  const operation = crypto.randomUUID();
  const source = execFileSync(
    resolve("target/debug/bank-fund"),
    [operation, account, "2500"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${fixture.database}`,
      },
      encoding: "utf8",
    },
  ).trim();
  expect(source).toMatch(/^[a-f0-9-]{36}$/);
  expect((await balance()).booked.minorUnits).toBe(
    (BigInt(before.booked.minorUnits) + 2500n).toString(),
  );
  await expect
    .poll(
      () =>
        sql(
          `SELECT state FROM finance.gl_exports WHERE source_batch_id='${source}'`,
        ),
      { timeout: 30000 },
    )
    .toBe("BOOKED");
  await reviewer.page.goto(staff + "/#/finance/corrections");
  await reviewer.page
    .getByLabel("Original funding journal ID", { exact: true })
    .fill(source);
  await reviewer.page
    .getByLabel("Correction reason", { exact: true })
    .fill(
      "Reverse this exact mistaken synthetic funding and retain its original financial evidence",
    );
  const stagedResponse = reviewer.page.waitForResponse(
    (r) =>
      r.url().endsWith("/api/v1/staff/finance/corrections") &&
      r.request().method() === "POST",
  );
  await reviewer.page
    .getByRole("button", {
      name: "Save funding correction request",
      exact: true,
    })
    .click();
  const staged = await stagedResponse;
  expect(staged.status()).toBe(202);
  const request = await staged.json(),
    path = `/api/v1/staff/finance/corrections/${request.id}`;
  const key = staged.request().headers()["idempotency-key"],
    input = staged.request().postDataJSON();
  expect(request.details.originalBatchId).toBe(source);
  expect(request.source.batch.journal.operationId).toBe(operation);
  expect(
    (
      await send(
        reviewer.context,
        path + "/execution",
        { detailsDigest: request.detailsDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(
        reviewer.context,
        path + "/approval",
        { detailsDigest: request.detailsDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await send(
        approver.context,
        path + "/approval",
        { detailsDigest: "0".repeat(64) },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(409);
  await approver.page.goto(staff + "/#/finance/corrections/" + request.id);
  await expect(
    approver.page.getByRole("button", {
      name: "Approve funding correction",
      exact: true,
    }),
  ).toBeDisabled();
  await approver.page
    .getByLabel(
      "I reviewed the original source, exact reversing legs, reason and digest.",
      { exact: true },
    )
    .check();
  await approver.page
    .getByRole("button", { name: "Approve funding correction", exact: true })
    .click();
  await expect(
    approver.page.getByText("Independent correction approval recorded.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    approver.page.getByRole("button", {
      name: "Execute approved correction",
      exact: true,
    }),
  ).toHaveCount(0);
  expect(
    (
      await send(
        approver.context,
        path + "/execution",
        { detailsDigest: request.detailsDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(403);
  await reviewer.page
    .getByRole("button", { name: "Refresh corrections", exact: true })
    .click();
  await expect(
    reviewer.page.getByRole("button", {
      name: "Execute approved correction",
      exact: true,
    }),
  ).toBeDisabled();
  await reviewer.page
    .getByLabel(
      "I intend to reverse this exact synthetic funding and debit the customer account.",
      { exact: true },
    )
    .check();
  let committed!: () => void;
  const nativeCommitted = new Promise<void>((resolve) => {
    committed = resolve;
  });
  await reviewer.page.route("**" + path + "/execution", async (route) => {
    const response = await route.fetch();
    expect(response.status()).toBe(200);
    expect((await response.json()).state).toBe("EXECUTED");
    committed();
    await route.abort("failed");
  });
  await reviewer.page
    .getByRole("button", { name: "Execute approved correction", exact: true })
    .click();
  await nativeCommitted;
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Native correction receipt",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    reviewer.page.getByText(
      "Recovered the original native correction receipt after the interrupted response.",
      { exact: true },
    ),
  ).toBeVisible();
  await reviewer.page.unroute("**" + path + "/execution");
  const saved = await (await reviewer.context.request.get(staff + path)).json();
  expect(saved.state).toBe("EXECUTED");
  expect(saved.receipt.executionId).toBe(request.id);
  expect(saved.receipt.originalBatchId).toBe(source);
  expect(saved.detailsDigest).toBe(request.detailsDigest);
  expect(saved.details.sourceDigest).toBe(request.details.sourceDigest);
  expect(saved.receipt.financialBatch.journal.postingKind).toBe("CORRECTION");
  expect(saved.receipt.financialBatch.journal.originalBatchId).toBe(source);
  const after = await balance();
  expect(after.booked.minorUnits).toBe(before.booked.minorUnits);
  expect(after.held.minorUnits).toBe(before.held.minorUnits);
  expect(after.available.minorUnits).toBe(before.available.minorUnits);
  const replay = await send(
    reviewer.context,
    path + "/execution",
    { detailsDigest: request.detailsDigest },
    "POST",
    undefined,
    true,
  );
  expect(replay.status()).toBe(200);
  expect((await replay.json()).receipt.batchId).toBe(saved.receipt.batchId);
  expect(
    sql(
      `SELECT count(*) FROM core.journal_batches WHERE original_batch='${source}' AND posting_kind='CORRECTION'`,
    ),
  ).toBe("1");
  expect(
    sql(
      `SELECT count(*) FROM core.finance_correction_receipts WHERE original_batch_id='${source}'`,
    ),
  ).toBe("1");
  const originalReplay = await send(
    reviewer.context,
    "/api/v1/staff/finance/corrections",
    input,
    "POST",
    key,
    true,
  );
  expect(originalReplay.status()).toBe(202);
  expect((await originalReplay.json()).id).toBe(request.id);
  const changed = structuredClone(input);
  changed.reason = "Different approved purpose";
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/corrections",
        changed,
        "POST",
        key,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(
        reviewer.context,
        "/api/v1/staff/finance/corrections",
        { originalBatchId: saved.receipt.batchId, reason: input.reason },
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(422);
  expect((await security.context.request.get(staff + path)).status()).toBe(403);
  expect((await alice.context.request.get(customer + path)).status()).toBe(403);
  expect(
    (
      await reviewer.context.request.post(staff + path + "/execution", {
        data: { detailsDigest: request.detailsDigest },
        headers: { Origin: staff },
      })
    ).status(),
  ).toBe(403);
  await expect
    .poll(
      () =>
        sql(
          `SELECT state FROM finance.gl_exports WHERE source_batch_id='${saved.receipt.batchId}'`,
        ),
      { timeout: 30000 },
    )
    .toBe("BOOKED");
  await reviewer.page
    .getByRole("button", { name: "Refresh corrections", exact: true })
    .click();
  await expect(
    reviewer.page.getByRole("link", {
      name: "Original GL export",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    reviewer.page.getByRole("link", {
      name: "Correction GL export",
      exact: true,
    }),
  ).toBeVisible();
  await reviewer.page.locator(".staff-sidebar").evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  await expect(
    reviewer.page.locator(".staff-sidebar .synthetic"),
  ).toBeInViewport();
  await accessible(reviewer.page);
  await reviewer.page.locator(".staff-sidebar").evaluate((node) => {
    node.scrollTop = 0;
  });
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/finance-correction-desktop.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await reviewer.page.screenshot({
    path: ".local/finance-correction-mobile.png",
    fullPage: true,
  });
  await reviewer.page.reload();
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Native correction receipt",
      exact: true,
    }),
  ).toBeVisible();
  expect(errors).toEqual([]);
  await reviewer.context.close();
  await approver.context.close();
  await security.context.close();
  await alice.context.close();
});

test("Finance captures immutable native periodic bases, retries original evidence and observes later facts without posting", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer", true),
    approver = await login(browser, "approver", true),
    security = await login(browser, "security", true),
    alice = await login(browser, "alice");
  const errors: string[] = [];
  reviewer.page.on("pageerror", (e) => errors.push(e.message));
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(fixture.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (statement: string) =>
    execFileSync(
      "podman",
      [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        fixture.database,
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        statement,
      ],
      { encoding: "utf8" },
    ).trim();
  const accounts = await (
    await alice.context.request.get(customer + "/api/v1/accounts")
  ).json();
  const account =
    accounts.find(
      (a: { state: string; debitBlocked: boolean; creditBlocked: boolean }) =>
        a.state === "ACTIVE" && !a.debitBlocked && !a.creditBlocked,
    )?.id ?? (await open(alice.context, "Payer"));
  const product = sql(
    `SELECT product_version_id FROM core.account_registry WHERE id='${account}'`,
  );
  const fund = (minor: string) =>
    execFileSync(
      resolve("target/debug/bank-fund"),
      [crypto.randomUUID(), account, minor],
      {
        env: {
          ...process.env,
          BANK_PROFILE: "local",
          BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${fixture.database}`,
        },
        encoding: "utf8",
      },
    ).trim();
  for (let i = 0; i < 6; i++) fund("10000");
  const native = () =>
    sql(
      "SELECT jsonb_build_object('accounts',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts),'journals',(SELECT count(*) FROM core.journal_batches),'legs',(SELECT count(*) FROM core.journal_entries))",
    );
  const day = (n: number) =>
    new Date(Date.now() + n * 86400000).toISOString().slice(0, 10);
  const today = day(0),
    yesterday = day(-1),
    until = day(3);
  const input = {
    policy: {
      id: crypto.randomUUID(),
      code: "BASIS_" + crypto.randomUUID().slice(0, 8).toUpperCase(),
      version: 1,
      label: "SYNTHETIC",
      activation: "PREVIEW_ONLY",
      productVersionId: product,
      effectiveFrom: yesterday,
      effectiveUntil: until,
      calendar: {
        id: crypto.randomUUID(),
        code: "BASIS_UTC",
        version: 1,
        timezone: "UTC",
        coverageFrom: yesterday,
        coverageUntil: day(20),
        businessWeekdays: [1, 2, 3, 4, 5, 6, 7],
        holidays: [day(1)],
        cutoff: "15:00:00",
        rollConvention: "FOLLOWING_14_DAYS",
      },
      calculation: {
        kind: "DAILY_CREDIT_ACCRUAL",
        annualBasisPoints: 365,
        dayCount: "ACT_365_FIXED",
        basis: "NONNEGATIVE_CLOSING_BOOKED",
        rounding: "HALF_UP_EACH_DAY",
      },
    },
    reason:
      "Capture native business-date evidence without enabling any charge or accrual posting",
  };
  const staged = await send(
    reviewer.context,
    "/api/v1/staff/finance/periodic-policies",
    input,
    "POST",
    crypto.randomUUID(),
    true,
  );
  expect(staged.status()).toBe(202);
  const policy = await staged.json(),
    path = "/api/v1/staff/finance/periodic-policies/" + policy.id;
  expect(
    (
      await send(
        approver.context,
        path + "/approval",
        { detailsDigest: policy.policyDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(200);
  expect(
    (
      await send(
        reviewer.context,
        path + "/publication",
        { detailsDigest: policy.policyDigest },
        "POST",
        undefined,
        true,
      )
    ).status(),
  ).toBe(200);
  await reviewer.page.goto(staff + "/#/finance/policies/" + policy.id);
  const region = reviewer.page.getByRole("region", {
    name: "Native ledger basis",
    exact: true,
  });
  await region.getByLabel("Basis account ID", { exact: true }).fill(account);
  await region.getByLabel("Basis period start", { exact: true }).fill(today);
  const before = native(),
    base = "/api/v1/staff/finance/periodic-bases";
  const waiting = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture or retry basis", exact: true })
    .click();
  const response = await waiting;
  expect(response.status()).toBe(201);
  const original = await response.json(),
    body = response.request().postDataJSON(),
    key = response.request().headers()["idempotency-key"];
  expect(original.basis.periodStatus).toBe("OPEN_PERIOD_PROVISIONAL");
  expect(original.basis.source).toBe("CORE_BUSINESS_DATE_BASIS_V1");
  expect(original.basis.sourceCount).toBeGreaterThan(5);
  expect(original.basis.observedClosingBooked).toBe(
    original.basis.currentBooked,
  );
  expect(original.basis.openingBooked).toBe("0");
  expect(original.basis.dueAt.slice(0, 10)).toBe(day(2));
  expect(original.calculation.provisional).toBe(true);
  expect(original.calculation.evaluatedAt).toBe(original.basis.coreAsOf);
  expect(original.calculation.amount.minorUnits).toBe(
    (
      (BigInt(original.basis.observedClosingBooked) * 365n + 1825000n) /
      3650000n
    ).toString(),
  );
  expect(native()).toBe(before);
  await expect(
    region.getByRole("heading", {
      name: "Captured Core observation",
      exact: true,
    }),
  ).toBeVisible();
  await expect(region.getByTestId("basis-source")).toHaveCount(5);
  const all = await (
    await reviewer.context.request.get(
      staff + base + "/" + original.basis.id + "/items?limit=100",
    )
  ).json();
  expect(all.items).toHaveLength(original.basis.sourceCount);
  const sum = all.items.reduce(
    (n: bigint, i: { deltaMinor: string }) => n + BigInt(i.deltaMinor),
    0n,
  );
  expect(sum.toString()).toBe(original.basis.observedClosingBooked);
  for (const item of all.items)
    expect(
      sql(
        `SELECT fingerprint FROM core.journal_batches WHERE id='${item.batchId}'`,
      ),
    ).toBe(item.journalFingerprint);
  await region
    .getByRole("button", { name: "Load more basis facts", exact: true })
    .click();
  await expect(region.getByTestId("basis-source")).toHaveCount(
    Math.min(10, original.basis.sourceCount),
  );
  const savedUrl = reviewer.page.url();
  await reviewer.page.reload();
  await expect(
    region.getByRole("heading", {
      name: "Captured Core observation",
      exact: true,
    }),
  ).toBeVisible();
  fund("777");
  const afterFund = native();
  const replay = await send(reviewer.context, base, body, "POST", key, true);
  expect(replay.status()).toBe(201);
  expect(await replay.json()).toEqual(original);
  expect(
    (
      await (
        await reviewer.context.request.get(
          staff + base + "/" + original.basis.id + "/items?limit=100",
        )
      ).json()
    ).items,
  ).toEqual(all.items);
  // Fresh capture uses a new key; no prior artifact is refreshed or replaced.
  await region.getByLabel("Basis account ID", { exact: true }).fill(account);
  await region.getByLabel("Basis period start", { exact: true }).fill(today);
  const freshWaiting = reviewer.page.waitForResponse(
    (r) => r.url().endsWith(base) && r.request().method() === "POST",
  );
  await region
    .getByRole("button", { name: "Capture fresh observation", exact: true })
    .click();
  const freshResponse = await freshWaiting;
  expect(freshResponse.status()).toBe(201);
  const fresh = await freshResponse.json();
  expect(fresh.basis.id).not.toBe(original.basis.id);
  expect(fresh.basis.sourceCount).toBe(original.basis.sourceCount + 1);
  expect(fresh.basis.observedClosingBooked).toBe(
    (BigInt(original.basis.observedClosingBooked) + 777n).toString(),
  );
  expect(native()).toBe(afterFund);
  const closedResponse = await send(
    reviewer.context,
    base,
    { ...body, periodStart: yesterday },
    "POST",
    crypto.randomUUID(),
    true,
  );
  expect(closedResponse.status()).toBe(201);
  const closed = await closedResponse.json();
  expect(closed.basis.periodStatus).toBe("CLOSED_PERIOD_OBSERVATION");
  expect(closed.basis.sourceCount).toBe(0);
  expect(closed.basis.observedClosingBooked).toBe("0");
  expect(BigInt(closed.basis.currentBooked)).toBeGreaterThan(0n);
  expect(closed.calculation).toBeNull();
  expect(closed.calculationReason).toBe(
    "ACCOUNT_REGISTERED_AFTER_PERIOD_START",
  );
  for (const [context, workforce] of [
    [alice.context, false],
    [security.context, true],
    [approver.context, true],
  ] as const)
    expect(
      (
        await send(context, base, body, "POST", crypto.randomUUID(), workforce)
      ).status(),
    ).toBe(403);
  expect(
    (
      await security.context.request.get(staff + base + "/" + original.basis.id)
    ).status(),
  ).toBe(403);
  expect(
    (
      await alice.context.request.get(customer + base + "/" + original.basis.id)
    ).status(),
  ).toBe(403);
  expect(
    (
      await reviewer.context.request.post(staff + base, {
        data: body,
        headers: { Origin: staff, "Idempotency-Key": crypto.randomUUID() },
      })
    ).status(),
  ).toBe(403);
  expect(
    (
      await send(
        reviewer.context,
        base,
        { ...body, periodStart: yesterday },
        "POST",
        key,
        true,
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await send(
        reviewer.context,
        base,
        { ...body, closingBooked: "999" },
        "POST",
        crypto.randomUUID(),
        true,
      )
    ).status(),
  ).toBe(422);
  expect(
    (
      await reviewer.context.request.get(
        staff + base + "/" + original.basis.id + "/items?limit=0",
      )
    ).status(),
  ).toBe(422);
  expect(
    (
      await reviewer.context.request.get(
        staff + base + "?policyId=" + input.policy.id + "&accountId=" + account,
      )
    ).status(),
  ).toBe(200);
  expect(native()).toBe(afterFund);
  await reviewer.page.goto(savedUrl);
  await expect(region.getByTestId("basis-closing")).toContainText(
    original.basis.observedClosingBooked + " minor units",
  );
  await approver.page.goto(savedUrl);
  const readOnly = approver.page.getByRole("region", {
    name: "Native ledger basis",
    exact: true,
  });
  await expect(
    readOnly.getByRole("heading", {
      name: "Captured Core observation",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    readOnly.getByRole("button", {
      name: "Capture or retry basis",
      exact: true,
    }),
  ).toHaveCount(0);
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/finance-basis-desktop.png",
    fullPage: true,
  });
  await reviewer.page.setViewportSize({ width: 390, height: 844 });
  await accessible(reviewer.page);
  expect(
    await reviewer.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await region.screenshot({ path: ".local/finance-basis-mobile.png" });
  // A genuine capture can commit while its browser response is delayed. Leaving
  // the policy workspace must not be undone when that old response arrives.
  await region.getByLabel("Basis account ID", { exact: true }).fill(account);
  await region.getByLabel("Basis period start", { exact: true }).fill(today);
  let releaseCapture!: () => void, captureCommitted!: () => void;
  const release = new Promise<void>((resolve) => {
    releaseCapture = resolve;
  });
  const committed = new Promise<void>((resolve) => {
    captureCommitted = resolve;
  });
  await reviewer.page.route("**" + base, async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    const actual = await route.fetch();
    expect(actual.status()).toBe(201);
    captureCommitted();
    await release;
    await route.fulfill({ response: actual });
  });
  await region
    .getByRole("button", { name: "Capture fresh observation", exact: true })
    .click();
  await committed;
  await reviewer.page.evaluate(() => {
    location.hash = "/finance";
  });
  await expect(
    reviewer.page.getByRole("heading", {
      name: "Account for every batch.",
      exact: true,
    }),
  ).toBeVisible();
  const delivered = reviewer.page.waitForEvent(
    "requestfinished",
    (r) => r.url().endsWith(base) && r.method() === "POST",
  );
  releaseCapture();
  await delivered;
  await reviewer.page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await expect(reviewer.page).toHaveURL(staff + "/#/finance");
  expect(native()).toBe(afterFund);

  expect(errors).toEqual([]);
  await reviewer.context.close();
  await approver.context.close();
  await security.context.close();
  await alice.context.close();
});
