// THROWAWAY #43: hand-written state expectations in independent xterm.js.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import xterm from '@xterm/headless';

const binary = process.argv[2];
assert.ok(binary, 'derive_state binary argument');
const row = (term, y) => term.buffer.active.getLine(term.buffer.active.baseY + y).translateToString(true);
const cell = (term, x, y) => term.buffer.active.getLine(term.buffer.active.baseY + y).getCell(x);

async function attach(input) {
  const result = spawnSync(binary, ['restore'], { input: Buffer.from(input) });
  assert.equal(result.status, 0, result.stderr?.toString());
  const term = new xterm.Terminal({ cols: 20, rows: 4, allowProposedApi: true });
  await write(term, result.stdout);
  return term;
}

function write(term, bytes) {
  return new Promise(resolve => term.write(typeof bytes === 'string' ? bytes : new Uint8Array(bytes), resolve));
}

const failures = [];
async function check(name, body) {
  try {
    await body();
    console.log(`PASS ${name}`);
  } catch (error) {
    failures.push(name);
    console.error(`FAIL ${name}: ${error.message}`);
  }
}

for (const alternate of [false, true]) {
  const prefix = alternate ? 'NORMAL\x1b[?1049h\x1b[H' : '';
  const screen = alternate ? 'alternate' : 'normal';
  await check(`tab-cell visible spacing ${screen}`, async () => {
    const term = await attach(prefix + '\x1b[3g\x1b[5G\x1bH\x1b[HA\tB');
    assert.equal(row(term, 0), 'A   B');
    term.dispose();
  });
  for (const stop of [false, true]) {
    await check(`last-column tab after resize ${screen} ${stop}`, async () => {
      const term = await attach(prefix + '\x1b[3g\x1b[5G\x1bH' + (stop ? '\x1b[20G\x1bH' : '') + '\x1b[H');
      term.resize(21, 4);
      await write(term, '\x1b[1;19H\tX');
      assert.equal(row(term, 0), ' '.repeat(stop ? 19 : 20) + 'X');
      term.dispose();
    });
  }
  await check(`BLOCKED empty tabs after host resize ${screen}`, async () => {
    const term = await attach(prefix + '\x1b[3g');
    await write(term, '\tX');
    assert.equal(row(term, 0), ' '.repeat(19) + 'X', 'empty tabs restored at attachment');
    term.resize(21, 4);
    await write(term, '\x1b[H\tY');
    assert.equal(cell(term, 8, 0).getChars(), 'Y', 'xterm resize reinstalls default tabs');
    term.dispose();
  });
  await check(`pending wrap ${screen}`, async () => {
    const term = await attach(prefix + '12345678901234567890\x1b[31m');
    await write(term, 'X');
    assert.equal(row(term, 0), '12345678901234567890');
    assert.equal(row(term, 1), 'X');
    assert.equal(cell(term, 0, 1).getFgColor(), 1, 'continuation uses current, not last-cell pen');
    term.dispose();
  });
  await check(`wide/combining pending wrap ${screen}`, async () => {
    const term = await attach(prefix + '123456789012345678\u3042\u0301');
    await write(term, 'X');
    assert.equal(cell(term, 18, 0).getChars(), '\u3042\u0301');
    assert.equal(cell(term, 18, 0).getWidth(), 2);
    assert.equal(row(term, 1), 'X');
    term.dispose();
  });
  await check(`bottom-row pending wrap ${screen}`, async () => {
    const term = await attach(prefix + '\x1b[4;1H12345678901234567890');
    await write(term, 'X');
    assert.equal(row(term, 2), '12345678901234567890');
    assert.equal(row(term, 3), 'X');
    term.dispose();
  });
  await check(`last-column without pending wrap ${screen}`, async () => {
    const term = await attach(prefix + '12345678901234567890\x1b[1;20H');
    await write(term, 'X');
    assert.equal(row(term, 0), '1234567890123456789X');
    assert.equal(row(term, 1), '');
    term.dispose();
  });
  await check(`BLOCKED saved pending wrap ${screen}: xterm DECRC clamps`, async () => {
    const term = await attach(prefix + '12345678901234567890\x1b7\x1b[H');
    assert.equal(term._core._bufferService.buffer.savedX, 20, 'wrap represented in saved cursor');
    await write(term, '\x1b8X');
    assert.equal(row(term, 0), '1234567890123456789X');
    assert.equal(row(term, 1), '', 'host does not restore saved wrap');
    term.dispose();
  });
  await check(`charset designation ${screen}`, async () => {
    const term = await attach(prefix + '\x1b(0\x1b)0\x1b*0\x1b+0');
    // Explicit shift tests designation separately from the private shift seam.
    await write(term, '\x0fq\x0eq\x1bnq\x1boq');
    assert.equal(row(term, 0), '\u2500\u2500\u2500\u2500');
    term.dispose();
  });
  await check(`saved effective charset mapping ${screen}`, async () => {
    const term = await attach(prefix + '\x1b(0\x1b7\x1b(B');
    await write(term, '\x1b8q');
    assert.equal(row(term, 0), '\u2500');
    term.dispose();
  });
  await check(`underline colour ${screen}`, async () => {
    const term = await attach(prefix + '\x1b[4;58:2::1:2:3mA\x1b[58:5:123mB\x1b[59mC');
    assert.equal(cell(term, 0, 0).getUnderlineColor(), 0x010203);
    assert.equal(cell(term, 1, 0).getUnderlineColor(), 123);
    assert.equal(cell(term, 2, 0).getUnderlineColorMode(), 0, 'default colour restored');
    term.dispose();
  });
  await check(`current underline colour and xterm saved-extra limitation ${screen}`, async () => {
    const term = await attach(prefix + '\x1b[4;58:5:123m\x1b7\x1b[58:2::1:2:3m');
    await write(term, 'A');
    assert.equal(cell(term, 0, 0).getUnderlineColor(), 0x010203);
    await write(term, '\x1b8B');
    assert.equal(cell(term, 0, 0).getUnderlineColor(), 0x010203, 'xterm does not save extended attrs');
    term.dispose();
  });
  for (const [code, shape, blink] of [
    [1, 'block', true], [2, 'block', false], [3, 'underline', true],
    [4, 'underline', false], [5, 'bar', true], [6, 'bar', false],
  ]) {
    await check(`cursor style ${screen} ${code}`, async () => {
      const term = await attach(prefix + `\x1b[${code} q`);
      const modes = term._core._inputHandler._coreService.decPrivateModes;
      assert.equal(modes.cursorStyle, shape);
      assert.equal(modes.cursorBlink, blink);
      term.dispose();
    });
  }
}

await check('E5 normal-only saved attributes', async () => {
  const term = await attach('\x1b[2;3H\x1b[31;1m\x1b7\x1b[H\x1b[34;22mA');
  await write(term, '\x1b8Z');
  assert.equal(cell(term, 2, 1).getChars(), 'Z');
  assert.equal(cell(term, 2, 1).getFgColor(), 1);
  assert.notEqual(cell(term, 2, 1).isBold(), 0);
  term.dispose();
});

await check('E5 xterm per-screen saved attributes on alternate and after normal return', async () => {
  const term = await attach('\x1b[2;3H\x1b[31;1mN\x1b[?1049h\x1b[3;5H\x1b[34;22m\x1b7\x1b[H\x1b[32mA');
  await write(term, '\x1b8Z');
  assert.equal(cell(term, 4, 2).getChars(), 'Z');
  assert.equal(cell(term, 4, 2).getFgColor(), 4);
  assert.equal(cell(term, 4, 2).isBold(), 0);
  await write(term, '\x1b[?1049lX');
  assert.equal(cell(term, 3, 1).getChars(), 'X');
  assert.equal(cell(term, 3, 1).getFgColor(), 1);
  assert.notEqual(cell(term, 3, 1).isBold(), 0);
  await write(term, '\x1b8Y');
  assert.equal(cell(term, 3, 1).getChars(), 'Y');
  assert.equal(cell(term, 3, 1).getFgColor(), 1);
  term.dispose();
});

if (failures.length) {
  console.error(`${failures.length} failed: ${failures.join(', ')}`);
  process.exitCode = 1;
}
