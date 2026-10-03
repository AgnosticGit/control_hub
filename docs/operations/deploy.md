# Production deploy: two hosts from GitHub Actions

ControlHub runs on two small Linux hosts (tested target: 1/8 OCPU, 1 GB RAM, x86_64, Ubuntu
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
4. **SSH key for the deploy:** `ssh-keygen -t ed25519 -f controlhub-deploy -N ""`, add
   `controlhub-deploy.pub` to `~/.ssh/authorized_keys` of the deploy user (default `ubuntu`) on
   both hosts. The user needs passwordless `sudo` for the first run (cloud images have it).
5. **Pinned host keys:** `ssh-keyscan -t ed25519 <storage-ip> <app-ip>` and compare the
   fingerprints with the cloud console before saving them as `DEPLOY_KNOWN_HOSTS`.
6. **GitHub settings** (Settings → Secrets and variables → Actions, repository level):

   | Secret | Value |
   |---|---|
   | `CONTROLHUB_SECRET_SEED` | 48+ random characters (`openssl rand -base64 48`). All service passwords derive from it. Never change it after the first deploy. |
   | `CONTROLHUB_OWNER_EMAIL` | the owner account, created on the first deploy |
   | `CONTROLHUB_OWNER_PASSWORD` | 12+ characters, no `'` and no line breaks. Only used to create the account; later changes are ignored (change it in `/app/account`). |
   | `DEPLOY_SSH_KEY` | private key from step 4 (the whole file) |
   | `DEPLOY_KNOWN_HOSTS` | output of step 5 |

   | Variable | Value |
   |---|---|
   | `CONTROLHUB_DOMAIN` | e.g. `controlhub.example.com` (no scheme) |
   | `APP_HOST` | public IP or name of the app host |
   | `STORAGE_HOST` | public IP or name of the storage host (SSH only) |
   | `STORAGE_PRIVATE_IP` | private IP of the storage host |
   | `DEPLOY_USER` | optional, default `ubuntu` |
   | `ACME_EMAIL` | optional, default the owner email (Let's Encrypt notices) |
   | `CONTROLHUB_LOGS_RETENTION_DAYS`, `…_SPANS_…`, `…_EVENTS_…` | optional, defaults 14 / 14 / 30 (ADR-0016) |

   The deploy job uses the `production` environment (created automatically; add required
   reviewers there to gate deploys). Registry access uses the job's own `GITHUB_TOKEN`.
7. **First deploy:** Actions → Deploy → Run workflow, tick `setup_hosts`. It installs Docker and
   the Compose plugin, adds 2 GB swap and creates `/opt/controlhub` on both hosts, then deploys.

## Releases and rollback

- Release: `git tag v0.1.0 && git push origin v0.1.0`. The workflow runs the full CI, builds and
  pushes `ghcr.io/<owner>/controlhub-*:v0.1.0`, migrates and restarts what changed, then checks
  `https://<domain>/api/v1/ready`.
- Manual run on a branch deploys `sha-<12>` images of that commit.
- Rollback: Run workflow on the previous tag. Migrations are forward-only, so a release must keep
  the schema compatible with the previous one.
- Shortening retention deletes data: the deploy stops with the number of affected rows; rerun
  with `shrink_retention` ticked to confirm.

## On the hosts

Everything lives in `/opt/controlhub` (compose file, rendered `.env`, mode 600):

```
cd /opt/controlhub
docker compose -f compose.app.yaml ps          # or compose.storage.yaml on the storage host
docker compose -f compose.app.yaml logs -f api
docker compose -f compose.storage.yaml run --rm migrate migrate   # what each deploy runs
```

Data volumes: `controlhub_pg-data`, `controlhub_clickhouse-data`, `controlhub_seaweedfs-data`
(storage), `controlhub_kafka-data`, `controlhub_caddy-data` (app; holds the TLS certificates).
Backups are not automated yet.

## Memory

Nothing is tuned for 1 GB yet. Measured idle with defaults: app host ≈ 470 MB (Kafka ≈ 360 MB),
storage host ≈ 560 MB (ClickHouse ≈ 430 MB). Under load both use the swap from `host-setup.sh`;
expect slower queries rather than crashes. Reducing Kafka heap and ClickHouse caches is a separate
step.
