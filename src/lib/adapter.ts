/**
 * Unified command dispatcher: when running inside Tauri we call `invokeTauri`
 * directly; when running in a browser we route through the HTTP API of
 * the headless web server. The goal is that all the existing Tauri
 * command call sites in `src/commands/*` can use this dispatcher with no
 * per-call branching.
 *
 * The existing call sites import the helpers in `src/commands/*` directly.
 * This module is the bridge layer they should call into. To keep the
 * patch footprint minimal, the existing helper modules keep their public
 * Tauri-flavoured signatures; web-only paths construct their payloads
 * here.
 */

import {
  IS_TAURI,
} from "@/lib/platform"
import {
  webRequest,
  WEB_ENDPOINTS,
  type WebFileNode,
  type WebProject,
  type ServerHealth,
  type WebSearchResponse,
  type WebGraphResponse,
  type WebTask,
  type WebUploadResult,
} from "@/lib/web-client"

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

let tauriInvoke: TauriInvoke | null = null
let tauriInvokePromise: Promise<TauriInvoke | null> | null = null

async function invokeTauri<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error(`Tauri command is unavailable in web mode: ${command}`)
  if (!tauriInvokePromise) {
    tauriInvokePromise = import("@tauri-apps/api/core")
      .then((mod) => {
        tauriInvoke = mod.invoke as TauriInvoke
        return tauriInvoke
      })
      .catch((error) => {
        tauriInvokePromise = null
        throw error
      })
  }
  const invokeTauri = tauriInvoke ?? (await tauriInvokePromise)
  if (!invokeTauri) throw new Error(`Failed to load Tauri command bridge: ${command}`)
  return invokeTauri<T>(command, args)
}

export interface AdapterHealth extends ServerHealth {}

export async function adapterHealth(): Promise<AdapterHealth> {
  if (IS_TAURI) {
    const status = await invokeTauri<string>("api_server_status")
    return {
      ok: true,
      status,
      authRequired: true,
      authConfigured: true,
      tokenSource: "store",
      enabled: true,
      mcpEnabled: false,
      allowUnauthenticated: false,
      allowLanAccess: false,
      agent: { chat: true, streaming: false },
    }
  }
  return webRequest<ServerHealth>(WEB_ENDPOINTS.health())
}

export async function adapterProjects(): Promise<WebProject[]> {
  if (IS_TAURI) {
    return invokeTauri<WebProject[]>("api_projects")
  }
  const res = await webRequest<{ projects: WebProject[]; currentProject: WebProject | null }>(
    WEB_ENDPOINTS.projects(),
  )
  return res.projects
}

export async function adapterListFiles(
  projectId: string,
  options: { root?: "wiki" | "sources" | "all"; recursive?: boolean; maxFiles?: number } = {},
): Promise<WebFileNode[]> {
  if (IS_TAURI) {
    return invokeTauri<WebFileNode[]>("api_list_files", {
      projectId,
      root: options.root ?? "wiki",
      recursive: options.recursive ?? true,
      maxFiles: options.maxFiles ?? 2000,
    })
  }
  const params = new URLSearchParams({
    root: options.root ?? "wiki",
    recursive: String(options.recursive ?? true),
    maxFiles: String(options.maxFiles ?? 2000),
  })
  const res = await webRequest<{ files: WebFileNode[] }>(
    `${WEB_ENDPOINTS.files(projectId)}?${params.toString()}`,
  )
  return res.files
}

export async function adapterReadFile(
  projectId: string,
  relPath: string,
): Promise<string> {
  if (IS_TAURI) {
    return invokeTauri<string>("api_read_file", { projectId, path: relPath })
  }
  const params = new URLSearchParams({ path: relPath })
  const res = await webRequest<{ content: string }>(
    `${WEB_ENDPOINTS.fileContent(projectId)}?${params.toString()}`,
  )
  return res.content
}

export async function adapterWriteFile(
  projectId: string,
  relPath: string,
  contents: string,
): Promise<void> {
  if (IS_TAURI) {
    return invokeTauri<void>("api_write_file", { projectId, path: relPath, contents })
  }
  await webRequest(WEB_ENDPOINTS.fileContent(projectId), {
    method: "POST",
    body: { path: relPath, contents },
  })
}

export async function adapterSearch(
  projectId: string,
  query: string,
  options: { topK?: number; includeContent?: boolean } = {},
): Promise<WebSearchResponse> {
  if (IS_TAURI) {
    return invokeTauri<WebSearchResponse>("api_search", {
      projectId,
      query,
      topK: options.topK ?? 10,
      includeContent: options.includeContent ?? false,
    })
  }
  return webRequest<WebSearchResponse>(WEB_ENDPOINTS.search(projectId), {
    method: "POST",
    body: {
      query,
      topK: options.topK ?? 10,
      includeContent: options.includeContent ?? false,
    },
  })
}

export async function adapterGraph(
  projectId: string,
  query: string,
  options: { nodeType?: string; limit?: number } = {},
): Promise<WebGraphResponse> {
  if (IS_TAURI) {
    return invokeTauri<WebGraphResponse>("api_graph", {
      projectId,
      query,
      nodeType: options.nodeType ?? "",
      limit: options.limit ?? 200,
    })
  }
  const params = new URLSearchParams({ q: query, limit: String(options.limit ?? 200) })
  if (options.nodeType) params.set("nodeType", options.nodeType)
  return webRequest<WebGraphResponse>(
    `${WEB_ENDPOINTS.graph(projectId)}?${params.toString()}`,
  )
}

export async function adapterRescan(projectId: string): Promise<{ ok: boolean; result: unknown }> {
  if (IS_TAURI) {
    return invokeTauri<{ ok: boolean; result: unknown }>("api_rescan", { projectId })
  }
  return webRequest<{ ok: boolean; result: unknown }>(WEB_ENDPOINTS.rescan(projectId), {
    method: "POST",
  })
}

export interface UploadTarget {
  files: File[]
  subdir?: string
}

export async function adapterUpload(
  projectId: string,
  target: UploadTarget,
): Promise<WebUploadResult> {
  const form = new FormData()
  for (const file of target.files) {
    form.append("files", file, file.name)
  }
  if (target.subdir) form.append("subdir", target.subdir)
  if (IS_TAURI) {
    return invokeTauri<WebUploadResult>("api_upload", {
      projectId,
      files: target.files.map((file) => ({
        name: file.name,
        size: file.size,
        type: file.type,
      })),
      subdir: target.subdir ?? "",
    })
  }
  return webRequest<WebUploadResult>(WEB_ENDPOINTS.upload(projectId), {
    method: "POST",
    formData: form,
  })
}

export async function adapterTasks(): Promise<WebTask[]> {
  if (IS_TAURI) {
    return invokeTauri<WebTask[]>("api_tasks")
  }
  const res = await webRequest<{ tasks: WebTask[] }>(WEB_ENDPOINTS.tasks())
  return res.tasks
}

export async function adapterTask(taskId: string): Promise<WebTask> {
  if (IS_TAURI) {
    return invokeTauri<WebTask>("api_task", { taskId })
  }
  return webRequest<WebTask>(WEB_ENDPOINTS.task(taskId))
}

export async function adapterCancelTask(taskId: string): Promise<{ ok: boolean; cancelled: boolean }> {
  if (IS_TAURI) {
    return invokeTauri<{ ok: boolean; cancelled: boolean }>("api_task_cancel", { taskId })
  }
  return webRequest<{ ok: boolean; cancelled: boolean }>(WEB_ENDPOINTS.taskCancel(taskId), {
    method: "POST",
  })
}

export async function adapterChat(
  projectId: string,
  body: { message: string; sessionId?: string; persistSession?: boolean },
): Promise<{
  ok: boolean
  sessionId: string
  message: { role: "assistant"; content: string }
  references?: unknown[]
  events?: unknown[]
  toolEvents?: unknown[]
  usage?: unknown
}> {
  if (IS_TAURI) {
    return invokeTauri("api_chat", { projectId, ...body })
  }
  return webRequest(WEB_ENDPOINTS.chat(projectId), {
    method: "POST",
    body: {
      sessionId: body.sessionId ?? "",
      message: body.message,
      persistSession: body.persistSession ?? true,
    },
  })
}
