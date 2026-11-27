#!/usr/bin/env bash
set -euo pipefail

readonly project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly output_dir="${project_dir}/.podman-build.local.out"
readonly image_tag="${1:-localhost/nutrition-assistant:0.3.5}"
container_id=""

cleanup() {
    if [[ -n "${container_id}" ]]; then
        podman rm --force "${container_id}" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

mkdir -p "${output_dir}"

podman run --rm --user 0 \
    --volume "${project_dir}:/source:ro" \
    --volume "${output_dir}:/output:rw" \
    --workdir /source \
    docker.io/library/rust:1.100-alpine3.22 \
    sh -ec 'apk add --no-cache musl-dev >/dev/null && CARGO_TARGET_DIR=/tmp/target cargo build --locked --release && cp /tmp/target/release/nutrition-assistant /output/nutrition-assistant'

cp "${project_dir}/run.sh" "${output_dir}/run.sh"
chmod 755 "${output_dir}/run.sh"

container_id="$(podman create ghcr.io/home-assistant/base:3.24 /bin/true)"
podman cp "${output_dir}/nutrition-assistant" "${container_id}:/usr/bin/nutrition-assistant"
podman cp "${output_dir}/run.sh" "${container_id}:/run.sh"
podman commit \
    --change 'ENTRYPOINT ["/init"]' \
    --change 'CMD ["/run.sh"]' \
    "${container_id}" "${image_tag}" >/dev/null
podman rm "${container_id}" >/dev/null
container_id=""

podman run --rm --entrypoint /usr/bin/nutrition-assistant "${image_tag}" --version
podman image inspect "${image_tag}" \
    --format 'image={{.Id}} arch={{.Architecture}} size={{.Size}} entrypoint={{json .Config.Entrypoint}} cmd={{json .Config.Cmd}}'
