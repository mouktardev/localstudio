import { createFileRoute, Outlet } from '@tanstack/react-router'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { AppSidebar } from '@/components/app-sidebar'
import { NotificationBell } from '@/components/notification-bell'
import { LogPanel } from '@/components/log-panel'
import { DatabaseViewerPanel } from '@/components/db-viewer-panel'
import { CompressionListener } from '@/components/compression-listener'
import { UpscalingListener } from '@/components/upscaling-listener'
import { BgRemovalListener } from '@/components/bg-removal-listener'
import { VideoCompressionListener } from '@/components/video-compression-listener'
import { ConversionListener } from '@/components/conversion-listener'
import { DownloadListener } from '@/components/download-listener'
import { DownloadStatusInit } from '@/components/download-status-init'
import { useValue } from '@/schema/tinybase-schema'
import UpdateChecker from '@/components/update-checker'
import { DbSyncButton } from '@/components/db-sync-button'

export const Route = createFileRoute('/_app')({
  component: RouteComponent,
})

function RouteComponent() {
  const logsOpen = useValue('logsOpen')
  const dbViewerOpen = useValue('dbViewerOpen')

  return (
    <SidebarProvider defaultOpen style={{ '--sidebar-width': '14rem' } as React.CSSProperties}>
      <CompressionListener />
      <UpscalingListener />
      <BgRemovalListener />
      <VideoCompressionListener />
      <ConversionListener />
      <DownloadListener />
      <DownloadStatusInit />
      <AppSidebar />
      <SidebarInset className="flex flex-col">
        <header className="bg-sidebar flex h-10 shrink-0 items-center justify-between gap-4 border-b px-4">
          <div className="flex items-center gap-4">
            <SidebarTrigger />
          </div>
          <div className="flex items-center gap-2">
            <NotificationBell />
          </div>
        </header>
        <main className="flex min-h-[calc(100vh-70px)] w-full flex-col">
          <Outlet />
        </main>
        <footer className="bg-sidebar mt-auto flex shrink-0 items-center justify-between border-t px-4">
          <DbSyncButton />
          <UpdateChecker />
        </footer>
        {logsOpen && <LogPanel />}
        {dbViewerOpen && <DatabaseViewerPanel />}
      </SidebarInset>
    </SidebarProvider>
  )
}
