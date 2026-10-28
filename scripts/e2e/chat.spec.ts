// Deep chat e2e: real prompt round-trips against the scripted agent.
//
// Covers the flows the static/PWA specs cannot:
//   1. prompt → streamed assistant text → completed turn;
//   2. a mid-turn tool approval (callback/call → decision → continuation);
//   3. backfill after a "hidden tab": events emitted while the client was
//      away are recovered by the visibility resync (the engine re-reads the
//      agent state) — WITHOUT a page reload. This is the stable-edition bug
//      ("15 minutes missing until you refresh") that must not come back.

import { expect, test, type Page } from '@playwright/test'

const LOCAL = process.env.VD_LOCAL_URL || 'http://127.0.0.1:8080'

async function openNewSession(page: Page) {
  await page.goto(LOCAL, { waitUntil: 'domcontentloaded' })
  await expect(page.locator('.sidebar-logout-btn')).toBeVisible({ timeout: 30_000 })
  await page.click('.new-session-btn')
  await expect(page.locator('textarea.input-card-editor')).toBeVisible({ timeout: 30_000 })
}

async function sendPrompt(page: Page, text: string) {
  const editor = page.locator('textarea.input-card-editor')
  await editor.fill(text)
  await editor.press('Enter')
}

test('prompt streams a reply and completes the turn', async ({ page }) => {
  await openNewSession(page)
  await sendPrompt(page, 'hello e2e')

  const assistant = page.locator('.message-list')
  await expect(assistant).toContainText('Working on it.', { timeout: 30_000 })
  await expect(assistant).toContainText('All done.', { timeout: 30_000 })
})

test('tool approval is asked mid-turn and the turn continues', async ({ page }) => {
  await openNewSession(page)
  await sendPrompt(page, '@approval please run it')

  const approve = page.locator('.tool-approval-btn').first()
  await expect(approve).toBeVisible({ timeout: 30_000 })
  await approve.click()

  await expect(page.locator('.message-list')).toContainText('All done.', { timeout: 40_000 })
})

test('hidden tab: missed events are backfilled without a reload', async ({ page }) => {
  await openNewSession(page)
  // "@gap": the agent keeps the rest of the reply in its own session state
  // after the first chunk, i.e. the client never receives it as an event.
  await sendPrompt(page, '@gap long running job')

  const list = page.locator('.message-list')
  await expect(list).toContainText('Working on it.', { timeout: 30_000 })
  // The remaining text is NOT on the client yet (it never reached the
  // stream). That is exactly the state after returning to a tab that
  // missed a long stretch of output.
  await expect(list).not.toContainText('All done.')

  // Returning to the tab fires visibilitychange; the app must ask the agent
  // for the authoritative state and repaint. No page.reload() here.
  await page.evaluate(() => {
    document.dispatchEvent(new Event('visibilitychange'))
  })

  await expect(list).toContainText('All done.', { timeout: 40_000 })
})
