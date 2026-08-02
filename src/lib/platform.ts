/**
 * Platform detection and shared runtime config.
 *
 * Two delivery targets ship from the same `npm run build` output:
 *
 * - Desktop (Tauri): `window.__TAURI_INTERNALS__` is defined, Tauri commands
 *   are routed through `@tauri-apps/api/core`'s `invoke`, and the OS shell is
 *   reachable via Tauri plugins.
 *
 * - Browser web: `window.__TAURI_INTERNALS__` is undefined, all Tauri calls
 *   must instead hit the headless Rust web server bundled in
 *   `llm-wiki-web` (see `src-tauri/src/web/`). The same JSON shapes flow
 *   across both runtimes so the React tree does not need to branch.
 */

export type Platform = "tauri" | "web"

export function detectPlatform(): Platform {
  if (typeof window === "undefined") return "web"
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown }
  if (w.__TAURI_INTERNALS__ || w.__TAURI__) return "tauri"
  return "web"
}

export const PLATFORM: Platform = detectPlatform()

export const IS_TAURI = PLATFORM === "tauri"
export const IS_WEB = PLATFORM === "web"

let cachedBaseUrl: string | null = null

/**
 * Resolve the base URL of the headless web server when running outside
 * Tauri. In Tauri mode the API server is in-process and there is no HTTP
 * to talk to (the desktop UI uses Tauri's `invoke` directly).
 */
export function getWebBaseUrl(): string {
  if (cachedBaseUrl) return cachedBaseUrl
  if (typeof window !== "undefined") {
    const w = window as unknown as { __LLM_WIKI_API_BASE_URL__?: string }
    if (typeof w.__LLM_WIKI_API_BASE_URL__ === "string" && w.__LLM_WIKI_API_BASE_URL__) {
      cachedBaseUrl = w.__LLM_WIKI_API_BASE_URL__
      return cachedBaseUrl
    }
  }
  const envBase =
    typeof import.meta !== "undefined" &&
    typeof (import.meta as unknown as { env?: Record<string, string> }).env?.VITE_WEB_API_BASE_URL === "string"
      ? (import.meta as unknown as { env: Record<string, string> }).env.VITE_WEB_API_BASE_URL
      : ""
  if (envBase) {
    cachedBaseUrl = envBase
    return cachedBaseUrl
  }
  if (typeof window !== "undefined" && window.location) {
    cachedBaseUrl = window.location.origin
    return cachedBaseUrl
  }
  cachedBaseUrl = "http://127.0.0.1:8080"
  return cachedBaseUrl
}

export function setWebBaseUrl(url: string): void {
  cachedBaseUrl = url
}
