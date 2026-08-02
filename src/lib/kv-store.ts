import { IS_TAURI } from "@/lib/platform"

type JsonValue = unknown
type KvLike = {
  get<T>(key: string): Promise<T | null>
  set<T>(key: string, value: T): Promise<void>
  save(): Promise<void>
  delete(key: string): Promise<void>
  keys(): Promise<string[]>
  has(key: string): Promise<boolean>
}

let tauriKv: KvLike | null = null
let tauriKvLoaded = false
let tauriKvLoading: Promise<KvLike | null> | null = null

const LS_PREFIX = "llm-wiki-kv:"

function lsKey(storeName: string, key: string): string {
  return `${LS_PREFIX}${storeName}:${key}`
}

function lsGetAll(storeName: string): Record<string, JsonValue> {
  if (typeof window === "undefined") return {}
  const out: Record<string, JsonValue> = {}
  const prefix = `${LS_PREFIX}${storeName}:`;
  for (let i = 0; i < window.localStorage.length; i += 1) {
    const k = window.localStorage.key(i)
    if (!k || !k.startsWith(prefix)) continue
    const raw = window.localStorage.getItem(k)
    if (raw === null) continue
    try {
      out[k.slice(prefix.length)] = JSON.parse(raw)
    } catch {
      out[k.slice(prefix.length)] = raw
    }
  }
  return out
}

function lsGet<T>(storeName: string, key: string): T | null {
  if (typeof window === "undefined") return null
  const raw = window.localStorage.getItem(lsKey(storeName, key))
  if (raw === null) return null
  try {
    return JSON.parse(raw) as T
  } catch {
    return null
  }
}

function lsSet<T>(storeName: string, key: string, value: T): void {
  if (typeof window === "undefined") return
  window.localStorage.setItem(lsKey(storeName, key), JSON.stringify(value))
}

function lsDelete(storeName: string, key: string): void {
  if (typeof window === "undefined") return
  window.localStorage.removeItem(lsKey(storeName, key))
}

function lsHas(storeName: string, key: string): boolean {
  if (typeof window === "undefined") return false
  return window.localStorage.getItem(lsKey(storeName, key)) !== null
}

function lsKeys(storeName: string): string[] {
  return Object.keys(lsGetAll(storeName))
}

function makeWebKv(storeName: string): KvLike {
  return {
    async get<T>(key: string): Promise<T | null> {
      return lsGet<T>(storeName, key)
    },
    async set<T>(key: string, value: T): Promise<void> {
      lsSet(storeName, key, value)
    },
    async save(): Promise<void> {
      // localStorage writes are durable on every set; no extra save.
    },
    async delete(key: string): Promise<void> {
      lsDelete(storeName, key)
    },
    async keys(): Promise<string[]> {
      return lsKeys(storeName)
    },
    async has(key: string): Promise<boolean> {
      return lsHas(storeName, key)
    },
  }
}

async function loadTauriKv(storeName: string): Promise<KvLike | null> {
  if (!IS_TAURI) return null
  const mod = await import("@tauri-apps/plugin-store")
  const store = await mod.load(storeName, { autoSave: true, defaults: {} })
  return store as unknown as KvLike
}

export async function getKvStore(storeName: string): Promise<KvLike> {
  if (tauriKvLoaded) {
    return (tauriKv ?? makeWebKv(storeName))
  }
  if (tauriKvLoading) {
    await tauriKvLoading
    return (tauriKv ?? makeWebKv(storeName))
  }
  tauriKvLoading = loadTauriKv(storeName)
  const loaded = await tauriKvLoading
  tauriKvLoading = null
  tauriKvLoaded = true
  tauriKv = loaded
  return (loaded ?? makeWebKv(storeName))
}

export function isWebKv(_name = "default"): boolean {
  return !IS_TAURI
}
