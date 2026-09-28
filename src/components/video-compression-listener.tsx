import { useEffect } from 'react'
import { useRouter } from '@tanstack/react-router'
import { listen } from '@tauri-apps/api/event'
import { useSetRowCallback, useDelRowCallback } from '@/schema/tinybase-schema'

interface VideoCompressionProgress {
  id: number
  progress: number
  message: string
  stage?: string
  status?: string
  fps?: number | null
  speed?: number | null
  out_time?: number | null
  eta_seconds?: number | null
}

export function VideoCompressionListener() {
  const router = useRouter()

  const setCompression = useSetRowCallback(
    'video_compressions',
    (param: VideoCompressionProgress) => param.id.toString(),
    (param: VideoCompressionProgress) => ({
      progress: param.progress,
      message: param.message,
      stage: param.stage ?? '',
      status: param.status ?? 'running',
      eta_seconds: param.eta_seconds ?? 0,
      speed: param.speed ?? 0,
    }),
    []
  )

  const delCompression = useDelRowCallback(
    'video_compressions',
    (param: VideoCompressionProgress) => param.id.toString()
  )

  useEffect(() => {
    const unlistenProgress = listen<VideoCompressionProgress>(
      'video-compression-progress',
      (event) => {
        const payload = event.payload
        // Explicit status replaces the old `0 || 100` conflation.
        const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
        if (status === 'done' || status === 'failed' || status === 'cancelled') {
          delCompression({ ...payload, status })
        } else {
          setCompression(payload)
        }
      }
    )

    const unlistenUpdated = listen('videos-updated', () => {
      router.invalidate()
    })

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenUpdated.then((fn) => fn())
    }
  }, [router, setCompression, delCompression])

  return null
}
