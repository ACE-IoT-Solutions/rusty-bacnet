// The generated reference pages (#1989): reference/cli, reference/python and
// reference/rust. Their generators (website/scripts/reference-*.mjs and
// reference-python.py) each have a --check mode, which the release runs
// (docs/ci.md); these tests don't compare the pages with fresh output, so a
// PR that adds an option or a stub method isn't failed for it. They check
// that each page has its generated region, parses, and links correctly, that
// every bacnet subcommand is on the CLI page, and that the crate map lists
// exactly the crates the release publishes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { cliDefinitions } from './cli-args.mjs';

const root = new URL('../../', import.meta.url);
const repo = new URL('../', root);
const docs = new URL('src/content/docs/', root);
const pages = {
  cli: { file: 'reference/cli.md', start: '<!-- generated:start', end: '<!-- generated:end -->' },
  python: { file: 'reference/python.md', start: '<!-- generated:start', end: '<!-- generated:end -->' },
  rust: { file: 'reference/rust.mdx', start: '{/* generated:start', end: '{/* generated:end */}' },
};
const read = name => readFile(new URL(name, docs), 'utf8');

/** The page and its generated region, checking the markers. */
async function load(key) {
  const { file, start, end } = pages[key];
  const text = await read(file);
  assert.equal(text.split(start).length, 2, `${file}: one generated:start marker`);
  assert.equal(text.split(end).length, 2, `${file}: one generated:end marker`);
  const region = text.slice(text.indexOf(start), text.indexOf(end));
  assert.ok(text.indexOf(start) < text.indexOf(end), `${file}: generated:start comes first`);
  assert.ok(region.split('\n').length > 20, `${file}: the generated region is filled in`);
  return { text, region };
}

// The heading slug Starlight gives a heading (github-slugger's rule).
const slug = heading => heading.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/\s/g, '-');

const slugs = new Set((await readdir(docs, { recursive: true }))
  .filter(name => /\.mdx?$/.test(name))
  .map(name => name.replaceAll('\\', '/').replace(/\.mdx?$/, '').replace(/(^|\/)index$/, '')));

test('each reference page parses, exports and links to pages and headings that exist', async () => {
  for (const [key, { file }] of Object.entries(pages)) {
    const { text } = await load(key);
    const page = frontmatter(text);
    assert.ok(page.title && page.description, `${file}: title and description`);
    const path = fileURLToPath(new URL(file, docs));
    const body = await plainBody(page.body, path, []);
    assert.doesNotMatch(body, /generated:(start|end)/, `${file}: the export leaves out the markers`);
    // Outside code, every site link names a page and every fragment a heading.
    const prose = page.body.replace(/^```[^\n]*\n[\s\S]*?^```\s*$/gm, '').replace(/`[^`\n]*`/g, '');
    const headings = new Set([...page.body.matchAll(/^#{2,6} (.+)$/gm)].map(match => slug(match[1])));
    for (const [, target] of prose.matchAll(/\]\(\/rusty-bacnet\/([^)#]*?)\/?(?:#[^)]*)?\)/g)) {
      assert.ok(slugs.has(target), `${file} links to /rusty-bacnet/${target}/, which is not a page`);
    }
    for (const [, fragment] of page.body.matchAll(/\]\(#([^)]+)\)/g)) {
      assert.ok(headings.has(fragment), `${file} links to #${fragment}, which is not a heading`);
    }
  }
});

test('the generated regions name no version', async () => {
  for (const key of Object.keys(pages)) {
    const { region } = await load(key);
    const found = [...region.matchAll(/(?<![\w.])v?(0\.\d+\.\d+)(?!\.?\d)/g)].map(match => match[0]);
    assert.deepEqual(found, [], `${pages[key].file}'s generated region names a version`);
  }
});

test('the CLI page has every bacnet subcommand, and marks the ones behind a feature', async () => {
  const { text, region } = await load('cli');
  for (const heading of ['## Exit status', '## Session behavior', '## Global options', '## Commands']) {
    assert.ok(text.includes(`\n${heading}\n`), `cli.md has ${heading}`);
  }
  const { commands } = await cliDefinitions();
  const sections = new Set([...region.matchAll(/^### bacnet (\S+)$/gm)].map(match => match[1]));
  const aliases = new Set([...region.matchAll(/^Aliases: (.+)\.$/gm)].flatMap(match => [...match[1].matchAll(/`([^`]+)`/g)].map(m => m[1])));
  for (const command of commands) {
    assert.ok(sections.has(command) || aliases.has(command), `cli.md lists bacnet ${command}`);
  }
  assert.ok(sections.size > 20);
  // A variant of `enum Command` behind #[cfg(feature = "...")] says so in its section.
  const args = await readFile(new URL('crates/bacnet-cli/src/args.rs', repo), 'utf8');
  const body = args.slice(args.indexOf('pub(crate) enum Command {'));
  for (const [, feature, variant] of body.matchAll(/#\[cfg\(feature = "([^"]+)"\)\]\s*(?:(?:#\[[^\]]*\]|\/\/\/[^\n]*)\s*)*([A-Z]\w*)/g)) {
    const name = variant.replace(/(?<!^)[A-Z]/g, letter => `-${letter}`).toLowerCase();
    const section = region.split(`### bacnet ${name}\n`)[1]?.split('\n### ')[0] ?? '';
    assert.ok(section.includes(`Needs the \`${feature}\` feature.`), `cli.md marks bacnet ${name} with the ${feature} feature`);
  }
  for (const [name, feature] of [['tui', 'tui'], ['capture', 'pcap'], ['mstp-ports', 'serial']]) {
    const section = region.split(`### bacnet ${name}\n`)[1]?.split('\n### ')[0] ?? '';
    assert.ok(section.includes(`Needs the \`${feature}\` feature.`), `cli.md marks bacnet ${name} with ${feature}`);
  }
});

test('the Python page has every group, and signatures in Python code', async () => {
  const { region } = await load('python');
  for (const group of ['Client', 'Server', 'Router', 'Shared endpoints', 'Types and values', 'Enumerations', 'Errors', 'Module functions']) {
    assert.ok(region.includes(`\n## ${group}\n`), `python.md has ## ${group}`);
  }
  for (const name of ['BACnetClient', 'BACnetServer', 'BACnetRouter', 'BacnetError']) {
    assert.ok(region.includes(`\n### ${name}\n`), `python.md has ### ${name}`);
  }
  assert.match(region, /#### BACnetClient\.read_property\n\n```python\ndef read_property\(/);
});

test('the crate map lists exactly the crates the release publishes, linked to their docs', async () => {
  const { text, region } = await load('rust');
  const manifest = await readFile(new URL('Cargo.toml', repo), 'utf8');
  const members = [...manifest.match(/^members = \[([\s\S]*?)^\]/m)[1].matchAll(/^\s*"([^"]+)"/gm)].map(match => match[1]);
  const published = [];
  for (const member of members) {
    const toml = await readFile(new URL(`${member}/Cargo.toml`, repo), 'utf8');
    if (/^publish = false$/m.test(toml)) continue;
    const name = toml.match(/^name = "([^"]+)"$/m)[1];
    const library = !/^\[\[bin\]\]$/m.test(toml) || /^\[lib\]$/m.test(toml);
    published.push({ name, own: !/^version\.workspace = true$/m.test(toml), library });
  }
  const rows = [...region.matchAll(/^\| <a href=\{`([^`]+)`\}>([^<]+)<\/a> \|/gm)].map(match => ({ href: match[1], name: match[2] }));
  assert.deepEqual(rows.map(row => row.name).sort(), published.map(crate => crate.name).sort());
  for (const crate of published) {
    const { href } = rows.find(row => row.name === crate.name);
    const expected = !crate.library ? `https://crates.io/crates/${crate.name}`
      : `https://docs.rs/${crate.name}/${crate.own ? 'latest' : '${release}'}`;
    assert.equal(href, expected, `${crate.name}'s link`);
    if (crate.own) assert.ok(text.includes(`\`${crate.name}\` versions on its own`), `rust.mdx says ${crate.name} versions on its own`);
  }
  assert.match(text, /^import \{ release, repository \} from '\.\.\/\.\.\/\.\.\/lib\/site\.mjs';$/m);
});
