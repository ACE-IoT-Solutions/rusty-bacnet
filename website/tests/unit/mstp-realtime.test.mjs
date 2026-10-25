import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';

const root = new URL('../../', import.meta.url);
const repo = 'https://github.com/jscott3201/rusty-bacnet';
const file = fileURLToPath(new URL('src/content/docs/connect/mstp-realtime.mdx', root));

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
  const parts = body.split('\n## On `dev` after the release\n');
  assert.equal(parts.length, 2, 'one dev section');
  const next = parts[1].indexOf('\n## ');
  assert.notEqual(next, -1, 'a section follows the dev one');
  const onDev = parts[1].slice(0, next);
  const released = parts[0] + parts[1].slice(next);
  assert.doesNotMatch(released, new RegExp(`${repo.replaceAll('.', '\\.')}/[a-z]+/dev/`));
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

// Long options from the `#[arg(...)]` fields of args.rs, mapped to whether
// they take a value, and the subcommands (variants of `enum Command`).
function cli(args) {
  const options = new Map();
  const paren = String.raw`(?:[^()]|\((?:[^()]|\([^()]*\))*\))*`;
  const field = new RegExp(String.raw`#\[arg\((${paren})\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\(crate\)\s+)?(\w+):\s*([^,\n]+)`, 'g');
  for (const [, attr, name, type] of args.matchAll(field)) {
    if (!/\blong\b/.test(attr)) continue;
    options.set(attr.match(/\blong\s*=\s*"([^"]+)"/)?.[1] ?? name.replaceAll('_', '-'), type.trim() !== 'bool');
  }
  const start = args.indexOf('pub(crate) enum Command {');
  assert.notEqual(start, -1, 'args.rs defines enum Command');
  const commands = new Set();
  for (const [, variant] of args.slice(start, args.indexOf('\n}\n', start)).matchAll(/^ {4}([A-Z]\w*)\s*(?:\{|,|\()/gm)) {
    commands.add(variant.replace(/(?<!^)[A-Z]/g, letter => `-${letter}`).toLowerCase());
  }
  return { options, commands };
}

test('every CLI option and subcommand in an MS/TP real-time sample is defined by the CLI', async () => {
  const { options, commands } = cli(await readFile(new URL('../crates/bacnet-cli/src/args.rs', root), 'utf8'));
  for (const flag of ['mstp', 'mstp-mac', 'mstp-baud', 'mstp-rt-priority', 'mstp-cpus', 'mstp-monitor']) {
    assert.equal(options.get(flag), true, `--${flag} takes a value`);
  }
  const { body } = await page();
  const lines = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)]
    .flatMap(match => match[1].split('\n'))
    .filter(line => line.startsWith('bacnet '));
  assert.equal(lines.length, 2, 'the participant and monitor samples');
  for (const line of lines) {
    const words = line.split(/\s+/);
    let command;
    for (let i = 1; i < words.length; i++) {
      const option = words[i].match(/^--([a-z][a-z0-9-]*)$/)?.[1];
      if (option) {
        assert.ok(options.has(option), `--${option} is a CLI option`);
        if (options.get(option)) i++;
      } else command ??= words[i];
    }
    assert.ok(commands.has(command), `${command} is a CLI subcommand`);
  }
  // The page's other flags, named in prose, exist too.
  for (const flag of ['mstp-max-master', 'mstp-max-info-frames']) {
    assert.ok(body.includes(`\`--${flag}\``) && options.has(flag), flag);
  }
});
