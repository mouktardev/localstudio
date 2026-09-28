// SQLite is the single source of truth for the library. The frontend keeps a
// hydrated cache (route loaders + local state) that is invalidated via the
// `images-updated` / `videos-updated` events -> router.invalidate(). This store
// only carries transient UI state: job progress and selections. Never mirror
// library rows (paths, sizes, thumbnails) into it, and never store base64 data.
import type { Store } from 'tinybase/store/with-schemas'
import * as UiReact from 'tinybase/ui-react/with-schemas'

export const tablesSchema = {
  clients: {
    name: { type: 'string' },
  },
  model_downloads: {
    downloaded: { type: 'boolean' },
  },
  compressions: {
    progress: { type: 'number' },
    message: { type: 'string' },
  },
  upscalings: {
    progress: { type: 'number' },
    message: { type: 'string' },
  },
  bg_removals: {
    progress: { type: 'number' },
    message: { type: 'string' },
  },
  video_bg_removals: {
    progress: { type: 'number' },
    message: { type: 'string' },
    eta_seconds: { type: 'number' },
  },
  video_compressions: {
    progress: { type: 'number' },
    message: { type: 'string' },
    stage: { type: 'string' },
    status: { type: 'string' },
    eta_seconds: { type: 'number' },
    speed: { type: 'number' },
  },
  image_conversions: {
    progress: { type: 'number' },
    message: { type: 'string' },
  },
  video_conversions: {
    progress: { type: 'number' },
    message: { type: 'string' },
    stage: { type: 'string' },
    status: { type: 'string' },
    eta_seconds: { type: 'number' },
    speed: { type: 'number' },
  },
} as const

export const valuesSchema = {
  version: { type: 'string', default: '0.1.0' },
  logsOpen: { type: 'boolean', default: false },
  logsUnread: { type: 'boolean', default: false },
  dbViewerOpen: { type: 'boolean', default: false },
  dbNeedsSync: { type: 'boolean', default: false },
  isDownloadingUpscale: { type: 'boolean', default: false },
  isDownloadingBgRemoval: { type: 'boolean', default: false },
  isDownloadingFfmpeg: { type: 'boolean', default: false },
  ffmpegAvailable: { type: 'boolean', default: false },
  upscaleDownloadProgress: { type: 'number', default: 0 },
  bgRemovalDownloadProgress: { type: 'number', default: 0 },
  ffmpegDownloadProgress: { type: 'number', default: 0 },
} as const

export type AppStore = Store<[typeof tablesSchema, typeof valuesSchema]>
const UiReactWithSchemas = UiReact as UiReact.WithSchemas<
  [typeof tablesSchema, typeof valuesSchema]
>

export const {
  Provider,
  useTablesState,
  useTableState,
  useRowState,
  useCellState,
  useParamValuesState,
  useValuesState,
  useParamValueState,
  useValueState,
  useCreateIndexes,
  useCreateRelationships,
  useCreatePersister,
  useCreateQueries,
  useCreateStore,
  CellProps,
  useTable,
  useTablesListener,
  useTableListener,
  useResultTable,
  useResultRow,
  RowView,
  RowProps,
  useAddRowCallback,
  useCell,
  useRow,
  useValues,
  useValue,
  useHasValue,
  useHasRow,
  useDelRowCallback,
  useRowIds,
  useSetPartialRowCallback,
  useRowListener,
  useSetPartialValuesCallback,
  useRelationships,
  RemoteRowView,
  useQueries,
  useResultCell,
  useResultSortedRowIds,
  useResultRowIds,
  useResultTableCellIds,
  useSetCellCallback,
  useSliceIds,
  useIndexes,
  IndexView,
  useSliceRowIds,
  SliceProps,
  SliceView,
  useStore,
  useSetTableCallback,
  useDelTableCallback,
  useLocalRowIds,
  CellView,
  ResultCellProps,
  ResultCellView,
  ResultRowView,
  useSetRowCallback,
  useSortedRowIds,
  useSetValueCallback,
} = UiReactWithSchemas
