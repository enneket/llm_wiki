import { describe, expect, it, vi, beforeEach } from "vitest"

vi.mock("@/lib/platform", () => ({
  IS_TAURI: false,
  IS_WEB: true,
  PLATFORM: "web" as const,
  detectPlatform: () => "web" as const,
  getWebBaseUrl: () => "http://localhost:19828",
  setWebBaseUrl: vi.fn(),
}))

const fetchMock = vi.fn()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal("fetch", fetchMock)
})

import {
  adapterProjects,
  adapterListFiles,
  adapterSearch,
  adapterTasks,
  adapterUpload,
  adapterHealth,
  adapterChat,
  adapterCancelTask,
  adapterGraph,
  adapterRescan,
} from "@/lib/adapter"

describe("adapter (web mode)", () => {
  it("adapterHealth returns parsed JSON", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, status: "running", authRequired: true }), {
        status: 200,
      }),
    )
    const result = await adapterHealth()
    expect(result.ok).toBe(true)
    expect(result.status).toBe("running")
  })

  it("adapterProjects returns the projects array", async () => {
    fetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: true, projects: [{ id: "1", name: "Demo", path: "/tmp", current: true }] }),
        { status: 200 },
      ),
    )
    const result = await adapterProjects()
    expect(result).toHaveLength(1)
    expect(result[0].id).toBe("1")
  })

  it("adapterListFiles passes query parameters", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, files: [] }), { status: 200 }),
    )
    await adapterListFiles("abc", { root: "sources", maxFiles: 50 })
    const url = fetchMock.mock.calls[0][0] as string
    expect(url).toContain("/api/v1/projects/abc/files")
    expect(url).toContain("root=sources")
    expect(url).toContain("maxFiles=50")
  })

  it("adapterSearch posts the query body", async () => {
    fetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: true, mode: "hybrid", tokenHits: 0, vectorHits: 0, graphHits: 0, results: [] }),
        { status: 200 },
      ),
    )
    await adapterSearch("p1", "hello world", { topK: 5 })
    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(init.method).toBe("POST")
    expect(JSON.parse(init.body as string)).toEqual({ query: "hello world", topK: 5, includeContent: false })
  })

  it("adapterUpload submits multipart form-data", async () => {
    fetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: true, saved: [], skipped: [], projectId: "p1" }),
        { status: 200 },
      ),
    )
    const file = new File(["data"], "a.txt", { type: "text/plain" })
    await adapterUpload("p1", { files: [file], subdir: "papers" })
    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(init.method).toBe("POST")
    expect(init.body).toBeInstanceOf(FormData)
  })

  it("adapterTasks returns the task list", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, tasks: [{ id: "t1", status: "pending" }] }), { status: 200 }),
    )
    const result = await adapterTasks()
    expect(result[0].id).toBe("t1")
  })

  it("adapterCancelTask posts to the cancel endpoint", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, cancelled: true }), { status: 200 }),
    )
    const result = await adapterCancelTask("t1")
    expect(result.cancelled).toBe(true)
    expect(fetchMock.mock.calls[0][0]).toContain("/api/v1/tasks/t1/cancel")
  })

  it("adapterChat posts the chat body", async () => {
    fetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: true, sessionId: "s1", message: { role: "assistant", content: "hi" } }),
        { status: 200 },
      ),
    )
    const result = await adapterChat("p1", { message: "hello" })
    expect(result.sessionId).toBe("s1")
    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(JSON.parse(init.body as string).message).toBe("hello")
  })

  it("adapterGraph encodes query string", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, nodes: [], edges: [] }), { status: 200 }),
    )
    await adapterGraph("p1", "foo bar", { limit: 50 })
    const url = fetchMock.mock.calls[0][0] as string
    expect(url).toContain("q=foo+bar")
    expect(url).toContain("limit=50")
  })

  it("adapterRescan posts to the rescan endpoint", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, result: {} }), { status: 200 }),
    )
    const result = await adapterRescan("p1")
    expect(result.ok).toBe(true)
    expect(fetchMock.mock.calls[0][0]).toContain("/sources/rescan")
  })
})
