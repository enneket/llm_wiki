import type { FileNode, WikiProject } from "@/types/wiki"
import { IS_TAURI } from "@/lib/platform"
import { webRequest, WEB_ENDPOINTS, type WebFileNode, type WebProject } from "@/lib/web-client"
import { isAbsolutePath } from "@/lib/path-utils"
import { useWikiStore } from "@/stores/wiki-store"
import {
  adapterListFiles,
  adapterReadFile,
  adapterWriteFile,
} from "@/lib/adapter"
import { normalizePath } from "@/lib/path-utils"

let tauriInvoke:
  | (<T>(cmd: string, args?: Record<string, unknown>) => Promise<T>)
  | null = null
let tauriInvokeLoaded = false
let tauriInvokeLoading: Promise<unknown> | null = null

async function tauri(): Promise<
  <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
> {
  if (tauriInvokeLoaded) return tauriInvoke as never
  if (tauriInvokeLoading) await tauriInvokeLoading
  if (tauriInvokeLoaded) return tauriInvoke as never
  if (!IS_TAURI) {
    tauriInvokeLoaded = true
    tauriInvoke = null
    return tauriInvoke as never
  }
  tauriInvokeLoading = import("@tauri-apps/api/core").then((mod) => {
    tauriInvoke = mod.invoke as never
    tauriInvokeLoaded = true
  })
  await tauriInvokeLoading
  tauriInvokeLoading = null
  return tauriInvoke as never
}

interface RawProject {
  name: string
  path: string
}

function activeWebProject(): { id: string; path: string } {
  const project = useWikiStore.getState().project
  if (!project) throw new Error("No project is open")
  return project
}

function relativeWebPath(path: string, projectPath: string): string {
  const normalizedPath = normalizePath(path)
  const normalizedProject = normalizePath(projectPath)
  if (normalizedPath === normalizedProject) return ""
  if (!normalizedPath.startsWith(`${normalizedProject}/`)) {
    throw new Error(`Path is outside the active project: ${path}`)
  }
  return normalizedPath.slice(normalizedProject.length + 1)
}

function webFileNodesToLocal(nodes: WebFileNode[]): FileNode[] {
  return nodes.map((node) => ({
    name: node.name,
    path: node.path,
    is_dir: node.isDir,
    children: node.children ? webFileNodesToLocal(node.children) : undefined,
  }))
}

function ensureProjectIdLocal(path: string): Promise<string> {
  return import("@/lib/project-identity").then((m) => m.ensureProjectId(path))
}

function upsertProjectInfoLocal(id: string, path: string, name: string): Promise<void> {
  return import("@/lib/project-identity").then((m) => m.upsertProjectInfo(id, path, name))
}

export async function readFile(
  path: string,
  options?: { extractImages?: boolean },
): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("read_file", {
      path,
      extractImages: options?.extractImages,
    })
  }
  const project = activeWebProject()
  return adapterReadFile(project.id, relativeWebPath(path, project.path))
}

export async function writeFile(path: string, contents: string): Promise<void> {
  assertAbsoluteFsPath("writeFile", path)
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("write_file", { path, contents })
  }
  const project = activeWebProject()
  return adapterWriteFile(project.id, relativeWebPath(path, project.path), contents)
}

export async function writeFileBase64(path: string, base64: string): Promise<void> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("write_file_base64", { path, base64 })
  }
  throw new Error("writeFileBase64 is only available in the desktop runtime")
}

export async function writeFileAtomic(path: string, contents: string): Promise<void> {
  assertAbsoluteFsPath("writeFileAtomic", path)
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("write_file_atomic", { path, contents })
  }
  const project = activeWebProject()
  return adapterWriteFile(project.id, relativeWebPath(path, project.path), contents)
}

export interface ListDirectoryOptions {
  includeHidden?: boolean
  maxDepth?: number
}

const pendingListDirectory = new Map<
  string,
  { request: Promise<FileNode[]>; shared: boolean }
>()

function cloneFileNodes(nodes: FileNode[]): FileNode[] {
  return nodes.map((node) => ({
    ...node,
    children: node.children ? cloneFileNodes(node.children) : node.children,
  }))
}

export async function listDirectory(
  path: string,
  includeHiddenOrOptions: boolean | ListDirectoryOptions = false,
): Promise<FileNode[]> {
  const options =
    typeof includeHiddenOrOptions === "boolean"
      ? { includeHidden: includeHiddenOrOptions }
      : includeHiddenOrOptions
  const includeHidden = options.includeHidden ?? false
  const maxDepth = options.maxDepth
  const requestKey = JSON.stringify([path, includeHidden, maxDepth ?? null])
  const pending = pendingListDirectory.get(requestKey)
  if (pending) {
    pending.shared = true
    return pending.request.then(cloneFileNodes)
  }
  if (IS_TAURI) {
    // Pre-claim the pending slot synchronously so any concurrent
    // caller arriving before the lazy `await tauri()` settles still
    // sees this request as in-flight and dedupes against it. Without
    // this the `await tauri()` microtask gap would let the second
    // call miss the pending check and double-issue the invoke.
    let resolveRequest!: (nodes: FileNode[]) => void
    let rejectRequest!: (err: unknown) => void
    const placeholder = new Promise<FileNode[]>((resolve, reject) => {
      resolveRequest = resolve
      rejectRequest = reject
    })
    const entry = { request: placeholder, shared: false }
    pendingListDirectory.set(requestKey, entry)
    try {
      const invoke = await tauri()
      const upstream = invoke<FileNode[]>("list_directory", {
        path,
        includeHidden,
        maxDepth,
      })
      upstream.then(resolveRequest, rejectRequest)
      // The `.finally(...)` cleanup below returns a derived promise
      // that mirrors `upstream`'s outcome (so it would reject when the
      // directory fetch fails). The rejection handler attached to
      // `entry.request` already covers the caller; this silent `.catch`
      // stops the cleanup's tail from leaking as an unhandled rejection
      // (which Vitest flags even though all 12 fs.test assertions pass).
      upstream
        .finally(() => {
          pendingListDirectory.delete(requestKey)
        })
        .catch(() => {})
    } catch (err) {
      pendingListDirectory.delete(requestKey)
      rejectRequest(err)
    }
    return entry.request.then((nodes) => (entry.shared ? cloneFileNodes(nodes) : nodes))
  }
  const project = activeWebProject()
  const rel = relativeWebPath(path, project.path)
  const root = rel === "" ? "all" : rel === "wiki" || rel.startsWith("wiki/") ? "wiki" : rel === "raw/sources" || rel.startsWith("raw/sources/") ? "sources" : "all"
  const nodes = await adapterListFiles(project.id, {
    root,
    recursive: true,
  })
  return webFileNodesToLocal(nodes)
}

export async function copyFile(source: string, destination: string): Promise<void> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke("copy_file", { source, destination })
  }
  throw new Error("copyFile is only available in the desktop runtime")
}

export async function copyDirectory(
  source: string,
  destination: string,
): Promise<string[]> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string[]>("copy_directory", { source, destination })
  }
  throw new Error("copyDirectory is only available in the desktop runtime")
}

export async function preprocessFile(path: string): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("preprocess_file", { path })
  }
  throw new Error("preprocessFile is only available in the desktop runtime")
}

export async function deleteFile(path: string): Promise<void> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke("delete_file", { path })
  }
  const project = activeWebProject()
  await webRequest(WEB_ENDPOINTS.fileContent(project.id), {
    method: "DELETE",
    body: { path: relativeWebPath(path, project.path) },
  })
}

export async function findRelatedWikiPages(
  projectPath: string,
  sourceName: string,
): Promise<string[]> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string[]>("find_related_wiki_pages", {
      projectPath,
      sourceName,
    })
  }
  throw new Error("findRelatedWikiPages is only available in the desktop runtime")
}

export async function createDirectory(path: string): Promise<void> {
  assertAbsoluteFsPath("createDirectory", path)
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("create_directory", { path })
  }
  activeWebProject()
  return
}

export async function fileExists(path: string): Promise<boolean> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<boolean>("file_exists", { path })
  }
  const project = activeWebProject()
  try {
    await adapterReadFile(project.id, relativeWebPath(path, project.path))
    return true
  } catch {
    return false
  }
}

export async function getFileModifiedTime(path: string): Promise<number> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<number>("get_file_modified_time", { path })
  }
  return 0
}

export async function getFileSize(path: string): Promise<number> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<number>("get_file_size", { path })
  }
  return 0
}

export async function getFileMd5(path: string): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("get_file_md5", { path })
  }
  return ""
}

export interface FileHistoryEntry {
  id: string
  path: string
  timestamp: number
  author: string
  tool: string
  content: string
}

export async function listFileHistory(
  projectPath: string,
  filePath: string,
): Promise<FileHistoryEntry[]> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<FileHistoryEntry[]>("list_file_history", {
      projectPath,
      filePath,
    })
  }
  return []
}

export async function restoreFileHistory(
  projectPath: string,
  filePath: string,
  entryId: string,
): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("restore_file_history", {
      projectPath,
      filePath,
      entryId,
    })
  }
  return ""
}

export async function applyTextSelectionEdit(input: {
  projectPath: string
  filePath: string
  prefix: string
  selectedText: string
  suffix: string
  replacement: string
}): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("apply_text_selection_edit", input)
  }
  return input.replacement
}

export interface PageLinkEntry {
  title: string
  path?: string
  snippet?: string
}

export interface PageLinksResponse {
  outgoing: PageLinkEntry[]
  backlinks: PageLinkEntry[]
  missing: PageLinkEntry[]
}

export async function getPageLinks(
  projectPath: string,
  filePath: string,
): Promise<PageLinksResponse> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<PageLinksResponse>("get_page_links", { projectPath, filePath })
  }
  return { outgoing: [], backlinks: [], missing: [] }
}

export async function createMissingWikiPage(
  projectPath: string,
  title: string,
  content?: string,
): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("create_missing_wiki_page", {
      projectPath,
      title,
      content,
    })
  }
  throw new Error(
    "createMissingWikiPage requires a Tauri runtime; in web mode post via /api/v1/projects/{id}/files/content",
  )
}

export interface FileBase64 {
  base64: string
  mimeType: string
}

export async function readFileAsBase64(path: string): Promise<FileBase64> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<FileBase64>("read_file_as_base64", { path })
  }
  return { base64: "", mimeType: "application/octet-stream" }
}

export async function createProject(
  name: string,
  path: string,
): Promise<WikiProject> {
  if (IS_TAURI) {
    const invoke = await tauri()
    const raw = await invoke<RawProject>("create_project", { name, path })
    const id = await ensureProjectIdLocal(raw.path)
    await upsertProjectInfoLocal(id, raw.path, raw.name)
    return { id, name: raw.name, path: raw.path }
  }
  const res = await webRequest<{ ok: boolean; project: WebProject }>(
    "/api/v1/projects",
    { method: "POST", body: { name, parent: path } },
  )
  if (!res.ok || !res.project) {
    throw new Error("Create project failed: empty response from server")
  }
  const project = res.project
  const id = await ensureProjectIdLocal(project.path)
  await upsertProjectInfoLocal(id, project.path, project.name)
  return { id, name: project.name, path: project.path }
}

export async function openProject(path: string): Promise<WikiProject> {
  if (IS_TAURI) {
    const invoke = await tauri()
    const raw = await invoke<RawProject>("open_project", { path })
    const id = await ensureProjectIdLocal(raw.path)
    await upsertProjectInfoLocal(id, raw.path, raw.name)
    return { id, name: raw.name, path: raw.path }
  }
  const res = await webRequest<{ ok: boolean; project: WebProject }>(
    `/api/v1/projects/by-path/${encodeURIComponent(path)}`,
  )
  if (!res.ok || !res.project) {
    throw new Error("Open project failed: empty response from server")
  }
  const project = res.project
  const id = await ensureProjectIdLocal(project.path)
  await upsertProjectInfoLocal(id, project.path, project.name)
  return { id, name: project.name, path: project.path }
}

export async function openProjectFolder(path: string): Promise<void> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("open_project_folder", { path })
  }
}

export async function openPathInProject(
  projectPath: string,
  targetPath: string,
): Promise<void> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<void>("open_path_in_project", { projectPath, targetPath })
  }
}

export async function clipServerStatus(): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("clip_server_status")
  }
  return "stopped"
}

export async function apiServerStatus(): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("api_server_status")
  }
  const res = await webRequest<{ status: string }>(WEB_ENDPOINTS.health())
  return res.status
}

export async function apiServerReloadConfig(): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("api_server_reload_config")
  }
  return "ok"
}

export async function mcpServerEntryPath(): Promise<string> {
  if (IS_TAURI) {
    const invoke = await tauri()
    return invoke<string>("mcp_server_entry_path")
  }
  return ""
}

export { webFileNodesToLocal }

function assertAbsoluteFsPath(operation: string, path: string): void {
  if (!isAbsolutePath(path)) {
    throw new Error(`${operation} requires an absolute path: ${path}`)
  }
}
