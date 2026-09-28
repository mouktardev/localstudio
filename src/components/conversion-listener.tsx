import { useEffect } from 'react'
import { useRouter } from '@tanstack/react-router'
import { listen } from '@tauri-apps/api/event'
import { useSetRowCallback, useDelRowCallback } from '@/schema/tinybase-schema'

interface ImageConversionProgress {
  id: number
  progress: number
  message: string
  stage?: string
  status?: string
}

interface VideoConversionProgress extends ImageConversionProgress {
  stage?: string
  status?: string
  speed?: number | null
  eta_seconds?: number | null
}

export function ConversionListener() {
  const router = useRouter()

  const setImageConversion = useSetRowCallback(
    'image_conversions',
    (param: ImageConversionProgress) => param.id.toString(),
    (param: ImageConversionProgress) => ({ progress: param.progress, message: param.message }),
    []
  )

  const delImageConversion = useDelRowCallback(
    'image_conversions',
    (param: ImageConversionProgress) => param.id.toString()
  )

  const setVideoConversion = useSetRowCallback(
    'video_conversions',
    (param: VideoConversionProgress) => param.id.toString(),
    (param: VideoConversionProgress) => ({
      progress: param.progress,
      message: param.message,
      stage: param.stage ?? '',
      status: param.status ?? 'running',
      eta_seconds: param.eta_seconds ?? 0,
      speed: param.speed ?? 0,
    }),
    []
  )

  const delVideoConversion = useDelRowCallback(
    'video_conversions',
    (param: VideoConversionProgress) => param.id.toString()
  )

  useEffect(() => {
    const unlistenImageProgress = listen<ImageConversionProgress>(
      'image-conversion-progress',
      (event) => {
        const payload = event.payload
        const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
        if (status === 'done' || status === 'failed' || status === 'cancelled') {
          delImageConversion({ ...payload, status })
        } else {
          setImageConversion(payload)
        }
      }
    )

    const unlistenVideoProgress = listen<VideoConversionProgress>(
      'video-conversion-progress',
      (event) => {
        const payload = event.payload
        const status = payload.status ?? (payload.progress >= 100 ? 'done' : 'running')
        if (status === 'done' || status === 'failed' || status === 'cancelled') {
          delVideoConversion({ ...payload, status })
        } else {
          setVideoConversion(payload)
        }
      }
    )

    const unlistenImagesUpdated = listen('images-updated', () => {
      router.invalidate()
    })

    const unlistenVideosUpdated = listen('videos-updated', () => {
      router.invalidate()
    })

    return () => {
      unlistenImageProgress.then((fn) => fn())
      unlistenVideoProgress.then((fn) => fn())
      unlistenImagesUpdated.then((fn) => fn())
      unlistenVideosUpdated.then((fn) => fn())
    }
  }, [router, setImageConversion, delImageConversion, setVideoConversion, delVideoConversion])

  return null
}
