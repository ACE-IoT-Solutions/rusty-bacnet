// The one rule for a page that shows a sample from website/examples (#2648):
// it gives the sample's note with <SampleNote sample="…" />, from
// src/lib/samples.mjs. A sample changed since the release follows the dev
// branch, and its note says so and where the release's copy is; a sample that
// is the release's stays on it, and its note says how to get the release.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { sampleManifests } from '../../scripts/content.mjs';
import { devSource, manifestState, noteMarkdown, sampleNote, samples, tagUrl } from '../../src/lib/samples.mjs';
import { release } from '../../src/lib/site.mjs';

const site = 'https://jscott3201.github.io/rusty-bacnet/';
const repo = new URL('../../../', import.meta.url);

/** Whether a sample follows dev: a Rust sample's manifest says so, a Python sample's `follows` key. */
export function followsDev(name, manifests) {
  const sample = samples[name];
  return sample.language === 'rust' ? manifestState(manifests[sample.path]) === null : sample.follows === 'dev';
}

/** Check that a page's plain-text body gives each named sample's note once. */
export async function assertSampleNotes(body, names, label) {
  const manifests = await sampleManifests();
  for (const name of names) {
    const sample = samples[name];
    const note = noteMarkdown(sampleNote(name, manifests)).replaceAll('](/rusty-bacnet/', `](${site}`);
    assert.equal(body.split(note).length - 1, 1, `${label}: ${name}'s note, once`);
    if (!followsDev(name, manifests)) {
      assert.ok(note.includes(`v${release}`), `${label}: ${name}'s note names the release`);
      continue;
    }
    assert.ok(note.includes('follows the `dev` branch'), `${label}: ${name} follows dev`);
    const where = sample.tagPage !== undefined ? `/blob/v${release}/website/src/content/docs/${sample.tagPage})`
      : sample.since !== undefined ? `new since v${release}` : `](${tagUrl(sample)})`;
    assert.ok(note.includes(where), `${label}: ${name}'s note says where the release's copy is`);
  }
}

/** Read the public release that the site's install and sample notes name. */
export function releasedManifest(path) {
  const tag = `refs/tags/v${release}`;
  if (spawnSync('git', ['show-ref', '--verify', '--quiet', tag], { cwd: repo }).status === 0) {
    return execFileSync('git', ['show', `${tag}:${path}`], { cwd: repo, encoding: 'utf8' });
  }
  // A public release PR can pin its samples before its tag exists.
  const read = name => readFileSync(new URL(name, repo), 'utf8');
  assert.match(read('crates/rusty-bacnet/pyproject.toml'), /^name = "rusty-bacnet"$/m, `fetch the site's ${tag}`);
  assert.equal(read('Cargo.toml').split('[workspace.package]')[1].match(/^version = "([^"]+)"$/m)[1], release);
  return read(path);
}

/** The public crate's independent or inherited version at the advertised tag. */
export function publishedVersion(crate) {
  const manifest = releasedManifest(`crates/${crate}/Cargo.toml`);
  if (/^version\.workspace = true$/m.test(manifest)) {
    const root = releasedManifest('Cargo.toml');
    return root.slice(root.indexOf('[workspace.package]')).match(/^version = "([^"]+)"$/m)[1];
  }
  return manifest.match(/^version = "([^"]+)"$/m)[1];
}

/** Check that a manifest takes every BACnet crate from dev, or pins each to its published version. */
export async function assertManifestSource(manifest, label) {
  const pinned = manifestState(manifest);
  const crates = manifest.split('\n').filter(line => line.startsWith('bacnet-'));
  assert.ok(crates.length > 0, label);
  if (pinned === null) {
    for (const line of crates) assert.ok(line.includes(`{ ${devSource}`), `${label}: ${line}`);
    return;
  }
  for (const [crate, version] of Object.entries(pinned)) {
    assert.equal(version, publishedVersion(crate), `${label}: ${crate} is pinned to its published version`);
  }
}
