import { describe, expect, it } from "vitest"
import { detectPlatform, IS_WEB, PLATFORM, getWebBaseUrl } from "@/lib/platform"

describe("platform detection", () => {
  it("returns 'web' in jsdom", () => {
    expect(detectPlatform()).toBe("web")
    expect(PLATFORM).toBe("web")
    expect(IS_WEB).toBe(true)
  })

  it("falls back to window.location.origin for the web base url", () => {
    const base = getWebBaseUrl()
    expect(base).toMatch(/^https?:\/\//)
  })
})
