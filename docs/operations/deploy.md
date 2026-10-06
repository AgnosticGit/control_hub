# Production deploy: two hosts from GitHub Actions

karbird runs on two small Linux hosts (tested target: 1/8 OCPU, 1 GB RAM, x86_64, Ubuntu
22.04/24.04). CI builds the images, the hosts only pull and run them (ADR-0017).

| Host | Runs | Public ports |
|---|---|---|
| `storage` | PostgreSQL, ClickHouse, SeaweedFS (S3 archive) | 22 only |
| `app` | Kafka, ingest, processor, API, executor, archiver, web, Caddy (TLS) | 22, 80, 443 |

Public URLs: `https://<domain>/` (site and `/app`), `https://<domain>/api/v1/…`, and OTLP/HTTP at
`https://<domain>/otlp` (SDKs: `OTEL_EXPORTER_OTLP_ENDPOINT=https://<domain>/otlp`).

## One-time setup

1. **Two VMs in the same private network (VCN/subnet).** Note the storage host's *private* IP.
2. **Firewall (cloud security list / NSG):**
   - `app`: ingress TCP 22, 80, 443 and UDP 443 from anywhere.
   - `storage`: ingress TCP 22 from anywhere; TCP 5432, 8123, 8333 **only from the app host's
     private IP**. These ports carry database traffic without TLS.
3. **DNS:** an `A` record for the domain pointing to the app host's public IP. Caddy obtains the
   certificate on first start, so the record must resolve before the first deploy.
4. **SSH key for the deploy:** `ssh-keygen -t ed25519 -f karbird-deploy -N ""`, add
   `karbird-deploy.pub` to `~/.ssh/authorized_keys` of the deploy user (default `ubuntu`) on
   both hosts. The user needs passwordless `sudo` for the first run (cloud images have it).
5. **Pinned host keys:** `ssh-keyscan -t ed25519 <storage-ip> <app-ip>` and compare the
   fingerprints with the cloud console before saving them as `DEPLOY_KNOWN_HOSTS`.
6. **GitHub settings** (Settings → Secrets and variables → Actions, repository level):

   | Secret | Value |
   |---|---|
   | `SECRET_SEED` | 48+ random characters (`openssl rand -base64 48`). All service passwords derive from it. Never change its value after the first deploy (renaming the secret is fine if the value stays the same). |
   | `OWNER_EMAIL` | the owner account, created on the first deploy |
   | `OWNER_PASSWORD` | 12+ characters, no `'` and no line breaks. Only used to create the account; later changes are ignored (change it in `/app/account`). |
   | `DEPLOY_SSH_KEY` | private key from step 4 (the whole file) |
   | `DEPLOY_KNOWN_HOSTS` | output of step 5 |

   | Variable | Value |
   |---|---|
   | `DOMAIN` | e.g. `karbird.example.com` (no scheme) |
   | `APP_HOST` | public IP or name of the app host |
   | `STORAGE_HOST` | public IP or name of the storage host (SSH only) |
   | `STORAGE_PRIVATE_IP` | private IP of the storage host |
   | `DEPLOY_USER` | optional, default `ubuntu` |
   | `ACME_EMAIL` | optional, default the owner email (Let's Encrypt notices) |
   | `KARBIRD_LOGS_RETENTION_DAYS`, `…_SPANS_…`, `…_EVENTS_…` | optional, defaults 14 / 14 / 30 (ADR-0016) |
   | `KARBIRD_SIGNUP` | optional, `open` (default) or `closed`: whether visitors can create an account at `/app/register` |
   | `KARBIRD_SIGNUP_PER_HOUR` | optional, default 20: sign-up attempts per hour across all clients |

   The deploy job uses the `production` environment (created automatically; add required
   reviewers there to gate deploys). Registry access uses the job's own `GITHUB_TOKEN`.
7. **First deploy:** Actions → Deploy → Run workflow, tick `setup_hosts`. It installs Docker and
   the Compose plugin, adds 2 GB swap and creates `/opt/karbird` on both hosts, then deploys.

## Releases and rollback

- Release: `git tag v0.1.0 && git push origin v0.1.0`. The workflow runs the full CI, builds and
  pushes `ghcr.io/<owner>/karbird-*:v0.1.0`, migrates and restarts what changed, then checks
  `https://<domain>/api/v1/ready`.
- Manual run on a branch deploys `sha-<12>` images of that commit.
- Rollback: Run workflow on the previous tag. Migrations are forward-only, so a release must keep
  the schema compatible with the previous one.
- Shortening retention deletes data: the deploy stops with the number of affected rows; rerun
  with `shrink_retention` ticked to confirm.

## Moving hosts deployed under the previous project name

Hosts deployed before the rename to karbird keep the old directory, Compose project, volumes,
images and GitHub settings; the new workflow does not touch them. The data on them is not kept
(fresh databases), so the move is a clean redeploy:

1. GitHub settings: create the secrets and variables of step 6 under their `KARBIRD_*` names with
   the same values (keep the secret seed), then delete the old ones.
2. On both hosts, with `OLD` set to the previous directory name under `/opt`:
   ```
   cd /opt/$OLD && docker compose -f compose.app.yaml down -v   # compose.storage.yaml on the storage host; deletes old volumes
   sudo rm -rf /opt/$OLD
   ```
   `down -v` deletes the old TLS certificates too; Caddy requests new ones on start.
3. Actions → Deploy → Run workflow with `setup_hosts` ticked: it creates `/opt/karbird` and deploys
   new `karbird-*` images. Old images in GHCR can be deleted under Packages.

## On the hosts

Everything lives in `/opt/karbird` (compose file, rendered `.env`, mode 600):

```
cd /opt/karbird
docker compose -f compose.app.yaml ps          # or compose.storage.yaml on the storage host
docker compose -f compose.app.yaml logs -f api
docker compose -f compose.storage.yaml run --rm migrate migrate   # what each deploy runs
```

Data volumes: `karbird_pg-data`, `karbird_clickhouse-data`, `karbird_seaweedfs-data`
(storage), `karbird_kafka-data`, `karbird_caddy-data` (app; holds the TLS certificates).
Backups are not automated yet.

## Memory

Memory limits are not tuned for 1 GB yet. Measured idle: app host ≈ 400 MB (Kafka ≈ 290 MB),
storage host ≈ 560 MB (ClickHouse ≈ 430 MB). On 1/8 OCPU Kafka runs a single-core JVM (serial
GC) with KRaft heartbeats every 10 s instead of 2 s; with the defaults the broker misses its own
heartbeats and never becomes ready. Under load both use the swap from `host-setup.sh`;
expect slower queries rather than crashes. Reducing Kafka heap and ClickHouse caches is a separate
step.

## Karbird agent (machine and container metrics)

The map's Infrastructure layer needs the Karbird agent on every machine that runs observed
services (ADR-0018). It is not part of the two-host deploy: it belongs to the machines you
observe, which may include these two. Create an ingest token with the `metrics` signal for the
target environment, copy `deploy/agent/` to the machine and run:

```sh
KARBIRD_OTLP_ENDPOINT=https://<domain>/otlp KARBIRD_INGEST_TOKEN=chi_… KARBIRD_HOST_NAME=vps-app \
  docker compose -f compose.yaml up -d
```

The agent (pinned `otel/opentelemetry-collector-contrib`, ≤ 128 MB) reads the host filesystem and
the Docker socket read-only and exports only the metrics the API reads. Give the services' SDK the
same `KARBIRD_HOST_NAME`; in containers the SDK reads `container.id` from cgroups, so instances are
matched to their container automatically. Metrics are kept `KARBIRD_METRICS_RETENTION_DAYS` (14).
