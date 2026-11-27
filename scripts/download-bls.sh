#!/usr/bin/env bash
set -euo pipefail

readonly script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly project_dir="$(cd -- "${script_dir}/.." && pwd)"
readonly destination="${1:-${project_dir}/config/BLS_4_0_Daten_2025_DE.xlsx}"
readonly download_url="https://blsdb.de/assets/uploads/BLS_4_0_2025_DE.zip"

if [[ -e "${destination}" ]]; then
    echo "Refusing to overwrite existing BLS file: ${destination}" >&2
    exit 1
fi

readonly temporary_dir="$(mktemp -d)"
trap 'rm -rf -- "${temporary_dir}"' EXIT

curl --fail --location --silent --show-error \
    "${download_url}" \
    --output "${temporary_dir}/bls.zip"
unzip -p \
    "${temporary_dir}/bls.zip" \
    'BLS_4_0_2025_DE/BLS_4_0_Daten_2025_DE.xlsx' \
    > "${temporary_dir}/BLS_4_0_Daten_2025_DE.xlsx"

mkdir -p -- "$(dirname -- "${destination}")"
install -m 0600 \
    "${temporary_dir}/BLS_4_0_Daten_2025_DE.xlsx" \
    "${destination}"

echo "Downloaded official BLS 4.0 data to ${destination}"
