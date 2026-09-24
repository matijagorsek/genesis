// After a build: every picture, clip and poster the pages reference exists in dist; every in-page anchor
// lands; the download cards' hooks are still there; the manual is self-contained enough to open offline;
// and the pages say the hard things plainly. A page with a broken picture on a machine with no network is
// worse than none, and a front page that stopped saying "10 to 30 minutes" would be a lie by omission.
import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';

const dist = resolve(new URL('..', import.meta.url).pathname, 'dist');
const pages = readdirSync(dist).filter((f) => f.endsWith('.html'));
let failures = 0;
const fail = (m) => { console.error('FAIL', m); failures++; };

for (const page of pages) {
  const html = readFileSync(join(dist, page), 'utf8');
  const ids = new Set([...html.matchAll(/ id="([^"]+)"/g)].map((m) => m[1]));
  for (const [, anchor] of html.matchAll(/href="#([^"]+)"/g)) if (!ids.has(anchor)) fail(`${page}: #${anchor} goes nowhere`);
  for (const [, src] of html.matchAll(/(?:src|poster)="([^"]+\.(?:png|jpg|jpeg|webp|avif|webm|svg))"/g)) {
    const rel = src.replace(/^\/genesis\//, '').replace(/^\//, '');
    if (!existsSync(join(dist, rel))) fail(`${page}: ${src} is not in dist`);
  }
  if (/omarchy/i.test(html)) fail(`${page}: names a distribution the pages do not name`);
}
const index = readFileSync(join(dist, 'index.html'), 'utf8');
for (const needle of ['id="dl"', 'id="rel"', 'api.github.com/repos/matijagorsek/genesis/releases', 'SHA256SUMS.', '10 to 30 minutes', 'Nothing, by default', 'one laptop'])
  if (!index.includes(needle)) fail(`index.html lost: ${needle}`);
const manual = readFileSync(join(dist, 'manual.html'), 'utf8');
if (/href="https:\/\/fonts\./.test(manual)) fail('manual.html loads web fonts; it must open offline');
if (/src="\/genesis\//.test(manual)) fail('manual.html has absolute picture paths; the image copy would break');
if (/<link rel="stylesheet" href="\//.test(manual)) fail('manual.html links an absolute stylesheet; opened as a file in the image it would be unstyled');
for (const phrase of ['10 to 30 minutes', "It isn't", 'Nothing restarts on its own', 'Nothing, by default', 'It is working, not stuck', 'rollback'])
  if (!manual.includes(phrase)) fail(`manual.html lost: ${phrase}`);
const mb = (p) => statSync(p).size / 1e6;
const clip = join(dist, 'screens/looks/make.webm'); if (existsSync(clip) && mb(clip) > 2) fail('the clip grew past 2 MB');
console.log(`${pages.length} pages checked, ${failures} failures`);
process.exit(failures ? 1 : 0);
