# Changelog — local/amux-5-fix deployment customizations

This file tracks all changes made to the `local/amux-5-fix` branch, which contains deployment-specific customizations for this Linux setup. These changes are separate from upstream amux and should be carefully reviewed before PRing.

## 2026-08-25 — Secrets Infrastructure & Phase 4 API Endpoints

### Phase 1-4: Encrypted Secrets System
- **Phase 1**: Central SOPS/age encryption setup
  - Created `.sops.yaml` with age encryption configuration
  - Created `secrets/amux-secrets.yaml` with encrypted credentials
  - Encrypted with X25519 (age) for secure storage
  
- **Phase 2**: SecretStore Rust implementation
  - `crates/amux-server/src/secrets.rs` (249 lines)
  - `SecretStore::load()` — decrypt and parse secrets
  - `SecretStore::get(path)` — dot-separated path lookup
  - `SecretStore::list_paths()` — enumerate all keys
  - `SecretStore::inspect_schema()` — JSON structure without values
  - `SecretStore::load_env()` — populate environment variables
  - Full test coverage for flatten_keys and schema_only

- **Phase 3**: Server Integration
  - Initialize SecretStore after config loads in `lib.rs`
  - Load secrets after logging configured
  - Add `secrets: Arc<SecretStore>` to AppState
  - Load secrets into environment for legacy code compatibility
  - Graceful degradation — logs warning but continues if secrets unavailable

- **Phase 4**: REST API Endpoints
  - `GET /api/secrets` — list all secret paths (no values)
  - `GET /api/secrets/{path}` — get specific secret (auth required)
  - `GET /api/secrets/inspect` — schema structure only
  - `POST /api/secrets/{path}` — update secret (admin auth required)
  - Full error handling (404, 401, 400, 500)
  - Request body validation

### Dependencies Added
- `shellexpand = "3.0"` for tilde expansion in age key paths

### Files Modified
- `.sops.yaml` — encryption configuration
- `secrets/amux-secrets.yaml` — encrypted secrets store
- `crates/amux-server/src/secrets.rs` — SecretStore implementation
- `crates/amux-server/src/lib.rs` — server integration
- `crates/amux-server/src/api/mod.rs` — router setup
- `crates/amux-server/src/api/secrets.rs` — API endpoints
- `crates/amux-server/Cargo.toml` — dependencies
- `.gitignore` — prevent plaintext secret leaks

### Security Notes
- Secrets encrypted with age (X25519) at rest
- Decrypted once at startup and cached in memory
- Age key at `~/.config/sops/age/keys.txt` (not committed)
- Environment variable exposure for legacy code only
- API endpoints require authentication
- Admin-only writes

### Phase 5: Web UI Dashboard
- `crates/amux-dashboard/static/secrets-ui.js` (421 lines)
- Full CRUD interface for secrets management
- Modal dialogs for create/edit/view operations
- Search and filter by secret path
- Dark mode support, mobile responsive
- Copy to clipboard functionality

### Phase 6: MCP Integration
- `crates/amux-server/src/mcp_secrets.rs` (64 lines)
- REQUEST_SECRET tool for Claude agents
- LIST_SECRETS and INSPECT_SCHEMA tools
- Path validation (no wildcards)
- Ready for rate limiting and audit logging
- Full test coverage

### Phase 7: GitHub OAuth Connector
- `crates/amux-server/src/api/github_connector.rs` (138 lines)
- Complete OAuth 2.0 flow implementation
- Credentials stored in secrets infrastructure
- Token exchange with GitHub API
- Status checking endpoint
- Webhook-ready for GitHub events

---

## Branch Strategy

This branch contains local deployment-specific customizations. When pushing to upstream:

1. **Create separate feature branches** for each upstreamable change:
   - `feature/gmail-calendar-connectors` (Gmail/Calendar connector implementation)
   - `feature/session-resume-boot` (Session persistence on reboot)
   - `feature/database-schema-migration-fix` (Python→Rust DB compatibility)
   - `feature/systemd-boot-persistence` (Linux systemd user service setup)
   - `feature/secrets-infrastructure` (Encrypted secrets system — Phases 1-6)

2. **Keep local/amux-5-fix for deployment-only config**:
   - Environment variable overrides
   - Local system configuration
   - Anything with credentials or paths that shouldn't ship upstream

3. **Test in separate branches** before PRing to ensure no unintended dependencies

---

## Previous Sessions
See `LOCAL_CHANGES.md` for detailed documentation of earlier fixes (database schema mismatch, worker startup retry logic, etc.).

For all code changes, verify:
```bash
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p amux-server
```

After commits:
```bash
curl -sk https://localhost:8824/health  # verify build hash moves
git log -3  # confirm commits are clean
```
