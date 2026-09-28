import { createFileRoute } from '@tanstack/react-router'
import { useState, useEffect } from 'react'
import { useValue, useCell, useSetValueCallback, useSetRowCallback } from '@/schema/tinybase-schema'
import {
  initDatabase,
  dbExists,
  getSetting,
  setSetting,
  revealInExplorer,
  selectFolder,
  getDbPath,
  getUpscaleSettings,
  setUpscaleSettings as saveUpscaleSettings,
  getModelStatus,
  downloadModel,
  getBgRemovalModelStatus,
  downloadBgRemovalModel,
  checkFfmpegStatus,
  downloadFfmpeg,
  getVideoJobLimit,
  setVideoJobLimit,
  getImageJobLimit,
  setImageJobLimit,
  getEncoderCapabilities,
  openExternalUrl,
} from '@/lib/tauri'
import { error as logError } from '@/lib/logger'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  FolderOpenIcon,
  FolderIcon,
  RefreshCw,
  EyeIcon,
  Sun,
  Moon,
  Monitor,
  Download,
  Scissors,
  Maximize2,
  Video,
} from 'lucide-react'
import { addNotification } from '@/lib/notifications'
import { useTheme } from '@/components/theme-provider'
import { formatBytes } from '@/lib/utils'

const MODEL_SIZES: Record<string, number> = {
  'realesrgan-x2': 54 * 1024 * 1024,
  'realesrgan-x4': 54 * 1024 * 1024,
}

const BG_REMOVAL_MODEL_SIZE = 176 * 1024 * 1024

export const Route = createFileRoute('/_app/settings')({
  loader: async () => {
    const settings = await getUpscaleSettings()
    const [
      dbInitialized,
      outputPath,
      dbPathResult,
      upscaleSettings,
      modelStatusX2,
      modelStatusX4,
      bgRemovalStatus,
      ffmpegStatus,
      updateChecksEnabled,
    ] = await Promise.all([
      dbExists(),
      getSetting('output'),
      getDbPath(),
      Promise.resolve(settings),
      getModelStatus('realesrgan-x2'),
      getModelStatus('realesrgan-x4'),
      getBgRemovalModelStatus(),
      checkFfmpegStatus(),
      getSetting('update_checks_enabled'),
    ])

    return {
      dbInitialized,
      outputPath,
      dbPath: dbPathResult,
      upscaleSettings,
      modelStatusX2,
      modelStatusX4,
      bgRemovalStatus,
      ffmpegStatus,
      updateChecksEnabled: updateChecksEnabled !== 'false',
    }
  },
  gcTime: 0,
  staleTime: 0,
  component: SettingsPage,
})

function SettingsPage() {
  const loaderData = Route.useLoaderData()
  const { theme, setTheme } = useTheme()

  const [isInitialized, setIsInitialized] = useState(loaderData.dbInitialized)
  const [outputPath, setOutputPath] = useState(loaderData.outputPath || '')
  const [dbPath, setDbPath] = useState(loaderData.dbPath || '')
  const [isInitializing, setIsInitializing] = useState(false)
  const [isChangingFolder, setIsChangingFolder] = useState(false)
  const [updateChecksEnabled, setUpdateChecksEnabled] = useState(loaderData.updateChecksEnabled)

  const [upscaleSettings, setUpscaleSettings] = useState(loaderData.upscaleSettings)
  const [maxVideoJobs, setMaxVideoJobs] = useState(2)
  const [maxImageJobs, setMaxImageJobs] = useState(2)
  const [hardwareAccel, setHardwareAccel] = useState('auto')
  const [hardwareOptions, setHardwareOptions] = useState<string[]>([])

  const isDownloadingUpscale = useValue('isDownloadingUpscale')
  const upscaleDownloadProgress = useValue('upscaleDownloadProgress')
  const startUpscaleDownload = useSetValueCallback('isDownloadingUpscale', () => true)
  const finishUpscaleDownload = useSetValueCallback('isDownloadingUpscale', () => false)

  const isDownloadingBgRemoval = useValue('isDownloadingBgRemoval')
  const bgRemovalDownloadProgress = useValue('bgRemovalDownloadProgress')
  const startBgRemovalDownload = useSetValueCallback('isDownloadingBgRemoval', () => true)
  const finishBgRemovalDownload = useSetValueCallback('isDownloadingBgRemoval', () => false)

  const currentModel = upscaleSettings?.model || 'realesrgan-x4'
  const upscaleDownloaded = useCell('model_downloads', currentModel, 'downloaded')
  const bgRemovalDownloaded = useCell('model_downloads', 'bria-rmbg-1.4', 'downloaded')

  const isDownloadingFfmpeg = useValue('isDownloadingFfmpeg')
  const ffmpegAvailable = useValue('ffmpegAvailable')
  const ffmpegDownloadProgress = useValue('ffmpegDownloadProgress')
  const startFfmpegDownload = useSetValueCallback('isDownloadingFfmpeg', () => true)
  const finishFfmpegDownload = useSetValueCallback('isDownloadingFfmpeg', () => false)

  const initModel = useSetRowCallback(
    'model_downloads',
    (info: { name: string; downloaded: boolean }) => info.name,
    (info: { name: string; downloaded: boolean }) => ({ downloaded: info.downloaded }),
    []
  )

  const markDownloaded = useSetRowCallback(
    'model_downloads',
    (name: string) => name,
    () => ({ downloaded: true }),
    []
  )

  const setInitFfmpeg = useSetValueCallback(
    'ffmpegAvailable',
    () => loaderData.ffmpegStatus?.available ?? false,
    [loaderData]
  )

  useEffect(() => {
    initModel({ name: 'realesrgan-x2', downloaded: loaderData.modelStatusX2?.downloaded ?? false })
    initModel({ name: 'realesrgan-x4', downloaded: loaderData.modelStatusX4?.downloaded ?? false })
    initModel({
      name: 'bria-rmbg-1.4',
      downloaded: loaderData.bgRemovalStatus?.downloaded ?? false,
    })
    setInitFfmpeg()
  }, [loaderData, initModel, setInitFfmpeg])

  useEffect(() => {
    getVideoJobLimit()
      .then(setMaxVideoJobs)
      .catch((err) => logError(`Failed to load video concurrency limit: ${err}`))
    getImageJobLimit()
      .then(setMaxImageJobs)
      .catch((err) => logError(`Failed to load image concurrency limit: ${err}`))
    getEncoderCapabilities()
      .then((caps) => setHardwareOptions(caps.hardware))
      .catch((err) => logError(`Failed to detect hardware encoders: ${err}`))
    getSetting('hardware_acceleration')
      .then((value) => {
        if (value) setHardwareAccel(value)
      })
      .catch((err) => logError(`Failed to load hardware acceleration setting: ${err}`))
  }, [])

  async function handleInitDatabase() {
    setIsInitializing(true)
    try {
      await initDatabase()
      setIsInitialized(true)
      const exists = await dbExists()
      if (exists) {
        const path = await getSetting('output')
        if (path) setOutputPath(path)
        const db = await getDbPath()
        if (db) setDbPath(db)
      }
    } catch (err) {
      logError(`Failed to initialize database: ${err}`)
    } finally {
      setIsInitializing(false)
    }
  }

  async function handleRevealFolder() {
    if (!outputPath) return
    try {
      await revealInExplorer(outputPath)
    } catch (err) {
      logError(`Failed to reveal folder: ${err}`)
    }
  }

  async function handleRevealDbFolder() {
    if (!dbPath) return
    try {
      await revealInExplorer(dbPath)
    } catch (err) {
      logError(`Failed to reveal database folder: ${err}`)
    }
  }

  async function handleChangeFolder() {
    setIsChangingFolder(true)
    try {
      const selected = await selectFolder()
      if (selected) {
        await setSetting('output', selected)
        setOutputPath(selected)
      }
    } catch (err) {
      logError(`Failed to change folder: ${err}`)
    } finally {
      setIsChangingFolder(false)
    }
  }

  return (
    <section className="customScrollStyle relative h-full max-h-[calc(100vh-67px)]">
      <div className="mx-auto max-w-2xl px-3 py-8">
        <h1 className="mb-6 text-3xl font-bold">Settings</h1>

        <Card className="mb-6 p-4">
          <div className="flex items-center justify-between gap-4">
            <CardTitle className="mb-0">Support Development</CardTitle>
            <Button
              variant="outline"
              className="shrink-0"
              onClick={async () => {
                try {
                  await openExternalUrl('https://mouktar.com/sponsor')
                } catch (e) {
                  logError(`Failed to open sponsor: ${e}`)
                }
              }}
            >
              Support
            </Button>
          </div>
        </Card>

        {/* Database Status */}
        <Card>
          <CardHeader>
            <CardTitle>Database</CardTitle>
            <CardDescription>
              Your local SQLite database. Use the DB button in the footer to sync it or the database
              viewer to browse it.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {!isInitialized && (
              <Button onClick={handleInitDatabase} disabled={isInitializing}>
                {isInitializing ? (
                  <>
                    <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
                    Initializing...
                  </>
                ) : (
                  'Initialize Database'
                )}
              </Button>
            )}
            {isInitialized && (
              <div className="space-y-4">
                <p className="text-muted-foreground text-sm">Database is ready to use.</p>
                <div className="flex gap-2">
                  <Button variant="outline" size="sm" onClick={handleRevealDbFolder}>
                    <EyeIcon className="mr-2 h-4 w-4" />
                    Reveal Database Folder
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>

        <Separator className="my-6" />

        {/* Output Folder */}
        <Card>
          <CardHeader>
            <CardTitle>Output Folder</CardTitle>
            <CardDescription>
              Configure where processed images and videos will be saved
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center gap-2">
              <FolderIcon className="text-muted-foreground h-5 w-5" />
              <code className="bg-muted relative flex-1 overflow-x-auto rounded px-[0.3rem] py-[0.2rem] font-mono text-sm">
                {outputPath || 'No folder selected'}
              </code>
            </div>

            <div className="flex gap-2">
              <Button variant="outline" onClick={handleRevealFolder} disabled={!outputPath}>
                <EyeIcon className="mr-2 h-4 w-4" />
                Reveal in Explorer
              </Button>
              <Button
                variant="outline"
                onClick={handleChangeFolder}
                disabled={isChangingFolder || !isInitialized}
              >
                <FolderOpenIcon className="mr-2 h-4 w-4" />
                {isChangingFolder ? 'Changing...' : 'Change Folder'}
              </Button>
            </div>
          </CardContent>
        </Card>

        <Separator className="my-6" />

        {/* Theme */}
        <Card className="my-4">
          <CardHeader>
            <CardTitle>Appearance</CardTitle>
            <CardDescription>Choose how the application looks</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex gap-2">
              <Button
                variant={theme === 'light' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setTheme('light')}
              >
                <Sun className="mr-2 h-4 w-4" />
                Light
              </Button>
              <Button
                variant={theme === 'dark' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setTheme('dark')}
              >
                <Moon className="mr-2 h-4 w-4" />
                Dark
              </Button>
              <Button
                variant={theme === 'system' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setTheme('system')}
              >
                <Monitor className="mr-2 h-4 w-4" />
                System
              </Button>
            </div>
          </CardContent>
        </Card>

        <Separator className="my-6" />

        {/* AI Image Processing */}
        <Card className="my-4">
          <CardHeader>
            <CardTitle>AI Image Processing</CardTitle>
            <CardDescription>Configure AI-powered image enhancement models</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            {/* Upscale Section */}
            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Maximize2 className="text-muted-foreground h-5 w-5" />
                <h3 className="font-semibold">Upscale</h3>
                <span className="text-muted-foreground text-xs">
                  Increase image resolution using AI super-resolution
                </span>
              </div>

              <div className="space-y-3">
                <div className="space-y-2">
                  <Label className="text-sm">Model</Label>
                  <Select
                    value={upscaleSettings?.model || 'realesrgan-x4'}
                    onValueChange={async (model) => {
                      try {
                        await saveUpscaleSettings(model)
                        setUpscaleSettings({ ...upscaleSettings!, model })
                      } catch (err) {
                        logError(`Failed to save upscale model: ${err}`)
                      }
                    }}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="realesrgan-x2">
                        <span>Swin2SR Classical x2</span>
                        <span className="text-muted-foreground ml-2 text-xs">(~54 MB)</span>
                      </SelectItem>
                      <SelectItem value="realesrgan-x4">
                        <span>Swin2SR Real-world x4</span>
                        <span className="text-muted-foreground ml-2 text-xs">(~54 MB)</span>
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-2">
                  <Label className="text-sm">Model Status</Label>
                  <div className="flex items-center gap-2">
                    <Badge variant={upscaleDownloaded ? 'default' : 'secondary'}>
                      {upscaleDownloaded ? 'Downloaded' : 'Not Downloaded'}
                    </Badge>
                    <span className="text-muted-foreground text-xs">
                      {formatBytes(MODEL_SIZES[upscaleSettings?.model || 'realesrgan-x4'] || 0)}
                    </span>
                    {isDownloadingUpscale && (
                      <div className="flex items-center gap-2">
                        <Progress value={upscaleDownloadProgress} className="w-24" />
                        <span className="text-xs">{upscaleDownloadProgress}%</span>
                      </div>
                    )}
                    {upscaleDownloaded && !isDownloadingUpscale && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          try {
                            const model = upscaleSettings?.model || 'realesrgan-x4'
                            const status = await getModelStatus(model)
                            if (status?.path) {
                              revealInExplorer(status.path)
                            }
                          } catch (err) {
                            logError(`Failed to reveal model: ${err}`)
                          }
                        }}
                      >
                        <EyeIcon className="mr-2 h-4 w-4" />
                        Reveal
                      </Button>
                    )}
                    {!upscaleDownloaded && !isDownloadingUpscale && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          startUpscaleDownload()
                          try {
                            const model = upscaleSettings?.model || 'realesrgan-x4'
                            await downloadModel(model)
                            markDownloaded(model)
                            await addNotification({
                              message: `${model} downloaded successfully`,
                              status: 'success',
                            })
                          } catch (err) {
                            logError(`Failed to download model: ${err}`)
                          } finally {
                            finishUpscaleDownload()
                          }
                        }}
                      >
                        <Download className="mr-2 h-4 w-4" />
                        Download
                      </Button>
                    )}
                  </div>
                </div>
              </div>
            </div>

            <Separator />

            {/* Background Removal Section */}
            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Scissors className="text-muted-foreground h-5 w-5" />
                <h3 className="font-semibold">Background Removal</h3>
                <span className="text-muted-foreground text-xs">
                  AI-powered background removal (processed on CPU)
                </span>
              </div>

              <div className="space-y-3">
                <div className="space-y-2">
                  <Label className="text-sm">Model</Label>
                  <Select value="bria-rmbg-1.4">
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="bria-rmbg-1.4">
                        <span>BRIA RMBG-1.4</span>
                        <span className="text-muted-foreground ml-2 text-xs">(~176 MB)</span>
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label className="text-sm">Model Status</Label>
                  <div className="flex items-center gap-2">
                    <Badge variant={bgRemovalDownloaded ? 'default' : 'secondary'}>
                      {bgRemovalDownloaded ? 'Downloaded' : 'Not Downloaded'}
                    </Badge>
                    <span className="text-muted-foreground text-xs">
                      {formatBytes(BG_REMOVAL_MODEL_SIZE)}
                    </span>
                    {isDownloadingBgRemoval && (
                      <div className="flex items-center gap-2">
                        <Progress value={bgRemovalDownloadProgress} className="w-24" />
                        <span className="text-xs">{bgRemovalDownloadProgress}%</span>
                      </div>
                    )}
                    {bgRemovalDownloaded && !isDownloadingBgRemoval && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          try {
                            const status = await getBgRemovalModelStatus()
                            if (status?.path) {
                              revealInExplorer(status.path)
                            }
                          } catch (err) {
                            logError(`Failed to reveal model: ${err}`)
                          }
                        }}
                      >
                        <EyeIcon className="mr-2 h-4 w-4" />
                        Reveal
                      </Button>
                    )}
                    {!bgRemovalDownloaded && !isDownloadingBgRemoval && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          startBgRemovalDownload()
                          try {
                            await downloadBgRemovalModel()
                            markDownloaded('bria-rmbg-1.4')
                            await addNotification({
                              message: 'bria-rmbg-1.4 downloaded successfully',
                              status: 'success',
                            })
                          } catch (err) {
                            logError(`Failed to download model: ${err}`)
                          } finally {
                            finishBgRemovalDownload()
                          }
                        }}
                      >
                        <Download className="mr-2 h-4 w-4" />
                        Download
                      </Button>
                    )}
                  </div>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>

        <Separator className="my-6" />

        {/* Video Processing Section */}
        <Card className="my-4">
          <CardHeader>
            <CardTitle>Video Processing</CardTitle>
            <CardDescription>Configure video processing tools for compression</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Video className="text-muted-foreground h-5 w-5" />
                <h3 className="font-semibold">FFmpeg</h3>
                <span className="text-muted-foreground text-xs">
                  Download FFmpeg for video compression and format conversion
                </span>
              </div>

              <div className="space-y-3">
                <div className="space-y-2">
                  <Label className="text-sm">FFmpeg Status</Label>
                  <div className="flex items-center gap-2">
                    <Badge variant={ffmpegAvailable ? 'default' : 'secondary'}>
                      {ffmpegAvailable ? 'Downloaded' : 'Not Downloaded'}
                    </Badge>
                    <span className="text-muted-foreground text-xs">~80 MB download</span>
                    {isDownloadingFfmpeg && (
                      <div className="flex items-center gap-2">
                        <Progress value={ffmpegDownloadProgress} className="w-24" />
                        <span className="text-xs">{ffmpegDownloadProgress}%</span>
                      </div>
                    )}
                    {ffmpegAvailable && !isDownloadingFfmpeg && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          try {
                            const status = await checkFfmpegStatus()
                            if (status?.path) {
                              revealInExplorer(status.path)
                            }
                          } catch (err) {
                            logError(`Failed to reveal FFmpeg: ${err}`)
                          }
                        }}
                      >
                        <EyeIcon className="mr-2 h-4 w-4" />
                        Reveal
                      </Button>
                    )}
                    {!ffmpegAvailable && !isDownloadingFfmpeg && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          startFfmpegDownload()
                          try {
                            await downloadFfmpeg()
                            await addNotification({
                              message: 'FFmpeg downloaded successfully',
                              status: 'success',
                            })
                          } catch (err) {
                            logError(`Failed to download FFmpeg: ${err}`)
                          } finally {
                            finishFfmpegDownload()
                          }
                        }}
                      >
                        <Download className="mr-2 h-4 w-4" />
                        Download
                      </Button>
                    )}
                  </div>
                </div>
              </div>
            </div>

            <Separator />

            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Video className="text-muted-foreground h-5 w-5" />
                <h3 className="font-semibold">Concurrency</h3>
                <span className="text-muted-foreground text-xs">
                  Maximum simultaneous video encodes
                </span>
              </div>
              <div className="space-y-2">
                <Label className="text-sm">Max concurrent video jobs</Label>
                <Select
                  value={String(maxVideoJobs)}
                  onValueChange={async (value) => {
                    const limit = Number.parseInt(value, 10)
                    setMaxVideoJobs(limit)
                    try {
                      await setVideoJobLimit(limit)
                    } catch (err) {
                      logError(`Failed to save video concurrency limit: ${err}`)
                    }
                  }}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {[1, 2, 3, 4, 6, 8].map((n) => (
                      <SelectItem key={n} value={String(n)}>
                        {n} at a time
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-muted-foreground text-xs">
                  Keep this low on laptops so other work stays responsive.
                </p>
              </div>

              <div className="space-y-2">
                <Label className="text-sm">Max concurrent image jobs</Label>
                <Select
                  value={String(maxImageJobs)}
                  onValueChange={async (value) => {
                    const limit = Number.parseInt(value, 10)
                    setMaxImageJobs(limit)
                    try {
                      await setImageJobLimit(limit)
                    } catch (err) {
                      logError(`Failed to save image concurrency limit: ${err}`)
                    }
                  }}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {[1, 2, 3, 4, 6, 8].map((n) => (
                      <SelectItem key={n} value={String(n)}>
                        {n} at a time
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-muted-foreground text-xs">
                  Applies to image compress / convert / upscale / background removal.
                </p>
              </div>
            </div>

            <Separator />

            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Video className="text-muted-foreground h-5 w-5" />
                <h3 className="font-semibold">Hardware acceleration</h3>
                <span className="text-muted-foreground text-xs">Off by default</span>
              </div>
              <div className="space-y-2">
                <Label className="text-sm">Encoder</Label>
                <Select
                  value={hardwareAccel}
                  onValueChange={async (value) => {
                    setHardwareAccel(value)
                    try {
                      await setSetting('hardware_acceleration', value)
                    } catch (err) {
                      logError(`Failed to save hardware acceleration: ${err}`)
                    }
                  }}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto (best available)</SelectItem>
                    <SelectItem value="off">Off (software H.264)</SelectItem>
                    {hardwareOptions.map((encoder) => (
                      <SelectItem key={encoder} value={encoder}>
                        {encoder}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-muted-foreground text-xs">
                  {hardwareOptions.length > 0
                    ? 'Only encoders verified by a test encode on this machine are listed. Auto uses the fastest available; falls back to software on failure.'
                    : 'No verified hardware encoder was found on this machine — software H.264 is used.'}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
        <Separator className="my-6" />
        {/* Update Settings */}
        <Card className="my-4">
          <CardHeader>
            <CardTitle>Updates</CardTitle>
            <CardDescription>Configure application update behavior</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-3">
              <Checkbox
                id="update-checks"
                checked={updateChecksEnabled}
                onCheckedChange={(checked) => {
                  const value = Boolean(checked)
                  setUpdateChecksEnabled(value)
                  setSetting('update_checks_enabled', value ? 'true' : 'false').catch((err) => {
                    logError(`Failed to save update check setting: ${err}`)
                  })
                }}
              />
              <label
                htmlFor="update-checks"
                className="text-sm leading-none font-medium peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
              >
                Enable automatic update checks
              </label>
            </div>
            <p className="text-muted-foreground mt-2 text-sm">
              When enabled, the app will periodically check for updates
            </p>
          </CardContent>
        </Card>
      </div>
    </section>
  )
}
