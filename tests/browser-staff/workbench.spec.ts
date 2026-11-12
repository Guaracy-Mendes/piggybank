import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type Page,
} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
const base = "http://127.0.0.1:4301";
async function login(browser: Browser, username: string, customer = false) {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(customer ? "http://127.0.0.1:4300" : base);
  await page
    .getByRole(customer ? "button" : "link", {
      name: customer ? /Sign in/ : /Sign in to operations/,
    })
    .click();
  await page.locator("#username").fill(username);
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL((customer ? "http://127.0.0.1:4300" : base) + "/**");
  return { context, page };
}
async function mutate(
  context: BrowserContext,
  path: string,
  body?: unknown,
  customer = false,
  method = "POST",
  key?: string,
) {
  const csrf = (await context.cookies()).find(
    (c) => c.name === "pb_csrf_" + (customer ? "customer" : "workforce"),
  )!.value;
  return context.request.fetch(
    (customer ? "http://127.0.0.1:4300" : base) + path,
    {
      method,
      data: body,
      headers: {
        Origin: customer ? "http://127.0.0.1:4300" : base,
        "X-CSRF-Token": csrf,
        ...(key ? { "Idempotency-Key": key } : {}),
      },
    },
  );
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
test("workforce review, exact publication and entitlement administration require independent current authority", async ({
  browser,
}) => {
  const reviewer = await login(browser, "reviewer");
  const approver = await login(browser, "approver");
  const security = await login(browser, "security");
  await expect(
    reviewer.page.getByRole("heading", {
      name: "A clear view of what needs you.",
    }),
  ).toBeVisible();
  await accessible(reviewer.page);
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (query: string) =>
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
        query,
      ],
      { encoding: "utf8" },
    ).trim();
  // Seed an original control request. The running worker obtains and verifies the signed REVIEW result from the real local provider process.
  const request = crypto.randomUUID(),
    resource = crypto.randomUUID();
  sql(
    `BEGIN; INSERT INTO simulator.scenarios VALUES('${request}','REVIEW'); INSERT INTO controls.check_requests(id,resource_id,kind,input_digest,policy_version,generation) VALUES('${request}','${resource}','IDENTITY','${"a".repeat(64)}','synthetic-controls-v1',1); INSERT INTO controls.check_heads VALUES('${resource}','IDENTITY','${request}'); COMMIT;`,
  );
  let caseId = "";
  await expect
    .poll(() => {
      caseId = sql(
        `SELECT id FROM controls.cases WHERE origin_id='${request}'`,
      );
      return caseId;
    })
    .toMatch(/^[a-f0-9-]{36}$/);
  await reviewer.page.goto(base + "/#/cases/" + caseId);
  await expect(
    reviewer.page.getByText("IDENTITY · Original decision: REVIEW"),
  ).toBeVisible();
  expect(
    (
      await security.context.request.get(
        base + "/api/v1/staff/cases/" + caseId + "/context",
      )
    ).status(),
  ).toBe(403);
  await reviewer.page
    .getByRole("button", { name: "Claim case", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("Case assigned to you.", { exact: true }),
  ).toBeVisible();
  await reviewer.page
    .getByRole("combobox", { name: "Disposition", exact: true })
    .selectOption("CLEAR_CONTROL");
  await reviewer.page
    .getByRole("button", { name: "Propose disposition", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("Disposition saved for independent approval.", {
      exact: true,
    }),
  ).toBeVisible();
  const ctx = await reviewer.context.request.get(
    base + "/api/v1/staff/cases/" + caseId + "/context",
  );
  const proposal = (await ctx.json()).proposals[0].id;
  expect(
    (
      await mutate(reviewer.context, `/api/v1/staff/cases/${caseId}/approval`, {
        proposalId: proposal,
      })
    ).status(),
  ).toBe(403);
  await approver.page.goto(base + "/#/cases/" + caseId);
  await approver.page
    .getByRole("button", { name: "Approve disposition", exact: true })
    .click();
  await expect(
    approver.page.getByText("CLOSED", { exact: true }),
  ).toBeVisible();
  expect(
    sql(`SELECT state FROM controls.check_requests WHERE id='${request}'`),
  ).toBe("MANUAL_ALLOW");
  // Real customer account + support object; case scope grants only the assigned reviewer access to account context.
  const customer = await login(browser, "alice", true);
  const customerBase = "http://127.0.0.1:4300";
  const products = await (
    await customer.context.request.get(customerBase + "/api/v1/products")
  ).json();
  const product = products[0];
  const created = await mutate(
    customer.context,
    "/api/v1/applications",
    { productVersionId: product.id, marketing: false },
    true,
    "POST",
    crypto.randomUUID(),
  );
  expect(created.status()).toBe(201);
  let app = await created.json();
  const appPath = "/api/v1/applications/" + app.id;
  app = await (
    await mutate(
      customer.context,
      appPath + "/profile",
      {
        version: app.version,
        profile: {
          fullName: "Synthetic Staff Test Customer",
          dateOfBirth: "1990-01-01",
          email: "staff-test@example.invalid",
          residency: "SYNTHETIC",
          consentVersion: "synthetic-consent-v1",
          marketing: false,
        },
      },
      true,
      "PUT",
    )
  ).json();
  expect(
    (
      await mutate(
        customer.context,
        appPath + "/evidence",
        { content: "Synthetic evidence for staff browser workflow." },
        true,
      )
    ).status(),
  ).toBe(200);
  const terms = await (
    await customer.context.request.get(
      customerBase + `/api/v1/products/${product.id}/terms`,
    )
  ).json();
  expect(
    (
      await mutate(
        customer.context,
        appPath + "/terms",
        { termsId: terms.id },
        true,
      )
    ).status(),
  ).toBe(200);
  expect(
    (
      await mutate(
        customer.context,
        appPath + "/submit",
        undefined,
        true,
        "POST",
        crypto.randomUUID(),
      )
    ).status(),
  ).toBe(202);
  await expect
    .poll(
      async () => {
        app = await (
          await customer.context.request.get(customerBase + appPath)
        ).json();
        return app.state;
      },
      { timeout: 30000 },
    )
    .toBe("ACTIVE");
  const service = await mutate(
    customer.context,
    `/api/v1/accounts/${app.accountId}/service-requests`,
    undefined,
    true,
    "POST",
    crypto.randomUUID(),
  );
  expect(service.status()).toBe(201);
  const supportId = (await service.json()).caseId;
  await reviewer.page.goto(base + "/#/cases/" + supportId);
  await reviewer.page
    .getByRole("button", { name: "Claim case", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Read account context", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("TST 0.00", { exact: true }),
  ).toBeVisible();
  expect(
    (
      await approver.context.request.get(
        base + `/api/v1/staff/cases/${supportId}/account`,
      )
    ).status(),
  ).toBe(403);
  await reviewer.page
    .getByRole("button", { name: "Request information", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("Customer information requested.", { exact: true }),
  ).toBeVisible();
  const supplied = await mutate(
    customer.context,
    `/api/v1/cases/${supportId}/information`,
    {
      requestId: crypto.randomUUID(),
      content: "Synthetic additional evidence for authorized support.",
    },
    true,
  );
  expect(supplied.status()).toBe(200);
  const evidenceId = (await supplied.json()).evidenceId;
  await reviewer.page
    .getByRole("button", { name: "Refresh queues", exact: true })
    .click();
  await expect(
    reviewer.page.getByRole("link", { name: "Download evidence", exact: true }),
  ).toBeVisible();
  expect(
    (
      await reviewer.context.request.get(
        base + `/api/v1/staff/cases/${supportId}/evidence/${evidenceId}`,
      )
    ).status(),
  ).toBe(200);
  expect(
    (
      await security.context.request.get(
        base + `/api/v1/staff/cases/${supportId}/evidence/${evidenceId}`,
      )
    ).status(),
  ).toBe(403);
  await accessible(reviewer.page);
  await reviewer.page.screenshot({
    path: ".local/staff-case.png",
    fullPage: true,
  });
  await reviewer.page
    .getByRole("combobox", { name: "Disposition", exact: true })
    .selectOption("RESOLVE_SERVICE");
  await reviewer.page
    .getByRole("button", { name: "Propose disposition", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("Disposition saved for independent approval.", {
      exact: true,
    }),
  ).toBeVisible();
  await approver.page.goto(base + "/#/cases/" + supportId);
  await approver.page
    .getByRole("button", { name: "Approve disposition", exact: true })
    .click();
  await expect(
    approver.page.getByText("CLOSED", { exact: true }),
  ).toBeVisible();
  // Exact product/pricing is reviewed in another session and only then published.
  await reviewer.page
    .getByRole("link", { name: "Products & pricing", exact: true })
    .click();
  await reviewer.page
    .getByLabel("Name or title", { exact: true })
    .fill("Synthetic staff browser product");
  await reviewer.page
    .getByLabel("Product code", { exact: true })
    .fill("STAFF_BROWSER");
  await reviewer.page
    .getByLabel("Flat fee · TST minor units", { exact: true })
    .fill("7");
  await reviewer.page
    .getByLabel("Variable fee · basis points", { exact: true })
    .fill("25");
  await reviewer.page
    .getByRole("button", { name: "Save configuration proposal", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("Immutable draft saved for independent review.", {
      exact: true,
    }),
  ).toBeVisible();
  const configs = await (
    await reviewer.context.request.get(base + "/api/v1/staff/configuration")
  ).json();
  const config = configs.items[0];
  expect(
    (
      await mutate(
        reviewer.context,
        `/api/v1/staff/configuration/${config.proposalId}/approval`,
      )
    ).status(),
  ).toBe(403);
  await approver.page
    .getByRole("link", { name: "Products & pricing", exact: true })
    .click();
  await expect(
    approver.page.getByText("TST 0.07 + 25 bps", { exact: true }),
  ).toBeVisible();
  await approver.page
    .getByRole("button", { name: "Approve configuration", exact: true })
    .click();
  await expect(
    approver.page.getByText("Independent configuration approval recorded.", {
      exact: true,
    }),
  ).toBeVisible();
  await reviewer.page
    .getByRole("button", { name: "Refresh proposals", exact: true })
    .click();
  await reviewer.page
    .getByRole("button", { name: "Publish approved version", exact: true })
    .click();
  await expect(
    reviewer.page.getByText("PUBLISHED", { exact: true }),
  ).toBeVisible();
  expect(
    sql(
      `SELECT document->'flatFee'->>'minorUnits' FROM core.product_versions WHERE id='${config.resourceId}'`,
    ),
  ).toBe("7");
  await accessible(reviewer.page);
  // Explicit role proposals are independently approved and revoked through audited APIs.
  const target = (
    await (await reviewer.context.request.get(base + "/api/v1/staff/me")).json()
  ).personId;
  await security.page
    .getByRole("link", { name: "Access register", exact: true })
    .click();
  await security.page
    .getByLabel("Canonical person ID", { exact: true })
    .fill(target);
  await security.page
    .getByRole("combobox", { name: "Role", exact: true })
    .selectOption("data_steward");
  await security.page
    .getByRole("button", { name: "Propose role grant", exact: true })
    .click();
  await expect(
    security.page.getByText("Role proposal saved for an independent person.", {
      exact: true,
    }),
  ).toBeVisible();
  const register = await (
    await security.context.request.get(base + "/api/v1/staff/roles")
  ).json();
  const role = register.proposals[0];
  expect(
    (
      await mutate(security.context, `/api/v1/staff/roles/${role.id}/approval`)
    ).status(),
  ).toBe(403);
  await approver.page
    .getByRole("link", { name: "Access register", exact: true })
    .click();
  await approver.page
    .getByRole("button", { name: "Approve role grant", exact: true })
    .click();
  await expect(
    approver.page.getByText("Independent role approval recorded.", {
      exact: true,
    }),
  ).toBeVisible();
  await security.page
    .getByRole("button", { name: "Refresh access register", exact: true })
    .click();
  const row = security.page
    .locator(".register-row")
    .filter({ has: security.page.getByText("data_steward", { exact: true }) })
    .filter({
      has: security.page.getByRole("button", {
        name: "Revoke grant",
        exact: true,
      }),
    });
  await row.getByRole("button", { name: "Revoke grant", exact: true }).click();
  await expect(
    security.page
      .locator(".register-row")
      .filter({ has: security.page.getByText("data_steward", { exact: true }) })
      .filter({ has: security.page.getByText("Revoked", { exact: true }) }),
  ).toBeVisible();
  await accessible(security.page);
  // Revoking queue membership immediately blocks an already authenticated reviewer.
  sql(
    `UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id='${target}' AND queue='support'`,
  );
  expect(
    (
      await reviewer.context.request.get(
        base + `/api/v1/staff/cases/${supportId}/context`,
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await customer.context.request.get(customerBase + "/api/v1/staff/cases")
    ).status(),
  ).toBe(403);
  expect(
    (await security.context.request.get(base + "/api/v1/accounts")).status(),
  ).toBe(403);
  expect(
    (
      await reviewer.context.request.get(
        "http://127.0.0.1:4300/api/v1/%73taff/cases",
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await reviewer.context.request.get(
        "http://127.0.0.1:4300/auth/%77orkforce/me",
      )
    ).status(),
  ).toBe(403);
  expect(
    (
      await security.context.request.get(base + "/api/v1/staff/%2e%2e/accounts")
    ).status(),
  ).toBe(403);
  await Promise.all([
    reviewer.context.close(),
    approver.context.close(),
    security.context.close(),
    customer.context.close(),
  ]);
});
