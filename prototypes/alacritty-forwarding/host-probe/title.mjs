// THROWAWAY #43: unsafe title-probe mode is evidence ONLY, not restoration.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import xterm from '@xterm/headless';
const binary = process.argv[2];
assert.ok(binary, 'derive_state binary argument');

for (const alternate of [false, true]) {
  const prefix = alternate ? '\x1b[?1049h' : '';
  for (const title of ['session;a;\u00e9', 'a\u009c\u009b2J']) {
    const result = spawnSync(binary, ['title-probe'], {
      input: Buffer.from(prefix + `\x1b]2;${title}\x1b\\`),
    });
    assert.equal(result.status, 0, result.stderr?.toString());
    assert.deepEqual(result.stdout, Buffer.from(`\x1b]2;${title}\x1b\\`), 'retained title is exact');
    const term = new xterm.Terminal({ cols: 20, rows: 4, allowProposedApi: true });
    const titles = [];
    term.onTitleChange(value => titles.push(value));
    await new Promise(resolve => term.write(prefix + 'SAFE', resolve));
    await new Promise(resolve => term.write(new Uint8Array(result.stdout), resolve));
    const row = term.buffer.active.getLine(term.buffer.active.baseY).translateToString(true);
    if (title === 'session;a;\u00e9') {
      assert.deepEqual(titles, [title]);
      assert.equal(row, 'SAFE');
      console.log(`PASS title public event -> ordinary OSC 2 (${alternate ? 'alternate' : 'normal'})`);
    } else {
      assert.deepEqual(titles, ['a']);
      assert.equal(row, '', 'raw retained C1 title erases host');
      console.log(`BLOCKED arbitrary title (${alternate ? 'alternate' : 'normal'}): retained C1 executes on host`);
    }
    term.dispose();
  }
}
