import ReactDOM from 'react-dom/client'
import { RouterProvider, createRouter } from '@tanstack/react-router'
// Import the generated route tree
import { routeTree } from './routeTree.gen'
import '@/style.css'
import { ThemeProvider } from './components/theme-provider'
import {
  useCreateStore,
  Provider as TinyBaseProvider,
  tablesSchema,
  valuesSchema,
  useCreateQueries,
} from '@/schema/tinybase-schema'
import { createQueries, createStore } from 'tinybase/with-schemas'
import { TooltipProvider } from '@/components/ui/tooltip'
import { setupLogger, startLogWatcher } from '@/lib/logger'
import { checkDbHealth } from '@/lib/tauri'
import { useEffect } from 'react'

setupLogger()
// Create a new router instance with scroll restoration enabled
const router = createRouter({
  routeTree,
  scrollRestoration: true,
})

// Register the router instance for type safety
declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
export function App() {
  const store = useCreateStore(() => createStore().setSchema(tablesSchema, valuesSchema))
  const queries = useCreateQueries(store, createQueries, [])

  useEffect(() => {
    if (store) {
      const stopLogWatcher = startLogWatcher(store)

      // Check DB health on startup
      checkDbHealth()
        .then((orphanCount) => {
          if (orphanCount > 0) {
            store.setValue('dbNeedsSync', true)
          }
        })
        .catch(console.error)

      return () => {
        stopLogWatcher()
      }
    }
  }, [store])

  return (
    <ThemeProvider defaultTheme="dark" storageKey="theme">
      <TinyBaseProvider store={store} queries={queries}>
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </TinyBaseProvider>
    </ThemeProvider>
  )
}
// Render the app
const rootElement = document.getElementById('root')!
if (!rootElement.innerHTML) {
  const root = ReactDOM.createRoot(rootElement)
  root.render(<App />)
}
