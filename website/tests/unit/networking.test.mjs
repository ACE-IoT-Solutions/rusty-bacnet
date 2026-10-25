import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';

const root = new URL('../../', import.meta.url);
const repo = 'https://github.com/jscott3201/rusty-bacnet';
const devHeading = '\n## On `dev` after the release\n';
// The networking pages describe the release. A page with post-release
// additions keeps them, and their dev links, in one section; everything
// outside that section (later sections included) is release-scoped.
const pages = {
  'connect/across-subnets': { dev: ['set_release_previous_bbmd', 'previous_release_refused'] },
  'connect/nat': { dev: null },
  'connect/multicast': { dev: ['set_multicast_persist_path', 'multicast_persist_failures'] },
  'connect/ipv6': { dev: null },
};

async function page(slug) {
  const file = fileURLToPath(new URL(`src/content/docs/${slug}.mdx`, root));
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

function split(body, slug) {
  const parts = body.split(devHeading);
  if (parts.length === 1) return { released: body, onDev: '' };
  assert.equal(parts.length, 2, `${slug}: one dev section`);
  const next = parts[1].indexOf('\n## ');
  assert.notEqual(next, -1, `${slug}: a section follows the dev one`);
  return { released: parts[0] + parts[1].slice(next), onDev: parts[1].slice(0, next) };
}

test('networking pages are release-scoped, with dev additions in one section', async () => {
  for (const [slug, { dev }] of Object.entries(pages)) {
    const { title, description, body } = await page(slug);
    assert.ok(title && description, slug);
    assert.ok(body.includes(`The page describes **v${release}**.`), slug);
    assert.ok(body.includes('\n## Next steps\n'), slug);
    const { released, onDev } = split(body, slug);
    assert.ok(released.includes(`${repo}/blob/v${release}/docs/`), `${slug}: release-pinned docs links`);
    assert.ok(!released.includes(`${repo}/blob/dev/`), `${slug}: dev links only in the dev section`);
    if (dev === null) {
      assert.equal(onDev, '', `${slug}: no dev section`);
      continue;
    }
    assert.ok(onDev.includes(`${repo}/blob/dev/docs/`), `${slug}: the dev section links dev docs`);
    assert.ok(body.includes('(#on-dev-after-the-release)'), `${slug}: the intro points at the dev section`);
    for (const name of dev) {
      assert.ok(onDev.includes(name), `${slug}: ${name} in the dev section`);
      assert.ok(!released.includes(name), `${slug}: ${name} only in the dev section`);
    }
  }
});

test('every CLI flag in a networking sample is defined by the CLI', async () => {
  const args = await readFile(new URL('../crates/bacnet-cli/src/args.rs', root), 'utf8');
  for (const slug of Object.keys(pages)) {
    const { body } = await page(slug);
    const blocks = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].map(match => match[1]);
    for (const block of blocks) {
      assert.match(block, /^bacnet /m, `${slug}: shell samples run the CLI`);
      for (const [, flag] of block.matchAll(/\s--([a-z][a-z0-9-]*)/g)) {
        const field = flag.replaceAll('-', '_');
        assert.match(args, new RegExp(`pub\\(crate\\) ${field}:`), `${slug}: --${flag} is a CLI option`);
      }
    }
  }
});
