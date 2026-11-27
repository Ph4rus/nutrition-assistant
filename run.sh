#!/command/with-contenv bashio
set -euo pipefail

readonly profile_path="/config/profile.yaml"
readonly catalog_path="/config/food_catalog.yaml"
readonly bls_path="/config/BLS_4_0_Daten_2025_DE.xlsx"

if [[ ! -f "${profile_path}" ]]; then
    bashio::log.fatal "Missing /config/profile.yaml; no provider was called."
    exit 1
fi
if [[ ! -f "${catalog_path}" ]]; then
    bashio::log.fatal "Missing /config/food_catalog.yaml; no provider was called."
    exit 1
fi
if [[ ! -f "${bls_path}" ]]; then
    bashio::log.fatal "Missing ${bls_path}; no provider was called."
    exit 1
fi

export OPENAI_API_KEY="$(bashio::config 'openai_api_key')"
export MEALIE_URL="$(bashio::config 'mealie_url')"
export MEALIE_API_TOKEN="$(bashio::config 'mealie_api_token')"
export OPENAI_MODEL="$(bashio::config 'openai_model')"

extra_args=()
if bashio::config.true 'skip_destination_check'; then
    extra_args+=(--skip-destination-check)
fi

bashio::log.info "Starting write-free dry-run for the next shopping-day cycle."
/usr/bin/nutrition-assistant next-dry-run \
    --profile "${profile_path}" \
    --catalog "${catalog_path}" \
    --bls-data "${bls_path}" \
    --database /data/planner.sqlite3 \
    --output /data/dry-run.json \
    "${extra_args[@]}"
