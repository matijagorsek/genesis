// Before every build: the screenshots CI writes into docs/screens become public/screens, and the decision
// log becomes content. The site never owns a picture or a decision; it shows what the repo already has.
import { cpSync, mkdirSync, existsSync, rmSync, copyFileSync } from 'node:fs';
import { resolve } from 'node:path';
const root = resolve(new URL('..', import.meta.url).pathname, '..');
const from = resolve(root, 'docs/screens'), to = resolve(root, 'site/public/screens');
rmSync(to, { recursive: true, force: true }); mkdirSync(to, { recursive: true });
for (const d of ['latest', 'fresh-run', 'more', 'phone', 'looks', 'installer']) {
  if (existsSync(resolve(from, d))) cpSync(resolve(from, d), resolve(to, d), { recursive: true });
}
mkdirSync(resolve(root, 'site/src/content/decisions'), { recursive: true });
copyFileSync(resolve(root, 'docs/decisions.md'), resolve(root, 'site/src/content/decisions/log.md'));
// the catalogue the machine ships, for the live widget on the front page
mkdirSync(resolve(root, 'site/src/data'), { recursive: true });
copyFileSync(resolve(root, 'system_files/usr/share/genesis/apps.json'), resolve(root, 'site/src/data/apps.json'));
// the design documents the README links to, at the addresses they had before the site was Astro
for (const f of ['brief', 'design-plan', 'review', 'next-plan', 'wow-plan', 'walkthrough']) {
  copyFileSync(resolve(root, `docs/genesis-${f}.html`), resolve(root, `site/public/genesis-${f}.html`));
}
await import('./og.mjs');
console.log('prebuild: screens, the decision log, the catalogue, the design documents and the share card');
