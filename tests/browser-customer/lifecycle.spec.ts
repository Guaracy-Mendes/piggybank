import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

test("customer completes persistent onboarding, sees exact balances and manages owned servicing", async ({
  page,
  context,
}) => {
  async function accessible() {
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
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: /Your everyday/ }),
  ).toBeVisible();
  await accessible();
  await page.getByRole("button", { name: "Sign in", exact: false }).click();
  await page.locator("#username").fill("alice");
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL("http://127.0.0.1:4300/**");
  await expect(
    page.getByRole("heading", { name: "Welcome to your space." }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Open an account", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "A few steps. A fresh start." }),
  ).toBeVisible();
  await accessible();
  await page
    .getByLabel("Full name", { exact: true })
    .fill("Synthetic Browser Customer");
  await page.getByLabel("Date of birth", { exact: true }).fill("1990-01-01");
  await page
    .getByLabel("Synthetic contact email", { exact: true })
    .fill("browser@example.invalid");
  await page
    .getByRole("checkbox", {
      name: "I agree to the use of my synthetic information for this application.",
    })
    .check();
  await page.getByRole("button", { name: "Save details", exact: true }).click();
  await expect(
    page.getByText("Your details are saved.", { exact: true }),
  ).toBeVisible();
  await page
    .getByLabel("Synthetic evidence", { exact: true })
    .fill("Synthetic browser identity evidence, not a real identity document.");
  await page
    .getByRole("button", { name: "Attach evidence", exact: true })
    .click();
  await expect(
    page.getByText("Your evidence is attached.", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("checkbox", { name: /I have read and accept version 1/ })
    .check();
  await page.getByRole("button", { name: "Accept terms", exact: true }).click();
  await expect(
    page.getByText("Your acceptance is saved with the exact terms version.", {
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Submit application", exact: false })
    .click();
  await expect(
    page.getByRole("heading", { name: "Your account is ready.", exact: true }),
  ).toBeVisible({ timeout: 30_000 });
  const applicationUrl = page.url();
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Your account is ready.", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "View your account", exact: false })
    .click();
  await expect(
    page.getByRole("heading", { name: "A clearer view of your money." }),
  ).toBeVisible();
  await expect(
    page.getByText("TST 0.00", { exact: true }).first(),
  ).toBeVisible();
  const accountResponse = await context.request.get(
    "http://127.0.0.1:4300/api/v1/accounts",
  );
  expect(accountResponse.status()).toBe(200);
  const account = (await accountResponse.json())[0].id;
  const manifest = JSON.parse(
    readFileSync(resolve(".local/customer-browser-fixture.json"), "utf8"),
  );
  expect(manifest.database).toMatch(/^piggybank_http_test_[a-f0-9]{32}$/);
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "500000"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
      stdio: "ignore",
    },
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByText("TST 5,000.00", { exact: true }).first(),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Refresh history", exact: true })
    .click();
  await expect(
    page.getByRole("cell", { name: "synthetic funding", exact: false }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Request support", exact: false })
    .click();
  await expect(page.getByText(/Your support request is saved/)).toBeVisible();
  await page
    .getByRole("button", { name: "Freeze outgoing activity", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Outgoing frozen", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("link", { name: "Profile & security", exact: true })
    .click();
  await page
    .getByLabel("New synthetic email", { exact: true })
    .fill("verified-browser@example.invalid");
  await page
    .getByRole("button", { name: "Request verified change", exact: true })
    .click();
  await expect(
    page.getByText("Your verified contact is updated.", { exact: true }),
  ).toBeVisible({ timeout: 30_000 });
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Hello, Synthetic." }),
  ).toBeVisible();
  await accessible();
  await page.screenshot({
    path: ".local/customer-overview.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Hello, Synthetic." }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: ".local/customer-mobile.png", fullPage: true });
  await accessible();
  await page.setViewportSize({ width: 1440, height: 1000 });
  // The customer channel does not proxy staff or provider authority, even with shared loopback cookies.
  expect(
    (
      await context.request.get("http://127.0.0.1:4300/api/v1/staff/cases")
    ).status(),
  ).toBe(403);
  // Exact values beyond JavaScript's Number precision remain exact in the real balance view.
  execFileSync(
    resolve("target/debug/bank-fund"),
    [crypto.randomUUID(), account, "9007199254240993"],
    {
      env: {
        ...process.env,
        BANK_PROFILE: "local",
        BANK_CORE_DATABASE_URL: `postgres://bank_core:synthetic-core-only@127.0.0.1:15432/${manifest.database}`,
      },
      stdio: "ignore",
    },
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByText("TST 90,071,992,547,409.93", { exact: true }).first(),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.setViewportSize({ width: 1440, height: 1000 });
  let navigation: { continueTo: string } | undefined;
  await page.route("**/auth/customer/logout", async (route) => {
    const response = await route.fetch({ maxRedirects: 0 });
    navigation = await response.json();
    await route.fulfill({ response });
  });
  await page.getByRole("button", { name: "Sign out", exact: true }).click();

  await expect(
    page.getByRole("heading", { name: /Your everyday/ }),
  ).toBeVisible({ timeout: 15_000 });
  expect(navigation).toBeDefined();
  expect(Object.keys(navigation!)).toEqual(["continueTo"]);
  expect(navigation!.continueTo).toMatch(
    /^\/auth\/customer\/end-session\/[A-Za-z0-9_-]+$/,
  );

  expect(
    (
      await context.request.get("http://127.0.0.1:4300/auth/customer/me")
    ).status(),
  ).toBe(401);
  expect(
    (
      await context.request.get(
        "http://127.0.0.1:4300" + navigation!.continueTo,
        { maxRedirects: 0 },
      )
    ).status(),
  ).toBe(401);
  await page.getByRole("button", { name: "Sign in", exact: false }).click();
  await expect(page.locator("#username")).toBeVisible();
  await page.locator("#username").fill("alice");
  await page.locator("#password").fill("Synthetic-Local-123!");
  await page.locator("#kc-login").click();
  await page.waitForURL("http://127.0.0.1:4300/**");
  await page.goto(applicationUrl);
  await expect(
    page.getByRole("heading", { name: "Your account is ready.", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("link", { name: "Skip to main content", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
});
