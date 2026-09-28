import { Link, useLocation } from '@tanstack/react-router'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from '@/components/ui/sidebar'
import { ImageDown, SettingsIcon, TerminalIcon, Folder, Video, Database } from 'lucide-react'
import { Separator } from '@/components/ui/separator'
import { useValue, useSetPartialValuesCallback } from '@/schema/tinybase-schema'
import packageJson from '../../package.json'
import { cn } from '@/lib/utils'

export function AppSidebar() {
  const location = useLocation()
  const { state } = useSidebar()
  const isCollapsed = state === 'collapsed'

  const logsOpen = useValue('logsOpen')
  const logsUnread = useValue('logsUnread')

  // The logs and database panels are mutually exclusive.
  const toggleLogsOpen = useSetPartialValuesCallback((_, store) => ({
    logsOpen: !store.getValue('logsOpen'),
    logsUnread: false,
    dbViewerOpen: false,
  }))

  const dbViewerOpen = useValue('dbViewerOpen')
  const toggleDbViewer = useSetPartialValuesCallback((_, store) => ({
    dbViewerOpen: !store.getValue('dbViewerOpen'),
    logsOpen: false,
  }))

  const isHomeActive = location.pathname === '/'
  const isVideosActive = location.pathname === '/videos'
  const isOutputActive = location.pathname === '/output'
  const isSettingsActive = location.pathname === '/settings'

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuButton
            size="sm"
            asChild
            tooltip={isCollapsed ? `localstudio v${packageJson.version}` : undefined}
          >
            <div className="flex items-center gap-2 px-2 py-7">
              <img
                src="/app-icon.png"
                alt="localstudio"
                className={cn(
                  isCollapsed ? 'size-6' : 'size-15',
                  'shrink-0 rounded-md object-cover'
                )}
              />
              <div className="group-data-[collapsible=icon]:hidden">
                <span className="text-muted-foreground text-[10px]">v{packageJson.version}</span>
              </div>
            </div>
          </SidebarMenuButton>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <Separator className="my-2" />
        <SidebarGroup>
          <SidebarGroupLabel>tools</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  asChild
                  isActive={isHomeActive}
                  tooltip={isCollapsed ? 'Images' : undefined}
                >
                  <Link to="/">
                    <ImageDown className="h-4 w-4" />
                    <span>Images</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  asChild
                  isActive={isVideosActive}
                  tooltip={isCollapsed ? 'Videos' : undefined}
                >
                  <Link to="/videos">
                    <Video className="h-4 w-4" />
                    <span>Videos</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  asChild
                  isActive={isOutputActive}
                  tooltip={isCollapsed ? 'Output' : undefined}
                >
                  <Link to="/output">
                    <Folder className="h-4 w-4" />
                    <span>Output</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <Separator className="my-2" />
        <SidebarGroup>
          <SidebarGroupLabel>settings</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  asChild
                  isActive={isSettingsActive}
                  tooltip={isCollapsed ? 'General' : undefined}
                >
                  <Link to="/settings">
                    <SettingsIcon className="h-4 w-4" />
                    <span>General</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="sm"
              onClick={toggleLogsOpen}
              isActive={logsOpen}
              tooltip={isCollapsed ? 'Toggle log panel' : undefined}
            >
              {logsUnread && !logsOpen && (
                <span className="bg-destructive absolute -top-1 -left-1 h-2.5 w-2.5 rounded-full" />
              )}
              <TerminalIcon className="h-4 w-4 shrink-0" />
              <span>logs</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="sm"
              onClick={toggleDbViewer}
              isActive={dbViewerOpen}
              tooltip={isCollapsed ? 'Toggle database viewer' : undefined}
            >
              <Database className="h-4 w-4 shrink-0" />
              <span>database</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  )
}
