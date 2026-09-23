// Before every build: the screenshots CI writes into docs/screens become public/screens, and the decision
// log becomes content. The site never owns a picture or a decision; it shows what the repo already has.
import { cpSync, mkdirSync, existsSync, rmSync, copyFileSync } from 'node:fs';
import { resolve } from 'node:path';
const root = resolve(new URL('..', import.meta.url).pathname, '..');
const from = resolve(root, 'docs/screens'), to = resolve(root, 'site/public/screens');
rmSync(to, { recursive: true, force: true }); mkdirSync(to, { recursive: true });
for (const d of ['latest', 'fresh-run', 'more', 'phone', 'looks']) {
  if (existsSync(resolve(from, d))) cpSync(resolve(from, d), resolve(to, d), { recursive: true });
}
mkdirSync(resolve(root, 'site/src/content/decisions'), { recursive: true });
copyFileSync(resolve(root, 'docs/decisions.md'), resolve(root, 'site/src/content/decisions/log.md'));
console.log('prebuild: screens and the decision log copied in');
