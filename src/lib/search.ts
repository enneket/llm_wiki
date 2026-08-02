import { invoke } from "@tauri-apps/api/core"
import { normalizePath } from "@/lib/path-utils"
import { useWikiStore } from "@/stores/wiki-store"
import { IS_TAURI } from "@/lib/platform"
import { webRequest, WEB_ENDPOINTS, type WebSearchResponse, type WebSearchHit } from "@/lib/web-client"

export interface ImageRef {
  url: string
  alt: string
}

export interface SearchResult {
  path: string
  title: string
  snippet: string
  titleMatch: boolean
  score: number
  vectorScore?: number
  images: ImageRef[]
}

interface BackendSearchResponse {
  // Reserved for result badges/debug UI. The backend already returns these
  // signals so API and WebView search share the same retrieval contract.
  mode: "keyword" | "vector" | "hybrid"
  results: SearchResult[]
  tokenHits: number
  vectorHits: number
  graphHits?: number
}

const STOP_WORDS = new Set([
  "的", "是", "了", "什么", "在", "有", "和", "与", "对", "从",
  "the", "is", "a", "an", "what", "how", "are", "was", "were",
  "do", "does", "did", "be", "been", "being", "have", "has", "had",
  "it", "its", "in", "on", "at", "to", "for", "of", "with", "by",
  "this", "that", "these", "those",
])

export function tokenizeQuery(query: string): string[] {
  const rawTokens = query
    .toLowerCase()
    .split(/[\s,，。！？、；：""''（）()\-_/\\·~～…]+/)
    .filter((t) => t.length > 1)
    .filter((t) => !STOP_WORDS.has(t))

  const tokens: string[] = []
  for (const token of rawTokens) {
    const hasCJK = /[\u4e00-\u9fff\u3400-\u4dbf]/.test(token)
    if (hasCJK && token.length > 2) {
      const chars = [...token]
      for (let i = 0; i < chars.length - 1; i++) tokens.push(chars[i] + chars[i + 1])
      for (const ch of chars) {
        if (!STOP_WORDS.has(ch)) tokens.push(ch)
      }
      tokens.push(token)
    } else {
      tokens.push(token)
    }
  }
  return [...new Set(tokens)]
}

export async function searchWiki(
  projectPath: string,
  query: string,
): Promise<SearchResult[]> {
  if (!query.trim()) return []
  const pp = normalizePath(projectPath)
  const project = useWikiStore.getState().project
  if (!IS_TAURI && project) {
    const response = await webRequest<WebSearchResponse>(WEB_ENDPOINTS.search(project.id), {
      method: "POST",
      body: { query, topK: 20, includeContent: false },
    })
    return response.results.map((hit: WebSearchHit) => adaptWebHit(hit, pp))
  }
  const embCfg = useWikiStore.getState().embeddingConfig
  const response = await invoke<BackendSearchResponse>("search_project", {
    projectPath: pp,
    query,
    topK: 20,
    includeContent: false,
    queryEmbedding: null,
    embeddingConfig: embCfg,
  })
  return response.results.map((result) => ({
    ...result,
    path: `${pp}/${normalizePath(result.path).replace(/^\/+/, "")}`,
  }))
}

function adaptWebHit(hit: WebSearchHit, projectPath: string): SearchResult {
  const rel = normalizePath(hit.path).replace(/^\/+/, "")
  return {
    path: `${projectPath}/${rel}`,
    title: hit.title,
    snippet: hit.snippet,
    titleMatch: hit.titleMatch,
    score: hit.score,
    vectorScore: hit.vectorScore,
    images: hit.images.map((image) => ({ url: image.url, alt: image.alt })),
  }
}
