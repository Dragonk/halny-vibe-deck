import crypto from "crypto"

/**
 * v1.0.37: session token format, kept free of Electron so it can be unit
 * tested and reused by any host.
 *
 *     v1.<base64url(payload)>.<base64url(HMAC-SHA256)>
 *
 * The expensive part of the old design was not the crypto but WHERE the
 * sessions lived: an in-memory Map, wiped by every process restart (app
 * self-update, watchdog, port recovery), which is why a phone had to log in
 * again after a few hours. A signed, self-describing token needs no server
 * state at all.
 *
 * Two limits and one kill switch:
 *   - SLIDING window: renewed while the device stays in use,
 *   - ABSOLUTE cap (`iat + absoluteMs`): a token can never live forever,
 *   - `ver`: bumped on every credentials change, so a password change
 *     invalidates every device without keeping a session table.
 */

export interface TokenPayload {
  /** username */
  u: string
  /** issued at (ms) */
  iat: number
  /** absolute expiry (ms) */
  exp: number
  /** credentials version */
  ver: number
  /** token id (revoke list key) */
  jti: string
  /** key id (rotation) */
  kid?: string
}

export interface TokenOptions {
  slidingMs: number
  /** hard cap measured from `iat` */
  absoluteMs: number
}

export function signToken(
  payload: Omit<TokenPayload, "kid">,
  kid: string,
  keyHex: string,
): string {
  const body = Buffer.from(JSON.stringify({ ...payload, kid })).toString("base64url")
  const mac = crypto.createHmac("sha256", Buffer.from(keyHex, "hex")).update(body).digest("base64url")
  return "v1." + body + "." + mac
}

export interface VerifyInput {
  token: string | undefined
  /** key material available to this host, by kid */
  keys: Record<string, string>
  /** current credentials version */
  ver: number
  /** absolute token lifetime cap */
  absoluteMs: number
  now?: number
  /** optional revoke lookup (jti) */
  isRevoked?: (jti: string) => boolean
}

export interface VerifiedToken {
  user: string
  jti: string
  payload: TokenPayload
}

/**
 * Constant-time signature check, then payload validation. Returns null for
 * anything that is not a currently valid token - callers treat null as 401.
 */
export function verifyTokenWith(input: VerifyInput): VerifiedToken | null {
  const { token, keys, ver, absoluteMs } = input
  const now = input.now ?? Date.now()
  if (!token) return null
  const parts = token.split(".")
  if (parts.length !== 3 || parts[0] !== "v1") return null
  const [, body, mac] = parts
  let payload: TokenPayload
  try {
    payload = JSON.parse(Buffer.from(body, "base64url").toString("utf8")) as TokenPayload
  } catch {
    return null
  }
  const kid = payload.kid && keys[payload.kid] ? payload.kid : undefined
  if (!kid) return null
  const expected = crypto.createHmac("sha256", Buffer.from(keys[kid], "hex")).update(body).digest()
  let given: Buffer
  try {
    given = Buffer.from(mac, "base64url")
  } catch {
    return null
  }
  if (given.length !== expected.length || !crypto.timingSafeEqual(given, expected)) return null

  if (!payload.u) return null
  if (typeof payload.exp !== "number" || payload.exp < now) return null
  if (typeof payload.iat !== "number" || now - payload.iat > absoluteMs) return null
  if (payload.ver !== ver) return null
  if (payload.jti && input.isRevoked?.(payload.jti)) return null
  return { user: payload.u, jti: payload.jti, payload }
}

/**
 * Build a fresh payload. `iat` is carried over from the previous token so the
 * absolute cap is measured from the FIRST login of this chain, not from the
 * last renewal.
 */
export function newPayload(
  user: string,
  ver: number,
  opts: TokenOptions,
  iat = Date.now(),
  now = Date.now(),
): Omit<TokenPayload, "kid"> {
  return {
    u: user,
    iat,
    exp: Math.min(now + opts.slidingMs, iat + opts.absoluteMs),
    ver,
    jti: crypto.randomBytes(16).toString("hex"),
  }
}

/** Should the cookie be re-issued now? (less than half the window left) */
export function needsRefresh(payload: TokenPayload, slidingMs: number, now = Date.now()): boolean {
  return payload.exp - now <= slidingMs * 0.5
}
