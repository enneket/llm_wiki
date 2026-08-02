// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import {
  webRequest,
  setAuthToken,
  WebApiError,
  getAuthToken,
} from "@/lib/web-client"
import { setWebBaseUrl } from "@/lib/platform"

const fetchMock = vi.fn()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal("fetch", fetchMock)
  setWebBaseUrl("http://localhost:19828")
  window.localStorage.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("webRequest", () => {
  it("uses JSON content-type for plain bodies and merges headers", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, value: 1 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    )

    const result = await webRequest<{ ok: true; value: number }>("/api/v1/test", {
      method: "POST",
      body: { foo: "bar" },
      headers: { "X-Custom": "yes" },
    })

    expect(result).toEqual({ ok: true, value: 1 })
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe("http://localhost:19828/api/v1/test")
    const headers = (init as RequestInit).headers as Record<string, string>
    expect(headers["X-Custom"]).toBe("yes")
    expect(headers["Content-Type"]).toBe("application/json")
    expect(headers.Accept).toBe("application/json")
    expect((init as RequestInit).body).toBe(JSON.stringify({ foo: "bar" }))
  })

  it("sends the bearer token from localStorage by default", async () => {
    setAuthToken("test-token-abc")
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true }), { status: 200 }),
    )
    await webRequest("/api/v1/health")
    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>
    expect(headers["X-LLM-Wiki-Token"]).toBe("test-token-abc")
  })

  it("throws WebApiError on non-2xx with structured payload", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: false, error: "nope" }), { status: 401 }),
    )
    await expect(webRequest("/api/v1/projects")).rejects.toBeInstanceOf(WebApiError)
  })

  it("supports multipart form uploads without forcing a Content-Type", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ ok: true, saved: [] }), { status: 200 }),
    )
    const form = new FormData()
    form.append("file", new Blob(["hello"], { type: "text/plain" }), "hello.txt")
    await webRequest("/api/v1/projects/p/uploads", {
      method: "POST",
      formData: form,
    })
    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(init.body).toBe(form)
    const headers = init.headers as Record<string, string>
    expect(headers["Content-Type"]).toBeUndefined()
  })

  it("exposes getAuthToken round-trip", () => {
    setAuthToken("abc")
    expect(getAuthToken()).toBe("abc")
    setAuthToken(null)
    expect(getAuthToken()).toBeNull()
  })
})
