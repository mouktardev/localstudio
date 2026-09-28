import { useEffect } from 'react'
import { useRouter } from '@tanstack/react-router'
import { listen } from '@tauri-apps/api/event'
import { useSetRowCallback, useDelRowCallback } from '@/schema/tinybase-schema'

interface UpscaleProgress {
  id: number
  progress: number
  message: string
  stage?: string
  status?: string
}

export function UpscalingListener() {
  const router = useRouter()

  const setUpscaling = useSetRowCallback(
    'upscalings',
    (param: UpscaleProgress) => param.id.toString(),
    (param: UpscaleProgress) => ({ progress: param.progress, message: param.message }),
    []
  )

  const delUpscaling = useDelRowCallback('upscalings', (param: UpscaleProgress) =>
    param.id.toString()
  )

  useEffect(() => {
    const unlistenProgress = listen<UpscaleProgress>('upscale-progress', (event) => {
      const payload = event.payload
      const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
      if (status === 'done' || status === 'failed' || status === 'cancelled') {
        delUpscaling({ ...payload, status })
      } else {
        setUpscaling(payload)
      }
    })

    const unlistenUpdated = listen('images-updated', () => {
      router.invalidate()
    })

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenUpdated.then((fn) => fn())
    }
  }, [router, setUpscaling, delUpscaling])

  return null
}
