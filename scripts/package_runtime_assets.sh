#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/runtime-assets"
VERSION="${1:-}"
TARGET="${2:-x86_64-unknown-linux-gnu}"

if [[ -z "${VERSION}" ]]; then
  echo "usage: $0 <version> [target]" >&2
  exit 1
fi

CURL_IMPERSONATE_IMAGE="${CURL_IMPERSONATE_IMAGE:-lexiforest/curl-impersonate:v1.5.1@sha256:0e43f7ef4007ce957fcd6532f4295d0193f9178f4e6eccb1a6f754d64f7f9fcd}"
ASSET_BASE="impcurl-runtime-v${VERSION}-${TARGET}"
ASSET_DIR="${OUT_DIR}/${ASSET_BASE}"
ASSET_TGZ="${OUT_DIR}/${ASSET_BASE}.tar.gz"
MIN_BYTES="${MIN_BYTES:-500000}"

case "${TARGET}" in
  x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl)
    DOCKER_PLATFORM="linux/amd64"
    ;;
  aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl)
    DOCKER_PLATFORM="linux/arm64"
    ;;
  *)
    echo "unsupported TARGET for runtime packaging: ${TARGET}" >&2
    exit 1
    ;;
esac

mkdir -p "${ASSET_DIR}"
rm -f "${ASSET_DIR}/libcurl-impersonate.so.4" "${ASSET_DIR}/libcurl-impersonate.so"

# Copy real library bytes out of container (follow symlinks with cp -L).
docker run --rm --platform="${DOCKER_PLATFORM}" \
  -v "${ASSET_DIR}:/out" \
  "${CURL_IMPERSONATE_IMAGE}" \
  sh -c '
    set -e
    cp -L /usr/local/lib/libcurl-impersonate.so.4 /out/libcurl-impersonate.so.4
    cp -L /usr/local/lib/libcurl-impersonate.so /out/libcurl-impersonate.so
  '

size_so4="$(wc -c < "${ASSET_DIR}/libcurl-impersonate.so.4" | tr -d " ")"
size_so="$(wc -c < "${ASSET_DIR}/libcurl-impersonate.so" | tr -d " ")"
if [[ "${size_so4}" -lt "${MIN_BYTES}" || "${size_so}" -lt "${MIN_BYTES}" ]]; then
  echo "runtime library looks wrong: so.4=${size_so4} bytes, so=${size_so} bytes" >&2
  exit 1
fi

tar -czf "${ASSET_TGZ}" -C "${ASSET_DIR}" .
(cd "${OUT_DIR}" && shasum -a 256 "$(basename "${ASSET_TGZ}")" > "${ASSET_BASE}.sha256")

echo "packaged: ${ASSET_TGZ}"
echo "checksum: ${OUT_DIR}/${ASSET_BASE}.sha256"
