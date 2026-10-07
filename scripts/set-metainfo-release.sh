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
# It is therefore idempotent: re-running it for a version already listed adds
# no duplicate entry.
#
# When a new entry is added, the screenshot URLs are pinned to that release's
# tag, so each tagged metainfo points at the screenshots it was released with.
#
# Release notes come from CHANGELOG.md. @semantic-release/changelog writes it
# before prepareCmd runs, so the release commit gets them; the build jobs run
# earlier and ship the entry without notes. An entry already listed but still
# without notes gets them on a re-run, which is also how older releases were
# backfilled:
#   for v in 0.17.0 0.16.0 ...; do scripts/set-metainfo-release.sh "$v"; done
set -euo pipefail

VERSION="${1:?usage: set-metainfo-release.sh <version> [date]}"
DATE="${2:-$(date -u +%F)}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METAINFO="${REPO_ROOT}/package/linux/AppDir/usr/share/metainfo/fr.dechriste.TinyXbox360BackupManager.metainfo.xml"
CHANGELOG="${REPO_ROOT}/CHANGELOG.md"
REPO_URL="https://github.com/jeanmatthieud/TinyXbox360BackupManager"
RAW_URL="https://raw.githubusercontent.com/jeanmatthieud/TinyXbox360BackupManager"

if [ ! -f "${METAINFO}" ]; then
    echo "error: metainfo not found at ${METAINFO}" >&2
    exit 1
fi

trap 'rm -f "${METAINFO}.tmp"' EXIT

if grep -q "<release version=\"${VERSION}\"" "${METAINFO}"; then
    echo "==> ${VERSION} already listed in metainfo"
else
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

    # The tag does not exist yet, but it will by the time anything (Flathub's
    # build, a software center) fetches these URLs.
    sed -i "s#${RAW_URL}/[^/]*/assets/#${RAW_URL}/v${VERSION}/assets/#" "${METAINFO}"
    echo "==> Pinned screenshots to v${VERSION}"
fi

# Turn this version's CHANGELOG.md section into an AppStream <description>:
# one <p> per "### Heading", one <li> per bullet, minus the commit links and the
# Markdown markup. The section ends at the next release or at the footer (---).
notes=""
if [ -f "${CHANGELOG}" ]; then
    notes="$(awk -v v="${VERSION}" '
        function xml(s) {
            gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s)
            return s
        }
        function close_list() { if (in_list) { print "                </ul>"; in_list = 0 } }
        /^## / {
            if (in_section) exit
            # "## [0.17.0](...)" or "## 0.17.0 (...)"
            head = $2; gsub(/[][]/, "", head); sub(/\(.*/, "", head)
            in_section = (head == v)
            next
        }
        !in_section { next }
        /^---/ { exit }
        /^### / {
            close_list()
            title = $0; sub(/^### +/, "", title)
            print "                <p>" xml(title) "</p>"
            next
        }
        /^\* / {
            item = $0; sub(/^\* +/, "", item)
            gsub(/\[[0-9a-f]+\]\([^)]*\)/, "", item)     # commit links
            gsub(/ *\(( *, *)*\)/, "", item)            # their leftover "( )"
            while (match(item, /\[[^]]*\]\([^)]*\)/)) {  # [text](url) -> text
                link = substr(item, RSTART, RLENGTH)
                sub(/\]\(.*/, "", link); sub(/^\[/, "", link)
                item = substr(item, 1, RSTART - 1) link substr(item, RSTART + RLENGTH)
            }
            gsub(/\*\*|`/, "", item)
            sub(/ +$/, "", item)
            if (!in_list) { print "                <ul>"; in_list = 1 }
            print "                    <li>" xml(item) "</li>"
        }
        END { close_list() }
    ' "${CHANGELOG}")"
fi

if [ -z "${notes}" ]; then
    echo "==> No CHANGELOG.md notes for ${VERSION}"
else
    # Insert before the entry's </release>, unless it already has a description.
    # Through the environment: awk -v would interpret backslashes in the notes.
    NOTES="${notes}" awk -v v="${VERSION}" '
        BEGIN { notes = ENVIRON["NOTES"] }
        index($0, "<release version=\"" v "\"") { in_entry = 1 }
        in_entry && /<description>/ { has_desc = 1 }
        in_entry && /<\/release>/ {
            if (!has_desc) {
                print "            <description>"
                print notes
                print "            </description>"
                added = 1
            }
            in_entry = 0
        }
        { print }
        END { if (added) print "==> Added release notes for " v > "/dev/stderr" }
    ' "${METAINFO}" > "${METAINFO}.tmp"
    mv "${METAINFO}.tmp" "${METAINFO}"
fi

# Best-effort: catch a malformed insertion right away when the tool is around.
# --no-net: the screenshots point at a tag that does not exist yet.
if command -v appstreamcli >/dev/null 2>&1; then
    appstreamcli validate --no-net "${METAINFO}" || true
fi
