// Playwright golden-scenario harness (RR-0025, Invariants 44/45/46).
//
// Boots the RUST server against a throwaway AMUX_HOME so every run starts
// from a deterministic state — the Python gate-contract suite was green for
// weeks on ambient machine state and red the moment CI ran it clean; this
// harness builds its own world instead.
import { defineConfig } from '@playwright/test';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';

// One temp home per run, created eagerly so the server and the tests agree.
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'amux-e2e-'));
const PORT = 18823; // fixed high port; nothing else in CI binds it

export default defineConfig({
  testDir: '.',
  timeout: 30_000,
  retries: 0,
  use: {
    baseURL: `https://localhost:${PORT}`,
    ignoreHTTPSErrors: true, // self-signed cert is the product behavior
  },
  projects: [
    { name: 'desktop', use: { viewport: { width: 1280, height: 800 } } },
    // Mobile is a first-class target (amux is mobile-first): 375px must
    // render without overflow.
    { name: 'mobile', use: { viewport: { width: 375, height: 667 } } },
  ],
  webServer: {
    command: `cargo run -p amux-server`,
    url: `https://localhost:${PORT}/health`,
    ignoreHTTPSErrors: true,
    reuseExistingServer: false,
    timeout: 180_000, // first cargo build is slow; later runs are cached
    env: {
      AMUX_HOME: home,
      AMUX_RS_PORT: String(PORT),
      // A browser test necessarily connects over loopback, which the server
      // deliberately auth-bypasses (Python parity). Without this, every
      // "rejects a bad token" assertion is a check that cannot pass — it was
      // read as an auth regression on 2026-08-09 when the code was correct.
      AMUX_RS_NO_LOOPBACK_BYPASS: '1',
    },
  },
});
