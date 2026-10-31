// THROWAWAY #43: replay derived bytes (stdin) into xterm.js headless 6.0.0,
// the parser family used by VS Code's integrated terminal. Prints the screen
// and whether row 0 still reads SAFE (i.e. no control was injected).
import xterm from '@xterm/headless';
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const term = new xterm.Terminal({ cols: 20, rows: 3, allowProposedApi: true });
term.write(new Uint8Array(Buffer.concat(chunks)), () => {
  const buffer = term.buffer.active;
  const rows = [...Array(3).keys()].map((y) => buffer.getLine(y).translateToString(true));
  console.log(JSON.stringify(rows));
  console.log(rows[0] === 'SAFE' ? 'host screen intact' : 'host screen ALTERED by derived bytes');
  term.dispose();
});
