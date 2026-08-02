/**
 * Unified opener for external URLs and project paths. In Tauri the
 * `@tauri-apps/plugin-opener` plugin is used; in a browser the
 * platform's `window.open` is used as the only safe default.
 */

import { IS_TAURI } from "@/lib/platform"

let tauriOpener: typeof import("@tauri-apps/plugin-opener") | null = null
let tauriOpenerLoading: Promise<typeof import("@tauri-apps/plugin-opener") | null> | null = null

async function loadTauriOpener(): Promise<typeof import("@tauri-apps/plugin-opener") | null> {
  if (!IS_TAURI) return null
  if (tauriOpener) return tauriOpener
  if (!tauriOpenerLoading) {
    tauriOpenerLoading = import("@tauri-apps/plugin-opener")
      .then((mod) => {
        tauriOpener = mod
        return mod
      })
      .catch(() => null)
  }
  return tauriOpenerLoading
}

export async function openExternalUrl(url: string): Promise<void> {
  if (!url) return
  const opener = await loadTauriOpener()
  if (opener?.openUrl) {
    try {
      await opener.openUrl(url)
      return
    } catch {
      /* fall through to window.open */
    }
  }
  if (typeof window !== "undefined") {
    window.open(url, "_blank", "noopener,noreferrer")
  }
}

export async function revealInFolder(path: string): Promise<void> {
  const opener = await loadTauriOpener()
  if (opener?.revealItemInDir) {
    try {
      await opener.revealItemInDir(path)
      return
    } catch {
      /* ignore in web mode */
    }
  }
}

export async function openPath(path: string): Promise<void> {
  const opener = await loadTauriOpener()
  if (opener?.openPath) {
    try {
      await opener.openPath(path)
      return
    } catch {
      /* ignore in web mode */
    }
  }
}
