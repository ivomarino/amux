// Playwright golden-scenario harness (RR-0025, Invariants 44/45/46).
//
// Boots the RUST server against a throwaway AMUX_HOME so every run starts
// from a deterministic state — the Python gate-contract suite was green for
// weeks on ambient machine state and red the moment CI ran it clean; this
// harness builds its own world instead.
import { defineConfig, devices } from '@playwright/test';
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
    // iOS SAFARI IS ITS OWN TARGET, not a viewport (AMUX tab-customizer, 2026-08-13).
    // A bottom-sheet fix passed desktop+mobile Chromium at 375px while Ethan still
    // could not see the menu on his phone — Chromium at a phone WIDTH is not Safari
    // on a phone. WebKit differs on exactly what this UI depends on: position:fixed
    // inside a transformed ancestor, env(safe-area-inset-*), and dvh/vh behaviour.
    // SCOPED to the layout/interaction specs this project exists to protect,
    // and the scope is a measured decision, not caution.
    //
    // Running the WHOLE suite here (the first version of this project) is what
    // you want in principle — a new mobile test should reach iOS without
    // anyone remembering this list. It does not work yet, and the reason is
    // not WebKit: every project shares ONE server and ONE pref store, so specs
    // that mutate global state race each other across projects. A third
    // project pushed that over the edge. Measured 2026-08-13: settings.spec.ts
    // failed 7 tests under the 3-project run and passed 27/27 when run alone
    // under this same WebKit target. Chasing those reds would have "fixed"
    // flakiness that is really a missing isolation boundary (AF-46).
    //
    // So: narrow claim, honestly true, green — rather than a broad claim that
    // is red for reasons unrelated to iOS. Widening this to the full suite is
    // the goal and it is blocked on cross-project isolation, not on effort.
    {
      name: 'ios-safari',
      use: { ...devices['iPhone 15'] },
      testMatch: /(tab-customizer|sw-fail-bar|phase0|message-resend)\.spec\.ts/,
    },
  ],
  webServer: {
    // Builds from COMMITTED HEAD, not this shared working tree — a peer
    // mid-edit used to fail runs with a Rust error against JS-only changes,
    // and a red run caused by a stranger's half-saved file is
    // indistinguishable from one caused by your own patch (AMUX-2924).
    // Uncommitted rust changes are announced by name every run;
    // AMUX_E2E_WORKING_TREE=1 opts back in to testing the tree.
    command: `bash ${path.join(__dirname, 'serve-head.sh')}`,
    url: `https://localhost:${PORT}/health`,
    ignoreHTTPSErrors: true,
    reuseExistingServer: false,
    // PIPE, not the default 'ignore'. serve-head.sh prints which uncommitted
    // rust changes are NOT under test, and that notice is the only thing
    // standing between "builds from HEAD" and a developer silently testing
    // code they did not write. A warning nobody can see is not a warning —
    // the default would have swallowed it whole (AMUX-2924).
    stdout: 'pipe',
    stderr: 'pipe',
    // 10 min, not 3: the HEAD-worktree build now uses its OWN target dir
    // (AMUX-2961 — sharing the fleet's dir let worktree dep-info poison repo
    // builds into silent no-ops), so its FIRST local build is fully cold.
    // Later runs are cached; CI skips the worktree entirely via
    // AMUX_E2E_WORKING_TREE=1.
    timeout: 600_000,
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
