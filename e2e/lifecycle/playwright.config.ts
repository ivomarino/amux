// One browser acceptance entry point: existing regressions + connected journeys.
// Keep every existing browser project; adding a spec automatically includes it.
import { defineConfig } from '@playwright/test';
import base from '../playwright.config';
import path from 'node:path';

const output = path.resolve(process.env.AMUX_LIFECYCLE_OUTPUT || 'test-results/lifecycle');
export default defineConfig({
  ...base,
  testDir: '..',
  testIgnore: ['**/live-*.spec.ts'],
  workers: 1, // specs mutate global prefs; one writer per isolated server
  fullyParallel: false,
  retries: 0,
  projects: base.projects!.map((project, index) => ({
    ...project, use: { ...project.use, baseURL: `https://localhost:${19823 + index * 10}` },
  })),
  webServer: (base.webServer as any[]).map((server, index) => ({
    ...server,
    command: `bash ${path.join(__dirname, 'serve.sh')}`,
    url: `https://localhost:${19823 + index * 10}/health`,
    env: { ...server.env, AMUX_RS_PORT: String(19823 + index * 10) },
  })),
  outputDir: path.join(output, 'browser-artifacts'),
  reporter: [
    ['line'],
    ['json', { outputFile: path.join(output, 'browser.json') }],
    ['html', { outputFolder: path.join(output, 'browser-report'), open: 'never' }],
  ],
  use: { ...base.use, trace: 'on', screenshot: 'on', video: 'on' },
});
