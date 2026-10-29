import test from "node:test"
import assert from "node:assert/strict"
import { createRequire } from "node:module"

const require = createRequire(import.meta.url)
const token = require("../../out/main/token.js")

const KEY = "11".repeat(32)
const KEYS = { k1: KEY }
const SLIDING = 180 * 24 * 60 * 60 * 1000
const ABSOLUTE = 365 * 24 * 60 * 60 * 1000
const opts = { slidingMs: SLIDING, absoluteMs: ABSOLUTE }

function makeToken(user = "dragonk", ver = 0, iatOffset = 0) {
  const now = Date.now() + iatOffset
  const payload = token.newPayload(user, ver, opts, now, now)
  return { token: token.signToken(payload, "k1", KEY), payload }
}

test("a freshly issued token verifies and carries the user", () => {
  const { token: t } = makeToken()
  const res = token.verifyTokenWith({ token: t, keys: KEYS, ver: 0, absoluteMs: ABSOLUTE })
  assert.equal(res.user, "dragonk")
})

test("a token signed with another key is rejected", () => {
  const { token: t } = makeToken()
  const res = token.verifyTokenWith({
    token: t,
    keys: { k1: "22".repeat(32) },
    ver: 0,
    absoluteMs: ABSOLUTE,
  })
  assert.equal(res, null)
})

test("a tampered payload is rejected", () => {
  const { token: t } = makeToken("user")
  const [v, body, mac] = t.split(".")
  const decoded = JSON.parse(Buffer.from(body, "base64url").toString("utf8"))
  decoded.u = "admin"
  const forged = [v, Buffer.from(JSON.stringify(decoded)).toString("base64url"), mac].join(".")
  assert.equal(
    token.verifyTokenWith({ token: forged, keys: KEYS, ver: 0, absoluteMs: ABSOLUTE }),
    null,
  )
  assert.equal(t, t)
})

test("an expired token is rejected", () => {
  const now = Date.now()
  const payload = token.newPayload("dragonk", 0, opts, now, now)
  const expired = { ...payload, exp: now - 1 }
  const t = token.signToken(expired, "k1", KEY)
  assert.equal(token.verifyTokenWith({ token: t, keys: KEYS, ver: 0, absoluteMs: ABSOLUTE }), null)
})

test("the absolute cap kills an otherwise fresh (renewed) token", () => {
  const now = Date.now()
  // issued a year and a day ago, renewed one minute ago
  const iat = now - ABSOLUTE - 1000
  const payload = token.newPayload("dragonk", 0, opts, iat, now)
  const t = token.signToken(payload, "k1", KEY)
  assert.equal(token.verifyTokenWith({ token: t, keys: KEYS, ver: 0, absoluteMs: ABSOLUTE }), null)
})

test("a credentials change invalidates every existing token", () => {
  const { token: t } = makeToken("dragonk", 0)
  assert.equal(
    token.verifyTokenWith({ token: t, keys: KEYS, ver: 1, absoluteMs: ABSOLUTE }),
    null,
  )
})

test("a revoked token id is rejected", () => {
  const { token: t, payload } = makeToken()
  const revoked = new Set([payload.jti])
  assert.equal(
    token.verifyTokenWith({
      token: t,
      keys: KEYS,
      ver: 0,
      absoluteMs: ABSOLUTE,
      isRevoked: (jti) => revoked.has(jti),
    }),
    null,
  )
})

test("the cookie is renewed only when less than half the window is left", () => {
  const now = Date.now()
  const fresh = { u: "u", iat: now, exp: now + SLIDING, ver: 0, jti: "a" }
  const old = { ...fresh, exp: now + SLIDING * 0.25 }
  assert.equal(token.needsRefresh(fresh, SLIDING, now), false)
  assert.equal(token.needsRefresh(old, SLIDING, now), true)
})

test("renewal keeps the original issued-at (absolute deadline does not move)", () => {
  const iat = Date.now() - 100 * 24 * 60 * 60 * 1000
  const renewed = token.newPayload("dragonk", 0, opts, iat)
  assert.equal(renewed.iat, iat)
  assert.ok(renewed.exp <= iat + ABSOLUTE)
})
