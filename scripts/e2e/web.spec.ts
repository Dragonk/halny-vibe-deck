// Golden e2e suite for the stable (Electron) edition.
//
// These scenarios are the reference behaviour the Rust edition (`next`
// branch of the same repository) is measured against: near-identical specs
// run there against its headless server, so "1:1 with the stable edition"
// is checked by machine instead of by eye.
//
// Runs in GitHub CI only (a runner has no live deck sessions).

import { expect, test } from '@playwright/test'

const BASE = process.env.VD_REMOTE_URL || 'https://127.0.0.1:8443'
const USER = process.env.VD_USER || 'e2e'
const PASS = process.env.VD_PASS || 'e2e-secret'

test('PWA surface: manifest, service worker, both icon sizes', async ({ request }) => {
  const manifest = await request.get(`${BASE}/manifest.webmanifest`)
  expect(manifest.status()).toBe(200)
  const m = await manifest.json()
  expect(m.name).toBe('Halny Vibe Deck')
  expect(m.id).toBe('/')
  const sizes = m.icons.map((i: { sizes: string }) => i.sizes)
  expect(sizes).toContain('256x256')
  expect(sizes).toContain('512x512')
  const purposes = m.icons.map((i: { purpose: string }) => i.purpose)
  expect(purposes).toContain('any')
  expect(purposes).toContain('maskable')

  const sw = await request.get(`${BASE}/sw.js`)
  expect(sw.status()).toBe(200)

  for (const [path, size] of [
    ['/icon-256.png', 256],
    ['/icon.png', 512],
  ] as const) {
    const icon = await request.get(`${BASE}${path}`)
    expect(icon.status()).toBe(200)
    expect(icon.headers()['content-type']).toBe('image/png')
    const body = await icon.body()
    expect(body.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a')
    expect(body.readUInt32BE(16)).toBe(size)
    expect(body.readUInt32BE(20)).toBe(size)
  }
})

test('login: reject, accept, then logout', async ({ page }) => {
  await page.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' })
  await expect(page.locator('#u')).toBeVisible()

  await page.fill('#u', USER)
  await page.fill('#p', 'wrong-password')
  await page.click('#b')
  await expect(page.locator('#e')).not.toBeEmpty({ timeout: 15_000 })
  await expect(page).toHaveURL(/\/login/)

  await page.fill('#u', USER)
  await page.fill('#p', PASS)
  await page.click('#b')
  await page.waitForURL((url) => !url.pathname.startsWith('/login'), { timeout: 30_000 })

  // Web client: theme + logout live at the bottom of the sessions list
  const logout = page.locator('.sidebar-logout-btn')
  await expect(logout).toBeVisible({ timeout: 30_000 })
  await logout.click()
  await page.waitForURL(/\/login/, { timeout: 30_000 })
})

test('shell renders the brand and the sidebar bottom controls', async ({ page }) => {
  await page.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' })
  await page.fill('#u', USER)
  await page.fill('#p', PASS)
  await page.click('#b')
  await page.waitForURL((url) => !url.pathname.startsWith('/login'), { timeout: 30_000 })

  const brand = page.locator('.sidebar-brand-name, .header-brand-name').first()
  await expect(brand).toContainText('Halny', { timeout: 30_000 })
  await expect(page.locator('.sidebar-logout-btn')).toBeVisible()
})
