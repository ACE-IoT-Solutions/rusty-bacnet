import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';

const root = new URL('../../', import.meta.url);
const repo = 'https://github.com/jscott3201/rusty-bacnet';
const devHeading = '\n## On `dev` after the release\n';
// Any link into the repository at the dev branch: blob/, tree/, raw/, commits/ and so on.
const devRef = new RegExp(`${repo.replaceAll('.', '\\.')}/[a-z]+/dev/`);
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
    assert.doesNotMatch(released, devRef, `${slug}: dev links only in the dev section`);
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

// The CLI's long options, from the `#[arg(...)]` fields in args.rs: name
// (`long = "..."` or the field name in kebab case) to whether it takes a value.
function cliOptions(args) {
  const options = new Map();
  const paren = String.raw`(?:[^()]|\((?:[^()]|\([^()]*\))*\))*`;
  const field = new RegExp(String.raw`#\[arg\((${paren})\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\(crate\)\s+)?(\w+):\s*([^,\n]+)`, 'g');
  for (const [, attr, name, type] of args.matchAll(field)) {
    if (!/\blong\b/.test(attr)) continue;
    const long = attr.match(/\blong\s*=\s*"([^"]+)"/)?.[1] ?? name.replaceAll('_', '-');
    options.set(long, type.trim() !== 'bool');
  }
  return options;
}

// The subcommands: the variants of `enum Command`, in kebab case, and their aliases.
function cliCommands(args) {
  const start = args.indexOf('pub(crate) enum Command {');
  assert.notEqual(start, -1, 'args.rs defines enum Command');
  const body = args.slice(start, args.indexOf('\n}\n', start));
  const names = new Set();
  for (const [, variant] of body.matchAll(/^ {4}([A-Z]\w*)\s*(?:\{|,|\()/gm)) {
    names.add(variant.replace(/(?<!^)[A-Z]/g, letter => `-${letter}`).toLowerCase());
  }
  for (const [, alias] of body.matchAll(/#\[command\(alias = "([^"]+)"\)\]/g)) names.add(alias);
  return names;
}

test('every CLI option and subcommand in a networking sample is defined by the CLI', async () => {
  const args = await readFile(new URL('../crates/bacnet-cli/src/args.rs', root), 'utf8');
  const options = cliOptions(args);
  const commands = cliCommands(args);
  // The parsers must see the options and commands the samples rely on.
  assert.equal(options.get('foreign-bbmd'), true);
  assert.equal(options.get('allow-write'), false);
  assert.ok(commands.has('bdt') && commands.has('whois-router') && commands.has('whois'));
  for (const slug of Object.keys(pages)) {
    const { body } = await page(slug);
    const blocks = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].map(match => match[1]);
    for (const line of blocks.flatMap(block => block.split('\n')).filter(Boolean)) {
      const words = line.split(/\s+/);
      assert.equal(words[0], 'bacnet', `${slug}: shell samples run the CLI`);
      let command;
      for (let i = 1; i < words.length; i++) {
        const option = words[i].match(/^--([a-z][a-z0-9-]*)$/)?.[1];
        if (option) {
          assert.ok(options.has(option), `${slug}: --${option} is a CLI option`);
          if (options.get(option)) i++;
        } else if (command === undefined) {
          command = words[i];
        }
      }
      assert.ok(commands.has(command), `${slug}: ${command} is a CLI subcommand`);
    }
  }
});
