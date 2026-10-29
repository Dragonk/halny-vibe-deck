// Playwright smoke suite for the Halny Vibe Deck headless server.
//
// Runs in GitHub CI only (never on the dev machine — that one runs live
// deck sessions). Covers what the static checks cannot: the served PWA
// surface, the rendered app shell in web mode (brand, sidebar bottom
// controls) and the remote login flow.

import { expect, test } from '@playwright/test'

const LOCAL = process.env.VD_LOCAL_URL || 'https://127.0.0.1:8080'
const REMOTE = process.env.VD_REMOTE_URL || 'https://127.0.0.1:8443'
const USER = process.env.VD_USER || 'e2e'
const PASS = process.env.VD_PASS || 'e2e-secret'

test('PWA surface: manifest, service worker, both icon sizes', async ({ request }) => {
  const manifest = await request.get(`${LOCAL}/manifest.webmanifest`)
  expect(manifest.status()).toBe(200)
  expect(manifest.headers()['content-type']).toContain('manifest+json')
  const m = await manifest.json()
  expect(m.id).toBe('/')
  expect(m.name).toBe('Halny Vibe Deck')
  const sizes = m.icons.map((i: { sizes: string }) => i.sizes)
  expect(sizes).toContain('256x256')
  expect(sizes).toContain('512x512')
  const purposes = m.icons.map((i: { purpose: string }) => i.purpose)
  expect(purposes).toContain('any')
  expect(purposes).toContain('maskable')

  const sw = await request.get(`${LOCAL}/sw.js`)
  expect(sw.status()).toBe(200)
  expect(await sw.text()).toContain('halny-vibe-deck-shell-')

  for (const [path, size] of [
    ['/icon-256.png', 256],
    ['/icon-512.png', 512],
  ] as const) {
    const icon = await request.get(`${LOCAL}${path}`)
    expect(icon.status()).toBe(200)
    expect(icon.headers()['content-type']).toBe('image/png')
    const body = await icon.body()
    // PNG magic + IHDR width/height
    expect(body.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a')
    expect(body.readUInt32BE(16)).toBe(size)
    expect(body.readUInt32BE(20)).toBe(size)
  }
})

test('web shell renders the brand and the web-only controls', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text())
  })
  await page.goto(LOCAL, { waitUntil: 'domcontentloaded' })

  // Stable v1.0.27 layout: the browser has no header — the sessions panel
  // owns the brand row and two floating buttons toggle the panels.
  await expect(page.locator('header')).toHaveCount(0)
  await expect(page.locator('.sidebar-brand .sidebar-brand-name')).toContainText('Halny')
  const mark = page.locator('.sidebar-brand svg').first()
  await expect(mark).toBeVisible()
  const box = await mark.boundingBox()
  expect(box && box.width > 8 && box.height > 8).toBeTruthy()

  // Web-only bottom controls (stable v1.0.33 placement)
  await expect(page.locator('.sidebar-logout-btn')).toBeVisible()
  // Floating preview toggle is present at any width
  await expect(page.locator('.web-fab-right')).toBeVisible()

  // No fatal client errors during hydration. The dioxus hot-reload socket
  // (`/_dioxus`) only exists under `dx serve`, and the PWA icon/manifest
  // fetches are allowed to 404 in dev — everything else must be clean.
  const fatal = errors.filter(
    // self-signed cert: chromium still logs SSL errors for subresource
    // fetches even with ignoreHTTPSErrors — expected, not fatal
    (e) => !/favicon|manifest|_dioxus|WebSocket|SSL|certificate/i.test(e),
  )
  expect(fatal).toEqual([])
})

test('remote login flow: reject, accept, logout', async ({ page }) => {
  await page.goto(`${REMOTE}/login`, { waitUntil: 'domcontentloaded' })
  await expect(page.locator('#u')).toBeVisible()
  await expect(page.locator('#p')).toBeVisible()

  await page.fill('#u', USER)
  await page.fill('#p', 'wrong-password')
  await page.click('#b')
  // Rejected: stays on /login and shows the localized error line
  await expect(page.locator('#e')).not.toBeEmpty({ timeout: 15_000 })
  await expect(page).toHaveURL(/\/login/)

  await page.fill('#u', USER)
  await page.fill('#p', PASS)
  // Assert the wire result of the login POST: a bare navigation timeout
  // hides whether it was 401, 429 or a 5xx.
  const [resp] = await Promise.all([
    page.waitForResponse(
      (r) => r.url().includes('/api/auth/login') && r.request().method() === 'POST',
    ),
    page.click('#b'),
  ])
  expect(resp.status(), `login POST for ${USER}`).toBe(200)
  await page.waitForURL((url) => !url.pathname.startsWith('/login'), { timeout: 30_000 })
  // Logged in: the app shell with the web-only bottom controls
  await expect(page.locator('.sidebar-logout-btn')).toBeVisible({ timeout: 30_000 })

  // Logout must clear the session and land back on the login page. The
  // sessions panel animates, so the click is forced (same as the stable
  // suite).
  await page.locator('.sidebar-logout-btn').evaluate((el) => (el as HTMLElement).click())
  await page.waitForURL(/\/login/, { timeout: 30_000 })
})
