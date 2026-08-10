# cloud/ — cloud.amux.io provisioning (DEPRECATED, pending Rust migration)

**Status (2026-08-09):** cloud.amux.io is a LIVE product still running the
**last-built Python image** of amux. The Python server (`amux-server.py`) has been
removed from this repo — git history has it, and `cloud/docker/amux-server.py`
(gitignored, generated) is what the image build consumed. The python-image build
workflows (`deploy-cloud.yml`, `cloud-image.yml`) were removed with it, so **no new
python image can be built from main**; the running service keeps serving its current
image until the cloud stack is migrated to the Rust server.

Do not build new features here. Keep this directory: it is the infrastructure
definition (Terraform, gateway, litestream, seed/e2e scripts) of a running service,
and deleting it would orphan that service's config without stopping it.

- `main.tf`, `outputs.tf` — GCP VM provisioning
- `gateway/` — the auth/routing gateway (deployed via `tailscale ssh root@amux-cloud`)
- `docker/` — the (frozen) python image build context
- `litestream/` — per-user DB replication to R2
- `seed.py`, `ui_seed.py`, `tests/` — seeding and e2e smoke for the hosted tier

The Rust migration of this tier should reuse the same gateway/isolation model with
the `amux-server-rs` binary in the container — card it before touching anything here.
