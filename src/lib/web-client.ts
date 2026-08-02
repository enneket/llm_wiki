/**
 * Browser-side client for the headless web server. Mirrors the shapes of
 * the Tauri `invoke` commands so the React layer does not need to know
 * which runtime it is talking to.
 *
 * The Tauri equivalents live in `src/commands/*` and `src/lib/llm-client.ts`.
 * Both paths converge on the same JSON contract documented in
 * `src-tauri/src/api_server.rs` and the new endpoints in
 * `src-tauri/src/web/backend.rs`.
 */

import { getWebBaseUrl, IS_TAURI } from "./platform"
import { API_SERVER_PORT, API_SERVER_BASE_URL } from "./api-server-constants"

export interface WebRequestOptions {
  method?: "GET" | "POST" | "PATCH" | "PUT" | "DELETE"
  body?: unknown
  formData?: FormData
  headers?: Record<string, string>
  token?: string
  signal?: AbortSignal
  /**
   * Raw binary response (ArrayBuffer). Used by file downloads.
   * The caller is responsible for turning the bytes back into a Blob.
   */
  rawResponse?: boolean
}

export interface WebErrorPayload {
  ok: false
  error: string
  status: number
}

export class WebApiError extends Error {
  readonly status: number
  readonly payload: unknown
  constructor(message: string, status: number, payload: unknown) {
    super(message)
    this.name = "WebApiError"
    this.status = status
    this.payload = payload
  }
}

export function getAuthToken(): string | null {
  if (typeof window === "undefined") return null
  try {
    const stored = window.localStorage.getItem("llm-wiki-api-token")
    return stored && stored.length > 0 ? stored : null
  } catch {
    return null
  }
}

export function setAuthToken(token: string | null): void {
  if (typeof window === "undefined") return
  try {
    if (token) {
      window.localStorage.setItem("llm-wiki-api-token", token)
    } else {
      window.localStorage.removeItem("llm-wiki-api-token")
    }
  } catch {
    /* ignore quota / privacy mode */
  }
}

function baseUrl(): string {
  if (IS_TAURI) return API_SERVER_BASE_URL
  return getWebBaseUrl()
}

function normalizePath(p: string): string {
  if (!p.startsWith("/")) return `/${p}`
  return p
}

function authHeaders(token: string | null): Record<string, string> {
  if (!token) return {}
  return { "X-LLM-Wiki-Token": token }
}

export async function webRequest<T>(path: string, options: WebRequestOptions = {}): Promise<T> {
  const url = `${baseUrl()}${normalizePath(path)}`
  const method = options.method ?? "GET"
  const headers: Record<string, string> = {
    Accept: "application/json",
    ...(options.headers ?? {}),
  }
  const token = options.token ?? getAuthToken()
  Object.assign(headers, authHeaders(token))

  let body: BodyInit | undefined
  if (options.formData) {
    body = options.formData
  } else if (options.body !== undefined) {
    body = JSON.stringify(options.body)
    if (!headers["Content-Type"]) headers["Content-Type"] = "application/json"
  }

  const res = await fetch(url, {
    method,
    headers,
    body,
    signal: options.signal,
  })

  if (options.rawResponse) {
    if (!res.ok) {
      const errPayload = (await safeReadJson(res)) as { error?: unknown } | null | undefined
      throw new WebApiError(
        typeof errPayload?.error === "string" ? errPayload.error : res.statusText,
        res.status,
        errPayload,
      )
    }
    return (await res.arrayBuffer()) as unknown as T
  }

  const text = await res.text()
  if (!res.ok) {
    let payload: unknown
    try {
      payload = text ? JSON.parse(text) : undefined
    } catch {
      payload = { ok: false, error: text || res.statusText }
    }
    const message =
      (payload && typeof payload === "object" && "error" in payload && typeof (payload as { error: unknown }).error === "string"
        ? (payload as { error: string }).error
        : res.statusText) || "Request failed"
    throw new WebApiError(message, res.status, payload)
  }
  if (!text) return undefined as T
  try {
    return JSON.parse(text) as T
  } catch (err) {
    throw new WebApiError(`Invalid JSON response: ${(err as Error).message}`, res.status, text)
  }
}

async function safeReadJson(res: Response): Promise<unknown> {
  try {
    const text = await res.text()
    if (!text) return undefined
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

export interface ServerHealth {
  ok: boolean
  status: string
  version?: string
  authRequired: boolean
  authConfigured: boolean
  tokenSource: string
  enabled: boolean
  mcpEnabled: boolean
  allowUnauthenticated: boolean
  allowLanAccess: boolean
  agent: { chat: boolean; streaming: boolean }
}

export interface ProjectListResponse {
  ok: boolean
  projects: WebProject[]
  currentProject: WebProject | null
}

export interface WebProject {
  id: string
  name: string
  path: string
  current: boolean
}

export interface WebFileNode {
  name: string
  path: string
  isDir: boolean
  size: number | null
  children: WebFileNode[] | null
}

export interface WebReviewItem {
  id: string
  type: string
  title: string
  description?: string
  resolved: boolean
  sourcePath?: string
  affectedPages?: string[]
  searchQueries?: string[]
  options?: Array<{ label: string; action: string }>
  createdAt?: number
  resolvedAction?: string
}

export interface WebSearchHit {
  path: string
  title: string
  snippet: string
  titleMatch: boolean
  score: number
  vectorScore?: number
  images: Array<{ url: string; alt: string }>
  content?: string
}

export interface WebSearchResponse {
  ok: boolean
  mode: string
  tokenHits: number
  vectorHits: number
  graphHits: number
  results: WebSearchHit[]
}

export interface WebGraphResponse {
  ok: boolean
  nodes: Array<{
    id: string
    label: string
    nodeType: string
    path: string
    linkCount: number
  }>
  edges: Array<{ source: string; target: string; weight: number }>
}

export interface WebTask {
  id: string
  projectId: string
  kind: "ingest" | "rescan" | "lint" | "sweep" | "enrich"
  status: "pending" | "running" | "done" | "failed" | "cancelled"
  target: string
  message?: string
  error?: string
  startedAt: number
  finishedAt?: number
  progress?: number
}

export interface WebTaskList {
  ok: boolean
  tasks: WebTask[]
}

export interface WebUploadResult {
  ok: boolean
  saved: Array<{ name: string; path: string; size: number }>
  skipped: Array<{ name: string; reason: string }>
  projectId: string
}

export const WEB_ENDPOINTS = {
  health: () => "/api/v1/health",
  projects: () => "/api/v1/projects",
  files: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/files`,
  fileContent: (projectId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/files/content`,
  reviews: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/reviews`,
  patchReview: (projectId: string, reviewId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/reviews/${encodeURIComponent(reviewId)}`,
  bulkResolveReviews: (projectId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/reviews/resolve`,
  search: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/search`,
  graph: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/graph`,
  rescan: (projectId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/sources/rescan`,
  chat: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/chat`,
  chatCancel: (projectId: string, sessionId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/chat/${encodeURIComponent(sessionId)}/cancel`,
  upload: (projectId: string) => `/api/v1/projects/${encodeURIComponent(projectId)}/uploads`,
  tasks: () => "/api/v1/tasks",
  task: (taskId: string) => `/api/v1/tasks/${encodeURIComponent(taskId)}`,
  taskCancel: (taskId: string) => `/api/v1/tasks/${encodeURIComponent(taskId)}/cancel`,
  events: () => "/api/v1/events",
  chatStream: (projectId: string) =>
    `/api/v1/projects/${encodeURIComponent(projectId)}/chat/stream`,
} as const

export const WEB_DEFAULTS = {
  port: API_SERVER_PORT,
} as const
