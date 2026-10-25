import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';

const file = fileURLToPath(new URL('../../src/content/docs/project/performance.mdx', import.meta.url));
const benchmarks = new URL('../../../Benchmarks.md', import.meta.url);
const flat = text => text.replace(/\s+/g, ' ');

async function page() {
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

// GitHub's heading anchor: lower case, punctuation dropped, spaces to hyphens.
const anchor = heading => heading.trim().toLowerCase().replace(/[^\p{L}\p{N} _-]/gu, '').replaceAll(' ', '-');

// Every figure the page quotes, with its unit. Each must appear in the
// repository's Benchmarks.md as written, so a refresh that changes one fails
// here until the page follows it.
const figures = [
  '29 microseconds', '25 microseconds', '48 microseconds',
  '2 to 8 microseconds', '38,600 requests per second',
  'about 82,000 ReadProperty requests per second', 'peaks at 5 to 10 clients',
  '0.5%', '49 megabytes', '85,400 operations per second',
  '90% to 100%', 'about 7 milliseconds per request',
  '93% of the wire limit at 9,600 baud, 52% at 115,200',
  '16 milliseconds by default on FTDI parts',
  'median of 43 microseconds against 22 direct', 'about 21 microseconds',
  '57,000 requests per second', "88% of the direct path's rate at 50 clients",
  '67 requests per second', 'about 1.4 milliseconds',
  '16 confirmed requests in flight per client',
  'median of 24 microseconds from 100 to 10,000 objects', 'about 1.3 kilobytes per Analog Input',
  '1,024 subscriptions', '10 writes per second', '64 subscriptions from one subscriber',
  'three and a half times', '256 results or 16,384 bytes',
  '70 to 100 microseconds', 'about 34,000 requests per second',
  '17.9 kilobytes of flash', '20% to 70% more requests', '16 gigabytes of memory',
];

test('performance page is release-scoped and links Benchmarks.md at the release', async () => {
  const { title, description, body } = await page();
  assert.ok(title && description);
  assert.ok(body.includes(`The page describes **v${release}**`));
  assert.ok(body.includes('(#on-dev-after-the-release)'));
  assert.ok(body.includes('\n## Next steps\n'));
  const { released, onDev } = splitDevSection(body, 'project/performance');
  assert.ok(onDev.includes(`${repo}/blob/dev/Benchmarks.md#101-repeated-comparison-for-1979`));
  for (const issue of [1978, 1979]) assert.ok(onDev.includes(`${repo}/issues/${issue}`), `#${issue}`);
  // The release-scoped text makes no capacity claim.
  assert.ok(released.includes('not capacity guarantees'));
  assert.ok(released.includes(`${repo}/blob/v${release}/Benchmarks.md#11-how-to-reproduce`));
});

test('every Benchmarks.md section the page links exists', async () => {
  const report = await readFile(benchmarks, 'utf8');
  const anchors = new Set([...report.matchAll(/^#{2,3} (.+)$/gm)].map(match => anchor(match[1])));
  const { body } = await page();
  const linked = [...body.matchAll(/\/Benchmarks\.md#([\w-]+)/g)].map(match => match[1]);
  assert.ok(linked.length >= 10);
  for (const name of linked) assert.ok(anchors.has(name), `Benchmarks.md#${name}`);
});

test('every figure the page quotes appears in Benchmarks.md with its unit', async () => {
  const report = flat(await readFile(benchmarks, 'utf8'));
  const { body } = await page();
  const text = flat(body);
  for (const figure of figures) {
    assert.ok(text.includes(figure), `page quotes ${figure}`);
    assert.ok(report.includes(figure), `Benchmarks.md states ${figure}`);
  }
});

test('the reproduction commands are the ones Benchmarks.md lists', async () => {
  const report = await readFile(benchmarks, 'utf8');
  const { body } = await page();
  const lines = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].flatMap(match => match[1].split('\n')).filter(Boolean);
  assert.equal(lines.length, 5);
  for (const line of lines) {
    const listed = line.replace('./target/release/stress-test ', '$ST ');
    assert.ok(report.split('\n').includes(listed), line);
  }
});
