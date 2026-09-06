#!/usr/bin/env node
// Read Gemini Code Assist quota through Gemini CLI's own authenticated client.
// This script never prints credentials or account identity; stdout is one JSON
// object containing only tier and quota fields returned by retrieveUserQuota.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

function answer(value) {
  process.stdout.write(JSON.stringify(value));
}

function unavailable(cause, reason, authType) {
  answer({ available: false, cause, reason, auth_type: authType || null });
  process.exit(0);
}

function findOnPath(name) {
  for (const dir of (process.env.PATH || '').split(path.delimiter)) {
    if (!dir) continue;
    const candidate = path.join(dir, name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return fs.realpathSync(candidate);
    } catch {}
  }
  return null;
}

const geminiHome = process.env.GEMINI_CLI_HOME || path.join(os.homedir(), '.gemini');
let settings = {};
try {
  settings = JSON.parse(fs.readFileSync(path.join(geminiHome, 'settings.json'), 'utf8'));
} catch {}
const authType = settings?.security?.auth?.selectedType || null;

// API-key and Vertex billing expose per-request/session accounting, but not a
// fixed subscription balance through retrieveUserQuota. Say that plainly.
if (authType && authType !== 'oauth-personal' && authType !== 'compute-default-credentials') {
  unavailable(
    'account_quota_not_reported',
    'This Gemini authentication mode does not report an account-wide remaining subscription quota.',
    authType,
  );
}

const executable = findOnPath(process.platform === 'win32' ? 'gemini.cmd' : 'gemini');
if (!executable) {
  unavailable('cli_missing', 'Gemini CLI is not installed on this host.', authType);
}
const bundleDir = path.dirname(executable);
let coreFiles = [];
try {
  coreFiles = fs.readdirSync(bundleDir)
    .filter((name) => /^core-[A-Z0-9]+\.js$/.test(name))
    .map((name) => path.join(bundleDir, name));
} catch {}
if (!coreFiles.length) {
  unavailable('unsupported_cli', 'Installed Gemini CLI has no readable quota client.', authType);
}

// New Gemini CLI releases use encrypted credential storage. The legacy
// oauth_creds.json remains supported when it exists.
if (!fs.existsSync(path.join(geminiHome, 'oauth_creds.json'))) {
  process.env.GEMINI_FORCE_ENCRYPTED_FILE_STORAGE = 'true';
}

let core = null;
for (const file of coreFiles) {
  try {
    const candidate = await import(pathToFileURL(file).href);
    if (candidate.getOauthClient && candidate.CodeAssistServer && candidate.AuthType) {
      core = candidate;
      break;
    }
  } catch {}
}
if (!core) {
  unavailable('unsupported_cli', 'Installed Gemini CLI has no readable quota client.', authType);
}

const config = {
  getProxy: () => undefined,
  isBrowserLaunchSuppressed: () => true,
  isInteractive: () => false,
};

try {
  const client = await core.getOauthClient(
    authType === 'compute-default-credentials'
      ? core.AuthType.COMPUTE_ADC
      : core.AuthType.LOGIN_WITH_GOOGLE,
    config,
  );
  const projectFromEnv =
    process.env.GOOGLE_CLOUD_PROJECT || process.env.GOOGLE_CLOUD_PROJECT_ID || undefined;
  const discovery = new core.CodeAssistServer(client, projectFromEnv);
  const loaded = await discovery.loadCodeAssist({
    cloudaicompanionProject: projectFromEnv,
    metadata: {
      ideType: 'IDE_UNSPECIFIED',
      platform: 'PLATFORM_UNSPECIFIED',
      pluginType: 'GEMINI',
      duetProject: projectFromEnv,
    },
    mode: 'HEALTH_CHECK',
  });
  const projectId = projectFromEnv || loaded?.cloudaicompanionProject;
  if (!projectId) {
    unavailable(
      'quota_project_unavailable',
      'Gemini is signed in, but its account quota project is not available.',
      authType,
    );
  }
  const server = new core.CodeAssistServer(client, projectId);
  const quota = await server.retrieveUserQuota({ project: projectId });
  answer({
    available: true,
    auth_type: authType || 'oauth-personal',
    tier: loaded?.paidTier
      ? { id: loaded.paidTier.id || null, name: loaded.paidTier.name || null }
      : { id: loaded?.currentTier?.id || null, name: loaded?.currentTier?.name || null },
    credits: loaded?.paidTier?.availableCredits || null,
    quota,
  });
} catch {
  unavailable(
    'probe_failed',
    'Gemini CLI could not read the signed-in account quota.',
    authType,
  );
}
