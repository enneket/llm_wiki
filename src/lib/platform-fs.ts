/**
 * Project-aware file operations that route to the correct backend for the
 * current runtime:
 *
 * - **Tauri desktop** (`IS_TAURI === true`): the same `@tauri-apps/api/core`
 *   `invoke()` calls that have always powered `src/commands/fs.ts`. Paths
 *   are absolute filesystem paths because Tauri commands take them directly.
 *
 * - **Browser web** (`IS_TAURI === false`): the headless web server's
 *   `/api/v1/projects/{id}/...` endpoints, routed through `adapter.ts`.
 *   The current project (from `useWikiStore`) is needed to convert the
 *   caller's absolute path into the project-relative path the API expects,
 *   so this module is intentionally a thin wrapper around the store +
 *   adapter pair rather than a drop-in fs replacement.
 *
 * The split exists because `src/commands/fs.ts` already throws on
 * non-Tauri platforms — replacing every direct caller with adapter-aware
 * variants would explode the patch footprint. The wrapper below gives the
 * most heavily-used call sites (file tree, sources view, search view)
 * one import that works in both runtimes.
 */

import { useWikiStore } from "@/stores/wiki-store"
import type { FileNode } from "@/types/wiki"
import { IS_TAURI } from "@/lib/platform"
import {
  adapterListFiles,
  adapterReadFile,
  adapterWriteFile,
} from "@/lib/adapter"
import { webRequest, WEB_ENDPOINTS } from "@/lib/web-client"
import {
  readFile as tauriReadFile,
  writeFile as tauriWriteFile,
  listDirectory as tauriListDirectory,
} from "@/commands/fs"
import { normalizePath } from "@/lib/path-utils"

function getActiveProject(): { id: string; path: string } | null {
  const project = useWikiStore.getState().project
  if (!project) return null
  return { id: project.id, path: project.path }
}

function toRelPath(absPath: string, projectPath: string): string {
  const normalized = normalizePath(absPath)
  const base = normalizePath(projectPath)
  if (normalized === base) return ""
  if (normalized.startsWith(`${base}/`)) {
    return normalized.slice(base.length + 1)
  }
  // Defensive fallback: caller passed something outside the project.
  // Treat the trailing path components as the project-relative form so
  // we don't 404 spuriously when the project root has trailing slashes
  // or other normalization quirks.
  return normalized
}

function webFileNodesToFileNodes(
  nodes: Awaited<ReturnType<typeof adapterListFiles>>,
): FileNode[] {
  return nodes.map((node) => ({
    name: node.name,
    path: node.path,
    is_dir: node.isDir,
    children: node.children ? webFileNodesToFileNodes(node.children) : undefined,
  }))
}

export interface PlatformFsError extends Error {
  status?: number
  unsupported?: boolean
}

function platformError(
  message: string,
  opts: { status?: number; unsupported?: boolean } = {},
): PlatformFsError {
  const err = new Error(message) as PlatformFsError
  err.status = opts.status
  err.unsupported = opts.unsupported
  return err
}

export async function platformReadFile(absPath: string): Promise<string> {
  if (IS_TAURI) return tauriReadFile(absPath)
  const project = getActiveProject()
  if (!project) {
    throw platformError(
      "Cannot read file in browser mode without an open project",
    )
  }
  return adapterReadFile(project.id, toRelPath(absPath, project.path))
}

export async function platformWriteFile(
  absPath: string,
  contents: string,
): Promise<void> {
  if (IS_TAURI) return tauriWriteFile(absPath, contents)
  const project = getActiveProject()
  if (!project) {
    throw platformError(
      "Cannot write file in browser mode without an open project",
    )
  }
  return adapterWriteFile(project.id, toRelPath(absPath, project.path), contents)
}

export interface PlatformListOptions {
  includeHidden?: boolean
  maxDepth?: number
  root?: "wiki" | "sources" | "all"
}

export async function platformListDirectory(
  absPath: string,
  options: PlatformListOptions = {},
): Promise<FileNode[]> {
  if (IS_TAURI) {
    return tauriListDirectory(absPath, {
      includeHidden: options.includeHidden,
      maxDepth: options.maxDepth,
    })
  }
  const project = getActiveProject()
  if (!project) {
    throw platformError(
      "Cannot list directory in browser mode without an open project",
    )
  }
  const base = normalizePath(project.path)
  const normalized = normalizePath(absPath)
  const rel = normalized === base
    ? ""
    : normalized.startsWith(`${base}/`)
      ? normalized.slice(base.length + 1)
      : normalized
  let root: "wiki" | "sources" | "all" = options.root ?? "all"
  if (rel === "") {
    root = "all"
  } else if (rel === "wiki" || rel.startsWith("wiki/")) {
    if (root === "all") root = "wiki"
  } else if (rel === "raw/sources" || rel.startsWith("raw/sources/")) {
    if (root === "all") root = "sources"
  }
  const nodes = await adapterListFiles(project.id, {
    root,
    recursive: options.maxDepth === undefined ? true : options.maxDepth > 1,
  })
  return webFileNodesToFileNodes(nodes)
}

export async function platformDeleteFile(absPath: string): Promise<void> {
  if (IS_TAURI) {
    const { deleteFile } = await import("@/commands/fs")
    return deleteFile(absPath)
  }
  const project = getActiveProject()
  if (!project) {
    throw platformError(
      "Cannot delete file in browser mode without an open project",
    )
  }
  const rel = toRelPath(absPath, project.path)
  await webRequest(WEB_ENDPOINTS.fileContent(project.id), {
    method: "DELETE",
    body: { path: rel },
  })
}

export { platformError }