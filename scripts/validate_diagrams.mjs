// Parse every Mermaid diagram in docs/ and fail on any that Mermaid rejects.
//
// A broken diagram is a silent failure: GitHub renders an error box where the picture should be,
// and nothing in the test suite notices. This catches it.
//
//   npm install mermaid@11 jsdom
//   node scripts/validate_diagrams.mjs docs
//
// Checks fenced ```mermaid blocks in .md files and whole .mmd files.
import { readFileSync, readdirSync } from 'fs';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!DOCTYPE html><body></body>', { pretendToBeVisual: true });
global.window = dom.window;
global.document = dom.window.document;
Object.defineProperty(global, 'navigator', { value: dom.window.navigator, configurable: true });

const mermaid = (await import('mermaid')).default;
mermaid.initialize({ startOnLoad: false });

const dir = process.argv[2] || 'docs';
const blocks = [];

function walk(d) {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const full = `${d}/${e.name}`;
    if (e.isDirectory()) { walk(full); continue; }
    const rel = full.replace(`${dir}/`, '');
    if (e.name.endsWith('.mmd')) {
      blocks.push({ file: rel, n: 1, src: readFileSync(full, 'utf8') });
    } else if (e.name.endsWith('.md')) {
      const text = readFileSync(full, 'utf8');
      const re = /```mermaid\n([\s\S]*?)```/g;
      let m, i = 0;
      while ((m = re.exec(text)) !== null) blocks.push({ file: rel, n: ++i, src: m[1] });
    }
  }
}
walk(dir);

let bad = 0;
for (const b of blocks) {
  try {
    await mermaid.parse(b.src);
    console.log(`  ok    ${b.file} #${b.n}  (${b.src.split('\n')[0].slice(0, 40)})`);
  } catch (e) {
    bad++;
    console.log(`  FAIL  ${b.file} #${b.n}: ${String(e.message || e).split('\n')[0].slice(0, 200)}`);
  }
}
console.log(`\n${blocks.length} diagrams, ${bad} invalid`);
process.exit(bad ? 1 : 0);
