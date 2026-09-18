#!/usr/bin/env node
// CloudStudio 工作空间访问认证: cookie -> CSRF -> workspace JWT
// 用法:
//   export CS_COOKIE='cloudstudio-session=<值>; cloudstudio-session-team=gh'
//   node cs_auth.mjs <spaceKey>      # 指定工作空间
//   node cs_auth.mjs                 # 自动取 status/list 第一个工作空间
// 输出 JPS + TOKEN + 可直接复制的 export 命令。
const BASE = 'https://cloudstudio.net';

function parseCookies(str) {
  const m = {};
  for (const part of (str || '').split(';')) {
    const [k, ...v] = part.trim().split('=');
    if (k) m[k.trim()] = v.join('=').trim();
  }
  return m;
}

function csrfToken(cookieValue) {
  // 复刻前端 Vq(): t=5381; t += (t<<5)+charCode; return t & 0x7FFFFFFF
  // JS 位运算 (t<<5) 原生即 ToInt32(t)<<5, 与前端完全一致
  let t = 5381;
  for (const ch of cookieValue) {
    t = t + ((t << 5) + ch.codePointAt(0));
  }
  return t & 0x7FFFFFFF;
}

async function api(path, cookie, csrf, method = 'GET', body) {
  const headers = { Cookie: cookie, 'User-Agent': 'Mozilla/5.0', 'X-XSRF-TOKEN': String(csrf) };
  if (body) headers['Content-Type'] = 'application/json';
  const r = await fetch(BASE + path, { method, headers, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (j.code !== 0) throw new Error(`${path} -> code=${j.code} msg=${j.msg || j.semanticization || ''}`);
  return j.data;
}

async function main() {
  const cookieStr = process.env.CS_COOKIE;
  if (!cookieStr) { console.error('需要 CS_COOKIE 环境变量(cloudstudio-session 等)'); process.exit(1); }
  const cookies = parseCookies(cookieStr);
  const session = cookies['cloudstudio-session'] || '';
  if (!session) { console.error('cookie 里没有 cloudstudio-session'); process.exit(1); }

  const csrf = csrfToken(session);
  console.error(`[auth] CSRF=${csrf} team=${cookies['cloudstudio-session-team'] || '?'}`);

  const me = await api('/api/user/info', cookieStr, csrf);
  console.error(`[auth] ok: ${me.authenticationUserInfo?.userName || '?'} (${me.authenticationUserInfo?.idp || '?'})`);

  let spaceKey = process.argv[2];
  if (!spaceKey) {
    const list = await api('/api/workspace/status/list', cookieStr, csrf);
    if (!list.length) throw new Error('status/list 为空: 请传 spaceKey 参数');
    spaceKey = list[0].spaceKey || list[0].space_key || list[0].id;
  }
  console.error(`[auth] spaceKey=${spaceKey}`);

  const ws = await api(`/api/workspace/v2/${spaceKey}`, cookieStr, csrf);
  const conn = ws.connections || {};
  const status = ws.status?.status || '?';
  if (status !== 'Running') console.error(`[auth] 警告: 工作空间状态=${status}, 请先在控制台启动`);
  const jps = conn.jupyterServer;
  if (!jps) throw new Error('connections.jupyterServer 为空');

  const sess = await api(`/api/workspace/${spaceKey}/sessions`, cookieStr, csrf);
  const token = sess.token;
  if (!token) throw new Error('sessions 未返回 token');

  console.log(`JPS   ${jps}`);
  console.log(`TOKEN ${token}`);
  console.log('---');
  console.log(`export CS_JPS='${jps}'`);
  console.log(`export CS_TOKEN='${token}'`);
}

main().catch((e) => { console.error('[auth] FAIL:', e.message); process.exit(1); });