import { useEffect } from 'react'
import { useRouter } from '@tanstack/react-router'
import { listen } from '@tauri-apps/api/event'
import { useSetRowCallback, useDelRowCallback } from '@/schema/tinybase-schema'

interface CompressionProgress {
  id: number
  progress: number
  message: string
  stage?: string
  status?: string
}

export function CompressionListener() {
  const router = useRouter()

  const setCompression = useSetRowCallback(
    'compressions',
    (param: CompressionProgress) => param.id.toString(),
    (param: CompressionProgress) => ({ progress: param.progress, message: param.message }),
    []
  )

  const delCompression = useDelRowCallback('compressions', (param: CompressionProgress) =>
    param.id.toString()
  )

  useEffect(() => {
    const unlistenProgress = listen<CompressionProgress>('compression-progress', (event) => {
      const payload = event.payload
      const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
      if (status === 'done' || status === 'failed' || status === 'cancelled') {
        delCompression({ ...payload, status })
      } else {
        setCompression(payload)
      }
    })

    const unlistenUpdated = listen('images-updated', () => {
      router.invalidate()
    })

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenUpdated.then((fn) => fn())
    }
  }, [router, setCompression, delCompression])

  return null
}
