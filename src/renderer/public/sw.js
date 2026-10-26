/* Halny Vibe Deck service worker (serve mode / PWA).
 * Cache-first for assets, network-first for the app shell; the API and the
 * the network. /ws is a websocket upgrade — the fetch handler never sees it. */
const CACHE = 'halny-shell-v1'

self.addEventListener('install', () => {
  self.skipWaiting()
})

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim())
})

self.addEventListener('fetch', (event) => {
  const req = event.request
  if (req.method !== 'GET') return
  const url = new URL(req.url)
  if (url.origin !== self.location.origin) return
  if (url.pathname === '/ws' || url.pathname.startsWith('/api/') || url.pathname === '/sw.js' || url.pathname === '/login') return

  // v0.7.2: the app shell (navigation) must be network-first - a stale
  // cached index.html would pin users to the previous release forever.
  const nav = url.pathname
  if (nav === '/' || nav === '/index.html') {
    event.respondWith(
      fetch(req)
        .then((res) => {
          if (res && res.ok) {
            const copy = res.clone()
            caches.open(CACHE).then((c) => c.put(req, copy))
          }
          return res
        })
        .catch(() => caches.match(req).then((hit) => hit || Response.error()))
    )
    return
  }

  event.respondWith(
    caches.match(req).then((hit) => {
      if (hit) return hit
      return fetch(req).then((res) => {
        if (res && res.ok) {
          const p = url.pathname
          const cacheable =
            p.startsWith('/assets/') ||
            p === '/' ||
            p === '/index.html' ||
            p === '/manifest.webmanifest' ||
            p === '/icon-256.png' ||
            p === '/icon.png'
          if (cacheable) {
            const copy = res.clone()
            caches.open(CACHE).then((c) => c.put(req, copy))
          }
        }
        return res
      })
    })
  )
})
