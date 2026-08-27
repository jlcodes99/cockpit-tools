import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const pagesCss = readFileSync(new URL('../pages.css', import.meta.url), 'utf8');
const polishCss = readFileSync(
  new URL('./opencode-go-polish.css', import.meta.url),
  'utf8',
);

test('OpenCode Go polish is loaded after the base page stylesheet', () => {
  const baseImport = '@import "./pages/opencode-go.css";';
  const polishImport = '@import "./pages/opencode-go-polish.css";';

  assert.ok(pagesCss.includes(polishImport));
  assert.ok(pagesCss.indexOf(polishImport) > pagesCss.indexOf(baseImport));
});

test('OpenCode Go keeps Cockpit row rhythm and resilient quota layout', () => {
  assert.match(
    polishCss,
    /\.opencode-go-summary\s*\{[^}]*padding:\s*18px 24px;/s,
  );
  assert.match(
    polishCss,
    /\.opencode-go-connection-grid\s*\{[^}]*grid-template-columns:\s*repeat\(2, minmax\(0, 1fr\)\);/s,
  );
  assert.match(
    polishCss,
    /@media \(max-width: 900px\)[\s\S]*grid-template-columns:\s*1fr;/,
  );
  assert.match(polishCss, /:focus-visible/);
  assert.match(polishCss, /@media \(prefers-reduced-motion: reduce\)/);
});
