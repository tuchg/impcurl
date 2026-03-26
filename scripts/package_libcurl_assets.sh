#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/libcurl-impersonate-assets"
VERSION="${1:-}"
TARGET="${2:-x86_64-unknown-linux-gnu}"

if [[ -z "${VERSION}" ]]; then
  echo "usage: $0 <version> [target]" >&2
  exit 1
fi

CURL_IMPERSONATE_IMAGE="${CURL_IMPERSONATE_IMAGE:-lexiforest/curl-impersonate:v1.5.1@sha256:0e43f7ef4007ce957fcd6532f4295d0193f9178f4e6eccb1a6f754d64f7f9fcd}"
CURL_IMPERSONATE_VERSION="${CURL_IMPERSONATE_VERSION:-1.5.1}"
CURL_CFFI_VERSION="${CURL_CFFI_VERSION:-0.11.3}"
ASSET_BASE="impcurl-libcurl-impersonate-v${VERSION}-${TARGET}"
ASSET_DIR="${OUT_DIR}/${ASSET_BASE}"
ASSET_TGZ="${OUT_DIR}/${ASSET_BASE}.tar.gz"
MIN_BYTES="${MIN_BYTES:-500000}"

DOCKER_PLATFORM=""
PACKAGING_MODE=""
CURL_CFFI_PLATFORM=""

case "${TARGET}" in
  x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl)
    PACKAGING_MODE="docker"
    DOCKER_PLATFORM="linux/amd64"
    ;;
  aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl)
    PACKAGING_MODE="docker"
    DOCKER_PLATFORM="linux/arm64"
    ;;
  x86_64-apple-darwin)
    PACKAGING_MODE="curl-cffi-wheel"
    CURL_CFFI_PLATFORM="macosx_10_9_x86_64"
    ;;
  aarch64-apple-darwin)
    PACKAGING_MODE="curl-cffi-wheel"
    CURL_CFFI_PLATFORM="macosx_11_0_arm64"
    ;;
  *)
    echo "unsupported TARGET for libcurl-impersonate asset packaging: ${TARGET}" >&2
    exit 1
    ;;
esac

mkdir -p "${ASSET_DIR}"
rm -f "${ASSET_DIR}"/libcurl-impersonate* "${ASSET_DIR}"/*.dylib "${ASSET_DIR}"/*.so* "${ASSET_DIR}"/*.dll 2>/dev/null || true

if [[ "${PACKAGING_MODE}" == "docker" ]]; then
  docker run --rm --platform="${DOCKER_PLATFORM}" \
    -v "${ASSET_DIR}:/out" \
    "${CURL_IMPERSONATE_IMAGE}" \
    sh -c '
      set -e
      cp -L /usr/local/lib/libcurl-impersonate.so.4 /out/libcurl-impersonate.so.4
      cp -L /usr/local/lib/libcurl-impersonate.so /out/libcurl-impersonate.so
    '
elif [[ "${PACKAGING_MODE}" == "curl-cffi-wheel" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "error: python3 is required for macOS libcurl-impersonate asset packaging" >&2
    exit 1
  fi
  if ! command -v unzip >/dev/null 2>&1; then
    echo "error: unzip is required for macOS libcurl-impersonate asset packaging" >&2
    exit 1
  fi

  TMP_DOWNLOAD_DIR="$(mktemp -d "${OUT_DIR}/.${ASSET_BASE}.wheel.XXXXXX")"
  TMP_EXTRACT_DIR="$(mktemp -d "${OUT_DIR}/.${ASSET_BASE}.extract.XXXXXX")"

  python3 -m pip download \
    --disable-pip-version-check \
    --no-deps \
    --only-binary=:all: \
    --platform "${CURL_CFFI_PLATFORM}" \
    --implementation cp \
    --python-version 311 \
    --abi abi3 \
    "curl-cffi==${CURL_CFFI_VERSION}" \
    -d "${TMP_DOWNLOAD_DIR}"

  WHEEL_PATH="$(find "${TMP_DOWNLOAD_DIR}" -maxdepth 1 -type f -name 'curl_cffi-*.whl' | head -n 1)"
  if [[ -z "${WHEEL_PATH}" ]]; then
    echo "failed to download curl-cffi wheel for ${TARGET}" >&2
    exit 1
  fi

  unzip -q "${WHEEL_PATH}" -d "${TMP_EXTRACT_DIR}"
  find "${TMP_EXTRACT_DIR}" -name '*.dylib' -exec cp -fL {} "${ASSET_DIR}/" \;

  rm -rf "${TMP_DOWNLOAD_DIR}" "${TMP_EXTRACT_DIR}"
else
  echo "unexpected packaging mode: ${PACKAGING_MODE}" >&2
  exit 1
fi

shared_count="$(find "${ASSET_DIR}" -maxdepth 1 -type f \( -name 'libcurl-impersonate*.so*' -o -name 'libcurl-impersonate*.dylib' -o -name 'libcurl-impersonate*.dll' \) | wc -l | tr -d ' ')"
if [[ "${shared_count}" == "0" ]]; then
  echo "asset packaging failed: no standalone libcurl-impersonate shared library found in ${ASSET_DIR}" >&2
  exit 1
fi

main_lib="$(find "${ASSET_DIR}" -maxdepth 1 -type f \( -name 'libcurl-impersonate.so.4' -o -name 'libcurl-impersonate.4.dylib' -o -name 'libcurl-impersonate.dll' -o -name 'libcurl-impersonate.dylib' -o -name 'libcurl-impersonate.so' \) | head -n 1)"
if [[ -z "${main_lib}" ]]; then
  echo "asset packaging failed: main libcurl-impersonate shared library is missing in ${ASSET_DIR}" >&2
  exit 1
fi

main_size="$(wc -c < "${main_lib}" | tr -d ' ')"
if [[ "${main_size}" -lt "${MIN_BYTES}" ]]; then
  echo "shared library looks wrong: ${main_lib} is ${main_size} bytes" >&2
  exit 1
fi

tar -czf "${ASSET_TGZ}" -C "${ASSET_DIR}" .
if command -v shasum >/dev/null 2>&1; then
  (cd "${OUT_DIR}" && shasum -a 256 "$(basename "${ASSET_TGZ}")" > "${ASSET_BASE}.sha256")
else
  (cd "${OUT_DIR}" && sha256sum "$(basename "${ASSET_TGZ}")" > "${ASSET_BASE}.sha256")
fi

echo "packaged: ${ASSET_TGZ}"
echo "checksum: ${OUT_DIR}/${ASSET_BASE}.sha256"
