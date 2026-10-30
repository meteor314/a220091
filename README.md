# A220091

This repository contains the A220091 verifier and its SQLite-backed campaign
runner. Docker Compose stores campaign state in the named volume
`a220091-data`.

## Run the campaign

Create a `.env` file with the campaign range, then start the campaign:

```powershell
@"
CAMPAIGN_START=1
CAMPAIGN_END=100000000
"@ | Set-Content .env
docker compose up -d campaign
docker compose logs -f campaign
```

Check the state without starting another campaign process:

```powershell
docker compose run --rm --entrypoint /usr/local/bin/a220091-campaign campaign status
```

## Import an existing database

Database files under `data/db` are host-side backups. Compose does not mount
that directory; the campaign always opens this path inside the Docker volume:

```text
/var/lib/a220091/campaign/campaign.db
```

The import below replaces the database in the volume with one selected from
`data/db`. It is intended for PowerShell from the repository root.

### 1. Stop the campaign

Do not copy a SQLite database while the campaign is running. Pause it cleanly,
then stop the service:

```powershell
docker compose run --rm --entrypoint /usr/local/bin/a220091-campaign campaign pause
docker compose stop campaign
```

If the campaign is not running, the `pause` command can be omitted.

### 2. Back up the current volume database

This creates a copy in `data/db` before changing the volume. Replace the
timestamped filename if you prefer a different name.

```powershell
$backup = "data/db/volume-backup-$(Get-Date -Format yyyyMMdd-HHmmss).db"
docker run --rm --user 0:0 `
  -v a220091-data:/var/lib/a220091 `
  -v "${PWD}\data\db:/exports" `
  alpine sh -c "cp /var/lib/a220091/campaign/campaign.db /exports/$(Split-Path -Leaf $backup)"
```

### 3. Copy the selected database into the volume

Set `$source` to the database you want to import. The destination filename
must remain `campaign.db`, because that is the path configured by
`compose.yml`.

```powershell
$source = "data/db/campaign-2026-10-22-175549.db"
docker run --rm --user 0:0 `
  -v a220091-data:/var/lib/a220091 `
  -v "${PWD}\data\db:/imports:ro" `
  alpine sh -c "rm -f /var/lib/a220091/campaign/campaign.db-wal /var/lib/a220091/campaign/campaign.db-shm; cp /imports/$(Split-Path -Leaf $source) /var/lib/a220091/campaign/campaign.db; chown 10001:10001 /var/lib/a220091/campaign/campaign.db"
```

If the source database has matching `-wal` or `-shm` files beside it, do not
import only the `.db` file. Recover or checkpoint that database first, or copy
the matching sidecar files together while the campaign is stopped.

### 4. Verify the imported campaign

```powershell
docker compose run --rm --entrypoint /usr/local/bin/a220091-campaign campaign status
```

Confirm that the reported range, chunk size, block size, and mode are the ones
you expect. When resuming, Compose values must match the immutable settings
stored in the imported database. Start it again only after that check:

```powershell
docker compose up -d campaign
```

### Direct inspection without importing

To inspect a host database without changing the Docker volume, pass it to the
campaign binary in a temporary container:

```powershell
docker run --rm `
  -v "${PWD}\data\db:/imports:ro" `
  --entrypoint /usr/local/bin/a220091-campaign `
  a220091:n100 status --db /imports/campaign-2026-10-22-175549.db
```

The image must already exist; build it with `docker compose build` if needed.

See [docs/CAMPAIGN.md](docs/CAMPAIGN.md) for campaign semantics and crash
recovery behavior.