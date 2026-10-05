#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Generates package/flatpak/cargo-sources.json from Cargo.lock.
#
# The Flatpak build sandbox has NO network access, so every crate (including the
# `iso2god` git dependency) must be declared as an explicit source with a known
# checksum. flatpak-cargo-generator.py walks Cargo.lock and emits exactly that.
#
# The output is generated, not committed: it weighs several MB and would churn on
# every dependency bump. The release workflow regenerates it before building.
#
# Requires: python3 with `aiohttp` and `tomlkit` (pip install aiohttp tomlkit).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${REPO_ROOT}/package/flatpak/cargo-sources.json"

# Pinned to a commit so regenerating twice yields the same tool, not whatever is
# on flatpak-builder-tools' master that day.
GENERATOR_REF="f03a673abe6ce189cea1c2857e2b44af2dd79d1f"
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/${GENERATOR_REF}/cargo/flatpak-cargo-generator.py"

CACHE_DIR="${REPO_ROOT}/target/flatpak-tools"
GENERATOR="${CACHE_DIR}/flatpak-cargo-generator.py"

mkdir -p "${CACHE_DIR}" "$(dirname "${OUT}")"

if [ ! -f "${GENERATOR}" ]; then
    echo "==> Downloading flatpak-cargo-generator.py"
    curl -fsSL -o "${GENERATOR}" "${GENERATOR_URL}"
fi

# The generator needs aiohttp + tomlkit. Rather than fight PEP 668 on the system
# interpreter, keep them in a throwaway venv under target/.
PYTHON=python3
if ! python3 -c "import aiohttp, tomlkit" 2>/dev/null; then
    VENV="${CACHE_DIR}/venv"
    # Also rebuilt when it no longer runs: a venv hardcodes its interpreter's
    # path, which a distribution upgrade or a moved checkout leaves dangling.
    if ! "${VENV}/bin/python" -m pip --version >/dev/null 2>&1; then
        echo "==> Creating venv"
        python3 -m venv --clear "${VENV}"
    fi
    # Unconditional: the venv may predate a change in the generator's imports.
    echo "==> Installing aiohttp + tomlkit"
    "${VENV}/bin/python" -m pip install --quiet --upgrade pip aiohttp tomlkit
    PYTHON="${VENV}/bin/python"
fi

echo "==> Generating ${OUT#"${REPO_ROOT}/"} from Cargo.lock"
"${PYTHON}" "${GENERATOR}" "${REPO_ROOT}/Cargo.lock" -o "${OUT}"

echo "==> Done ($(wc -c <"${OUT}") bytes)"
