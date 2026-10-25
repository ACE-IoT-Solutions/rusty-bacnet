import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';
import { checkCliLine, cliDefinitions } from './cli-args.mjs';

const file = fileURLToPath(new URL('../../src/content/docs/connect/mstp-realtime.mdx', import.meta.url));

async function page() {
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

test('MS/TP real-time page is release-scoped, with the baud scan in the dev section', async () => {
  const { title, description, body } = await page();
  assert.ok(title && description);
  assert.ok(body.includes(`The page describes **v${release}**.`));
  assert.ok(body.includes('(#on-dev-after-the-release)'));
  assert.ok(body.includes('\n## Next steps\n'));
  const { released, onDev } = splitDevSection(body, 'connect/mstp-realtime');
  assert.ok(onDev.includes(`${repo}/blob/dev/docs/CLI.md`));
  for (const name of [':mstp scan', '#1873']) {
    assert.ok(onDev.includes(name), name);
    assert.ok(!released.includes(name), `${name} only in the dev section`);
  }
  for (const path of [
    'docs/mstp-qualification.md',
    'docs/rust-api.md#real-time-scheduling-and-cpu-affinity-linux',
    'docs/rust-api.md#mstp-serial-rs-485',
    'docs/CLI.md#mstp',
    'Benchmarks.md#44-mstp',
  ]) assert.ok(released.includes(`${repo}/blob/v${release}/${path}`), path);
  // No qualification claim: the open hardware issues stay named.
  for (const issue of [501, 502, 707]) assert.ok(released.includes(`${repo}/issues/${issue}`), `#${issue}`);
  assert.ok(released.includes('No adapter or host is qualified for on-wire timing yet.'));
});

test('every CLI option and subcommand in an MS/TP real-time sample is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  for (const flag of ['mstp', 'mstp-mac', 'mstp-baud', 'mstp-rt-priority', 'mstp-cpus', 'mstp-monitor']) {
    assert.equal(cli.options.get(flag), true, `--${flag} takes a value`);
  }
  const { body } = await page();
  // The page's shell blocks also hold sysfs and cargo commands; check the CLI's.
  const lines = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)]
    .flatMap(match => match[1].split('\n'))
    .filter(line => line.startsWith('bacnet '));
  assert.equal(lines.length, 2, 'the participant and monitor samples');
  for (const line of lines) checkCliLine(line, cli, 'connect/mstp-realtime');
  // The ring flags the page names in prose exist too.
  for (const flag of ['mstp-max-master', 'mstp-max-info-frames']) {
    assert.ok(body.includes(`\`--${flag}\``) && cli.options.has(flag), flag);
  }
});
