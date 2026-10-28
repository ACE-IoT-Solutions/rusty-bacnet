import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { devRef, repo, splitDevSection } from './dev-section.mjs';
import { checkCliLine, cliDefinitions } from './cli-args.mjs';

const root = new URL('../../', import.meta.url);
// The networking pages describe the release. A page with post-release
// additions keeps them, and their dev links, in one section; everything
// outside that section (later sections included) is release-scoped.
const pages = {
  'connect/across-subnets': { dev: ['set_release_previous_bbmd', 'previous_release_refused'] },
  'connect/nat': { dev: null },
  'connect/multicast': { dev: ['set_multicast_persist_path', 'multicast_persist_failures', 'BbmdChanges::multicast_group'] },
  'connect/ipv6': { dev: null },
};

async function page(slug) {
  const file = fileURLToPath(new URL(`src/content/docs/${slug}.mdx`, root));
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

test('networking pages are release-scoped, with dev additions in one section', async () => {
  for (const [slug, { dev }] of Object.entries(pages)) {
    const { title, description, body } = await page(slug);
    assert.ok(title && description, slug);
    assert.ok(body.includes(`The page describes **v${release}**.`), slug);
    assert.ok(body.includes('\n## Next steps\n'), slug);
    const { released, onDev } = splitDevSection(body, slug);
    assert.ok(released.includes(`${repo}/blob/v${release}/docs/`), `${slug}: release-pinned docs links`);
    if (dev === null) {
      assert.equal(onDev, '', `${slug}: no dev section`);
      continue;
    }
    assert.ok(onDev.includes(`${repo}/blob/dev/docs/`), `${slug}: the dev section links dev docs`);
    assert.match(onDev, devRef);
    assert.ok(body.includes('(#on-dev-after-the-release)'), `${slug}: the intro points at the dev section`);
    for (const name of dev) {
      assert.ok(onDev.includes(name), `${slug}: ${name} in the dev section`);
      assert.ok(!released.includes(name), `${slug}: ${name} only in the dev section`);
    }
  }
});

test('every CLI option and subcommand in a networking sample is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  // The parsers must see the options and commands the samples rely on.
  assert.equal(cli.options.get('foreign-bbmd'), true);
  assert.equal(cli.options.get('allow-write'), false);
  assert.ok(cli.commands.has('bdt') && cli.commands.has('whois-router') && cli.commands.has('whois'));
  for (const slug of Object.keys(pages)) {
    const { body } = await page(slug);
    const blocks = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].map(match => match[1]);
    for (const line of blocks.flatMap(block => block.split('\n')).filter(Boolean)) {
      checkCliLine(line, cli, slug);
    }
  }
});
