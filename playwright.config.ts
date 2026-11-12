import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/browser',
  workers: 1,
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:4180',
    browserName: 'chromium',
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
      : {},
  },
  // Auth traces/screenshots can contain one-time codes or session material.
  // Keep them disabled; assertions below deliberately avoid printing tokens.
  webServer: {
    command: 'scripts/cargo.sh run --locked --bin bank-api',
    url: 'http://127.0.0.1:4180/health/ready',
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
