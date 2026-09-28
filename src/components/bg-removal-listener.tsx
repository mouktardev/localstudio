import { useEffect } from 'react'
import { useRouter } from '@tanstack/react-router'
import { listen } from '@tauri-apps/api/event'
import { useSetRowCallback, useDelRowCallback } from '@/schema/tinybase-schema'

interface BgRemovalProgress {
  id: number
  progress: number
  message: string
  stage?: string
  status?: string
}

export function BgRemovalListener() {
  const router = useRouter()

  const setBgRemoval = useSetRowCallback(
    'bg_removals',
    (param: BgRemovalProgress) => param.id.toString(),
    (param: BgRemovalProgress) => ({ progress: param.progress, message: param.message }),
    []
  )

  const delBgRemoval = useDelRowCallback('bg_removals', (param: BgRemovalProgress) =>
    param.id.toString()
  )

  useEffect(() => {
    const unlistenProgress = listen<BgRemovalProgress>('bg-removal-progress', (event) => {
      const payload = event.payload
      const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
      if (status === 'done' || status === 'failed' || status === 'cancelled') {
        delBgRemoval({ ...payload, status })
      } else {
        setBgRemoval(payload)
      }
    })

    const unlistenUpdated = listen('images-updated', () => {
      router.invalidate()
    })

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenUpdated.then((fn) => fn())
    }
  }, [router, setBgRemoval, delBgRemoval])

  return null
}
