/**
 * Web-mode file picker that mirrors the shape of
 * `@tauri-apps/plugin-dialog`'s `open()` for multi-file selection. The
 * Tauri-flavoured call sites in `src/components/sources/*` continue to
 * use `open()` directly; this module provides the browser equivalent
 * for the web mode so we can keep the import flow consistent.
 */

export interface PickedFile {
  name: string
  size: number
  type: string
  file: File
}

export interface WebFilePickerOptions {
  multiple?: boolean
  accept?: string
  title?: string
}

const EXTENSION_TO_MIME: Record<string, string> = {
  md: "text/markdown",
  mdx: "text/mdx",
  txt: "text/plain",
  org: "text/org",
  rtf: "application/rtf",
  pdf: "application/pdf",
  html: "text/html",
  htm: "text/html",
  xml: "application/xml",
  json: "application/json",
  jsonl: "application/jsonl",
  csv: "text/csv",
  tsv: "text/tab-separated-values",
  yaml: "application/yaml",
  yml: "application/yaml",
  ndjson: "application/x-ndjson",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
  tiff: "image/tiff",
  avif: "image/avif",
  heic: "image/heic",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
  avi: "video/x-msvideo",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  ogg: "audio/ogg",
  flac: "audio/flac",
  m4a: "audio/mp4",
}

export function isWebFilePickerSupported(): boolean {
  return typeof window !== "undefined" && typeof window.document !== "undefined"
}

function extensionOf(name: string): string {
  const idx = name.lastIndexOf(".")
  if (idx < 0) return ""
  return name.slice(idx + 1).toLowerCase()
}

export function normalizeAccept(accept?: string): string | undefined {
  if (!accept) return undefined
  if (accept.includes(",")) return accept
  const parts = accept
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean)
  const seen = new Set<string>()
  const out: string[] = []
  for (const part of parts) {
    if (part.startsWith(".")) {
      const ext = part.slice(1).toLowerCase()
      if (!ext || seen.has(ext)) continue
      seen.add(ext)
      const mime = EXTENSION_TO_MIME[ext]
      if (mime) out.push(mime)
      out.push(part)
    } else {
      out.push(part)
    }
  }
  return out.join(",")
}

export function pickFilesBrowser(options: WebFilePickerOptions = {}): Promise<PickedFile[] | null> {
  return new Promise((resolve) => {
    if (!isWebFilePickerSupported()) {
      resolve(null)
      return
    }
    const input = document.createElement("input")
    input.type = "file"
    if (options.multiple) input.multiple = true
    const accept = normalizeAccept(options.accept)
    if (accept) input.accept = accept
    input.style.position = "fixed"
    input.style.left = "-9999px"
    input.style.top = "0"
    document.body.appendChild(input)
    let settled = false
    const cleanup = () => {
      if (input.parentNode) input.parentNode.removeChild(input)
    }
    const finish = (result: PickedFile[] | null) => {
      if (settled) return
      settled = true
      cleanup()
      resolve(result)
    }
    input.addEventListener("change", () => {
      const files = input.files
      if (!files || files.length === 0) {
        finish(null)
        return
      }
      const result: PickedFile[] = []
      for (let i = 0; i < files.length; i += 1) {
        const f = files.item(i)
        if (!f) continue
        result.push({
          name: f.name,
          size: f.size,
          type: f.type || EXTENSION_TO_MIME[extensionOf(f.name)] || "application/octet-stream",
          file: f,
        })
      }
      finish(result)
    })
    input.addEventListener("cancel", () => finish(null))
    window.addEventListener(
      "focus",
      () => {
        setTimeout(() => {
          if (!settled) finish(null)
        }, 200)
      },
      { once: true },
    )
    input.click()
  })
}

export function inferSubdirFromFiles(files: PickedFile[]): string {
  if (files.length === 0) return ""
  const first = files[0]
  const dot = first.name.lastIndexOf(".")
  return dot > 0 ? first.name.slice(0, dot) : first.name
}
