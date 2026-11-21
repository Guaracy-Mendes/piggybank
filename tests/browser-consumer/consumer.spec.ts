import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type Locator,
  type Page,
} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
const offset = Number(process.env.BANK_BROWSER_PORT_OFFSET ?? 0);
const customer = `http://127.0.0.1:${4300 + offset}`,
  staff = `http://127.0.0.1:${4301 + offset}`;
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

async function approveConsumer(page: Page, user = "alice") {
  await page
    .getByRole("link", { name: /Confirm identity to authorize/ })
    .click();
  await expect(page.locator("#password")).toBeVisible();
  const username = page.locator("#username");
  if ((await username.isVisible()) && (await username.isEditable()))
    await username.fill(user);
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL(customer + "/**");
  await page
    .getByRole("button", { name: "Submit approved instruction", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Instruction receipt", exact: true }),
  ).toBeVisible();
}
test("official consumer workspace posts vault savings and issuer-approved investment with responsive controls", async ({
  browser,
}) => {
  const alice = await login(browser, "alice"),
    page = alice.page,
    errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const account = await open(alice.context, "UI Consumer");
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(fixture.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  execFileSync("podman", [
    "exec",
    "piggybank-postgres-1",
    "psql",
    "-U",
    "dev_admin",
    "-d",
    fixture.database,
    "-v",
    "ON_ERROR_STOP=1",
    "-c",
    `SELECT * FROM core.fund_synthetic('${crypto.randomUUID()}','${account}','4f1a780b-3e31-48ef-a294-000000000003',100000)`,
  ]);
  await page.goto(customer + "/#/home");
  await page.reload();
  await expect(
    page.getByRole("heading", { name: /Good to see you/ }),
  ).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/save/new");
  await page.getByLabel("Vault name", { exact: true }).fill("Rainy Day");
  await page.getByLabel("Target amount (TST)").fill("500.00");
  await page.getByRole("button", { name: "Create vault", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "✦ Rainy Day", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Amount (TST)", { exact: true }).fill("50.00");
  await page.getByRole("button", { name: "Submit vault transfer" }).click();
  await expect(
    page.getByText("TST 50.00", { exact: true }).first(),
  ).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/spend");
  await page.getByRole("button", { name: "Budgets", exact: true }).click();
  await page.getByLabel("Monthly limit (TST)").fill("200.00");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("TST 0.00 / TST 200.00")).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/invest");
  await page
    .getByRole("checkbox", { name: /I accept the synthetic brokerage terms/ })
    .check();
  await page.getByRole("button", { name: "Open simulated brokerage" }).click();
  await page.getByLabel("Transfer amount (TST)").fill("200.00");
  await page.getByRole("button", { name: "Continue transfer" }).click();
  await expect(page.getByText(/Buying power: TST 200.00/)).toBeVisible();
  await page.goto(customer + "/#/invest/trade/AAPL");
  await page.getByLabel("Trade amount (TST)").fill("25.00");
  await page.getByRole("button", { name: "Review order", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Review & authorize", exact: true }),
  ).toBeVisible();
  await accessible(page);
  await approveConsumer(page);
  await expect(
    page.getByRole("heading", { name: "FILLED", exact: true }),
  ).toBeVisible();
  const nativeWealth = await (
    await alice.context.request.get(customer + "/api/v1/investments")
  ).json();
  const nativeBatch = nativeWealth.orders.find(
    (o: { symbol: string; state: string }) =>
      o.symbol === "AAPL" && o.state === "FILLED",
  ).fill.batchId as string;
  expect(nativeBatch).toMatch(/^[a-f0-9-]{36}$/);
  await expect
    .poll(
      () =>
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
            `SELECT state FROM finance.gl_exports WHERE source_batch_id='${nativeBatch}'`,
          ],
          { encoding: "utf8" },
        ).trim(),
      { timeout: 15000 },
    )
    .toBe("BOOKED");
  await page.goto(customer + "/#/invest");
  await expect(page.getByRole("cell", { name: "Apple Inc." })).toBeVisible();
  await accessible(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: ".local/official-investments-desktop.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("navigation", { name: "Mobile navigation" }),
  ).toBeVisible();
  await accessible(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".local/official-investments-mobile.png",
    fullPage: true,
  });
  await page
    .getByRole("navigation", { name: "Mobile navigation" })
    .getByRole("link", { name: "Profile", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Profile & Preferences" }),
  ).toBeVisible();
  await accessible(page);
  expect(errors).toEqual([]);
  await alice.context.close();
});

test("wallet funds, card freeze and round-ups, token revocation and native PDF survive real issuer approval", async ({
  browser,
}) => {
  const bob = await login(browser, "bob"),
    page = bob.page,
    account = await open(bob.context, "UI Wallet");
  await page.reload();
  await page.goto(customer + "/#/wallet");
  await page.getByLabel("Institution name").fill("Local Evaluation Bank");
  await page.getByLabel("Account label", { exact: true }).fill("Test funds");
  await page.getByLabel("Last four digits").fill("1234");
  await page.getByRole("button", { name: "Connect synthetic account" }).click();
  await page.getByLabel("Add test money (TST)").fill("500.00");
  await page.getByRole("button", { name: "Review test deposit" }).click();
  await approveConsumer(page, "bob");
  await expect(
    page.getByRole("heading", { name: "POSTED", exact: true }),
  ).toBeVisible();
  await page.goto(customer + "/#/wallet");
  await page.getByLabel("Card label").fill("Everyday test");
  await page.getByRole("button", { name: "Create synthetic card" }).click();
  await expect(
    page.getByText("Everyday test", { exact: true }).last(),
  ).toBeVisible();
  await page.getByRole("button", { name: "Freeze card", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Review synthetic purchase" }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Unfreeze card", exact: true })
    .click();
  const vault = await send(
    bob.context,
    "/api/v1/consumer/commands",
    {
      action: "VAULT_CREATE",
      input: { name: "Round-up goal", targetMinor: "50000" },
    },
    "POST",
    crypto.randomUUID(),
  );
  expect(vault.status()).toBe(200);
  await page.goto(customer + "/#/save");
  await page.getByRole("button", { name: "Review round-up settings" }).click();
  await approveConsumer(page, "bob");
  // An approved merchant saving rule (UI-13) saves a fixed amount per matching purchase.
  await page.goto(customer + "/#/save");
  await page.getByLabel("Merchant or category name").fill("Local Coffee");
  await page.getByLabel("Save per purchase (TST)").fill("1.00");
  await page.getByLabel("Weekly cap (TST)").last().fill("2.00");
  await page.getByRole("button", { name: "Review saving rule" }).click();
  await approveConsumer(page, "bob");
  await page.goto(customer + "/#/wallet");
  await page.getByLabel("Merchant", { exact: true }).fill("Local Coffee");
  await page.getByLabel("Purchase amount (TST)").fill("7.50");
  await page.getByRole("button", { name: "Review synthetic purchase" }).click();
  await approveConsumer(page, "bob");
  const snap = await (
    await bob.context.request.get(customer + "/api/v1/consumer")
  ).json();
  expect(snap.roundups.savedMinor).toBe("50");
  expect(snap.saveRules[0].savedLifetimeMinor).toBe("100");
  expect(snap.analytics.spentMinor).toBe("750");
  await page.goto(customer + "/#/preferences");
  await page.getByLabel("Token label").fill("Local CX access");
  await page.getByRole("button", { name: "Review scoped API token" }).click();
  await approveConsumer(page, "bob");
  const token = (await page
    .locator(".ui-code")
    .filter({ hasText: "Copy once:" })
    .textContent())!
    .replace("Copy once:", "")
    .trim();
  expect(
    (
      await bob.context.request.get(
        customer + "/api/v1/consumer/integrations/accounts:read",
        { headers: { Authorization: "Bearer " + token } },
      )
    ).status(),
  ).toBe(200);
  expect(
    (
      await bob.context.request.get(
        customer + "/api/v1/consumer/integrations/portfolio:read",
        { headers: { Authorization: "Bearer " + token } },
      )
    ).status(),
  ).toBe(401);
  await page.goto(customer + "/#/preferences");
  await page.getByRole("button", { name: "Revoke token", exact: true }).click();
  await expect(page.getByText("Revoked", { exact: true })).toBeVisible();
  expect(
    (
      await bob.context.request.get(
        customer + "/api/v1/consumer/integrations/accounts:read",
        { headers: { Authorization: "Bearer " + token } },
      )
    ).status(),
  ).toBe(401);
  const from = new Date().toISOString().slice(0, 7) + "-01",
    to = new Date(Date.now() + 86400000).toISOString().slice(0, 10);
  await page.getByLabel("From (inclusive)").fill(from);
  await page.getByLabel("To (exclusive)").fill(to);
  await page.getByRole("button", { name: "Create activity report" }).click();
  await expect(page.getByRole("link", { name: "Download PDF" })).toBeVisible();
  const pdfPath = await page
    .getByRole("link", { name: "Download PDF" })
    .getAttribute("href");
  const pdf = await bob.context.request.get(customer + pdfPath);
  expect(pdf.status()).toBe(200);
  expect(pdf.headers()["content-type"]).toBe("application/pdf");
  expect((await pdf.body()).toString("ascii", 0, 8)).toBe("%PDF-1.4");
  await page.getByLabel("Appearance").selectOption("dark");
  await page.getByRole("button", { name: "Save", exact: true }).first().click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await accessible(page);
  await page.goto(customer + "/#/notifications");
  await page.getByRole("button", { name: "Mark all as read" }).click();
  await page.getByRole("checkbox", { name: "Unread only" }).check();
  await expect(page.getByText("You’re all caught up.")).toBeVisible();
  await bob.context.close();
});

test("recurring edits retain issuer-approved versions, execution receipts and portfolio allocation controls", async ({
  browser,
}) => {
  const alice = await login(browser, "alice"),
    page = alice.page;
  await page.goto(customer + "/#/save/new");
  await page.getByLabel("Vault name", { exact: true }).fill("Steady progress");
  await page.getByLabel("Target amount (TST)").fill("100.00");
  const future = new Date(Date.now() + 14 * 86400000)
      .toISOString()
      .slice(0, 10),
    tomorrow = new Date(Date.now() + 86400000).toISOString().slice(0, 10);
  await page.getByLabel("Target date", { exact: true }).fill(future);
  await expect(page.getByText(/About TST 50.00 per week/)).toBeVisible();
  await page.getByRole("button", { name: "Create vault", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Member contributions", exact: true }),
  ).toBeVisible();
  await page.goto(customer + "/#/invest/automate");
  await page.getByLabel("Instruction kind").selectOption("SAVE");
  await page
    .getByLabel("Destination vault")
    .selectOption({ label: "Steady progress" });
  await page.getByLabel("Amount (TST, minimum 5.00)").fill("5.00");
  await page
    .getByRole("combobox", { name: "Frequency", exact: true })
    .selectOption("MONTHLY");
  await page.getByLabel("First execution date (UTC)").fill(tomorrow);
  await page
    .getByRole("button", { name: "Review recurring instruction", exact: true })
    .click();
  await approveConsumer(page);
  await page.getByRole("link", { name: "Done →", exact: true }).click();
  const card = page.getByRole("article").filter({ hasText: "Steady progress" });
  await card
    .getByRole("button", { name: "Pause schedule", exact: true })
    .click();
  await expect(card.getByText("PAUSED", { exact: true })).toBeVisible();
  await card
    .getByRole("button", { name: "Edit instruction", exact: true })
    .click();
  await page.getByLabel("Amount (TST, minimum 5.00)").fill("10.00");
  await page
    .getByRole("combobox", { name: "Frequency", exact: true })
    .selectOption("WEEKLY");
  await page
    .getByRole("button", { name: "Review updated instruction" })
    .click();
  await expect(page.locator(".ui-review-nested")).toBeVisible();
  await expect(page.getByText("[object Object]", { exact: true })).toHaveCount(
    0,
  );
  await accessible(page);
  await approveConsumer(page);
  await expect(
    page.getByRole("heading", { name: "PAUSED", exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Done →", exact: true }).click();
  const updated = page
    .getByRole("article")
    .filter({ hasText: "Steady progress" });
  await expect(updated.getByText("PAUSED", { exact: true })).toBeVisible();
  await updated
    .getByRole("button", { name: "Resume schedule", exact: true })
    .click();
  await expect(updated.getByText("ACTIVE", { exact: true })).toBeVisible();
  await expect(page.getByText("TST 43.34", { exact: true })).toBeVisible();
  await page
    .getByRole("checkbox", {
      name: "Show previous versions & cancelled schedules",
    })
    .check();
  await expect(page.getByText("REPLACED", { exact: true })).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/invest/automate");
  await page
    .getByRole("combobox", { name: "Asset", exact: true })
    .selectOption("VOO");
  await page.getByLabel("Amount (TST, minimum 5.00)").fill("5.00");
  await page
    .getByRole("button", { name: "Review recurring instruction", exact: true })
    .click();
  await approveConsumer(page);
  await page.getByRole("link", { name: "Done →", exact: true }).click();
  await expect
    .poll(
      async () => {
        const r = await (
          await alice.context.request.get(customer + "/api/v1/investments")
        ).json();
        return r.automationSummary.totalInvestedMinor;
      },
      { timeout: 15000 },
    )
    .toBe("500");
  await page
    .getByRole("article")
    .filter({ hasText: "VOO" })
    .getByText(/Execution receipts/)
    .click();
  await expect(
    page
      .getByRole("article")
      .filter({ hasText: "VOO" })
      .getByText(/Most recent five receipts/),
  ).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/invest/analytics");
  await page.getByRole("button", { name: "Sector", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Sector", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Broad market", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Holdings", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Position performance", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Income outlook", exact: true }),
  ).toBeVisible();
  await accessible(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await accessible(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".local/ui60-analytics-mobile.png",
    fullPage: true,
  });
  await alice.context.close();
});

test("savings milestones show native streaks, monthly net worth, badges and a balance-free share card", async ({
  browser,
}) => {
  const alice = await login(browser, "alice"),
    page = alice.page;
  await page.goto(customer + "/#/home");
  await expect(
    page.getByRole("link", {
      name: /^Current saving streak \d+ days?\. View milestones$/,
    }),
  ).toBeVisible();
  await expect(page.getByText(/^since \d{4}-\d{2}-01 \(UTC\)$/)).toBeVisible();
  await expect(
    page.getByRole("list", { name: "Personal net worth by month" }),
  ).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/save");
  await expect(page.getByText("LIFETIME INTEREST EARNED")).toBeVisible();
  await page.getByRole("link", { name: "View streaks & badges →" }).click();
  await expect(
    page.getByRole("heading", { name: "Milestones", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("img", { name: /^Saving activity: \d+ saving day/ }),
  ).toBeVisible();
  await expect(
    page
      .getByRole("listitem")
      .filter({ hasText: "First deposit" })
      .getByText(/^Unlocked \d{4}-\d{2}-\d{2}$/),
  ).toBeVisible();
  await expect(
    page
      .getByRole("listitem")
      .filter({ hasText: "100 Round-Ups" })
      .getByText(/^Locked · \d+ of 100$/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create milestone card" }).click();
  await expect(
    page.getByRole("img", { name: /^Milestone card: / }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Save image ↓" }),
  ).toHaveAttribute("download", /^piggybank-[a-z0-9_]+\.png$/);
  // Regaining window focus revalidates the session in the background; it must not unmount the workspace and drop
  // page state (the cause of two earlier flakes of this test, where the card vanished before the assertion).
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/auth/customer/me")),
    page.evaluate(() => window.dispatchEvent(new Event("focus"))),
  ]);
  await expect(page.getByText("Opening your secure session…")).toHaveCount(0);
  await expect(
    page.getByRole("img", { name: /^Milestone card: / }),
  ).toBeVisible();
  await accessible(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await accessible(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".local/ui61-milestones-mobile.png",
    fullPage: true,
  });
  await page.goto(customer + "/#/preferences");
  await page.getByLabel("Appearance").selectOption("dark");
  await page.getByRole("button", { name: "Save", exact: true }).first().click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.goto(customer + "/#/save/milestones");
  await expect(
    page.getByRole("heading", { name: "Badges", exact: true }),
  ).toBeVisible();
  await accessible(page);
  await alice.context.close();
});
test("monthly statements are readiness-gated, retrieved once, and held or restricted by independent records staff", async ({
  browser,
}) => {
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (q: string) =>
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
        q,
      ],
      { encoding: "utf8" },
    ).trim();
  const clock = (at: string) =>
    sql(
      `CREATE OR REPLACE FUNCTION core.periodic_clock() RETURNS timestamptz LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ SELECT ${at} $$`,
    );
  const now = new Date(),
    start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1)),
    next = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1)),
    label = start.toLocaleDateString("en", {
      month: "long",
      year: "numeric",
      timeZone: "UTC",
    });
  const alice = await login(browser, "alice"),
    page = alice.page;
  try {
    // While the month is open nothing is published.
    await page.goto(customer + "/#/documents");
    await expect(
      page.getByRole("heading", {
        name: "Statements & documents",
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("Available after the month closes (UTC).").first(),
    ).toBeVisible();
    clock(`'${next.toISOString().slice(0, 10)}T00:00:00Z'::timestamptz`);
    await page.reload();
    const statements = page
      .getByRole("list", { name: "Monthly statements" })
      .first();
    await expect(statements.getByText(label, { exact: true })).toBeVisible();
    // The worker cycle may publish first; creation is idempotent either way.
    const create = page.getByRole("button", {
      name: `Create ${label} statement`,
    });
    if (await create.isVisible()) await create.click();
    await page.getByRole("button", { name: `View ${label}` }).click();
    await expect(
      page.getByRole("heading", { name: `${label} statement`, exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Closing balance", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(/^Content SHA-256 [a-f0-9]{64}$/),
    ).toBeVisible();
    await accessible(page);
    // Single-use, owner-bound references.
    const docs = await (
      await alice.context.request.get(customer + "/api/v1/documents")
    ).json();
    const accountId = docs.accounts[0].accountId as string,
      version = docs.accounts[0].periods.find(
        (p: { periodStart: string }) =>
          p.periodStart === start.toISOString().slice(0, 10),
      ).versions[0];
    expect(BigInt(version.closingMinor)).toBe(
      BigInt(version.openingMinor) + BigInt(version.totals.netMinor),
    );
    const link = await (
      await send(
        alice.context,
        `/api/v1/documents/statements/${version.id}/links`,
      )
    ).json();
    const href = customer + "/api/v1/documents/downloads/" + link.token;
    const first = await alice.context.request.get(href + "?format=pdf");
    expect(first.status()).toBe(200);
    expect(first.headers()["content-type"]).toBe("application/pdf");
    expect((await alice.context.request.get(href)).status()).toBe(410);
    const bob = await login(browser, "bob");
    expect(
      (
        await send(
          bob.context,
          `/api/v1/documents/statements/${version.id}/links`,
        )
      ).status(),
    ).toBe(404);
    await bob.context.close();
    // A verified deletion request.
    await page.getByLabel("Request").selectOption("DELETION");
    await page
      .getByLabel("Details (optional)")
      .fill("Please delete my statements.");
    await page.getByRole("button", { name: "Submit request" }).click();
    await expect(
      page.getByText("Request received.", { exact: false }),
    ).toBeVisible();
    await expect(page.getByText("In review").first()).toBeVisible();
    // Records officer: hold, restricted disposal, request outcome, access evidence.
    const officer = await login(browser, "reviewer", true),
      ops = officer.page;
    await ops.goto(staff + "/#/records/" + accountId);
    await expect(
      ops.getByRole("heading", { name: "Statement periods", exact: true }),
    ).toBeVisible();
    await ops
      .getByLabel("Reason", { exact: true })
      .fill("Synthetic litigation hold");
    await ops.getByLabel("Case or matter reference").fill("CASE-WP25");
    await ops.getByRole("button", { name: "Place legal hold" }).click();
    await expect(
      ops.getByText("Legal hold placed.", { exact: false }),
    ).toBeVisible();
    await ops
      .getByLabel("Disposal reason")
      .first()
      .fill("Customer request review");
    await ops.getByRole("button", { name: "Request disposal" }).first().click();
    await expect(
      ops.getByText(
        /^Disposal restricted: retention until \d{4}-\d{2}-\d{2}\.$/,
      ),
    ).toBeVisible();
    await expect(ops.getByRole("cell", { name: "ALREADY_USED" })).toBeVisible();
    await ops.getByRole("button", { name: "Process request" }).first().click();
    await expect(ops.getByText("Request processed: restricted.")).toBeVisible();
    await accessible(ops);
    // Independent release; the placer has no release control.
    await expect(ops.getByRole("button", { name: "Release hold" })).toHaveCount(
      0,
    );
    const approver = await login(browser, "approver", true);
    await approver.page.goto(staff + "/#/records/" + accountId);
    await approver.page.getByLabel("Release reason").fill("Matter closed");
    await approver.page.getByRole("button", { name: "Release hold" }).click();
    await expect(approver.page.getByText("Legal hold released.")).toBeVisible();
    await officer.context.close();
    await approver.context.close();
    await page.reload();
    await expect(
      page.getByText("Kept: retention period applies").first(),
    ).toBeVisible();
    await page.setViewportSize({ width: 390, height: 844 });
    await accessible(page);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: ".local/ui62-documents-mobile.png",
      fullPage: true,
    });
  } finally {
    clock("clock_timestamp()");
    await alice.context.close();
  }
});
test("operations monitor records a fixed snapshot, owns monitoring alerts with independent disposition and customers see safe messages", async ({
  browser,
}) => {
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  const sql = (q: string) =>
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
        q,
      ],
      { encoding: "utf8" },
    ).trim();
  const alice = await login(browser, "alice"),
    page = alice.page;
  // The statement published in the previous journey produces one safe, deduplicated message.
  await page.goto(customer + "/#/notifications");
  await expect(
    page.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Your statement is ready").first()).toBeVisible({
    timeout: 30000,
  });
  const body = await page.locator(".ui-mini-card").first().innerText();
  expect(body).not.toMatch(/\d+\.\d{2}/);
  const activity = page.getByRole("checkbox", {
    name: /for Payment activity/,
  });
  await expect(activity).toBeChecked();
  await activity.click();
  await expect(activity).not.toBeChecked();
  await accessible(page);
  // Seed a structured-credit pattern as projected durable facts on Alice's account.
  const docs = await (
    await alice.context.request.get(customer + "/api/v1/documents")
  ).json();
  const account = docs.accounts[0].accountId as string;
  sql(`DO $$ DECLARE b uuid; BEGIN FOR i IN 1..3 LOOP b:=gen_random_uuid();
    INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint) VALUES(b,b,'SYNTHETIC_FUNDING','browser-fixture');
    INSERT INTO core.journal_entries VALUES(b,'${account}',9500),(b,'4f1a780b-3e31-48ef-a294-000000000003',-9500);
    INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) VALUES(b,'${account}',b,'{}',gen_random_uuid());
  END LOOP; END $$`);
  await alice.context.close();
  const reviewer = await login(browser, "reviewer", true),
    ops = reviewer.page;
  await ops.goto(staff + "/#/operations");
  await expect(
    ops.getByRole("heading", {
      name: "Monitor backlogs, alerts and data quality.",
    }),
  ).toBeVisible();
  await ops.getByRole("button", { name: "Record operations snapshot" }).click();
  await expect(ops.getByText("Operations snapshot recorded.")).toBeVisible();
  await expect(ops.getByText(/^[a-f0-9]{64}$/)).toBeVisible();
  await expect(ops.getByText(/^Transaction projection:/)).toBeVisible();
  await expect
    .poll(
      async () => {
        await ops.reload();
        await ops
          .getByRole("heading", { name: "Transaction monitoring", exact: true })
          .waitFor();
        return ops.getByText("structured credits", { exact: true }).count();
      },
      { timeout: 30000 },
    )
    .toBe(1);
  // The monitoring alert pages the seeded primary on-call (reviewer), who acknowledges it.
  await expect
    .poll(
      async () => {
        await ops.reload();
        await ops
          .getByRole("heading", { name: "On-call pages", exact: true })
          .waitFor();
        return ops.getByRole("button", { name: "Acknowledge page" }).count();
      },
      { timeout: 30000 },
    )
    .toBeGreaterThan(0);
  await ops.getByRole("button", { name: "Acknowledge page" }).first().click();
  await expect(ops.getByText("Page acknowledged.")).toBeVisible();
  await ops
    .getByLabel("Rationale")
    .fill("Synthetic savings transfers explained");
  await ops.getByRole("button", { name: "Propose disposition" }).click();
  await expect(
    ops.getByText("Disposition proposed for independent approval."),
  ).toBeVisible();
  await expect(
    ops.getByRole("button", { name: "Approve disposition" }),
  ).toHaveCount(0);
  await accessible(ops);
  const approver = await login(browser, "approver", true);
  await approver.page.goto(staff + "/#/operations");
  await approver.page
    .getByRole("button", { name: "Approve disposition" })
    .click();
  await expect(
    approver.page.getByText("Disposition approved; case closed."),
  ).toBeVisible();
  await expect(
    approver.page.getByText("no further action").first(),
  ).toBeVisible();
  await accessible(approver.page);
  await reviewer.context.close();
  await approver.context.close();
});
test("security lifecycle: customer signs out everywhere, staff recovery executes, access is recertified and findings are independently disposed", async ({
  browser,
}) => {
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const sql = (q: string) =>
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
        q,
      ],
      { encoding: "utf8" },
    ).trim();
  // Customer: revoke every session from Profile & security.
  const alice = await login(browser, "alice"),
    page = alice.page;
  await page.goto(customer + "/#/profile");
  await expect(
    page.getByRole("heading", { name: "Lost a phone or shared a device?" }),
  ).toBeVisible();
  await accessible(page);
  await page.getByLabel("I understand every device will be signed out").check();
  await page.getByRole("button", { name: "Sign out everywhere" }).click();
  await expect(
    page.getByRole("button", { name: /Sign in/ }).first(),
  ).toBeVisible({ timeout: 30000 });
  expect(
    (await alice.context.request.get(customer + "/api/v1/accounts")).status(),
  ).toBe(401);
  await alice.context.close();
  // Staff-assisted lost-device recovery for Bob (low risk, verified contact) executes at once.
  const bobPerson = sql(
    "SELECT l.person_id FROM controls.person_links l JOIN core.parties p ON p.issuer=l.issuer AND p.subject=l.subject ORDER BY p.created_at DESC LIMIT 1",
  );
  const support = await login(browser, "reviewer", true),
    ops = support.page;
  await ops.goto(staff + "/#/security");
  await expect(
    ops.getByRole("heading", {
      name: "Recovery, access reviews and findings.",
    }),
  ).toBeVisible();
  await ops.getByLabel("Customer person identifier").fill(bobPerson);
  await ops
    .getByLabel("Recovery reason")
    .fill("Customer reported a lost phone on the verified contact");
  await ops.getByRole("button", { name: "Request recovery" }).click();
  await expect(
    ops.getByText(/Recovery (executed|needs independent security approval)/),
  ).toBeVisible({ timeout: 30000 });
  await accessible(ops);
  // Security admin opens a recertification and records a finding; an approver decides both.
  const admin = await login(browser, "security", true);
  await admin.page.goto(staff + "/#/security");
  await admin.page.getByRole("button", { name: "Open access review" }).click();
  await expect(admin.page.getByText("Access review opened.")).toBeVisible();
  await admin.page.getByLabel("Reference (SF-000)").fill("SF-101");
  await admin.page
    .getByLabel("Finding title")
    .fill("Synthetic provider key is a fixed local constant");
  await admin.page.getByLabel("Area", { exact: true }).fill("simulator");
  await admin.page
    .getByLabel("Description")
    .fill(
      "Local-only synthetic HMAC key; production requires managed secrets.",
    );
  await admin.page.getByRole("button", { name: "Record finding" }).click();
  await expect(admin.page.getByText("Finding recorded.")).toBeVisible();
  await accessible(admin.page);
  const approver = await login(browser, "approver", true),
    ap = approver.page;
  await ap.goto(staff + "/#/security");
  await ap
    .getByRole("button", { name: /^Keep / })
    .first()
    .click();
  await expect(ap.getByText("Access kept.")).toBeVisible();
  await expect(
    ap.getByText("Another reviewer must decide").first(),
  ).toBeVisible();
  await ap.getByLabel("Outcome for SF-101").selectOption("ACCEPTED_RISK");
  await ap
    .getByLabel("Disposition for SF-101")
    .fill("Accepted for the synthetic local profile only");
  await ap.getByRole("button", { name: "Record disposition" }).click();
  await expect(ap.getByText("Finding disposed.")).toBeVisible();
  await expect(ap.getByText(/medium · accepted risk/)).toBeVisible();
  await accessible(ap);
  for (const c of [support, admin, approver]) await c.context.close();
});

// Earlier journeys can leave a customer with open risk signals that send a new opening to control review, so the
// partner journey reuses an active account when one exists.
async function activeAccount(context: BrowserContext, label: string) {
  const accounts = (await (
    await context.request.get(customer + "/api/v1/accounts")
  ).json()) as { id: string; state: string }[];
  return accounts.find((a) => a.state === "ACTIVE")?.id ?? open(context, label);
}

test("partner consent and account sharing: maker/checker registry, customer authorization and disconnection, delegate access", async ({
  browser,
}) => {
  test.setTimeout(300_000);
  const api = `http://127.0.0.1:${4180 + offset}`;
  // Staff: a security administrator proposes the partner; a different person approves it.
  const admin = await login(browser, "security", true);
  await admin.page.goto(staff + "/#/partners");
  await expect(
    admin.page.getByRole("heading", { name: "Partners & consent" }),
  ).toBeVisible();
  await admin.page.getByLabel("Client ID").fill("partner-budget");
  await admin.page.getByLabel("Display name").fill("Synthetic Budget App");
  await admin.page.getByLabel("payments:initiate").check();
  await admin.page.getByRole("button", { name: "Propose client" }).click();
  await expect(
    admin.page.getByText(
      /partner-budget proposed\. A second person must approve it\./,
    ),
  ).toBeVisible();
  await accessible(admin.page);
  const approver = await login(browser, "approver", true);
  await approver.page.goto(staff + "/#/partners");
  await approver.page
    .getByLabel("Reason for partner-budget")
    .fill("Synthetic contract reviewed");
  await approver.page.getByRole("button", { name: "Approve" }).click();
  await expect(
    approver.page.getByText("partner-budget: approve recorded."),
  ).toBeVisible();
  // The partner asks for consent with a real client-credentials token.
  const token = (
    await (
      await fetch(
        "http://127.0.0.1:18280/realms/partners/protocol/openid-connect/token",
        {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: "grant_type=client_credentials&client_id=partner-budget&client_secret=synthetic-partner-budget-only",
        },
      )
    ).json()
  ).access_token as string;
  const partner = (path: string, consent?: string, init: RequestInit = {}) =>
    fetch(api + path, {
      ...init,
      headers: {
        Authorization: "Bearer " + token,
        "Piggybank-Api-Version": "2026-11-01",
        ...(consent ? { "Consent-Id": consent } : {}),
        ...(init.headers ?? {}),
      },
    });
  const requested = await partner("/partner/v1/consents", undefined, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Idempotency-Key": crypto.randomUUID(),
    },
    body: JSON.stringify({ scopes: ["accounts:read"], durationDays: 30 }),
  });
  expect(requested.status).toBe(201);
  const consent = (await requested.json()).consentId as string;
  // Customer: authorizes from the bank link with a chosen account, then disconnects.
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Partneralice");
  // Arriving from the partner's link is a full page load (the account opened above must be listed).
  await page.goto(customer + "/#/connected/" + consent);
  await page.reload();
  await expect(
    page.getByRole("heading", {
      name: "Synthetic Budget App would like to connect to your accounts",
    }),
  ).toBeVisible();
  await expect(page.getByText("See balances and transactions")).toBeVisible();
  await accessible(page);
  await page.getByRole("checkbox").first().check();
  await page.getByRole("button", { name: "Allow access" }).click();
  await expect(
    page.getByText("Synthetic Budget App is now connected."),
  ).toBeVisible();
  const accounts = await partner("/partner/v1/accounts", consent);
  expect(accounts.status).toBe(200);
  expect(
    (await accounts.json()).accounts.map(
      (a: { accountId: string }) => a.accountId,
    ),
  ).toEqual([account]);
  await page
    .getByRole("button", { name: "Disconnect Synthetic Budget App" })
    .click();
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(
    page.getByText("Synthetic Budget App can no longer access your accounts."),
  ).toBeVisible();
  expect((await partner("/partner/v1/accounts", consent)).status).toBe(403);
  await accessible(page);
  // Sharing: Bob gives Alice his reference; Alice shares view-only access; Bob sees it.
  const bob = await login(browser, "bob");
  await activeAccount(bob.context, "Partnerbob");
  await bob.page.goto(customer + "/#/sharing");
  await bob.page.getByRole("button", { name: "Show my reference" }).click();
  const reference = (
    await bob.page.getByTestId("delegate-reference").innerText()
  ).trim();
  expect(reference).toMatch(/^DLG-[A-Z2-9]{8}$/);
  await page.goto(customer + "/#/sharing");
  await page.reload();
  await page.getByLabel("Account to share").selectOption(account);
  await page.getByLabel("Their reference").fill(reference);
  await page.getByRole("button", { name: "Share access" }).click();
  await expect(
    page.getByText("Access shared. They can use it straight away."),
  ).toBeVisible();
  await bob.page.reload();
  await expect(
    bob.page.getByRole("button", { name: "Give up access" }),
  ).toBeVisible();
  expect(
    (
      await bob.context.request.get(
        customer + `/api/v1/accounts/${account}/balance`,
      )
    ).status(),
  ).toBe(200);
  await accessible(bob.page);
  await page.reload();
  await page.getByRole("button", { name: "Stop sharing" }).click();
  await expect(page.getByText("Sharing stopped.")).toBeVisible();
  expect(
    (
      await bob.context.request.get(
        customer + `/api/v1/accounts/${account}/balance`,
      )
    ).status(),
  ).toBe(404);
  for (const c of [admin, approver, alice, bob]) await c.context.close();
});

test("assisted call center session: customer authorizes in the app, agent reads and freezes, customer ends it", async ({
  browser,
}) => {
  test.setTimeout(240_000);
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const sql = (q: string) =>
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
        q,
      ],
      { encoding: "utf8" },
    ).trim();
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Assistedalice");
  const person = sql(
    `SELECT person_id FROM core.account_relationships WHERE account_id='${account}' AND kind='OWNER'`,
  );
  const agent = await login(browser, "reviewer", true),
    ops = agent.page;
  await ops.goto(staff + "/#/assisted");
  await expect(
    ops.getByRole("heading", { name: "Assisted service" }),
  ).toBeVisible();
  await ops.getByLabel("Channel").selectOption("CALL_CENTER");
  await ops
    .getByLabel("How you verified the customer")
    .selectOption("VERIFIED_CONTACT_CALLBACK");
  await ops.getByLabel("Customer person identifier").fill(person);
  await ops.getByLabel("Purpose").fill("Caller reports a lost card");
  await ops.getByRole("button", { name: "Open session" }).click();
  await expect(
    ops.getByText(
      "Session opened. Ask the customer to authorize it in their app.",
    ),
  ).toBeVisible();
  await accessible(ops);
  await page.goto(customer + "/#/sharing");
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Assistance requests" }),
  ).toBeVisible();
  await expect(
    page.getByText("They can never make payments for you.", { exact: false }),
  ).toBeVisible();
  await accessible(page);
  await page.getByRole("button", { name: "Allow assistance" }).click();
  await expect(
    page.getByText("You allowed our call center to help with your accounts."),
  ).toBeVisible();
  await ops.reload();
  await ops.getByRole("button", { name: "View accounts" }).first().click();
  await ops
    .getByRole("button", { name: /^Freeze account / })
    .first()
    .click();
  await expect(
    ops.getByText("Protective freeze applied. Outgoing payments are blocked."),
  ).toBeVisible();
  expect(
    sql(`SELECT debit_blocked FROM core.ledger_accounts WHERE id='${account}'`),
  ).toBe("t");
  await page.reload();
  await page.getByRole("button", { name: "End assistance" }).click();
  await expect(page.getByText("Assistance ended.")).toBeVisible();
  await ops.reload();
  await expect(ops.getByText("closed").first()).toBeVisible();
  await accessible(ops);
  // Leave the shared customer unfrozen for later journeys (staff support lifts the protective restriction).
  sql(
    `UPDATE core.ledger_accounts SET debit_blocked=false WHERE id='${account}'`,
  );
  for (const c of [alice, agent]) await c.context.close();
});

test("ATM access: customer links a card token, signed network withdrawal holds and posts once, staff see the channel and the customer blocks the card", async ({
  browser,
}) => {
  test.setTimeout(240_000);
  const { createHmac } = await import("node:crypto");
  const api = `http://127.0.0.1:${4180 + offset}`;
  const atm = async (document: Record<string, unknown>) => {
    const body = JSON.stringify(document);
    const response = await fetch(api + "/channels/atm/v1/messages", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Piggybank-Atm-Signature": createHmac(
          "sha256",
          "synthetic-atm-network-key-local-only",
        )
          .update(body)
          .digest("hex"),
      },
      body,
    });
    expect(response.status).toBe(200);
    return response.json();
  };
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Atmalice");
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "5000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
    },
  );
  await page.reload();
  await page
    .getByRole("link", { name: "Profile & security", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "ATM access" })).toBeVisible();
  await page.getByLabel("Account for ATM access").selectOption(account);
  await page.getByRole("button", { name: "Add ATM access" }).click();
  await expect(page.getByText("ATM access added.")).toBeVisible();
  await accessible(page);
  const cards = (await (
    await alice.context.request.get(customer + "/api/v1/atm/cards")
  ).json()) as { token: string; state: string }[];
  const token = cards.find((c) => c.state === "ACTIVE")!.token;
  const base = {
    terminalId: "SIM-ATM-002",
    cardToken: token,
    pinVerified: true,
  };
  const auth = await atm({
    ...base,
    messageId: "BRW-" + crypto.randomUUID().slice(0, 18),
    kind: "AUTHORIZATION",
    operation: "CASH_WITHDRAWAL",
    amountMinor: 1000,
  });
  expect(auth.result).toBe("APPROVED");
  const done = await atm({
    ...base,
    messageId: "BRW-" + crypto.randomUUID().slice(0, 18),
    kind: "COMPLETION",
    authorizationId: auth.authorizationId,
  });
  expect(done.result).toBe("COMPLETED");
  expect(
    (
      await atm({
        ...base,
        messageId: "BRW-" + crypto.randomUUID().slice(0, 18),
        kind: "AUTHORIZATION",
        operation: "DEPOSIT",
        amountMinor: 1000,
      })
    ).reason,
  ).toBe("OPERATION_NOT_SUPPORTED");
  const reviewer = await login(browser, "reviewer", true);
  await reviewer.page.goto(staff + "/#/atm");
  await expect(
    reviewer.page.getByRole("heading", { name: "ATM channel" }),
  ).toBeVisible();
  await expect(reviewer.page.getByText("SIM-ATM-002").first()).toBeVisible();
  await accessible(reviewer.page);
  await page.reload();
  await page
    .getByRole("button", { name: `Block card ending ${token.slice(-4)}` })
    .click();
  await expect(
    page.getByText("ATM access blocked. It cannot be used again."),
  ).toBeVisible();
  expect(
    (
      await atm({
        ...base,
        messageId: "BRW-" + crypto.randomUUID().slice(0, 18),
        kind: "INQUIRY",
      })
    ).reason,
  ).toBe("CARD_NOT_ACTIVE");
  for (const c of [alice, reviewer]) await c.context.close();
});

test("next-day batch transfer: recipient, web step-up approval, operator window close and staff rail view", async ({
  browser,
}) => {
  test.setTimeout(240_000);
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Batchalice");
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "20000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
    },
  );
  await page.reload();
  await page.getByRole("link", { name: "Payments", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Next-day transfers" }),
  ).toBeVisible();
  await page.getByLabel("Recipient name").fill("Landlord");
  await page.getByLabel("Routing number").fill("021000021");
  await page.getByLabel("Account number").fill("123456789");
  await page.getByRole("button", { name: "Save recipient" }).click();
  await expect(page.getByText("Recipient saved.")).toBeVisible();
  await page.getByLabel("From account").selectOption(account);
  await page
    .getByLabel("To recipient")
    .selectOption({ label: "Landlord · account ending 6789" });
  await page.getByLabel("Amount (TST)").fill("12.50");
  await page.getByRole("button", { name: "Prepare transfer" }).click();
  await expect(page.getByText("Needs your approval").first()).toBeVisible();
  await accessible(page);
  await page
    .getByRole("button", { name: "Confirm identity to approve" })
    .first()
    .click();
  await expect(page.locator("#password")).toBeVisible();
  const username = page.locator("#username");
  if ((await username.isVisible()) && (await username.isEditable()))
    await username.fill("alice");
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL(customer + "/**");
  await page.getByRole("link", { name: "Payments", exact: true }).click();
  await page.getByRole("button", { name: "Complete approval" }).first().click();
  await expect(
    page.getByText("Approved. The money is held for the next batch."),
  ).toBeVisible();
  await expect(
    page.getByText("Waiting for the next batch").first(),
  ).toBeVisible();
  const ops = await login(browser, "reviewer", true);
  await ops.page.goto(staff + "/#/batch");
  await expect(
    ops.page.getByRole("heading", { name: "Batch rail" }),
  ).toBeVisible();
  await ops.page.getByRole("button", { name: "Close window now" }).click();
  await expect(
    ops.page.getByText(/Window closed: \d+ entries sealed into a file\./),
  ).toBeVisible();
  await accessible(ops.page);
  await page.reload();
  await expect(page.getByText("Sent to the rail").first()).toBeVisible();
  for (const c of [alice, ops]) await c.context.close();
});

test("governed warehouse: staff backfill, purpose-checked snapshot with lineage and control totals, row trace, reproduction and recorded denial", async ({
  browser,
}) => {
  test.setTimeout(180_000);
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const alice = await login(browser, "alice");
  const account = await activeAccount(alice.context, "Warehousealice");
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "4200"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
    },
  );
  const ops = await login(browser, "reviewer", true),
    page = ops.page;
  await page.goto(staff + "/#/analytics");
  await expect(
    page.getByRole("heading", { name: "Governed warehouse" }),
  ).toBeVisible();
  // The worker projects Core events asynchronously; backfill until the funding entries are in the warehouse.
  await expect
    .poll(
      async () => {
        for (const dataset of ["account_states", "ledger_entries"]) {
          await page.getByLabel("Dataset to backfill").selectOption(dataset);
          await page.getByRole("button", { name: "Run backfill" }).click();
          await expect(page.getByText(/^Backfill read \d+/)).toBeVisible();
        }
        return (
          Number(
            (await page.getByText(/^Backfill read \d+/).textContent())?.match(
              /(\d+) new/,
            )?.[1] ?? 0,
          ) +
          Number(
            (await page.getByText(/^Backfill read \d+/).textContent())?.match(
              /(\d+) already/,
            )?.[1] ?? 0,
          )
        );
      },
      { timeout: 60_000, intervals: [2_000] },
    )
    .toBeGreaterThan(0);
  await page.getByLabel("Purpose").selectOption("FINANCIAL_REPORTING");
  await page.getByRole("button", { name: "Create snapshot" }).click();
  await expect(page.getByText(/^Snapshot created/)).toBeVisible();
  const detail = page.getByRole("article");
  await expect(
    detail.getByRole("heading", { name: /^Snapshot / }),
  ).toBeVisible();
  await expect(
    detail.getByText(/ledger_entries: \d+ facts · schema v1/),
  ).toBeVisible();
  await expect(
    detail.getByRole("cell", { name: "Core journal" }),
  ).toBeVisible();
  await accessible(page);
  await detail
    .getByRole("button", { name: "Trace row 1", exact: true })
    .click();
  await expect(
    detail.getByRole("heading", { name: "Row 1 sources" }),
  ).toBeVisible();
  await expect(
    detail.getByText(/transaction_views · schema v1 · run/).first(),
  ).toBeVisible();
  await detail
    .getByRole("button", { name: "Reproduce from stored cuts" })
    .click();
  await expect(
    page.getByText(
      "Reproduced: recomputing from the stored cuts gives the same digest.",
    ),
  ).toBeVisible();
  // No dataset permits marketing use: the request is refused and recorded.
  await page.getByLabel("Purpose").selectOption("MARKETING");
  await page.getByRole("button", { name: "Create snapshot" }).click();
  await expect(
    page.getByText(
      "Request denied and recorded. The reason is listed under recorded denials.",
    ),
  ).toBeVisible();
  await expect(
    page.getByText(/marketing · purpose not permitted for dataset · /).first(),
  ).toBeVisible();
  await accessible(page);
  for (const c of [alice, ops]) await c.context.close();
});

test("audiences and journey partner: consented build, pseudonymous export, partner advice with why, dismissal, withdrawal and outcome report", async ({
  browser,
}) => {
  test.setTimeout(240_000);
  const { createHmac } = await import("node:crypto");
  const api = `http://127.0.0.1:${4180 + offset}`;
  const key = "synthetic-journey-partner-key-local-only";
  const sign = (payload: string) =>
    createHmac("sha256", key).update(payload).digest("hex");
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Journeyalice");
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "3100"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
    },
  );
  await page.goto(customer + "/#/notifications");
  await expect(
    page.getByRole("heading", { name: "Suggestions and notices" }),
  ).toBeVisible();
  const marketing = page.getByRole("checkbox", {
    name: /for Product announcements/,
  });
  if (!(await marketing.isChecked())) await marketing.click();
  await expect(marketing).toBeChecked();
  // Staff: bring the warehouse up to date, then build and export a consented audience.
  const ops = await login(browser, "reviewer", true),
    staffPage = ops.page;
  await staffPage.goto(staff + "/#/analytics");
  await expect(
    staffPage.getByRole("heading", { name: "Governed warehouse" }),
  ).toBeVisible();
  await expect
    .poll(
      async () => {
        for (const dataset of ["account_states", "ledger_entries"]) {
          await staffPage
            .getByLabel("Dataset to backfill")
            .selectOption(dataset);
          await staffPage.getByRole("button", { name: "Run backfill" }).click();
          await expect(staffPage.getByText(/^Backfill read \d+/)).toBeVisible();
        }
        const facts = Number(
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
              "-c",
              `SELECT count(*) FROM analytics.facts WHERE account_id='${account}'`,
            ],
            { encoding: "utf8" },
          ).trim(),
        );
        return facts;
      },
      { timeout: 60_000, intervals: [2_000] },
    )
    .toBeGreaterThan(1);
  await staffPage.getByRole("link", { name: "Audiences & journeys" }).click();
  await expect(
    staffPage.getByRole("heading", { name: "Audiences & journeys" }),
  ).toBeVisible();
  await staffPage
    .getByRole("button", { name: "Build savings_starter audience" })
    .click();
  await expect(
    staffPage.getByText(/^Audience built: [1-9]\d* of/),
  ).toBeVisible();
  await staffPage
    .getByRole("button", { name: /^Export .* to SIM_JOURNEY$/ })
    .first()
    .click();
  await expect(
    staffPage.getByText(/^Exported [1-9]\d* pseudonymous tokens/),
  ).toBeVisible();
  await accessible(staffPage);
  // Partner: fetch the export (tokens only) and send advice to each member.
  const overview = await (
    await ops.context.request.get(staff + "/api/v1/staff/audiences")
  ).json();
  const exportId = overview.audiences[0].exports[0].id as string;
  const fetched = await fetch(
    api + `/channels/journeys/v1/exports/${exportId}`,
    {
      headers: {
        "Piggybank-Journey-Partner": "SIM_JOURNEY",
        "Piggybank-Journey-Signature": sign(
          `GET\n/channels/journeys/v1/exports/${exportId}\nSIM_JOURNEY`,
        ),
      },
    },
  );
  expect(fetched.status).toBe(200);
  const delivery = await fetched.json();
  expect(JSON.stringify(delivery)).not.toContain(account);
  for (const [n, token] of (delivery.members as string[]).entries()) {
    const body = JSON.stringify({
      messageId: `BRW-${crypto.randomUUID().slice(0, 8)}-${n}`,
      memberToken: token,
      recommendationCode: "SAVINGS_TIP",
      title: "A weekly savings habit",
      body: "Some people move a fixed amount each week. It is always your choice.",
      actionRoute: "#/accounts",
    });
    const sent = await fetch(api + "/channels/journeys/v1/recommendations", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Piggybank-Journey-Partner": "SIM_JOURNEY",
        "Piggybank-Journey-Signature": sign(body),
      },
      body,
    });
    expect((await sent.json()).result).toBe("ACCEPTED");
  }
  // Customer: the advice explains itself, can be dismissed, and disappears when marketing is withdrawn.
  await page.reload();
  const advice = page
    .locator(".ui-mini-card")
    .filter({ hasText: "A weekly savings habit" });
  await expect(advice).toBeVisible();
  await advice.getByText("Why am I seeing this?").click();
  await expect(
    advice.getByText(/A partner you allowed marketing from suggested this/),
  ).toBeVisible();
  await accessible(page);
  await page
    .getByRole("button", { name: "Dismiss Put part of each deposit aside" })
    .click();
  await expect(
    page.getByRole("button", {
      name: "Dismiss Put part of each deposit aside",
    }),
  ).toHaveCount(0);
  await marketing.click();
  await expect(marketing).not.toBeChecked();
  await expect(
    page.getByText(
      "Product announcements are off, so only service notices appear here.",
    ),
  ).toBeVisible();
  await expect(page.getByText("A weekly savings habit")).toHaveCount(0);
  // Staff: the outcome report states its denominators and flags.
  await staffPage.reload();
  await staffPage
    .getByRole("button", { name: /^Report outcomes for / })
    .first()
    .click();
  await expect(staffPage.getByText(/^Outcome report recorded/)).toBeVisible();
  await expect(
    staffPage.getByText(/members credited after build/).first(),
  ).toBeVisible();
  await accessible(staffPage);
  for (const c of [alice, ops]) await c.context.close();
});

test("payment routing: operator proposes a shadow policy, a different approver applies it, and measured health is shown", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const ops = await login(browser, "reviewer", true);
  await ops.page.goto(staff + "/#/routing");
  await expect(
    ops.page.getByRole("heading", { name: "Payment routing" }),
  ).toBeVisible();
  await expect(
    ops.page.getByRole("heading", {
      name: "Measured provider health (last hour)",
    }),
  ).toBeVisible();
  await expect(
    ops.page.getByRole("cell", { name: "SYNTHETIC_RAIL_B" }),
  ).toBeVisible();
  await ops.page
    .getByRole("button", { name: "Propose health-weighted policy (shadow)" })
    .click();
  await expect(
    ops.page.getByText(/proposed\. A different approver must decide\./),
  ).toBeVisible();
  await accessible(ops.page);
  const approver = await login(browser, "approver", true);
  await approver.page.goto(staff + "/#/routing");
  await approver.page
    .getByRole("button", { name: /^Approve change / })
    .first()
    .click();
  await expect(approver.page.getByText("Change applied.")).toBeVisible();
  await expect(approver.page.getByText(/· shadow v\d+/)).toBeVisible();
  await accessible(approver.page);
  for (const c of [ops, approver]) await c.context.close();
});

test("liquidity forecast: a confirmed breach restricts, an independent approver books the automated action and lifts the restriction", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const psql = (sql: string) =>
    execFileSync("podman", [
      "exec",
      "piggybank-postgres-1",
      "psql",
      "-U",
      "dev_admin",
      "-d",
      fixture.database,
      "-v",
      "ON_ERROR_STOP=1",
      "-c",
      sql,
    ]);
  // Synthetic warehouse history: 21 business days of settlement outflow before today.
  const run = crypto.randomUUID();
  const clearing = "4f1a780b-3e31-48ef-a294-000000000003";
  psql(
    `INSERT INTO analytics.ingestion_runs VALUES('${run}','ledger_entries',1,'BACKFILL',now()-interval '30 days',now(),21,21,0,0,0,now(),NULL,NULL,now(),now())`,
  );
  const rows = Array.from({ length: 21 }, (_, i) => {
    const offset = i + 1;
    const day = new Date(Date.now() - offset * 86_400_000)
      .toISOString()
      .slice(0, 10);
    const operation = crypto.randomUUID();
    const record = JSON.stringify({
      batchId: crypto.randomUUID(),
      operationId: operation,
      postingKind: "EXTERNAL_PAYMENT",
      accountId: clearing,
      accountKind: "SETTLEMENT",
      amountMinor: String(10000 + (offset % 3) * 1000),
      currency: "TST",
      businessDate: day,
    });
    return `('ledger_entries',1,'${operation}|EXTERNAL_PAYMENT|${clearing}','fixture','${day}','${clearing}','${record}'::jsonb,'browser-liquidity:${offset}',now(),'${run}')`;
  });
  psql(
    `INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,business_date,account_id,record,source_ref,source_recorded_at,run_id) VALUES${rows.join(",")}`,
  );
  psql(
    `INSERT INTO analytics.watermarks VALUES('ledger_entries',now(),'${run}',now(),now()) ON CONFLICT(dataset) DO UPDATE SET checked_at=now()`,
  );
  const ops = await login(browser, "reviewer", true);
  const publish = async (available: string) => {
    const asOf = new Date();
    const feed = await ops.context.request.post(
      "http://127.0.0.1:4182/admin/liquidity-positions",
      {
        headers: {
          "X-Liquidity-Key":
            "synthetic-local-liquidity-authentication-key-2026",
        },
        data: {
          evidenceId: crypto.randomUUID(),
          provider: "SYNTHETIC_LIQUIDITY",
          scope: "SYNTHETIC_TST_CLEARING",
          currency: { code: "TST", scale: 2 },
          version: Date.now() * 1000,
          asOf: asOf.toISOString(),
          validUntil: new Date(asOf.getTime() + 60000).toISOString(),
          settled: available,
          available,
          reserved: "0",
          expectedIncoming: "0",
          expectedOutgoing: "0",
        },
      },
    );
    expect(feed.status()).toBe(201);
  };
  await publish("105000");
  await ops.page.goto(staff + "/#/liquidity");
  await expect(
    ops.page.getByRole("heading", { name: "Liquidity forecast", level: 1 }),
  ).toBeVisible();
  await ops.page.getByRole("button", { name: "Run forecast now" }).click();
  await expect(ops.page.getByText("Forecast recorded.")).toBeVisible();
  await expect(ops.page.getByTestId("latest-forecast")).toContainText("breach");
  await expect(ops.page.getByTestId("confirmed-available")).toHaveText(
    "TST 1,050.00",
  );
  await expect(ops.page.getByTestId("restriction")).toContainText("Restricted");
  await ops.page.getByRole("button", { name: /^Confirm action / }).click();
  await expect(
    ops.page.getByText("Action confirmed. A different approver must decide."),
  ).toBeVisible();
  await ops.page.getByRole("button", { name: "Propose lift" }).click();
  await expect(ops.page.getByText(/^Lift proposed\./)).toBeVisible();
  await accessible(ops.page);
  const approver = await login(browser, "approver", true);
  await approver.page.goto(staff + "/#/liquidity");
  await approver.page.getByRole("button", { name: /^Approve action / }).click();
  await expect(
    approver.page.getByText("Action approved and booked by Core."),
  ).toBeVisible();
  await expect(approver.page.getByTestId("action-executed")).toContainText(
    "Core batch",
  );
  await publish("900000");
  await ops.page.getByRole("button", { name: "Run forecast now" }).click();
  await expect(ops.page.getByTestId("latest-forecast")).toContainText(
    "within limits",
  );
  await approver.page.reload();
  await approver.page.getByRole("button", { name: /^Approve lift / }).click();
  await expect(approver.page.getByText("Restriction lifted.")).toBeVisible();
  await expect(approver.page.getByTestId("restriction")).toContainText("Open");
  await accessible(approver.page);
  for (const c of [ops, approver]) await c.context.close();
});

test("outcome KPIs: staff compute a labelled local snapshot with numerators, denominators and exclusions", async ({
  browser,
}) => {
  const staffer = await login(browser, "approver", true);
  await staffer.page.goto(staff + "/#/outcomes");
  await expect(
    staffer.page.getByRole("heading", { name: "Outcome KPIs", level: 1 }),
  ).toBeVisible();
  await staffer.page
    .getByRole("button", { name: "Compute KPI snapshot" })
    .click();
  await expect(staffer.page.getByText("KPI snapshot recorded.")).toBeVisible();
  const table = staffer.page.getByTestId("kpi-table");
  await expect(table).toContainText("KPI-05");
  await expect(table).toContainText("manual sample required");
  await expect(table).toContainText("SLA: local assumed pending dec-04");
  await expect(
    staffer.page.getByText("local synthetic not production outcomes"),
  ).toBeVisible();
  await accessible(staffer.page);
  await staffer.context.close();
});

test("split, favourites and safe-to-spend: a customer splits an outgoing transaction with a favourite and sees a labelled safe-to-spend figure", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const psql = (sql: string) =>
    execFileSync("podman", [
      "exec",
      "piggybank-postgres-1",
      "psql",
      "-U",
      "dev_admin",
      "-d",
      fixture.database,
      "-v",
      "ON_ERROR_STOP=1",
      "-c",
      sql,
    ]);
  const alice = await login(browser, "alice"),
    bob = await login(browser, "bob");
  const aliceAccount = await activeAccount(alice.context, "Split alice");
  const bobAccount = await activeAccount(bob.context, "Split bob");
  const reference = (
    await (
      await send(
        bob.context,
        `/api/v1/accounts/${bobAccount}/receiving-reference`,
      )
    ).json()
  ).reference as string;
  psql(
    `SELECT * FROM core.fund_synthetic('${crypto.randomUUID()}','${aliceAccount}','4f1a780b-3e31-48ef-a294-000000000003',5000)`,
  );
  psql(
    `SELECT * FROM core.transfer('${crypto.randomUUID()}','${aliceAccount}','${bobAccount}',1001)`,
  );
  const page = alice.page;
  // Favourite the other customer's receiving reference.
  await page.goto(customer + "/#/payments");
  await page.getByLabel("Receiving reference", { exact: true }).fill(reference);
  await page.getByLabel("Name", { exact: true }).fill("Bob");
  await page.getByRole("button", { name: "Save favourite" }).click();
  await expect(page.getByTestId("favorites")).toContainText("Bob");
  // Split the outgoing transaction, including my own share.
  await page.goto(customer + "/#/spend/activity");
  await page.reload();
  await page
    .getByRole("button", { name: /^View transaction / })
    .first()
    .click();
  await page
    .getByLabel("Receiving references (comma separated, up to 9)")
    .fill(reference);
  await page.getByLabel("Memo").fill("Dinner");
  await page.getByRole("button", { name: "Split this expense" }).click();
  await expect(page.getByTestId("split-summary")).toContainText(
    "with 1 person",
  );
  await expect(page.getByTestId("split-summary")).toContainText(
    "your share TST 5.01",
  );
  await accessible(page);
  // Safe-to-spend is shown with its rule.
  await page.goto(customer + "/#/spend");
  await expect(page.getByTestId("safe-to-spend")).toBeVisible();
  await expect(
    page.getByText("A simple local rule, not a forecast or advice.", {
      exact: false,
    }),
  ).toBeVisible();
  await accessible(page);
  // The other customer receives the request as part of a split.
  await bob.page.goto(customer + "/#/payments");
  await expect(
    bob.page.getByText("Dinner · REQUESTED · part of a split"),
  ).toBeVisible();
  for (const c of [alice, bob]) await c.context.close();
});

test("vault photo goal: the owner attaches and removes a bounded goal photo", async ({
  browser,
}) => {
  const alice = await login(browser, "alice"),
    page = alice.page;
  await activeAccount(alice.context, "Photo alice");
  const created = await send(
    alice.context,
    "/api/v1/consumer/commands",
    {
      action: "VAULT_CREATE",
      input: { name: "Photo goal", targetMinor: "25000" },
    },
    "POST",
    crypto.randomUUID(),
  );
  expect(created.status()).toBe(200);
  const vault = (await created.json()).id as string;
  await page.goto(customer + "/#/save/" + vault);
  await page.reload();
  await page
    .getByLabel("Goal photo (PNG or JPEG, up to 32 KiB)")
    .setInputFiles({
      name: "goal.png",
      mimeType: "image/png",
      buffer: Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
        "base64",
      ),
    });
  await expect(
    page.getByRole("img", { name: "Goal photo for Photo goal" }),
  ).toBeVisible();
  await accessible(page);
  await page.getByRole("button", { name: "Remove goal photo" }).click();
  await expect(
    page.getByRole("img", { name: "Goal photo for Photo goal" }),
  ).toHaveCount(0);
  await alice.context.close();
});

test("QR payment codes: a receiving reference and an open request render as faithful QR codes and a scanned code only fills in a recipient", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const mod = await import("qrcode-generator");
  const qrcode = (mod.default ?? mod) as typeof mod.default;
  // Re-encode the payload independently and compare it module-for-module with the rendered SVG path.
  const faithful = async (svg: Locator) => {
    const payload = (await svg.getAttribute("data-payload"))!;
    const qr = qrcode(0, "M");
    qr.addData(payload, "Byte");
    qr.make();
    const n = qr.getModuleCount();
    let expected = "";
    for (let r = 0; r < n; r++)
      for (let c = 0; c < n; c++)
        if (qr.isDark(r, c)) expected += `M${c + 4} ${r + 4}h1v1h-1z`;
    expect(await svg.locator("path").getAttribute("d")).toBe(expected);
    expect(await svg.getAttribute("viewBox")).toBe(`0 0 ${n + 8} ${n + 8}`);
    return payload;
  };
  const alice = await login(browser, "alice"),
    bob = await login(browser, "bob");
  const aliceAccount = await activeAccount(alice.context, "QR alice");
  await activeAccount(bob.context, "QR bob");
  // A request shares the requester's own receiving reference, so Alice creates hers.
  expect(
    (
      await send(
        alice.context,
        `/api/v1/accounts/${aliceAccount}/receiving-reference`,
      )
    ).status(),
  ).toBe(200);
  // Bob shows his receiving reference as text and QR.
  await bob.page.goto(customer + "/#/payments");
  await bob.page.reload();
  await bob.page
    .getByRole("button", { name: "Show receiving reference" })
    .click();
  const bobQr = bob.page.getByRole("img", {
    name: "QR code for your receiving reference",
  });
  await expect(bobQr).toBeVisible();
  const payload = await faithful(bobQr);
  const reference = (await bob.page
    .locator(".receiving-reference > code")
    .textContent())!.trim();
  expect(payload).toBe(`pbtst:pay?ref=${reference.toUpperCase()}`);
  await accessible(bob.page);
  // Alice pastes the decoded code: it fills the recipient reference and pays nothing.
  const page = alice.page;
  await page.goto(customer + "/#/payments");
  await page.reload();
  await page
    .getByLabel("Payment code (text from a scanned QR, optional)")
    .fill("pbtst:pay?ref=NOTAREFERENCE");
  await page.getByRole("button", { name: "Use payment code" }).click();
  await expect(
    page.getByText("That is not a PiggyBank TST payment code."),
  ).toBeVisible();
  await page
    .getByLabel("Payment code (text from a scanned QR, optional)")
    .fill(payload);
  await page.getByRole("button", { name: "Use payment code" }).click();
  await expect(page.getByLabel("Shared receiving reference")).toHaveValue(
    reference.toUpperCase(),
  );
  await expect(page.getByTestId("code-info")).toContainText(
    "nothing is paid yet",
  );
  // Alice's open request can be shared as a QR code carrying the amount and memo.
  const created = await send(
    alice.context,
    "/api/v1/consumer/commands",
    {
      action: "REQUEST_MONEY",
      input: {
        accountId: aliceAccount,
        receivingReference: reference,
        amountMinor: "1250",
        memo: "Tickets",
      },
    },
    "POST",
    crypto.randomUUID(),
  );
  expect(created.status()).toBe(200);
  await page.reload();
  // Earlier journeys may leave other open requests; open the one for this request.
  await page
    .locator("article", { hasText: "Tickets" })
    .getByText("Share this request as a QR code")
    .click();
  const requestQr = page.getByRole("img", {
    name: "QR code requesting TST 12.50",
  });
  await expect(requestQr).toBeVisible();
  const requestPayload = await faithful(requestQr);
  expect(requestPayload).toMatch(
    /^pbtst:pay\?ref=PB[0-9A-F]{32}&amount=1250&memo=Tickets$/,
  );
  await accessible(page);
  for (const c of [alice, bob]) await c.context.close();
});

test("savings interest: a customer joins with the exact terms, sees the rate and boosts, completes the one-off challenge and a friend uses the referral code", async ({
  browser,
}) => {
  const fixture = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  const alice = await login(browser, "alice"),
    page = alice.page;
  const account = await activeAccount(alice.context, "Interest alice");
  execFileSync("podman", [
    "exec",
    "piggybank-postgres-1",
    "psql",
    "-U",
    "dev_admin",
    "-d",
    fixture.database,
    "-v",
    "ON_ERROR_STOP=1",
    "-c",
    `SELECT * FROM core.fund_synthetic('${crypto.randomUUID()}','${account}','4f1a780b-3e31-48ef-a294-000000000003',100000)`,
  ]);
  const created = await send(
    alice.context,
    "/api/v1/consumer/commands",
    {
      action: "VAULT_CREATE",
      input: { name: "Interest goal", targetMinor: "100000" },
    },
    "POST",
    crypto.randomUUID(),
  );
  expect(created.status()).toBe(200);
  const vault = (await created.json()).id as string;

  // UI-09: the savings hub invites the customer to earn interest.
  await page.goto(customer + "/#/save");
  await page.reload();
  await page.getByRole("link", { name: /^Earn up to 4\.59% APY/ }).click();
  await expect(
    page.getByRole("heading", { name: "Interest & boosts", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("3.04% APY", { exact: true })).toBeVisible();
  const join = page.getByRole("button", { name: "Join and start earning" });
  await expect(join).toBeDisabled();
  await accessible(page);
  await page
    .getByLabel(/I agree to these synthetic savings interest terms/)
    .check();
  await join.click();

  // UI-14: the current rate, its breakdown and next month's streak weeks.
  await expect(
    page.getByRole("list", { name: "Rate breakdown" }),
  ).toContainText("Base rate 3.00%");
  await expect(
    page.getByRole("list", { name: "Rate breakdown" }),
  ).toContainText(/New customer \+1\.00%/);
  await expect(
    page.getByRole("list", { name: "Saving weeks this month" }),
  ).toBeVisible();
  const code = (await page.getByTestId("referral-code").textContent())!.trim();
  expect(code).toMatch(/^PB[0-9A-F]{8}$/);

  // The one-off challenge completes from an actual vault deposit and pays its bonus once.
  await page.getByLabel("Challenge vault").selectOption(vault);
  await page.getByRole("button", { name: "Start challenge" }).click();
  await expect(
    page.getByRole("progressbar", { name: "Challenge progress" }),
  ).toBeVisible();
  await accessible(page);
  await page.goto(customer + "/#/save/" + vault);
  await page.getByLabel("Amount (TST)", { exact: true }).fill("500.00");
  await page.getByRole("button", { name: "Submit vault transfer" }).click();
  await expect(
    page.getByText("TST 500.00", { exact: true }).first(),
  ).toBeVisible();
  await page.goto(customer + "/#/save/interest");
  await expect(
    page.getByText(/completed \d{4}-\d{2}-\d{2}; TST 10\.00 bonus added/),
  ).toBeVisible({ timeout: 20_000 });

  // The hub now shows the earned rate and per-vault interest.
  await page.goto(customer + "/#/save");
  await expect(
    page.getByRole("link", { name: /^Earning \d\.\d\d% APY/ }),
  ).toBeVisible();
  await expect(
    page
      .getByText(/^Interest earned TST \d+\.\d\d · \d\.\d\d% a year$/)
      .first(),
  ).toBeVisible();
  await page.goto(customer + "/#/save/interest");
  await page.setViewportSize({ width: 390, height: 844 });
  await accessible(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".local/ui80-interest-mobile.png",
    fullPage: true,
  });

  // A friend enters the code within 30 days of their first account; the referrer sees it pending.
  const bob = await login(browser, "bob");
  await activeAccount(bob.context, "Interest bob");
  await bob.page.goto(customer + "/#/save/interest");
  await bob.page.reload();
  await bob.page
    .getByLabel("Were you referred? Enter the code")
    .fill(code.toLowerCase());
  await bob.page.getByRole("button", { name: "Use referral code" }).click();
  await expect(
    bob.page.getByText(
      /^You used a referral code on \d{4}-\d{2}-\d{2}\. Keep at least TST 100\.00 in your accounts through \d{4}-\d{2}-\d{2}\.$/,
    ),
  ).toBeVisible();
  await accessible(bob.page);
  await page.reload();
  await expect(
    page.getByRole("list", { name: "Your referrals" }),
  ).toContainText("waiting for the 30-day balance");
  await bob.context.close();
  await alice.context.close();
});

test("self-service registration and email verification: a new customer creates a sign-in at the issuer with a live strength meter, enforced policy and terms, then verifies the email with a one-time code", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const issuer = "http://127.0.0.1:18280";
  const email = `registration-${crypto.randomUUID().slice(0, 8)}@example.invalid`;
  const adminToken = async () => {
    const r = await fetch(
      issuer + "/realms/master/protocol/openid-connect/token",
      {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "password",
          client_id: "admin-cli",
          username: "local-admin",
          password:
            process.env.KEYCLOAK_ADMIN_PASSWORD ??
            "synthetic-keycloak-admin-only",
        }),
      },
    );
    return ((await r.json()) as { access_token: string }).access_token;
  };
  const context = await browser.newContext();
  const page = await context.newPage();
  try {
    await page.goto(customer);
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(
      page.getByRole("heading", { name: "Create your PiggyBank sign-in" }),
    ).toBeVisible();
    await expect(page.getByText("First, create your sign-in.")).toBeVisible();
    await expect(
      page.getByLabel("I accept the synthetic demo terms"),
    ).toBeVisible();
    const password = page.locator("#password"),
      strength = page.locator("#pb-password-strength-text");
    await password.fill("weakpass");
    await expect(strength).toContainText("Weak (1 of 3 requirements met)");
    await password.fill("Synth-Pa");
    await expect(strength).toContainText("Good (3 of 3 requirements met)");
    await password.fill("Synthetic-Pass-2026");
    await expect(strength).toContainText("Strong (3 of 3 requirements met)");
    // The meter only guides; the issuer's policy refuses a password without an uppercase letter on submit.
    await page.locator("#username").fill(email.split("@")[0]);
    await page.locator("#email").fill(email);
    await page.locator("#firstName").fill("Reggie");
    await page.locator("#lastName").fill("Synthetic");
    await password.fill("synthetic-pass-2026");
    await page.locator("#password-confirm").fill("synthetic-pass-2026");
    await page.locator("#termsAccepted").check();
    await page.locator("#kc-register-form [type=submit]").click();
    await expect(page.getByText(/upper case/i).first()).toBeVisible();
    await expect(page).toHaveURL(/18280/);
    // A compliant password, with the terms accepted, creates the sign-in and returns signed in.
    await page.locator("#password").fill("Synthetic-Pass-2026");
    await page.locator("#password-confirm").fill("Synthetic-Pass-2026");
    if (!(await page.locator("#termsAccepted").isChecked()))
      await page.locator("#termsAccepted").check();
    await page.locator("#kc-register-form [type=submit]").click();
    await page.waitForURL(customer + "/**");
    const me = await context.request.get(customer + "/auth/customer/me");
    expect(me.status()).toBe(200);
    expect((await me.json()).actor.trust).toBe("CUSTOMER");
    await accessible(page);
    // UI-03: the new customer lands on Profile & security and verifies the email with a one-time code.
    const fixture = JSON.parse(
      readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
    );
    const outbox = () =>
      execFileSync("podman", [
        "exec",
        "piggybank-postgres-1",
        "psql",
        "-U",
        "dev_admin",
        "-d",
        fixture.database,
        "-tAc",
        `SELECT substring(body from '([0-9]{6})') FROM simulator.contact_outbox WHERE destination='${email}' ORDER BY created_at DESC LIMIT 1`,
      ])
        .toString()
        .trim();
    await expect(
      page.getByRole("heading", { name: "Verify your email" }),
    ).toBeVisible();
    await expect(page.getByText("r•••@example.invalid").first()).toBeVisible();
    await page.getByRole("button", { name: "Send code" }).click();
    await expect(
      page.getByRole("button", { name: /^Resend code in \d+s$/ }),
    ).toBeDisabled();
    const code = outbox();
    expect(code).toMatch(/^[0-9]{6}$/);
    const wrong = code === "000000" ? "111111" : "000000";
    await page.getByLabel("Digit 1 of 6").fill(wrong);
    await expect(
      page.getByText("That code did not match. 4 attempt(s) left."),
    ).toBeVisible();
    await accessible(page);
    // Pasting or autofilling the code into the first box spreads it across all six and submits.
    await page.getByLabel("Digit 1 of 6").fill(code);
    await expect(page.getByText("Email verified.")).toBeVisible();
    await expect(page.getByText("Verified", { exact: true })).toBeVisible();
    const token = await adminToken();
    const [user] = (await (
      await fetch(
        `${issuer}/admin/realms/customers/users?exact=true&email=${encodeURIComponent(email)}`,
        { headers: { Authorization: "Bearer " + token } },
      )
    ).json()) as { emailVerified: boolean }[];
    expect(user.emailVerified).toBe(true);
  } finally {
    await context.close();
    // Restore the shared issuer: remove the synthetic registration.
    const token = await adminToken();
    const users = (await (
      await fetch(
        `${issuer}/admin/realms/customers/users?exact=true&email=${encodeURIComponent(email)}`,
        { headers: { Authorization: "Bearer " + token } },
      )
    ).json()) as { id: string }[];
    for (const u of users)
      await fetch(`${issuer}/admin/realms/customers/users/${u.id}`, {
        method: "DELETE",
        headers: { Authorization: "Bearer " + token },
      });
  }
});
