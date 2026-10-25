import https from "https"
import fs from "fs"
import path from "path"
import crypto from "crypto"
import { WebSocketServer, WebSocket } from "ws"
import selfsigned from "selfsigned"
import { app } from "electron"
import { bus } from "./bus"
import { localHandlers } from "./ipc-registry"
import { loadConfig } from "./config"
import type { ServeConfig } from "./config"
import type { IncomingMessage, ServerResponse } from "http"

// ---------- credentials (scrypt + salt, no md5) ----------

export function makeSalt(): string {
  return crypto.randomBytes(16).toString("hex")
}

export function hashPassword(password: string, saltHex: string): string {
  return crypto.scryptSync(password, Buffer.from(saltHex, "hex"), 64).toString("hex")
}

export function verifyCredentials(username: string, password: string, serve: ServeConfig): boolean {
  if (!serve.username || !serve.salt || !serve.hash) return false
  if (username !== serve.username) return false
  const candidate = Buffer.from(hashPassword(password, serve.salt), "hex")
  const stored = Buffer.from(serve.hash, "hex")
  if (candidate.length !== stored.length) return false
  return crypto.timingSafeEqual(candidate, stored)
}

export function credentialsConfigured(serve: ServeConfig): boolean {
  return Boolean(serve.username && serve.salt && serve.hash)
}

// ---------- tokens / cookies ----------

const TOKEN_TTL_MS = 30 * 24 * 60 * 60 * 1000
const COOKIE_NAME = "nexus_token"
const tokens = new Map<string, { user: string; expires: number }>()

function createToken(user: string): string {
  const token = crypto.randomBytes(32).toString("hex")
  tokens.set(token, { user, expires: Date.now() + TOKEN_TTL_MS })
  return token
}

function verifyToken(token: string | undefined): { user: string } | null {
  if (!token) return null
  const entry = tokens.get(token)
  if (!entry) return null
  if (entry.expires < Date.now()) {
    tokens.delete(token)
    return null
  }
  entry.expires = Date.now() + TOKEN_TTL_MS
  return { user: entry.user }
}

function parseCookies(header: string | undefined): Record<string, string> {
  const out: Record<string, string> = {}
  if (!header) return out
  for (const part of header.split(";")) {
    const idx = part.indexOf("=")
    if (idx > 0) out[part.slice(0, idx).trim()] = decodeURIComponent(part.slice(idx + 1).trim())
  }
  return out
}

function tokenFromRequest(req: IncomingMessage): { user: string } | null {
  const cookies = parseCookies(req.headers.cookie)
  return verifyToken(cookies[COOKIE_NAME])
}

// brute-force guard: 8 attempts / 10 min / IP
const attempts = new Map<string, { count: number; resetAt: number }>()
function rateLimited(ip: string): boolean {
  const now = Date.now()
  const entry = attempts.get(ip)
  if (!entry || entry.resetAt < now) {
    attempts.set(ip, { count: 1, resetAt: now + 10 * 60 * 1000 })
    return false
  }
  entry.count += 1
  return entry.count > 8
}

// ---------- self-signed certificate ----------

function userDataServeDir(): string {
  const dir = path.join(app.getPath("userData"), "serve")
  fs.mkdirSync(dir, { recursive: true })
  return dir
}

/**
 * Custom cert paths win when configured; otherwise a self-signed
 * certificate is generated and cached in userData until <30 days from
 * expiry, then regenerated.
 */
export async function ensureCertificate(serve: ServeConfig): Promise<{ certPem: string; keyPem: string; generated: boolean }> {
  if (serve.certPath && serve.keyPath) {
    const certPem = fs.readFileSync(serve.certPath, "utf8")
    const keyPem = fs.readFileSync(serve.keyPath, "utf8")
    return { certPem, keyPem, generated: false }
  }
  const dir = userDataServeDir()
  const certFile = path.join(dir, "cert.pem")
  const keyFile = path.join(dir, "key.pem")
  const metaFile = path.join(dir, "cert-meta.json")
  const expiresSoon = Date.now() + 30 * 24 * 60 * 60 * 1000
  if (fs.existsSync(certFile) && fs.existsSync(keyFile) && fs.existsSync(metaFile)) {
    try {
      const meta = JSON.parse(fs.readFileSync(metaFile, "utf8")) as { expires: number }
      if (!meta.expires || meta.expires > expiresSoon) {
        return {
          certPem: fs.readFileSync(certFile, "utf8"),
          keyPem: fs.readFileSync(keyFile, "utf8"),
          generated: false,
        }
      }
    } catch {
      /* regenerate below */
    }
  }
    const pems = await selfsigned.generate([{ name: "commonName", value: "Mistral Vibe Nexus" }], {
      notAfterDate: new Date(Date.now() + 825 * 24 * 60 * 60 * 1000),
      keySize: 2048,
      algorithm: "sha256",
      extensions: [{
        name: "subjectAltName",
        altNames: [
          { type: 2, value: "localhost" },
          { type: 7, ip: "127.0.0.1" },
          { type: 7, ip: "::1" },
        ],
      }],
    })
  fs.writeFileSync(certFile, pems.cert)
  fs.writeFileSync(keyFile, pems.private)
  fs.writeFileSync(metaFile, JSON.stringify({ expires: Date.now() + 825 * 24 * 60 * 60 * 1000 }))
  return { certPem: pems.cert, keyPem: pems.private, generated: true }
}

export async function regenerateCertificate(serve: ServeConfig): Promise<{ certPem: string; keyPem: string }> {
  if (serve.certPath && serve.keyPath) {
    throw new Error("Custom certificate in use - remove cert/key paths first")
  }
  const dir = userDataServeDir()
    const pems = await selfsigned.generate([{ name: "commonName", value: "Mistral Vibe Nexus" }], {
      notAfterDate: new Date(Date.now() + 825 * 24 * 60 * 60 * 1000),
      keySize: 2048,
      algorithm: "sha256",
    })
  fs.writeFileSync(path.join(dir, "cert.pem"), pems.cert)
  fs.writeFileSync(path.join(dir, "key.pem"), pems.private)
  fs.writeFileSync(
    path.join(dir, "cert-meta.json"),
    JSON.stringify({ expires: Date.now() + 825 * 24 * 60 * 60 * 1000 }),
  )
  return { certPem: pems.cert, keyPem: pems.private }
}

// ---------- login page ----------

function loginPageHtml(): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mistral Vibe Nexus</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; min-height:100vh; display:flex; align-items:center; justify-content:center;
    background:#111115; color:#e8e8ef; font-family:'Segoe UI',system-ui,sans-serif; }
  .card { width:340px; padding:36px 34px; background:#1a1a22; border:1px solid #2a2a36;
    border-radius:14px; box-shadow:0 12px 40px rgba(0,0,0,.5); }
  h1 { margin:0 0 6px; font-size:19px; font-weight:700; }
  .sub { margin:0 0 24px; color:#8a8a98; font-size:12.5px; line-height:1.5; }
  label { display:block; margin:14px 0 6px; font-size:12px; color:#a0a0b0; }
  input { width:100%; padding:10px 12px; background:#111115; border:1px solid #2a2a36;
    border-radius:8px; color:#e8e8ef; font-size:14px; outline:none; }
  input:focus { border-color:#5b6cff; }
  button { margin-top:22px; width:100%; padding:11px; background:#5b6cff; color:#fff;
    border:none; border-radius:8px; font-size:14px; font-weight:600; cursor:pointer; }
  button:hover { background:#6d7cff; }
  button:disabled { opacity:.6; cursor:default; }
  .err { margin-top:14px; color:#ff6b6b; font-size:12.5px; min-height:16px; }
  .lock { text-align:center; font-size:30px; margin-bottom:10px; }
</style>
</head>
<body>
<div class="card">
  <div class="lock">&#128274;</div>
  <h1>Mistral Vibe Nexus</h1>
  <div class="sub">Sign in to access the hosted instance. / Zaloguj si&ecirc;, aby uzyska&cacute; dost&eogon;p do hostowanej instancji.</div>
  <form id="f" autocomplete="off">
    <label for="u" data-i18n="user">Username</label>
    <input id="u" name="username" type="text" autocomplete="username" required>
    <label for="p" data-i18n="pass">Password</label>
    <input id="p" name="password" type="password" autocomplete="current-password" required>
    <button id="b" type="submit" data-i18n="login">Sign in</button>
    <div class="err" id="e"></div>
  </form>
</div>
<script>
  (function(){
    var pl = (navigator.language||'').toLowerCase().indexOf('pl')===0;
    if (pl) {
      document.querySelector('[data-i18n=user]').textContent='Login';
      document.querySelector('[data-i18n=pass]').textContent='Has\u0142o';
      document.querySelector('[data-i18n=login]').textContent='Zaloguj si\u0119';
    }
    document.getElementById('f').addEventListener('submit', function(ev){
      ev.preventDefault();
      var b=document.getElementById('b'); b.disabled=true;
      fetch('/api/auth/login',{method:'POST',headers:{'Content-Type':'application/json'},
        body:JSON.stringify({username:document.getElementById('u').value,
        password:document.getElementById('p').value})})
      .then(function(r){
        if (r.ok) { location.href='/'; return; }
        b.disabled=false;
        document.getElementById('e').textContent = pl ?
          'Nieprawid\u0142owy login lub has\u0142o.' : 'Invalid username or password.';
      }).catch(function(){ b.disabled=false;
        document.getElementById('e').textContent='Connection error.'; });
    });
  })();
</script>
</body>
</html>`
}

// ---------- static renderer ----------

const MIME: Record<string, string> = {
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  mjs: "text/javascript; charset=utf-8",
  css: "text/css; charset=utf-8",
  json: "application/json; charset=utf-8",
  webmanifest: "application/manifest+json",
  png: "image/png",
  svg: "image/svg+xml",
  ico: "image/x-icon",
  woff2: "font/woff2",
  woff: "font/woff",
  wasm: "application/wasm",
  map: "application/json",
  txt: "text/plain; charset=utf-8",
  md: "text/markdown; charset=utf-8",
  pdf: "application/pdf",
}

function sendFile(res: ServerResponse, filePath: string): void {
  let data: Buffer
  try {
    data = fs.readFileSync(filePath)
  } catch {
    res.writeHead(404, { "Content-Type": "text/plain" })
    res.end("Not found")
    return
  }
  const ext = filePath.split(".").pop()?.toLowerCase() ?? ""
  const mime = MIME[ext] ?? "application/octet-stream"
  res.writeHead(200, {
    "Content-Type": mime,
    "Cache-Control": "no-cache",
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Strict-Transport-Security": "max-age=31536000",
  })
  res.end(data)
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
  })
  res.end(JSON.stringify(body))
}

async function readBody(req: IncomingMessage, limit = 64 * 1024): Promise<Buffer> {
  const chunks: Buffer[] = []
  let size = 0
  return new Promise((resolve, reject) => {
    req.on("data", (c: Buffer) => {
      size += c.length
      if (size > limit) {
        reject(new Error("Body too large"))
        req.destroy()
        return
      }
      chunks.push(c)
    })
    req.on("end", () => resolve(Buffer.concat(chunks)))
    req.on("error", reject)
  })
}

// ---------- server lifecycle ----------

let server: https.Server | null = null
let wss: WebSocketServer | null = null
let servePort = 0

export function isServeRunning(): boolean {
  return server !== null && server.listening
}

export function serveStatus(): { running: boolean; port: number; url: string } {
  return { running: isServeRunning(), port: servePort, url: servePort ? `https://<host>:${servePort}` : "" }
}

/** Channels a web client may NEVER invoke (local-only features). */
const WEB_BLOCKED = new Set([
  "window:minimize",
  "window:maximize",
  "window:close",
  "dialog:open-folder",
  "dialog:open-file",
  "shell:open-path",
  "setup:install",
  "serve:start",
  "serve:stop",
  "serve:save-config",
  "serve:set-credentials",
  "serve:regen-cert",
  "remote:install-acp",
])

/**
 * Guard for config:save arriving from a web client: the serve section
 * (credentials, port, certs, enabled flag) is managed ONLY locally.
 */
function protectServeSection(args: unknown[]): unknown[] {
  const cfg = loadConfig()
  const next = args.slice()
  if (next[0] && typeof next[0] === "object") {
    const obj = next[0] as Record<string, unknown>
    obj.serve = cfg.serve
  }
  return next
}

async function dispatchForWeb(ch: string, args: unknown[]): Promise<unknown> {
  if (WEB_BLOCKED.has(ch)) {
    throw new Error(`Channel ${ch} is local-only`)
  }
  const handler = localHandlers.get(ch)
  if (!handler) {
    throw new Error(`Unknown channel: ${ch}`)
  }
  const callArgs = ch === "config:save" ? protectServeSection(args) : args
  return await Promise.resolve(handler(null as never, ...(callArgs as never[])))
}

export async function startServe(serve: ServeConfig, rendererDir: string): Promise<void> {
  if (isServeRunning()) throw new Error("Serve already running")
  if (!credentialsConfigured(serve)) {
    throw new Error("Serve credentials not configured - set login & password in local Settings or via --user/--password")
  }
  const { certPem, keyPem } = await ensureCertificate(serve)
  servePort = serve.port

  const reqHandler = (req: IncomingMessage, res: ServerResponse): void => {
    const url = (req.url ?? "/").split("?")[0]
    const ip = req.socket.remoteAddress ?? "?"

    if (req.method === "POST" && url === "/api/auth/login") {
      void (async () => {
        try {
          if (rateLimited(ip)) {
            sendJson(res, 429, { ok: false, error: "Too many attempts" })
            return
          }
          const body = JSON.parse((await readBody(req)).toString("utf8")) as { username?: string; password?: string }
          const ok = verifyCredentials(String(body.username ?? ""), String(body.password ?? ""), serve)
          if (!ok) {
            sendJson(res, 401, { ok: false, error: "Invalid credentials" })
            return
          }
          const token = createToken(String(body.username))
          res.writeHead(200, {
            "Content-Type": "application/json",
            "Set-Cookie": `${COOKIE_NAME}=${token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=${TOKEN_TTL_MS / 1000}`,
            "Cache-Control": "no-store",
          })
          res.end(JSON.stringify({ ok: true }))
        } catch {
          sendJson(res, 400, { ok: false, error: "Bad request" })
        }
      })()
      return
    }

    if (req.method === "POST" && url === "/api/auth/logout") {
      const cookies = parseCookies(req.headers.cookie)
      if (cookies[COOKIE_NAME]) tokens.delete(cookies[COOKIE_NAME])
      res.writeHead(200, {
        "Set-Cookie": `${COOKIE_NAME}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0`,
        "Content-Type": "application/json",
      })
      res.end(JSON.stringify({ ok: true }))
      return
    }

    if (url === "/login") {
      if (tokenFromRequest(req)) {
        res.writeHead(302, { Location: "/" })
        res.end()
        return
      }
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
      res.end(loginPageHtml())
      return
    }

    if (!tokenFromRequest(req)) {
      if (url === "/" || url === "/index.html") {
        res.writeHead(302, { Location: "/login" })
        res.end()
        return
      }
      sendJson(res, 401, { ok: false, error: "Unauthorized" })
      return
    }

    if (url === "/") {
      sendFile(res, path.join(rendererDir, "index.html"))
      return
    }
    if (url === "/sw.js") {
      res.writeHead(200, {
        "Content-Type": "text/javascript; charset=utf-8",
        "Service-Worker-Allowed": "/",
        "Cache-Control": "no-cache",
      })
      res.end(fs.readFileSync(path.join(rendererDir, "sw.js")))
      return
    }

    let safe = url
    while (safe.startsWith("../")) safe = safe.slice(3)
    while (safe.startsWith("..\\/")) safe = safe.slice(3)
    const norm = path.normalize(safe)

    const filePath = path.join(rendererDir, norm)
    if (!filePath.startsWith(rendererDir)) {
      sendJson(res, 403, { ok: false, error: "Forbidden" })
      return
    }
    sendFile(res, filePath)
  }

  server = https.createServer({ key: keyPem, cert: certPem }, reqHandler)
  server.listen(serve.port, () => {
    console.log(`[serve] HTTPS listening on https://0.0.0.0:${serve.port} (HTTP is not supported)`)
  })
  server.on("error", (err) => {
    console.error("[serve] server error:", err.message)
    server = null
  })

  // Reject unauthorized upgrades BEFORE the websocket handshake completes.
  wss = new WebSocketServer({ noServer: true, maxPayload: 2 * 1024 * 1024 })
  server.on("upgrade", (req, socket, head) => {
    if (!tokenFromRequest(req)) {
      socket.write("HTTP/1.1 401 Unauthorized\r\n\r\n")
      socket.destroy()
      return
    }
    wss?.handleUpgrade(req, socket, head, (ws) => wss?.emit("connection", ws, req))
  })
  wss.on("connection", (ws, req) => {
    const auth = tokenFromRequest(req)
    if (!auth) {
      ws.close(4401, "unauthorized")
      return
    }
    ws.on("message", (raw: Buffer) => {
      let msg: { t?: string; id?: string; ch?: string; args?: unknown[] }
      try {
        msg = JSON.parse(raw.toString("utf8"))
      } catch {
        return
      }
      if (msg.t !== "invoke" || !msg.id || !msg.ch) return
      void dispatchForWeb(msg.ch, msg.args ?? [])
        .then((value) => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ t: "res", id: msg.id, ok: true, value }))
          }
        })
        .catch((err: Error) => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ t: "res", id: msg.id, ok: false, error: err.message }))
          }
        })
    })
  })

  bus.on("bus-event", busListener)
}

const busListener = (ev: { ch: string; args: unknown[] }): void => {
  if (!wss) return
  const payload = JSON.stringify({ t: "ev", ch: ev.ch, args: ev.args })
  for (const client of wss.clients) {
    if (client.readyState === WebSocket.OPEN) client.send(payload)
  }
}

export function stopServe(): void {
  bus.removeListener("bus-event", busListener)
  if (wss) {
    for (const c of wss.clients) c.close(1001)
    wss.close()
    wss = null
  }
  if (server) {
    server.close()
    server = null
  }
  console.log("[serve] stopped")
}
