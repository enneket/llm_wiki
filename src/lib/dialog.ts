import { IS_TAURI } from "@/lib/platform"

export interface PickedFile {
  name: string
  size: number
  type: string
  file: File
}

export interface PickedDialogResult {
  files: PickedFile[]
  directory: { name: string; path: string } | null
}

export interface PickFilesOptions {
  multiple?: boolean
  accept?: string
  title?: string
}

export interface PickDirectoryOptions {
  title?: string
}

let tauriDialogModule:
  | typeof import("@tauri-apps/plugin-dialog")
  | null = null
let tauriDialogLoaded = false
let tauriDialogLoading: Promise<typeof import("@tauri-apps/plugin-dialog") | null> | null = null

async function loadTauriDialog(): Promise<
  typeof import("@tauri-apps/plugin-dialog") | null
> {
  if (!IS_TAURI) return null
  if (tauriDialogLoaded) return tauriDialogModule
  if (tauriDialogLoading) return tauriDialogLoading
  tauriDialogLoading = import("@tauri-apps/plugin-dialog")
    .then((mod) => {
      tauriDialogModule = mod
      tauriDialogLoaded = true
      return mod
    })
    .catch(() => null)
  return tauriDialogLoading
}

function normalizeAccept(accept?: string): string | undefined {
  if (!accept) return undefined
  return accept
}

function isWebFilePickerSupported(): boolean {
  return typeof window !== "undefined" && typeof window.document !== "undefined"
}

function extensionOf(name: string): string {
  const idx = name.lastIndexOf(".")
  if (idx < 0) return ""
  return name.slice(idx + 1).toLowerCase()
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

function buildAcceptList(accept: string): string {
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

function browserPickFiles(options: PickFilesOptions): Promise<PickedFile[] | null> {
  return new Promise((resolve) => {
    if (!isWebFilePickerSupported()) {
      resolve(null)
      return
    }
    const input = document.createElement("input")
    input.type = "file"
    if (options.multiple) input.multiple = true
    const accept = normalizeAccept(options.accept)
    if (accept) input.accept = buildAcceptList(accept)
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
    input.click()
  })
}

function browserPickDirectory(
  _options: PickDirectoryOptions,
): Promise<{ name: string; path: string } | null> {
  return new Promise((resolve) => {
    if (!isWebFilePickerSupported()) {
      resolve(null)
      return
    }
    const input = document.createElement("input")
    input.type = "file"
    ;(input as HTMLInputElement & { webkitdirectory?: boolean }).webkitdirectory = true
    input.style.position = "fixed"
    input.style.left = "-9999px"
    input.style.top = "0"
    document.body.appendChild(input)
    let settled = false
    const cleanup = () => {
      if (input.parentNode) input.parentNode.removeChild(input)
    }
    const finish = (result: { name: string; path: string } | null) => {
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
      const f = files[0]
      const relPath = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name
      const name = relPath.includes("/") ? relPath.split("/")[0] : relPath
      finish({
        name,
        path: `web://picked/${encodeURIComponent(name)}`,
      })
    })
    input.addEventListener("cancel", () => finish(null))
    input.click()
  })
}

export async function pickFiles(options: PickFilesOptions = {}): Promise<PickedFile[] | null> {
  if (IS_TAURI) {
    const mod = await loadTauriDialog()
    if (mod) {
      const accept = options.accept
      const filters = accept
        ? [
            {
              name: options.title ?? "Files",
              extensions: accept
                .split(",")
                .map((p) => p.trim())
                .filter((p) => p.startsWith(".") ? p.slice(1) : p)
                .flatMap((p) => p.split(";"))
                .map((p) => (p.startsWith(".") ? p.slice(1) : p))
                .filter(Boolean),
            },
          ]
        : undefined
      const selected = await mod.open({
        multiple: options.multiple ?? false,
        filters,
        title: options.title,
      })
      if (!selected) return null
      const arr = Array.isArray(selected) ? selected : [selected]
      return arr.map((path) => ({
        name: path.split(/[\\/]/).pop() ?? path,
        size: 0,
        type: "application/octet-stream",
        file: new File([new Uint8Array()], path.split(/[\\/]/).pop() ?? path, {
          type: "application/octet-stream",
        }),
      }))
    }
  }
  return browserPickFiles(options)
}

export async function pickDirectory(
  options: PickDirectoryOptions = {},
): Promise<string | null> {
  if (IS_TAURI) {
    const mod = await loadTauriDialog()
    if (mod) {
      const selected = await mod.open({
        directory: true,
        multiple: false,
        title: options.title,
      })
      if (!selected || Array.isArray(selected)) return null
      return selected
    }
  }
  const dir = await browserPickDirectory(options)
  return dir?.path ?? null
}

export async function pickMessage(
  message: string,
  options: { title?: string; kind?: "info" | "warning" | "error" } = {},
): Promise<void> {
  if (IS_TAURI) {
    const mod = await loadTauriDialog()
    if (mod) {
      try {
        await mod.message(message, {
          title: options.title,
          kind: options.kind,
        })
        return
      } catch {
        // fall through
      }
    }
  }
  if (typeof window !== "undefined") {
    window.alert(`${options.title ?? ""}\n\n${message}`)
  }
}
