#!/command/with-contenv bashio
set -euo pipefail

readonly profile_path="/config/profile.yaml"
readonly catalog_path="/config/food_catalog.yaml"
readonly bls_path="/config/BLS_4_0_Daten_2025_DE.xlsx"
readonly private_preview_path="/data/dry-run.json"
readonly visible_preview_path="/config/dry-run.json"
readonly visible_review_path="/config/review.md"
readonly staged_preview_path="/config/.dry-run.json.tmp"
readonly staged_review_path="/config/.review.md.tmp"
readonly private_revision_path="/data/revision.json"
readonly visible_revision_path="/config/revision.json"
readonly visible_revision_review_path="/config/revision-review.md"
readonly staged_revision_path="/config/.revision.json.tmp"
readonly staged_revision_review_path="/config/.revision-review.md.tmp"

cleanup() {
    rm -f \
        "${staged_preview_path}" \
        "${staged_review_path}" \
        "${staged_revision_path}" \
        "${staged_revision_review_path}"
}
trap cleanup EXIT

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

run_dry_run() {
    local extra_args=()
    local planner_status
    if bashio::config.true 'skip_destination_check'; then
        extra_args+=(--skip-destination-check)
    fi

    bashio::log.info "Starting write-free dry-run for the next shopping-day cycle."
    set +e
    /usr/bin/nutrition-assistant next-dry-run \
        --profile "${profile_path}" \
        --catalog "${catalog_path}" \
        --bls-data "${bls_path}" \
        --database /data/planner.sqlite3 \
        --output "${private_preview_path}" \
        "${extra_args[@]}"
    planner_status=$?
    set -e

    if [[ ("${planner_status}" -eq 0 || "${planner_status}" -eq 2) && -f "${private_preview_path}" ]]; then
        /usr/bin/nutrition-assistant review \
            --input "${private_preview_path}" \
            --output "${staged_review_path}"
        cp "${private_preview_path}" "${staged_preview_path}"
        chmod 600 "${staged_preview_path}" "${staged_review_path}"
        mv "${staged_preview_path}" "${visible_preview_path}"
        mv "${staged_review_path}" "${visible_review_path}"
        bashio::log.info "Private review exported to /config/review.md."
    fi
    return "${planner_status}"
}

run_refinement() {
    local recipe_id
    local instruction
    local planner_status
    recipe_id="$(bashio::config 'refine_recipe_id')"
    instruction="$(bashio::config 'refine_instruction')"
    if [[ -z "${recipe_id}" || -z "${instruction}" ]]; then
        bashio::log.fatal "Refine operation requires refine_recipe_id and refine_instruction."
        return 1
    fi
    if [[ ! -f "${visible_preview_path}" ]]; then
        bashio::log.fatal "Missing /config/dry-run.json; run a dry-run before refinement."
        return 1
    fi

    bashio::log.info "Starting locked single-recipe refinement for ${recipe_id}."
    set +e
    /usr/bin/nutrition-assistant refine \
        --profile "${profile_path}" \
        --catalog "${catalog_path}" \
        --bls-data "${bls_path}" \
        --database /data/planner.sqlite3 \
        --input "${visible_preview_path}" \
        --recipe-id "${recipe_id}" \
        --instruction "${instruction}" \
        --output "${private_revision_path}"
    planner_status=$?
    set -e

    if [[ ("${planner_status}" -eq 0 || "${planner_status}" -eq 2) && -f "${private_revision_path}" ]]; then
        /usr/bin/nutrition-assistant review-diff \
            --parent "${visible_preview_path}" \
            --revision "${private_revision_path}" \
            --output "${staged_revision_review_path}"
        cp "${private_revision_path}" "${staged_revision_path}"
        chmod 600 "${staged_revision_path}" "${staged_revision_review_path}"
        mv "${staged_revision_path}" "${visible_revision_path}"
        mv "${staged_revision_review_path}" "${visible_revision_review_path}"
        bashio::log.info "Private revision exported to /config/revision-review.md."
    fi
    return "${planner_status}"
}

case "$(bashio::config 'operation')" in
    dry_run)
        run_dry_run
        ;;
    refine)
        run_refinement
        ;;
    *)
        bashio::log.fatal "Unsupported operation."
        exit 1
        ;;
esac
