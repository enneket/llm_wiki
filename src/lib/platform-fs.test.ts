// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  fetch: vi.fn(),
  storeState: {
    project: { id: "p1", name: "Demo", path: "/data/projects/demo" } as
      | { id: string; name: string; path: string }
      | null,
  },
}))

vi.mock("@/lib/platform", () => ({
  IS_TAURI: false,
  IS_WEB: true,
  PLATFORM: "web",
  detectPlatform: () => "web",
  getWebBaseUrl: () => "http://localhost:19828",
  setWebBaseUrl: vi.fn(),
}))

vi.mock("@/stores/wiki-store", () => ({
  useWikiStore: {
    getState: () => mocks.storeState,
  },
}))

vi.mock("@/commands/fs", () => ({
  readFile: vi.fn(async () => {
    throw new Error("readFile should not run in web mode tests")
  }),
  writeFile: vi.fn(async () => {
    throw new Error("writeFile should not run in web mode tests")
  }),
  listDirectory: vi.fn(async () => {
    throw new Error("listDirectory should not run in web mode tests")
  }),
}))

import {
  platformListDirectory,
  platformReadFile,
  platformWriteFile,
} from "@/lib/platform-fs"

beforeEach(() => {
  mocks.fetch.mockReset()
  vi.stubGlobal("fetch", mocks.fetch)
  mocks.storeState.project = { id: "p1", name: "Demo", path: "/data/projects/demo" }
})

function makeFetchOk(json: unknown): Response {
  return new Response(JSON.stringify(json), { status: 200 })
}

describe("platform-fs (web mode)", () => {
  it("platformReadFile converts absolute paths to project-relative and hits the API", async () => {
    mocks.fetch.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, content: "# hello" }), { status: 200 }),
    )
    const content = await platformReadFile("/data/projects/demo/wiki/index.md")
    expect(content).toBe("# hello")
    const [url] = mocks.fetch.mock.calls[0]
    expect(url).toBe(
      "http://localhost:19828/api/v1/projects/p1/files/content?path=wiki%2Findex.md",
    )
  })

  it("platformWriteFile posts JSON contents for the relative path", async () => {
    mocks.fetch.mockResolvedValue(
      new Response(JSON.stringify({ ok: true }), { status: 200 }),
    )
    await platformWriteFile("/data/projects/demo/wiki/page.md", "hi")
    const [url, init] = mocks.fetch.mock.calls[0]
    expect(url).toBe("http://localhost:19828/api/v1/projects/p1/files/content")
    expect(init.method).toBe("POST")
    expect(JSON.parse(init.body)).toEqual({
      path: "wiki/page.md",
      contents: "hi",
    })
  })

  it("platformListDirectory forwards recursive listing for /wiki", async () => {
    mocks.fetch.mockResolvedValue(
      new Response(
        JSON.stringify({
          ok: true,
          projectId: "p1",
          root: "wiki",
          files: [
            { name: "index.md", path: "wiki/index.md", isDir: false, size: 1 },
          ],
        }),
        { status: 200 },
      ),
    )
    const nodes = await platformListDirectory("/data/projects/demo/wiki")
    expect(nodes[0].name).toBe("index.md")
    expect(nodes[0].is_dir).toBe(false)
    const [url] = mocks.fetch.mock.calls[0]
    expect(url).toContain("/api/v1/projects/p1/files")
    expect(url).toContain("root=wiki")
  })

  it("platformListDirectory maps raw/sources to root=sources", async () => {
    mocks.fetch.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: true, projectId: "p1", root: "sources", files: [] }),
        { status: 200 },
      ),
    )
    await platformListDirectory("/data/projects/demo/raw/sources", {
      includeHidden: true,
    })
    const [url] = mocks.fetch.mock.calls[0]
    expect(url).toContain("root=sources")
  })

  it("platformReadFile fails fast without an open project", async () => {
    mocks.storeState.project = null
    await expect(platformReadFile("/data/projects/demo/wiki/index.md")).rejects.toThrow(
      /open project/i,
    )
  })

  it("propagates API errors as thrown messages", async () => {
    mocks.fetch.mockResolvedValue(
      new Response(JSON.stringify({ ok: false, error: "Forbidden" }), {
        status: 403,
      }),
    )
    await expect(
      platformReadFile("/data/projects/demo/wiki/secret.md"),
    ).rejects.toThrow(/Forbidden/)
  })

  // Sanity: ensure the success-shape fetch helper is referenced so a
  // future refactor that drops its usage still gets caught here.
  it("makeFetchOk returns a JSON response", async () => {
    const res = makeFetchOk({ ok: true })
    expect(await res.json()).toEqual({ ok: true })
  })
})