// SPDX-License-Identifier: GPL-3.0-only
//
// semantic-release configuration.
//
// Release flow (see .github/workflows/release.yml):
//   1. A dry-run computes the next version and exposes it to the workflow via
//      `verifyReleaseCmd` (which runs even in --dry-run, unlike `prepareCmd`).
//   2. The build jobs bump Cargo.toml to that version and compile so the binary
//      embeds the correct `env!("CARGO_PKG_VERSION")` (used by the self-updater).
//   3. The real run generates CHANGELOG.md, bumps Cargo.toml/Cargo.lock, commits,
//      tags `vX.Y.Z` and creates the GitHub Release with the built assets.
//
// The static footer (donation links + install instructions) from
// package/release-body.md is appended to the GitHub Release body only, so it
// stays out of CHANGELOG.md and the release commit message.

import { readFileSync } from 'node:fs';

const footer = '\n\n---\n\n' + readFileSync('./package/release-body.md', 'utf8');
// releaseBodyTemplate is a lodash template: the footer must hold no delimiter.
if (/<%|\$\{/.test(footer)) {
  throw new Error('package/release-body.md must not contain "<%" or "${"');
}

export default {
  branches: ['main'],
  // tagFormat defaults to 'v${version}', matching crates/core/src/updates.rs.
  plugins: [
    ['@semantic-release/commit-analyzer', { preset: 'conventionalcommits' }],
    [
      '@semantic-release/release-notes-generator',
      { preset: 'conventionalcommits' },
    ],
    ['@semantic-release/changelog', { changelogFile: 'CHANGELOG.md' }],
    [
      '@semantic-release/exec',
      {
        // Runs during --dry-run too: exports the computed version to the workflow.
        verifyReleaseCmd: 'echo "version=${nextRelease.version}" >> "$GITHUB_OUTPUT"',
        // Runs only on a real release: bump Cargo.toml, resync Cargo.lock and
        // record the release in the AppStream metainfo.
        prepareCmd: 'bash scripts/bump-version.sh ${nextRelease.version}',
      },
    ],
    [
      '@semantic-release/git',
      {
        assets: [
          'CHANGELOG.md',
          'Cargo.toml',
          'Cargo.lock',
          'package/linux/AppDir/usr/share/metainfo/fr.dechriste.TinyXbox360BackupManager.metainfo.xml',
        ],
        message: 'chore(release): ${nextRelease.version} [skip ci]\n\n${nextRelease.notes}',
      },
    ],
    [
      '@semantic-release/github',
      {
        assets: [{ path: 'dist/**' }],
        releaseBodyTemplate: '<%= nextRelease.notes %>' + footer,
      },
    ],
  ],
};
