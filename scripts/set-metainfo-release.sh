#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Prepend a <release> entry to the AppStream metainfo, newest first.
#
# Called from two places, because the version is needed at two different times:
#   - the build jobs, so the AppImage and Flatpak of THIS release ship a
#     metainfo that already lists it;
#   - scripts/bump-version.sh (semantic-release prepareCmd), so the release
#     commit records it in git.
# It is therefore idempotent: re-running it for a version already listed is a
# no-op, not a duplicate entry.
set -euo pipefail

VERSION="${1:?usage: set-metainfo-release.sh <version> [date]}"
DATE="${2:-$(date -u +%F)}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METAINFO="${REPO_ROOT}/package/linux/AppDir/usr/share/metainfo/fr.dechriste.TinyXbox360BackupManager.metainfo.xml"
REPO_URL="https://github.com/jeanmatthieud/TinyXbox360BackupManager"

if [ ! -f "${METAINFO}" ]; then
    echo "error: metainfo not found at ${METAINFO}" >&2
    exit 1
fi

if grep -q "<release version=\"${VERSION}\"" "${METAINFO}"; then
    echo "==> ${VERSION} already listed in metainfo, nothing to do"
    exit 0
fi

trap 'rm -f "${METAINFO}.tmp"' EXIT

awk -v v="${VERSION}" -v d="${DATE}" -v url="${REPO_URL}" '
    { print }
    !inserted && /<releases>/ {
        print "        <release version=\"" v "\" date=\"" d "\">"
        print "            <url type=\"details\">" url "/releases/tag/v" v "</url>"
        print "        </release>"
        inserted = 1
    }
    END {
        if (!inserted) {
            print "error: no <releases> element found" > "/dev/stderr"
            exit 1
        }
    }
' "${METAINFO}" > "${METAINFO}.tmp"

mv "${METAINFO}.tmp" "${METAINFO}"
echo "==> Added release ${VERSION} (${DATE}) to metainfo"

# Best-effort: catch a malformed insertion right away when the tool is around.
if command -v appstreamcli >/dev/null 2>&1; then
    appstreamcli validate "${METAINFO}" || true
fi
