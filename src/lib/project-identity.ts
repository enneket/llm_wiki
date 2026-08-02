import { IS_TAURI } from "@/lib/platform"
import { getKvStore } from "@/lib/kv-store"
import { normalizePath } from "@/lib/path-utils"

const STORE_NAME = "app-state.json"
const REGISTRY_KEY = "projectRegistry"

export interface ProjectIdentity {
  id: string
  createdAt: number
}

export interface ProjectRegistryEntry {
  id: string
  path: string
  name: string
  lastOpened: number
}

export type ProjectRegistry = Record<string, ProjectRegistryEntry>

function generateProjectId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID()
  }
  const rand = Math.random().toString(16).slice(2)
  return `project-${Date.now().toString(16)}-${rand}`
}

async function readFileWeb(path: string): Promise<string | null> {
  if (typeof window === "undefined") return null
  const raw = window.localStorage.getItem(`llm-wiki-fs:${path}`)
  return raw
}

async function writeFileWeb(path: string, contents: string): Promise<void> {
  if (typeof window === "undefined") return
  window.localStorage.setItem(`llm-wiki-fs:${path}`, contents)
}

async function readFileDesktop(path: string): Promise<string> {
  const { readFile } = await import("@/commands/fs")
  return readFile(path)
}

async function writeFileDesktop(path: string, contents: string): Promise<void> {
  const { writeFile } = await import("@/commands/fs")
  return writeFile(path, contents)
}

async function readFileAny(path: string): Promise<string | null> {
  if (IS_TAURI) {
    try {
      return await readFileDesktop(path)
    } catch {
      return null
    }
  }
  return readFileWeb(path)
}

async function writeFileAny(path: string, contents: string): Promise<void> {
  if (IS_TAURI) {
    return writeFileDesktop(path, contents)
  }
  return writeFileWeb(path, contents)
}

function identityPath(projectPath: string): string {
  return `${normalizePath(projectPath)}/.llm-wiki/project.json`
}

export async function ensureProjectId(projectPath: string): Promise<string> {
  const path = identityPath(projectPath)
  try {
    const raw = await readFileAny(path)
    if (raw) {
      const parsed = JSON.parse(raw) as ProjectIdentity
      if (parsed?.id && typeof parsed.id === "string") {
        return parsed.id
      }
    }
  } catch {
    // missing or corrupt — fall through to create
  }
  const identity: ProjectIdentity = {
    id: generateProjectId(),
    createdAt: Date.now(),
  }
  try {
    await writeFileAny(path, JSON.stringify(identity, null, 2))
  } catch (err) {
    console.warn("[project-identity] failed to write identity file:", err)
  }
  return identity.id
}

async function getStore() {
  return getKvStore(STORE_NAME)
}

export async function loadRegistry(): Promise<ProjectRegistry> {
  try {
    const store = await getStore()
    const registry = await store.get<ProjectRegistry>(REGISTRY_KEY)
    return registry ?? {}
  } catch {
    return {}
  }
}

async function saveRegistry(registry: ProjectRegistry): Promise<void> {
  const store = await getStore()
  await store.set(REGISTRY_KEY, registry)
}

export async function upsertProjectInfo(
  id: string,
  path: string,
  name: string,
): Promise<void> {
  const registry = await loadRegistry()
  registry[id] = {
    id,
    path: normalizePath(path),
    name,
    lastOpened: Date.now(),
  }
  await saveRegistry(registry)
}

export async function getProjectPathById(id: string): Promise<string | null> {
  const registry = await loadRegistry()
  return registry[id]?.path ?? null
}

export async function getProjectIdByPath(path: string): Promise<string | null> {
  const normalized = normalizePath(path)
  const registry = await loadRegistry()
  for (const entry of Object.values(registry)) {
    if (entry.path === normalized) return entry.id
  }
  return null
}
