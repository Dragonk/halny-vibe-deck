import type { AppAPI } from '../../preload/index'

declare global {
  interface Window {
    api: AppAPI
  }
}

export {}
