import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/browser-customer",
  workers: 1,
  timeout: 90_000,
  use: {
    baseURL: "http://127.0.0.1:4300",
    browserName: "chromium",
    trace: "off",
    screenshot: "off",
    video: "off",
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
      : {},
  },
  webServer: {
    command: "python3 tests/browser-customer/server.py",
    url: "http://127.0.0.1:4300",
    reuseExistingServer: false,
    timeout: 120_000,
    gracefulShutdown: { signal: "SIGTERM", timeout: 20_000 },
  },
});
