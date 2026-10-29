import { defineConfig } from '@playwright/test'

// Stable-edition golden e2e (GitHub CI only). The headless serve listener
// uses a self-signed certificate, hence ignoreHTTPSErrors.
export default defineConfig({
  testDir: '.',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  retries: 1,
  workers: 1,
  reporter: [['list']],
  use: {
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    video: 'off',
  },
})
