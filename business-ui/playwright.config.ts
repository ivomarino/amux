import { defineConfig, devices } from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  testMatch: '*.spec.ts',
  fullyParallel: false,
  workers: 1,
  timeout: 90000,
  expect: { timeout: 15000 },
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: 'http://127.0.0.1:3101',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'desktop',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1440, height: 1000 },
      },
      testIgnore: 'live.spec.ts',
    },
    {name:'embedded',use:{...devices['Desktop Chrome'],baseURL:'http://127.0.0.1:18824',viewport:{width:1440,height:1000}},testIgnore:'live.spec.ts'},
    {
      name: 'live',
      use: {
        ...devices['Desktop Chrome'],
        baseURL: process.env.AMUX_LIVE_URL || 'http://127.0.0.1:3100',
        ignoreHTTPSErrors: true,
        viewport: { width: 1440, height: 1000 },
      },
      testMatch: 'live.spec.ts',
    },
  ],
  webServer: process.env.AMUX_LIVE_TEST === '1' ? [] : [
    {
      command: 'node tests/fixture-server.mjs',
      url: 'http://127.0.0.1:18824/health',
      reuseExistingServer: !process.env.CI,
    },
    {
      command:
        'VINEXT_NO_DEV_LOCK=1 AMUX_TEST_INSTANCE=1 AMUX_SERVER_URL=http://127.0.0.1:18824 npm run dev -- --port 3101',
      url: 'http://127.0.0.1:3101',
      reuseExistingServer: !process.env.CI,
      timeout: 120000,
    },
  ],
});
