// THROWAWAY #43: green checks below CONFIRM gaps; they do not qualify restore.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import xterm from '@xterm/headless';
const binary = process.argv[2];
assert.ok(binary, 'derive_state binary argument');

async function replay(input, continuation) {
  const derived = spawnSync(binary, ['restore'], { input: Buffer.from(input) });
  assert.equal(derived.status, 0, derived.stderr?.toString());
  const term = new xterm.Terminal({ cols: 20, rows: 4, allowProposedApi: true });
  await new Promise(resolve => term.write(new Uint8Array(derived.stdout), resolve));
  await new Promise(resolve => term.write(continuation, resolve));
  const rows = [...Array(4).keys()].map(y =>
    term.buffer.active.getLine(term.buffer.active.baseY + y).translateToString(true));
  term.dispose();
  return rows;
}

for (const alternate of [false, true]) {
  const prefix = alternate ? '\x1b[?1049h' : '';
  const input = prefix + '\x1b7\x1b[2;3r\x1b[?6h\x1b8';
  const result = spawnSync(binary, ['restore'], { input: Buffer.from(input) });
  assert.notEqual(result.status, 0, 'outside-region origin state must not silently misrestore');
  assert.match(result.stderr.toString(), /BLOCKED: origin-mode cursor outside scroll region/u);
  const term = new xterm.Terminal({ cols: 20, rows: 4, allowProposedApi: true });
  await new Promise(resolve => term.write(input + 'X', resolve));
  assert.equal(term.buffer.active.getLine(term.buffer.active.baseY).translateToString(true), '');
  assert.equal(term.buffer.active.getLine(term.buffer.active.baseY + 1).translateToString(true), 'X');
  term.dispose();
  console.log(`BLOCKED origin cursor outside region (${alternate ? 'alternate' : 'normal'}): Alacritty DECRC row 0, xterm row 1`);
}

for (const alternate of [false, true]) {
  const prefix = alternate ? 'N\x1b[?1049h\x1b[H' : '';
  const screen = alternate ? 'alternate' : 'normal';
  for (const [label, input, continuation, desired, actual] of [
    ['scroll region', '\x1b[2;3r\x1b[3;1H', '\nX', ['', '', 'X', ''], ['', '', '', 'X']],
    ['tabs', '\x1b[3g\x1b[5G\x1bH\x1b[H', '\tX', ['    X', '', '', ''], ['        X', '', '', '']],
    ['charset shift', '\x1b)0\x0e', 'q', ['\u2500', '', '', ''], ['q', '', '', '']],
    ['origin + region', '\x1b[2;3r\x1b[?6h', '\x1b[1;1HX', ['', 'X', '', ''], ['X', '', '', '']],
  ]) {
    const restored = await replay(prefix + input, continuation);
    assert.notDeepEqual(desired, actual, 'independent expected behaviour differs from baseline');
    assert.deepEqual(restored, desired, `${label} ${screen}`);
    console.log(`PASS ${label} ${screen}`);
  }
}
