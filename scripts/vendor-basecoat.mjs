import { copyFile, mkdir, readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const source = join(root, 'node_modules', 'basecoat-css');
const target = join(root, 'crates', 'amux-dashboard', 'static', 'vendor', 'basecoat');
const pkg = JSON.parse(await readFile(join(source, 'package.json'), 'utf8'));

if (pkg.version !== '1.0.2') {
  throw new Error(`Expected basecoat-css 1.0.2, found ${pkg.version}`);
}

await mkdir(target, { recursive: true });
await Promise.all([
  copyFile(join(source, 'dist', 'basecoat.cdn.min.css'), join(target, 'basecoat.min.css')),
  copyFile(join(source, 'dist', 'js', 'all.min.js'), join(target, 'all.min.js')),
  copyFile(join(source, 'LICENSE.md'), join(target, 'LICENSE.md')),
]);

console.log(`Vendored basecoat-css ${pkg.version} into the dashboard app shell.`);
