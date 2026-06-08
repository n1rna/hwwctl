#!/usr/bin/env bash
# Build and package the BitBox02 simulator bundle.
#
# Expected environment:
#   BUNDLE_VERSION  – version string for bundle-info.json (default: "dev")
#   FIRMWARE_DIR    – path to checked-out bitbox02-firmware repo (default: "bitbox02-firmware")
#
# Produces: hwwctl-bitbox02-linux-x86_64.tar.gz in the current directory.
set -euo pipefail

BUNDLE_VERSION="${BUNDLE_VERSION:-dev}"
FIRMWARE_DIR="$(cd "${FIRMWARE_DIR:-bitbox02-firmware}" && pwd)"
WORK_DIR="$(pwd)"
PLATFORM="linux-x86_64"
BUNDLE_DIR="${WORK_DIR}/hwwctl-bitbox02-${PLATFORM}"

echo "==> Building BitBox02 simulator from ${FIRMWARE_DIR}"

cd "${FIRMWARE_DIR}"
make simulator

# Locate the simulator binary (build dir name varies: build-build-noasan, build-sim, etc).
BIN=$(find "${FIRMWARE_DIR}" -path '*/build*/bin/simulator' -type f 2>/dev/null | head -1)
if [ -z "${BIN}" ]; then
    BIN=$(find "${FIRMWARE_DIR}" -path '*/build*' -type f -executable -name '*simulator*' 2>/dev/null | grep -v CMake | head -1)
fi
if [ -z "${BIN}" ]; then
    echo "ERROR: simulator binary not found"
    echo "==> Looking for executables in build dirs:"
    find "${FIRMWARE_DIR}" -path '*/build*' -type f -executable 2>/dev/null | grep -v CMake | grep -v '.o' | head -20
    exit 1
fi
echo "==> Found simulator binary: ${BIN}"

echo "==> Packaging bundle: ${BUNDLE_DIR}"
cd "${WORK_DIR}"
rm -rf "${BUNDLE_DIR}"
mkdir -p "${BUNDLE_DIR}"

cp "${BIN}" "${BUNDLE_DIR}/bitbox02-simulator"
chmod +x "${BUNDLE_DIR}/bitbox02-simulator"

CONTENTS=$(cd "${BUNDLE_DIR}" && find . -type f | sort | jq -R -s 'split("\n") | map(select(length > 0))')
cat > "${BUNDLE_DIR}/bundle-info.json" <<EOF
{
  "wallet_type": "bitbox02",
  "version": "${BUNDLE_VERSION}",
  "platform": "${PLATFORM}",
  "build_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "contents": ${CONTENTS}
}
EOF

# `manifest.json` is what the hwwctl daemon's BundleManager reads at
# runtime to locate the emulator binary. Ship it inside the tarball
# so a consumer can `tar -x --strip-components=1` straight into
# ~/.hwwctl/bundles/bitbox02/ and have the daemon find the binary
# without an extra install step. `installed_at` here is actually the
# build timestamp — BundleManager doesn't enforce strict semantics on
# the field, and an auto-installed bundle overwrites this manifest
# with its own at install time anyway.
SIZE_BYTES=$(du -sb "${BUNDLE_DIR}" | cut -f1)
cat > "${BUNDLE_DIR}/manifest.json" <<EOF
{
  "wallet_type": "bitbox02",
  "version": "${BUNDLE_VERSION}",
  "platform": "${PLATFORM}",
  "installed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "size_bytes": ${SIZE_BYTES},
  "emulator_binary": "bitbox02-simulator",
  "firmware_dir": null,
  "build_info": null
}
EOF

tar czf "${WORK_DIR}/hwwctl-bitbox02-${PLATFORM}.tar.gz" -C "${WORK_DIR}" "hwwctl-bitbox02-${PLATFORM}"
echo "==> Done: hwwctl-bitbox02-${PLATFORM}.tar.gz"
