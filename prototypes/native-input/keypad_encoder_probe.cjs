'use strict';

// Throwaway #56 diagnostic: load, but never modify, the shipped xterm bundle.
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');

const expectedHash = '45b51f9aea7f7dfd5b1b076ef45e188eec8be0a9aac7349d303cc4058aaa3c98';
const archivePath = process.argv[2] ??
  '/Applications/Visual Studio Code.app/Contents/Resources/app/node_modules.asar';

function loadShippedTerminal() {
  const archive = readFileSync(archivePath);
  const header = JSON.parse(archive.subarray(16, 16 + archive.readUInt32LE(12)));
  function readEntry(path) {
    let entry = header;
    for (const part of path.split('/')) {
      entry = entry.files[part];
    }
    assert.equal(entry.unpacked, undefined, 'Expected an archived dependency');
    const start = 8 + archive.readUInt32LE(4) + Number(entry.offset);
    assert.ok(Number.isSafeInteger(start) && start >= 16);
    assert.ok(start + entry.size <= archive.length);
    return archive.subarray(start, start + entry.size);
  }
  const metadata = JSON.parse(readEntry('@xterm/xterm/package.json'));
  const source = readEntry('@xterm/xterm/lib/xterm.js');
  const hash = createHash('sha256').update(source).digest('hex');
  assert.equal(metadata.version, '6.1.0-beta.304', 'Different xterm version requires new qualification');
  assert.equal(hash, expectedHash, 'Different shipped bundle requires new qualification');
  const context = {
    module: { exports: {} }, exports: {}, console,
    setTimeout, clearTimeout, setInterval, clearInterval, structuredClone, performance,
    navigator: { platform: 'MacIntel', userAgent: 'Isolated keypad encoder probe' },
  };
  vm.runInNewContext(source.toString(), context, { filename: 'shipped-xterm.js', timeout: 5000 });
  return { Terminal: context.module.exports.Terminal, version: metadata.version, hash };
}

function write(terminal, output) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Shipped parser did not finish')), 2000);
    terminal.write(output, () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

function event(key, code, keyCode, location) {
  return {
    key, code, keyCode, location,
    shiftKey: false, altKey: false, ctrlKey: false, metaKey: false, repeat: false,
  };
}

async function main() {
  const { Terminal, version, hash } = loadShippedTerminal();
  const terminal = new Terminal({ allowProposedApi: true });
  const stages = [];
  try {
    assert.equal(terminal._core.browser.isMac, true);
    for (const [mode, output, enabled] of [
      ['normal', '\x1b>', false],
      ['application', '\x1b=', true],
      ['handoff-normal', '\x1b>', false],
    ]) {
      await write(terminal, output);
      assert.equal(terminal.modes.applicationKeypadMode, enabled, 'Parsed keypad mode is incorrect');
      for (const input of [event('1', 'Numpad1', 97, 3), event('1', 'Digit1', 49, 0)]) {
        // This private seam is read-only diagnosis, not a production adapter.
        const encoded = terminal._core._keyboardService.evaluateKeyDown(input).key;
        assert.equal(typeof encoded, 'string');
        const expected = enabled && input.code === 'Numpad1' ? '\x1bOq' : '1';
        stages.push({
          mode, application_keypad: terminal.modes.applicationKeypadMode,
          event: input, expected_hex: Buffer.from(expected).toString('hex'),
          encoded_hex: Buffer.from(encoded).toString('hex'), matched: encoded === expected,
        });
      }
    }
    for (const [output, expected] of [['\x1b[?1l', '\x1b[A'], ['\x1b[?1h', '\x1bOA']]) {
      await write(terminal, output);
      assert.equal(
        terminal._core._keyboardService.evaluateKeyDown(event('ArrowUp', 'ArrowUp', 38, 0)).key,
        expected,
        'Application cursor positive control failed',
      );
    }
    const matched = stages.every(stage => stage.matched);
    console.log(JSON.stringify({
      status: matched ? 'PASS' : 'FAIL',
      host: 'VS Code 1.144.0 shipped dependency',
      xterm_version: version, xterm_sha256: hash,
      input_source: 'Explicit synthetic DOM events; not a physical/native keyboard test',
      parser_modes_verified: true, cursor_positive_control: true, stages,
    }, null, 2));
    process.exitCode = matched ? 0 : 1;
  } finally {
    terminal.dispose();
  }
}

main().catch(error => {
  console.error(`Keypad encoder probe failed: ${error.name}: ${error.message}`);
  process.exitCode = 1;
});
