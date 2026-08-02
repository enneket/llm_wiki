import { useWikiStore } from "@/stores/wiki-store"
import { enqueueIngest } from "./ingest-queue"
import { hasUsableLlm } from "@/lib/has-usable-llm"
import { refreshProjectFileTree } from "@/lib/project-file-tree-refresh"

const POLL_INTERVAL = 3000
const CLIP_BASE = "http://127.0.0.1:19827"
let intervalId: ReturnType<typeof setInterval> | null = null

export async function notifyClipServerOfProject(path: string): Promise<void> {
  try {
    await fetch(`${CLIP_BASE}/project`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    })
  } catch {
    // ignore
  }
}

export async function notifyClipServerOfProjects(
  projects: Array<{ name: string; path: string }>,
): Promise<void> {
  try {
    await fetch(`${CLIP_BASE}/projects`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ projects }),
    })
  } catch {
    // ignore
  }
}

export function startClipWatcher() {
  if (intervalId) return

  intervalId = setInterval(async () => {
    try {
      const res = await fetch(`${CLIP_BASE}/clips/pending`, { method: "GET" })
      const data = await res.json()

      if (!data.ok || !data.clips || data.clips.length === 0) return

      const store = useWikiStore.getState()
      const project = store.project

      for (const clip of data.clips) {
        const clipProjectPath: string = clip.projectPath
        const clipFilePath: string = clip.filePath

        if (project && clipProjectPath === project.path) {
          await refreshProjectFileTree(project.path, { projectId: project.id })

          if (hasUsableLlm(store.llmConfig)) {
            enqueueIngest(project.id, clipFilePath).catch((err) => {
              console.error("Failed to enqueue web clip:", err)
            })
          }
        }
      }
    } catch {
      // ignore
    }
  }, POLL_INTERVAL)
}

export function stopClipWatcher() {
  if (intervalId) {
    clearInterval(intervalId)
    intervalId = null
  }
}
