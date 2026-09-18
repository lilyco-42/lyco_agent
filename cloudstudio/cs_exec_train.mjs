//! CloudStudio Jupyter 内核执行器 (train-timeout 版): 4 小时超时, 用于 GRPO/SFT 训练。
//! 与 cs_exec3.mjs 相同机制, 但超时改为 4 小时 (14400000ms)。
//! 平台只杀非 kernel 进程, kernel 内训练执行受保护。
//! 用法: node cs_exec_train.mjs --file <path.py>
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';

const COOKIE = process.env.CS_COOKIE;
const JPS = process.env.CS_JPS;
function csrfToken(c){let t=5381;for(const ch of c)t=t+((t<<5)+ch.codePointAt(0));return t&0x7FFFFFFF;}
async function mintToken(){
  const sess = COOKIE.match(/cloudstudio-session=([^;]+)/)[1];
  const r = await fetch('https://cloudstudio.net/api/workspace/' + JPS.split('--')[0].replace('https://','') + '/sessions',
    { headers: { Cookie: COOKIE, 'User-Agent': 'Mozilla/5.0', 'X-XSRF-TOKEN': String(csrfToken(sess)) } });
  const j = await r.json();
  const tok = j?.data?.token;
  if (!tok) throw new Error('JWT mint failed: ' + JSON.stringify(j).slice(0, 150));
  return tok;
}
const TOKEN = await mintToken();
let code;
const fIdx = process.argv.indexOf('--file');
if (fIdx >= 0) {
  code = fs.readFileSync(process.argv[fIdx + 1], 'utf-8');
} else {
  code = process.argv[2] || "print('hi')";
}
let kernelId = process.argv.find(a => a.startsWith('--kernel'))?.split('=')[1];
const SESSION = randomUUID();

async function createKernel() {
  const r = await fetch(`${JPS}/api/kernels`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${TOKEN}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'python3' }),
  });
  const d = await r.json();
  return d.id;
}

function execOnKernel(kid, code) {
  return new Promise((resolve) => {
    const url = `${JPS.replace('https', 'wss')}/api/kernels/${kid}/channels?session_id=${SESSION}&token=${TOKEN}`;
    const ws = new WebSocket(url, { headers: { Authorization: `Bearer ${TOKEN}` } });
    const out = [];
    const msgId = randomUUID();
    const finish = (r) => { try { ws.close(); } catch {} setTimeout(() => process.exit(0), 3000); resolve(r); };
    const timer = setTimeout(() => { finish({ error: 'TIMEOUT', out: out.join('') }); }, 14400000);

    ws.onopen = () => {
      const header = { msg_id: msgId, username: 'cloudstudio', session: SESSION, msg_type: 'execute_request', version: '5.3' };
      const req = { header, parent_header: {}, metadata: {}, channel: 'shell',
        content: { code, silent: false, store_history: false, user_expressions: {}, allow_stdin: false, stop_on_error: true } };
      ws.send(JSON.stringify(req));
    };
    ws.onmessage = (ev) => {
      let msg;
      try { msg = JSON.parse(ev.data); } catch { return; }
      const t = msg.header?.msg_type;
      const pid = msg.parent_header?.msg_id;
      if (pid !== msgId) return;
      if (t === 'stream') out.push(msg.content.text);
      else if (t === 'execute_result') out.push(String(msg.content.data?.['text/plain'] ?? '') + '\n');
      else if (t === 'error') { out.push('ERROR: ' + (msg.content.ename||'') + ': ' + (msg.content.evalue||'') + '\n' + (msg.content.traceback||[]).join('\n')); }
      else if (t === 'execute_reply' && msg.content.status === 'error') { out.push('EXEC ERROR: ' + JSON.stringify(msg.content).slice(0, 300)); }
      else if (t === 'status' && msg.content.execution_state === 'idle') {
        clearTimeout(timer);
        finish({ out: out.join('') });
      }
    };
    ws.onerror = (e) => { clearTimeout(timer); finish({ error: 'WS_ERROR', out: out.join('') }); };
    ws.onclose = () => { clearTimeout(timer); finish({ out: out.join('') }); };
  });
}

const kid = kernelId || await createKernel();
const res = await execOnKernel(kid, code);
console.log(res.error ? `[${res.error}]\n` : '', res.out);
process.exit(0);
