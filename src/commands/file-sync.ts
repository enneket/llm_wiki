import { IS_TAURI } from "@/lib/platform"
import { webRequest, WEB_ENDPOINTS, type WebTask } from "@/lib/web-client"

let tauriInvoke:
  | (<T>(cmd: string, args?: Record<string, unknown>) => Promise<T>)
  | null = null
let tauriInvokeLoaded = false

async function tauri(): Promise<
  <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
> {
  if (tauriInvokeLoaded) return tauriInvoke as never
  tauriInvokeLoaded = true
  if (!IS_TAURI) {
    tauriInvoke = null
    return tauriInvoke as never
  }
  const mod = await import("@tauri-apps/api/core")
  tauriInvoke = mod.invoke as never
  return tauriInvoke as never
}

export type FileChangeKind = "created" | "modified" | "deleted"
export type FileChangeStatus =
  | "pending"
  | "processing"
  | "done"
  | "failed"
  | "superseded"

export interface FileChangeTask {
  id: string
  projectId: string
  path: string
  kind: FileChangeKind
  status: FileChangeStatus
  hashBefore?: string | null
  hashAfter?: string | null
  size?: number | null
  mtimeMs?: number | null
  createdAt: number
  updatedAt: number
  retryCount: number
  error?: string | null
  needsRerun: boolean
}

export interface FileChangeQueue {
  version: number
  tasks: FileChangeTask[]
}

export interface FileChangeRescanResult {
  queue: FileChangeQueue
  changedTasks: FileChangeTask[]
}

export interface FileSyncPayload {
  projectId: string
  tasks: FileChangeTask[]
}

export interface NormalizedSourceWatchConfig {
  enabled: boolean
  autoIngest: boolean
  includeExtensions: string[]
  excludeExtensions: string[]
  excludeDirs: string[]
  excludeGlobs: string[]
  maxFileSizeMb: number
}

function normalizeSourceWatchConfig(
  config: unknown,
): NormalizedSourceWatchConfig {
  if (!config || typeof config !== "object") {
    return {
      enabled: true,
      autoIngest: true,
      includeExtensions: [],
      excludeExtensions: [],
      excludeDirs: [],
      excludeGlobs: [],
      maxFileSizeMb: 100,
    }
  }
  const c = config as Record<string, unknown>
  return {
    enabled: c.enabled !== false,
    autoIngest: c.autoIngest !== false,
    includeExtensions: Array.isArray(c.includeExtensions)
      ? (c.includeExtensions as string[])
      : [],
    excludeExtensions: Array.isArray(c.excludeExtensions)
      ? (c.excludeExtensions as string[])
      : [],
    excludeDirs: Array.isArray(c.excludeDirs) ? (c.excludeDirs as string[]) : [],
    excludeGlobs: Array.isArray(c.excludeGlobs) ? (c.excludeGlobs as string[]) : [],
    maxFileSizeMb:
      typeof c.maxFileSizeMb === "number" && c.maxFileSizeMb > 0
        ? (c.maxFileSizeMb as number)
        : 100,
  }
}

function webTaskToFileChange(task: WebTask): FileChangeTask {
  return {
    id: task.id,
    projectId: task.projectId,
    path: task.target,
    kind: "modified",
    status:
      task.status === "done"
        ? "done"
        : task.status === "failed"
          ? "failed"
          : task.status === "running"
            ? "processing"
            : "pending",
    createdAt: task.startedAt,
    updatedAt: task.finishedAt ?? task.startedAt,
    retryCount: 0,
    error: task.error ?? null,
    needsRerun: false,
  }
}

function emptyQueue(): FileChangeQueue {
  return { version: 1, tasks: [] }
}

export function startProjectFileWatcher(
  projectId: string,
  projectPath: string,
  sourceWatchConfig?: NormalizedSourceWatchConfig,
): Promise<FileChangeRescanResult> {
  if (IS_TAURI) {
    return tauri().then((invoke) =>
      invoke<FileChangeRescanResult>("start_project_file_watcher", {
        projectId,
        projectPath,
        sourceWatchConfig: normalizeSourceWatchConfig(sourceWatchConfig),
      }),
    )
  }
  return webRequest<{ ok: boolean; tasks?: WebTask[] }>(
    WEB_ENDPOINTS.rescan(projectId),
    {
      method: "POST",
      body: {
        triggerIngest: true,
        sourceWatchConfig: normalizeSourceWatchConfig(sourceWatchConfig),
      },
    },
  ).then((res) => ({
    queue: {
      version: 1,
      tasks: (res.tasks ?? []).map(webTaskToFileChange),
    },
    changedTasks: (res.tasks ?? []).map(webTaskToFileChange),
  }))
}

export function stopProjectFileWatcher(): Promise<void> {
  if (IS_TAURI) {
    return tauri().then((invoke) => invoke<void>("stop_project_file_watcher"))
  }
  return Promise.resolve()
}

export function rescanProjectFiles(
  projectId: string,
  projectPath: string,
  sourceWatchConfig?: NormalizedSourceWatchConfig,
): Promise<FileChangeRescanResult> {
  if (IS_TAURI) {
    return tauri().then((invoke) =>
      invoke<FileChangeRescanResult>("rescan_project_files", {
        projectId,
        projectPath,
        sourceWatchConfig: normalizeSourceWatchConfig(sourceWatchConfig),
      }),
    )
  }
  return webRequest<{ ok: boolean; tasks?: WebTask[] }>(
    WEB_ENDPOINTS.rescan(projectId),
    {
      method: "POST",
      body: {
        triggerIngest: true,
        sourceWatchConfig: normalizeSourceWatchConfig(sourceWatchConfig),
      },
    },
  ).then((res) => ({
    queue: {
      version: 1,
      tasks: (res.tasks ?? []).map(webTaskToFileChange),
    },
    changedTasks: (res.tasks ?? []).map(webTaskToFileChange),
  }))
}

export function getFileChangeQueue(projectPath: string): Promise<FileChangeQueue> {
  if (IS_TAURI) {
    return tauri().then((invoke) =>
      invoke<FileChangeQueue>("get_file_change_queue", { projectPath }),
    )
  }
  return webRequest<{ ok: boolean; tasks: WebTask[] }>(WEB_ENDPOINTS.tasks())
    .then((res) => ({
      version: 1,
      tasks: res.tasks.map(webTaskToFileChange),
    }))
    .catch(() => emptyQueue())
}

export function retryFileChangeTask(
  projectId: string,
  projectPath: string,
  taskId: string,
): Promise<FileChangeQueue> {
  if (IS_TAURI) {
    return tauri().then((invoke) =>
      invoke<FileChangeQueue>("retry_file_change_task", {
        projectId,
        projectPath,
        taskId,
      }),
    )
  }
  return webRequest<{ ok: boolean; cancelled: boolean }>(
    WEB_ENDPOINTS.taskCancel(taskId),
    { method: "POST" },
  )
    .then(() => emptyQueue())
    .catch(() => emptyQueue())
}

export function ignoreFileChangeTask(
  projectId: string,
  projectPath: string,
  taskId: string,
): Promise<FileChangeQueue> {
  if (IS_TAURI) {
    return tauri().then((invoke) =>
      invoke<FileChangeQueue>("ignore_file_change_task", {
        projectId,
        projectPath,
        taskId,
      }),
    )
  }
  return webRequest<{ ok: boolean; cancelled: boolean }>(
    WEB_ENDPOINTS.taskCancel(taskId),
    { method: "POST" },
  )
    .then(() => emptyQueue())
    .catch(() => emptyQueue())
}
