// THROWAWAY #43: independent xterm.js parser/screen and link-service oracle.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import xterm from '@xterm/headless';

const binary = process.argv[2];
assert.ok(binary, 'derive_state binary argument');
const osc = (id, uri) => `\x1b]8;id=${id};${uri}\x1b\\`;
const close = '\x1b]8;;\x1b\\';
const controls = Array.from({ length: 32 }, (_, i) => String.fromCharCode(0x80 + i)).join('') + '\x7f';
const cases = [
  ['erase', '\u009dx', 'http://a/\u009c\u009b2J', 'http://a/%C2%9C%C2%9B2J'],
  ['all controls', controls, `http://a/${controls}`,
    'http://a/' + Array.from({ length: 32 }, (_, i) => `%C2%${(0x80 + i).toString(16).toUpperCase()}`).join('') + '%7F'],
  ['benign escapes', 'g', 'https://a/a;b?q=%7F&c=%C2%9C', 'https://a/a;b?q=%7F&c=%C2%9C'],
  ['benign UTF8', 'g', 'https://a/\u00e9/\u3042', 'https://a/\u00e9/\u3042'],
  ['C0 dropped', 'a\x01b', 'http://a/\x02y', 'http://a/y'],
];

function derive(input, mode) {
  const child = spawnSync(binary, [mode], { input: Buffer.from(input), maxBuffer: 32 * 1024 * 1024 });
  assert.equal(child.status, 0, child.stderr?.toString());
  return child.stdout;
}

async function replay(output) {
  const term = new xterm.Terminal({ cols: 20, rows: 4, allowProposedApi: true });
  await new Promise(resolve => term.write(new Uint8Array(output), resolve));
  return term;
}

function row(term, y) {
  return term.buffer.active.getLine(term.buffer.active.baseY + y).translateToString(true);
}

// Oracle-only use of version-pinned xterm internals: no hyperlink API exists
// on public IBufferCell. This is never a seam in the Alacritty implementation.
function link(term, x, y) {
  const cell = term.buffer.active.getLine(term.buffer.active.baseY + y).getCell(x);
  return term._core._oscLinkService.getLinkData(cell.extended.urlId);
}

for (const mode of ['restore', 'repaint']) {
  for (const [label, id, uri, expected] of cases) {
    const input = `SAFE\r\n${osc(id, uri)}A${close}-${osc('other', uri)}C${close}${osc(id, uri)}B${close}`;
    const output = derive(input, mode);
    const term = await replay(output);
    assert.equal(row(term, 0), 'SAFE', `${mode} ${label}: host erased`);
    assert.doesNotMatch(output.toString('utf8'), /[\x7f-\x9f]/u, `${mode} ${label}: raw controls`);
    assert.equal(row(term, 1), 'A-CB', `${mode} ${label}: cells`);
    const a = link(term, 0, 1);
    const c = link(term, 2, 1);
    const b = link(term, 3, 1);
    assert.equal(a.uri, expected, `${mode} ${label}: URI`);
    assert.deepEqual(a, b, `${mode} ${label}: split group`);
    assert.notEqual(a.id, c.id, `${mode} ${label}: merged groups`);
    assert.equal(c.uri, expected);
    assert.match(a.id, /^[\x20-\x7e]+$/u);
    assert.notEqual(a.id, id);
    assert.equal(link(term, 1, 1), undefined);
    term.dispose();
  }
  // Reuse the prior malformed corpus; expectations describe engine-retained
  // state, not an assumption that raw app controls are safe to replay.
  for (const [label, payload, expected] of [
    ['ST', '\x1b]8;;http://x\x1b\\\x1b[2JL', 'http://x'],
    ['BEL', '\x1b]8;;http://x\x07\x1b[31mL', 'http://x'],
    ['CAN', '\x1b]8;;http://a\x18L', 'http://a'],
    ['invalid URI', '\x1b]8;;http://\xff\x1b\\L', undefined],
    ['invalid id', '\x1b]8;id=\xff;http://ok\x1b\\L', 'http://ok'],
    ['invalid segment', '\x1b]8;;http://a;\xff;b\x1b\\L', 'http://a;;b'],
    ['nested', '\x1b]8;id=1;http://a\x1b\\\x1b]8;id=2;http://b\x1b\\L', 'http://b'],
    ['unterminated', '\x1b]8;;http://aL\x1b[mM', 'http://aL'],
  ]) {
    const input = label.startsWith('invalid') ? Buffer.from(payload, 'latin1') : Buffer.from(payload);
    const term = await replay(derive(input, mode));
    assert.equal(row(term, 0), label === 'unterminated' ? 'M' : 'L', `${mode} ${label}`);
    assert.equal(link(term, 0, 0)?.uri, expected, `${mode} ${label}`);
    term.dispose();
  }
  const largeUri = 'a'.repeat(1 << 20);
  const term = await replay(derive(`SAFE\r\n${osc('large', largeUri)}L${close}`, mode));
  assert.equal(row(term, 0), 'SAFE');
  assert.equal(row(term, 1), 'L');
  assert.equal(link(term, 0, 1).uri, largeUri, 'prior 1 MiB corpus link not dropped');
  term.dispose();
}
console.log('PASS E2: xterm.js restore + repaint, hostile/malformed corpus, screens, grouping, benign bytes');
