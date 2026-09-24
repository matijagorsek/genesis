// The share card: what X, Mastodon, Slack and a chat show when the link is pasted. Built from the real
// desktop screenshot the build captured, darkened, with the two lines that matter. Written to
// public/og.png before every build so it is never older than the screenshot.
import sharp from 'sharp';
import { existsSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(new URL('..', import.meta.url).pathname, '..');
const shot = resolve(root, 'docs/screens/latest/04-desktop.png');
const out = resolve(root, 'site/public/og.png');
mkdirSync(resolve(root, 'site/public'), { recursive: true });

const W = 1200, H = 630;
const text = Buffer.from(`<svg width="${W}" height="${H}" xmlns="http://www.w3.org/2000/svg">
  <defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0b0f14" stop-opacity=".35"/><stop offset="1" stop-color="#0b0f14" stop-opacity=".92"/></linearGradient></defs>
  <rect width="${W}" height="${H}" fill="url(#g)"/>
  <circle cx="86" cy="84" r="22" fill="none" stroke="#5fb5bd" stroke-width="6"/><circle cx="86" cy="84" r="8" fill="#5fb5bd"/>
  <text x="124" y="96" font-family="Helvetica, Arial, sans-serif" font-size="34" font-weight="700" fill="#eef3f8">Genesis</text>
  <text x="70" y="430" font-family="Helvetica, Arial, sans-serif" font-size="74" font-weight="700" fill="#eef3f8" letter-spacing="-2">The desktop with the</text>
  <text x="70" y="512" font-family="Helvetica, Arial, sans-serif" font-size="74" font-weight="700" fill="#5fb5bd" letter-spacing="-2">assistant inside.</text>
  <text x="72" y="575" font-family="Helvetica, Arial, sans-serif" font-size="26" fill="#a9b4c1">A model that lives on your machine. Nothing leaves it.</text>
</svg>`);

const base = existsSync(shot)
  ? sharp(shot).resize(W, H, { fit: 'cover', position: 'centre' })
  : sharp({ create: { width: W, height: H, channels: 3, background: '#0b0f14' } });
await base.composite([{ input: text }]).png({ compressionLevel: 9 }).toFile(out);
console.log('og: public/og.png');
