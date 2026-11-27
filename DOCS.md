# Nutrition Assistant Home Assistant app

The included app packages Nutrition Assistant as a manual, one-shot dry-run. It creates a private
preview and exits. It does not publish to Mealie and does not modify Home Assistant.

## Required files

Place the following private files in the app's `app_config` directory:

- `profile.yaml`, based on `config/profile.example.yaml`;
- `food_catalog.yaml`, based on `config/food_catalog.example.yaml`;
- `BLS_4_0_Daten_2025_DE.xlsx`, downloaded from the official BLS distribution.

The example catalog contains a synthetic placeholder. Replace it with explicitly reviewed BLS
mappings before generating a real plan.

## App options

- `openai_api_key`: OpenAI API key used only at runtime;
- `openai_model`: Responses API model name;
- `mealie_url`: URL reachable from the app container;
- `mealie_api_token`: dedicated Mealie API token;
- `skip_destination_check`: development-only escape hatch for tests without Mealie.

For the Home Assistant Community Apps Mealie installation, the internal Supervisor-network URL is
normally `http://a0d7b954-mealie:9001`. No external Mealie port is required for app-to-app
communication.

Never commit option values, API tokens, private profiles, generated previews, or the BLS dataset.

## Persistence and backups

The SQLite ledger, feedback, and `dry-run.json` are stored under the app-private `/data` directory.
Home Assistant also backs up the public app configuration directory. Keep both together: losing
the ledger loses idempotency and remote-object ownership information.

The app declares `backup: cold`, so Home Assistant stops it while copying `/data`. This prevents a
backup from capturing SQLite during an active transaction. A run interrupted before an immutable
artifact or remote object exists can be retried safely.

## Operation

The app uses `startup: once` and `boot: manual`. Starting it runs `next-dry-run` for the shopping
day configured in the private profile. Review `/data/dry-run.json` outside Git before approving any
later publishing phase.

Do not automate app startup until several manual dry-runs have completed successfully and backup
restore has been tested.
