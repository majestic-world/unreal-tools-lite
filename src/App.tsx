import { useEffect, useMemo, useRef, useState, type PointerEvent, type SVGProps } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import "./App.css";

type IconName =
  | "brand"
  | "image"
  | "refresh"
  | "folder"
  | "file"
  | "settings"
  | "search"
  | "chevron"
  | "download"
  | "plus"
  | "grid"
  | "list"
  | "info"
  | "close"
  | "minimize"
  | "maximize"
  | "eye"
  | "swap"
  | "upload"
  | "copy"
  | "resize"
  | "sun"
  | "moon";

const iconPaths: Record<IconName, string> = {
  brand: "M7.4 3.6 10 6.2l-2.4 2.4L5 6m8.7-2.4L11 6.2l2.4 2.4L16 6m-7.4 8.2L6 16.8l2.6 2.6m6.8-5.2 2.6 2.6-2.6 2.6M10.5 12h3",
  image: "M4 4h16v16H4V4Zm0 12 4.3-4.3 3.1 3.1 2.1-2.1L20 19M8 8.5h.01",
  refresh: "M20 11a8 8 0 1 0 1 4.1M20 4v7h-7",
  folder: "M3.5 6.5h6l1.8 2h9.2v9.7a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2V6.5Z",
  file: "M7 2.8h6.7L18 7.1v13.1H7V2.8Zm6.4 0v4.5H18M9.5 12h6m-6 3h6",
  settings: "M12 8.4A3.6 3.6 0 1 0 12 15.6 3.6 3.6 0 0 0 12 8.4Zm0-5.1 1 .2.7 2a7.3 7.3 0 0 1 1.7 1l2-.7.8.8-.7 2c.4.5.7 1.1 1 1.7l2 .7.2 1-.2 1-.2.7-2 .7a7.3 7.3 0 0 1-1 1.7l.7 2-.8.8-2-.7a7.3 7.3 0 0 1-1.7 1l-.7 2-1 .2-1-.2-.7-2a7.3 7.3 0 0 1-1.7-1l-2 .7-.8-.8.7-2a7.3 7.3 0 0 1-1-1.7l-2-.7-.2-1 .2-1 .2-.7 2-.7c.3-.6.6-1.2 1-1.7l-.7-2 .8-.8 2 .7a7.3 7.3 0 0 1 1.7-1l.7-2 1-.2Z",
  search: "m20 20-4.1-4.1m1.6-4.4a6 6 0 1 1-12 0 6 6 0 0 1 12 0Z",
  chevron: "m9 18 6-6-6-6",
  download: "M12 3v11m0 0 4-4m-4 4-4-4M5 20h14",
  plus: "M12 5v14M5 12h14",
  grid: "M4 4h6v6H4V4Zm10 0h6v6h-6V4ZM4 14h6v6H4v-6Zm10 0h6v6h-6v-6Z",
  list: "M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01",
  info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-10v5m0-8v.1",
  close: "m6 6 12 12M18 6 6 18",
  minimize: "M6 17h12",
  maximize: "M7 7h10v10H7V7Z",
  eye: "M2.5 12s3.5-5.5 9.5-5.5 9.5 5.5 9.5 5.5-3.5 5.5-9.5 5.5S2.5 12 2.5 12Zm12.5 0a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
  swap: "m7 7 3-3m-3 3 3 3M7 7h10a3 3 0 0 1 3 3v1m-3 6-3 3m3-3-3-3m3 3H7a3 3 0 0 1-3-3v-1",
  upload: "M12 21V9m0 0 4 4m-4-4-4 4M5 4h14",
  copy: "M8 8h11v12H8V8Zm-3-4h11v3M5 4v12h2",
  resize: "M4 9V4h5M20 15v5h-5M4 4l6 6M20 20l-6-6M15 4h5v5M20 4l-6 6M9 20H4v-5M4 20l6-6",
  sun: "M12 3v2m0 14v2M3 12h2m14 0h2M5.6 5.6 7 7m10 10 1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z",
  moon: "M20 15.5A8 8 0 0 1 8.5 4 8.2 8.2 0 1 0 20 15.5Z",
};

function Icon({ name, size = 18, ...props }: { name: IconName; size?: number } & SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>
      <path d={iconPaths[name]} />
    </svg>
  );
}

const navigation = [
  { id: "utx", label: "Texturas UTX", hint: "Assets e Split9", icon: "image" as const },
  { id: "extract", label: "Utx Extract", hint: "Extração em lote", icon: "download" as const },
  { id: "resize", label: "Redimensionar", hint: "DDS e TGA em lote", icon: "resize" as const },
  { id: "geodata", label: "Converter Geodata", hint: "L2J, L2G e CONV_DAT", icon: "refresh" as const },
];

const settingsNavigation = { id: "settings", label: "Configurações", hint: "Preferências do app", icon: "settings" as const };
type Theme = "dark" | "light";
type UtxViewMode = "list" | "gallery";

type UtxFormat = "P8" | "RGBA7" | "RGB16" | "DXT1" | "RGB8" | "RGBA8" | "NODATA" | "DXT3" | "DXT5" | "L8" | "G16" | "UNKNOWN";

type UtxEntry = {
  name: string;
  format: UtxFormat;
  exportIndex: number;
  width: number;
  height: number;
  hasAlpha: boolean;
  hasSplit9: boolean;
  split9X1: number;
  split9X2: number;
  split9X3: number;
  split9Y1: number;
  split9Y2: number;
  split9Y3: number;
};

type UtxPreview = { dataUrl: string; width: number; height: number };
type UtxImportSummary = {
  replaced: number;
  added: number;
  skipped: number;
  failed: number;
  errors: string[];
  logPath?: string | null;
};
type UtxImportProgress = { completed: number; total: number; phase: string; fileName: string };
type UtxExportProgress = { completed: number; total: number; fileName: string };
type UtxTextureProperties = {
  alpha: boolean | null;
  masked: boolean | null;
  uClamp: number | null;
  vClamp: number | null;
  uClampMode: number | null;
  vClampMode: number | null;
  split9: boolean | null;
  split9X1: number;
  split9X2: number;
  split9X3: number;
  split9Y1: number;
  split9Y2: number;
  split9Y3: number;
  animation: {
    animNext: number | null;
    maxFrameRate: number | null;
    minFrameRate: number | null;
    oneTimeAnimLoop: boolean | null;
    primeCount: number | null;
    totalFrameNum: number | null;
  };
};
type UtxPropertyForm = {
  alpha: boolean;
  masked: boolean;
  uClamp: string;
  vClamp: string;
  uClampMode: string;
  vClampMode: string;
  split9Enabled: boolean;
  split9X1: string;
  split9X2: string;
  split9X3: string;
  split9Y1: string;
  split9Y2: string;
  split9Y3: string;
  animationEnabled: boolean;
  animNext: string;
  maxFrameRate: string;
  minFrameRate: string;
  oneTimeAnimLoop: boolean;
  primeCount: string;
  totalFrameNum: string;
};
type UtxPropertiesDialog = { entry: UtxEntry; form: UtxPropertyForm; loading: boolean };
type UtxBatchPropertyChoice = "keep" | "enabled" | "disabled";
type UtxBatchPropertyForm = {
  alpha: UtxBatchPropertyChoice;
  masked: UtxBatchPropertyChoice;
  updateSplit9: boolean;
  split9Enabled: boolean;
  split9X1: string;
  split9X2: string;
  split9X3: string;
  split9Y1: string;
  split9Y2: string;
  split9Y3: string;
};
type UtxBatchPropertiesDialog = { entries: UtxEntry[]; form: UtxBatchPropertyForm };
type UtxDuplicateDialog = { source: UtxEntry; group: string; name: string };
type UtxRenameDialog = { source: UtxEntry; name: string };

type TextureResizeProgress = { completed: number; total: number; fileName: string };
type TextureResizeSummary = { outputDirectory: string; totalFiles: number; resizedFiles: number; preservedFiles: number; copiedMetadata: number; failedFiles: number; errors: string[] };
type GeodataOutputFormat = "l2j" | "convDat" | "l2g";
type GeodataProgress = { completed: number; total: number; fileName: string };
type GeodataSummary = { outputDirectory: string; totalFiles: number; convertedFiles: number; copiedFiles: number; skippedFiles: number; failedFiles: number; workers: number; errors: string[] };
type UtxExtractMode = "original" | "png";
type UtxExtractProgress = { completed: number; total: number; packageName: string; fileName: string };
type UtxExtractSummary = { packages: number; exported: number; skipped: number; failed: number; errors: string[]; outputDirectory: string };
type AppSettings = Record<string, string>;

type Toast = { message: string; isError?: boolean; isInfo?: boolean };

const LAST_UTX_OPEN_DIRECTORY = "unreal-tools.utx.last-open-directory";
const LAST_UTX_EXPORT_DIRECTORY = "unreal-tools.utx.last-export-directory";
const LAST_UTX_NEW_DIRECTORY = "unreal-tools.utx.last-new-directory";
const UTX_RECENT_FILES = "unreal-tools.utx.recent-files";
const LAST_UTX_EXTRACT_INPUT_DIRECTORY = "unreal-tools.utx-extract.last-input-directory";
const LAST_UTX_EXTRACT_OUTPUT_DIRECTORY = "unreal-tools.utx-extract.last-output-directory";
const LAST_TEXTURE_RESIZE_DIRECTORY = "unreal-tools.texture-resize.directory";
const LAST_GEODATA_INPUT_DIRECTORY = "unreal-tools.geodata.input-directory";
const LAST_GEODATA_OUTPUT_DIRECTORY = "unreal-tools.geodata.output-directory";
const APP_THEME = "unreal-tools.appearance.theme";
const UTX_GALLERY_PAGE_SIZE = 84;
const UE2_TEXTURE_RESOLUTIONS = [4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048] as const;

const APP_SETTINGS_KEYS = [
  LAST_UTX_OPEN_DIRECTORY,
  LAST_UTX_EXPORT_DIRECTORY,
  LAST_UTX_NEW_DIRECTORY,
  UTX_RECENT_FILES,
  LAST_UTX_EXTRACT_INPUT_DIRECTORY,
  LAST_UTX_EXTRACT_OUTPUT_DIRECTORY,
  LAST_TEXTURE_RESIZE_DIRECTORY,
  LAST_GEODATA_INPUT_DIRECTORY,
  LAST_GEODATA_OUTPUT_DIRECTORY,
  APP_THEME,
] as const;

let persistedAppSettings: AppSettings = {};
let appSettingsReady = false;
let appSettingsSaveQueue: Promise<void> = Promise.resolve();

function fileStem(path: string) {
  return path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "recurso";
}

function utxExtensionFor(entry: UtxEntry) {
  return entry.format === "RGBA8" ? "tga" : entry.format.startsWith("DXT") ? "dds" : "bin";
}

function utxFileStem(name: string) {
  const separatorIndex = name.lastIndexOf(".");
  return separatorIndex >= 0 ? name.slice(separatorIndex + 1) : name;
}

function packageNameFor(entry: UtxEntry) {
  return entry.name.includes(".") ? entry.name.split(".")[0] : "Pacote principal";
}

function defaultUtxPackage(packages: string[]) {
  return packages.includes("Pacote principal") ? "Pacote principal" : packages[0] ?? null;
}

function isDxt(entry: UtxEntry) {
  return entry.format === "DXT1" || entry.format === "DXT3" || entry.format === "DXT5";
}

function isPreviewableTexture(entry: UtxEntry) {
  return entry.format === "RGBA8" || isDxt(entry);
}

function numberText(value: number | null | undefined) {
  return value === null || value === undefined ? "" : String(value);
}

function propertyFormFromState(properties: UtxTextureProperties, entries: UtxEntry[]): UtxPropertyForm {
  const animNextEntry = properties.animation.animNext && properties.animation.animNext > 0
    ? entries.find((entry) => entry.exportIndex + 1 === properties.animation.animNext)
    : undefined;
  return {
    alpha: properties.alpha ?? false,
    masked: properties.masked ?? false,
    uClamp: numberText(properties.uClamp),
    vClamp: numberText(properties.vClamp),
    uClampMode: numberText(properties.uClampMode),
    vClampMode: numberText(properties.vClampMode),
    split9Enabled: properties.split9 ?? false,
    split9X1: numberText(properties.split9X1),
    split9X2: numberText(properties.split9X2),
    split9X3: numberText(properties.split9X3),
    split9Y1: numberText(properties.split9Y1),
    split9Y2: numberText(properties.split9Y2),
    split9Y3: numberText(properties.split9Y3),
    animationEnabled: Boolean(properties.animation.animNext && properties.animation.animNext !== 0),
    animNext: animNextEntry?.name ?? "",
    maxFrameRate: numberText(properties.animation.maxFrameRate ?? 0),
    minFrameRate: numberText(properties.animation.minFrameRate ?? 0),
    oneTimeAnimLoop: properties.animation.oneTimeAnimLoop ?? false,
    primeCount: numberText(properties.animation.primeCount ?? 0),
    totalFrameNum: numberText(properties.animation.totalFrameNum ?? 0),
  };
}

function emptyUtxPropertyForm(): UtxPropertyForm {
  return {
    alpha: false, masked: false, uClamp: "", vClamp: "", uClampMode: "", vClampMode: "",
    split9Enabled: false, split9X1: "0", split9X2: "0", split9X3: "0", split9Y1: "0", split9Y2: "0", split9Y3: "0",
    animationEnabled: false, animNext: "", maxFrameRate: "0", minFrameRate: "0", oneTimeAnimLoop: false, primeCount: "0", totalFrameNum: "0",
  };
}

function emptyUtxBatchPropertyForm(): UtxBatchPropertyForm {
  return {
    alpha: "keep",
    masked: "keep",
    updateSplit9: false,
    split9Enabled: false,
    split9X1: "0",
    split9X2: "0",
    split9X3: "0",
    split9Y1: "0",
    split9Y2: "0",
    split9Y3: "0",
  };
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function plural(count: number, singular: string, pluralForm: string) {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}

function formatDuration(milliseconds: number) {
  const totalSeconds = Math.max(0, Math.round(milliseconds / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0 ? `${minutes}m ${String(seconds).padStart(2, "0")}s` : `${seconds}s`;
}

function directoryOf(path: string) {
  const separatorIndex = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return separatorIndex > 0 ? path.slice(0, separatorIndex) : undefined;
}

function legacySettingValue(key: string) {
  return window.localStorage.getItem(key) || undefined;
}

function rememberedValue(key: string) {
  return persistedAppSettings[key] ?? (appSettingsReady ? undefined : legacySettingValue(key));
}

function rememberedDirectory(key: string) {
  return rememberedValue(key);
}

function queueAppSettingsSave() {
  const settings = { ...persistedAppSettings };
  appSettingsSaveQueue = appSettingsSaveQueue
    .catch(() => undefined)
    .then(() => invoke<void>("app_settings_save", { settings }))
    .catch((error) => console.error("Unable to save app settings.", error));
}

function rememberValue(key: string, value: string) {
  persistedAppSettings[key] = value;
  if (appSettingsReady) queueAppSettingsSave();
}

function rememberDirectory(key: string, directory: string | undefined) {
  if (directory) rememberValue(key, directory);
}

function fileInDirectory(directory: string | undefined, fileName: string) {
  if (!directory) return fileName;
  const separator = directory.includes("\\") ? "\\" : "/";
  return `${directory}${directory.endsWith("/") || directory.endsWith("\\") ? "" : separator}${fileName}`;
}

function storedUtxRecentFiles() {
  try {
    const files = JSON.parse(rememberedValue(UTX_RECENT_FILES) ?? "[]");
    return Array.isArray(files)
      ? files.filter((file): file is string => typeof file === "string" && file.trim().length > 0).slice(0, 10)
      : [];
  } catch {
    return [];
  }
}

function legacyAppSettings(): AppSettings {
  return APP_SETTINGS_KEYS.reduce<AppSettings>((settings, key) => {
    const value = legacySettingValue(key);
    if (value !== undefined) settings[key] = value;
    return settings;
  }, {});
}

function App() {
  const [active, setActive] = useState("utx");
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<Toast | null>(null);
  const [utxImportProgress, setUtxImportProgress] = useState<UtxImportProgress | null>(null);
  const [utxExportProgress, setUtxExportProgress] = useState<UtxExportProgress | null>(null);
  const [utxImportFiles, setUtxImportFiles] = useState<string[] | null>(null);
  const [utxImportSourceDialog, setUtxImportSourceDialog] = useState(false);
  const [utxImportGroup, setUtxImportGroup] = useState("");
  const [utxExportScopeDialog, setUtxExportScopeDialog] = useState(false);
  const [utxNewDialog, setUtxNewDialog] = useState(false);
  const [utxNewName, setUtxNewName] = useState("");
  const [utxNewDirectory, setUtxNewDirectory] = useState<string | null>(null);
  const [utxFilePath, setUtxFilePath] = useState<string | null>(null);
  const [utxRecentFiles, setUtxRecentFiles] = useState<string[]>(storedUtxRecentFiles);
  const [utxRecentMenuOpen, setUtxRecentMenuOpen] = useState(false);
  const [utxEntries, setUtxEntries] = useState<UtxEntry[]>([]);
  const [utxFilter, setUtxFilter] = useState("Todos");
  const [utxQuery, setUtxQuery] = useState("");
  const [utxPackage, setUtxPackage] = useState<string | null>(null);
  const [utxViewMode, setUtxViewMode] = useState<UtxViewMode>("gallery");
  const [utxGalleryPage, setUtxGalleryPage] = useState(0);
  const [utxGalleryPreviews, setUtxGalleryPreviews] = useState<Record<number, UtxPreview>>({});
  const [utxGallerySelection, setUtxGallerySelection] = useState<Set<number>>(() => new Set());
  const [utxGallerySelectionAnchor, setUtxGallerySelectionAnchor] = useState<number | null>(null);
  const [utxGalleryActionsOpen, setUtxGalleryActionsOpen] = useState(false);
  const [utxGalleryContextMenu, setUtxGalleryContextMenu] = useState<{ entry: UtxEntry; x: number; y: number } | null>(null);
  const [utxPropertiesDialog, setUtxPropertiesDialog] = useState<UtxPropertiesDialog | null>(null);
  const [utxBatchPropertiesDialog, setUtxBatchPropertiesDialog] = useState<UtxBatchPropertiesDialog | null>(null);
  const [utxDuplicateDialog, setUtxDuplicateDialog] = useState<UtxDuplicateDialog | null>(null);
  const [utxRenameDialog, setUtxRenameDialog] = useState<UtxRenameDialog | null>(null);
  const [utxViewer, setUtxViewer] = useState<{ entry: UtxEntry; preview?: UtxPreview; loading: boolean } | null>(null);
  const [utxExtractFiles, setUtxExtractFiles] = useState<string[]>([]);
  const [utxExtractOutputDirectory, setUtxExtractOutputDirectory] = useState<string | null>(() => rememberedDirectory(LAST_UTX_EXTRACT_OUTPUT_DIRECTORY) ?? null);
  const [utxExtractMode, setUtxExtractMode] = useState<UtxExtractMode>("original");
  const [utxExtractProgress, setUtxExtractProgress] = useState<UtxExtractProgress | null>(null);
  const [utxExtractSummary, setUtxExtractSummary] = useState<UtxExtractSummary | null>(null);
  const [utxExtractElapsedMs, setUtxExtractElapsedMs] = useState<number | null>(null);
  const [textureResizeDirectory, setTextureResizeDirectory] = useState<string | null>(() => rememberedDirectory(LAST_TEXTURE_RESIZE_DIRECTORY) ?? null);
  const [textureResizeSourceResolution, setTextureResizeSourceResolution] = useState("64");
  const [textureResizeTargetResolution, setTextureResizeTargetResolution] = useState("32");
  const [textureResizeProgress, setTextureResizeProgress] = useState<TextureResizeProgress | null>(null);
  const [textureResizeSummary, setTextureResizeSummary] = useState<TextureResizeSummary | null>(null);
  const [geodataInputDirectory, setGeodataInputDirectory] = useState<string | null>(() => rememberedDirectory(LAST_GEODATA_INPUT_DIRECTORY) ?? null);
  const [geodataOutputDirectory, setGeodataOutputDirectory] = useState<string | null>(() => rememberedDirectory(LAST_GEODATA_OUTPUT_DIRECTORY) ?? null);
  const [geodataOutputFormat, setGeodataOutputFormat] = useState<GeodataOutputFormat>("l2j");
  const [geodataProgress, setGeodataProgress] = useState<GeodataProgress | null>(null);
  const [geodataSummary, setGeodataSummary] = useState<GeodataSummary | null>(null);
  const [theme, setTheme] = useState<Theme>(() => rememberedValue(APP_THEME) === "dark" ? "dark" : "light");
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [appSettingsLoaded, setAppSettingsLoaded] = useState(false);
  const previewRequest = useRef(0);
  const utxGalleryPreviewCache = useRef<Record<number, UtxPreview>>({});
  const toastTimeout = useRef<ReturnType<typeof window.setTimeout> | null>(null);

  const utxPackages = useMemo(() => Array.from(new Set(utxEntries.map(packageNameFor))).sort((left, right) => left.localeCompare(right)), [utxEntries]);
  const selectedUtxEntries = useMemo(
    () => utxPackage ? utxEntries.filter((entry) => packageNameFor(entry) === utxPackage) : [],
    [utxEntries, utxPackage],
  );
  const visibleUtxEntries = useMemo(() => selectedUtxEntries.filter((entry) => {
    const matchesFormat = utxFilter === "Todos" || (utxFilter === "RGBA8" ? entry.format === "RGBA8" : isDxt(entry));
    return matchesFormat && entry.name.toLowerCase().includes(utxQuery.trim().toLowerCase());
  }), [selectedUtxEntries, utxFilter, utxQuery]);
  const utxEntriesForBulkExport = useMemo(
    () => selectedUtxEntries.filter((entry) => utxFilter === "Todos" || (utxFilter === "RGBA8" ? entry.format === "RGBA8" : isDxt(entry))),
    [selectedUtxEntries, utxFilter],
  );
  const previewableUtxEntries = useMemo(() => visibleUtxEntries.filter(isPreviewableTexture), [visibleUtxEntries]);
  const utxGalleryPageCount = Math.max(1, Math.ceil(visibleUtxEntries.length / UTX_GALLERY_PAGE_SIZE));
  const utxGallerySelectedCount = utxGallerySelection.size;
  const selectedUtxGalleryEntries = useMemo(
    () => visibleUtxEntries.filter((entry) => utxGallerySelection.has(entry.exportIndex)),
    [utxGallerySelection, visibleUtxEntries],
  );
  const galleryUtxEntries = useMemo(
    () => visibleUtxEntries.slice(utxGalleryPage * UTX_GALLERY_PAGE_SIZE, (utxGalleryPage + 1) * UTX_GALLERY_PAGE_SIZE),
    [utxGalleryPage, visibleUtxEntries],
  );
  const utxStats = useMemo(() => ({
    rgba8: selectedUtxEntries.filter((entry) => entry.format === "RGBA8").length,
    dxt: selectedUtxEntries.filter(isDxt).length,
  }), [selectedUtxEntries]);
  const utxDuplicateNameInUse = useMemo(() => {
    if (!utxDuplicateDialog) return false;
    const group = utxDuplicateDialog.group.trim().toLowerCase();
    const name = utxDuplicateDialog.name.trim().toLowerCase();
    if (!group || !name) return false;
    return utxEntries.some((entry) => packageNameFor(entry).toLowerCase() === group && utxFileStem(entry.name).toLowerCase() === name);
  }, [utxDuplicateDialog, utxEntries]);
  const utxRenameNameInUse = useMemo(() => {
    if (!utxRenameDialog) return false;
    const group = packageNameFor(utxRenameDialog.source).toLowerCase();
    const name = utxRenameDialog.name.trim().toLowerCase();
    if (!name) return false;
    return utxEntries.some((entry) => entry.exportIndex !== utxRenameDialog.source.exportIndex && packageNameFor(entry).toLowerCase() === group && utxFileStem(entry.name).toLowerCase() === name);
  }, [utxRenameDialog, utxEntries]);
  const utxImportPercent = utxImportProgress && utxImportProgress.total > 0
    ? Math.round((utxImportProgress.completed / utxImportProgress.total) * 100)
    : 0;
  const utxExportPercent = utxExportProgress && utxExportProgress.total > 0
    ? Math.round((utxExportProgress.completed / utxExportProgress.total) * 100)
    : 0;
  const utxExtractPercent = utxExtractProgress && utxExtractProgress.total > 0
    ? Math.round((utxExtractProgress.completed / utxExtractProgress.total) * 100)
    : 0;
  const textureResizePercent = textureResizeProgress && textureResizeProgress.total > 0
    ? Math.round((textureResizeProgress.completed / textureResizeProgress.total) * 100)
    : 0;
  const geodataPercent = geodataProgress && geodataProgress.total > 0
    ? Math.round((geodataProgress.completed / geodataProgress.total) * 100)
    : 0;

  const activeItem = active === settingsNavigation.id ? settingsNavigation : navigation.find((item) => item.id === active) ?? navigation[0];
  const appWindow = getCurrentWindow();

  async function toggleMaximize() {
    await appWindow.toggleMaximize();
  }

  function startWindowDrag(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    void appWindow.startDragging().catch((error: unknown) => console.error("Não foi possível mover a janela:", error));
  }

  function closeWindow() {
    void appWindow.close().catch((error: unknown) => console.error("Não foi possível fechar a janela:", error));
  }

  function dismissToast() {
    if (toastTimeout.current !== null) {
      window.clearTimeout(toastTimeout.current);
      toastTimeout.current = null;
    }
    setToast(null);
  }

  function notify(message: string, isError = false, isInfo = false) {
    if (toastTimeout.current !== null) {
      window.clearTimeout(toastTimeout.current);
    }
    setToast({ message, isError, isInfo });
    toastTimeout.current = window.setTimeout(() => {
      setToast(null);
      toastTimeout.current = null;
    }, 15_000);
  }

  function toggleUtxViewMode() {
    const nextMode = utxViewMode === "list" ? "gallery" : "list";
    if (nextMode === "gallery") setUtxViewer(null);
    setUtxViewMode(nextMode);
  }

  function openUtxGalleryContextMenu(entry: UtxEntry, x: number, y: number) {
    if (!utxGallerySelection.has(entry.exportIndex)) {
      setUtxGallerySelection(new Set([entry.exportIndex]));
      setUtxGallerySelectionAnchor(entry.exportIndex);
    }
    setUtxGalleryActionsOpen(false);
    setUtxGalleryContextMenu({
      entry,
      x: Math.min(x, window.innerWidth - 196),
      y: Math.min(y, window.innerHeight - 256),
    });
  }

  function selectUtxGalleryEntry(entry: UtxEntry, selectRange: boolean, toggleSelection: boolean) {
    const entryPosition = visibleUtxEntries.findIndex((candidate) => candidate.exportIndex === entry.exportIndex);
    const anchorPosition = utxGallerySelectionAnchor === null
      ? -1
      : visibleUtxEntries.findIndex((candidate) => candidate.exportIndex === utxGallerySelectionAnchor);

    if (entryPosition < 0) return;

    if (!selectRange) {
      if (toggleSelection) {
        setUtxGallerySelection((current) => {
          const next = new Set(current);
          if (next.has(entry.exportIndex)) next.delete(entry.exportIndex);
          else next.add(entry.exportIndex);
          return next;
        });
      } else {
        setUtxGallerySelection((current) => {
          if (current.has(entry.exportIndex)) {
            const next = new Set(current);
            next.delete(entry.exportIndex);
            return next;
          }
          return new Set([entry.exportIndex]);
        });
      }
      setUtxGallerySelectionAnchor(entry.exportIndex);
      return;
    }

    if (anchorPosition < 0) {
      setUtxGallerySelection((current) => new Set(current).add(entry.exportIndex));
      setUtxGallerySelectionAnchor(entry.exportIndex);
      return;
    }

    const rangeStart = Math.min(anchorPosition, entryPosition);
    const rangeEnd = Math.max(anchorPosition, entryPosition);
    setUtxGallerySelection((current) => {
      const next = new Set(current);
      for (const candidate of visibleUtxEntries.slice(rangeStart, rangeEnd + 1)) next.add(candidate.exportIndex);
      return next;
    });
  }

  async function copyUtxTexturePath(entry: UtxEntry) {
    const packageName = utxFilePath ? fileStem(utxFilePath) : "";
    const texturePath = packageName && !entry.name.startsWith(`${packageName}.`) ? `${packageName}.${entry.name}` : entry.name;
    await copyToClipboard(texturePath, "Path da textura");
  }

  function updateUtxPropertyForm(update: Partial<UtxPropertyForm>) {
    setUtxPropertiesDialog((current) => current ? { ...current, form: { ...current.form, ...update } } : current);
  }

  function updateUtxBatchPropertyForm(update: Partial<UtxBatchPropertyForm>) {
    setUtxBatchPropertiesDialog((current) => current ? { ...current, form: { ...current.form, ...update } } : current);
  }

  function openUtxBatchProperties(entriesToEdit: UtxEntry[]) {
    if (entriesToEdit.length === 0) return;
    setUtxGalleryActionsOpen(false);
    setUtxBatchPropertiesDialog({ entries: entriesToEdit, form: emptyUtxBatchPropertyForm() });
  }

  function openUtxDuplicate(entry: UtxEntry) {
    setUtxGalleryContextMenu(null);
    setUtxDuplicateDialog({
      source: entry,
      group: packageNameFor(entry),
      name: `${utxFileStem(entry.name)}_copy`,
    });
  }

  async function duplicateUtxTexture() {
    const dialog = utxDuplicateDialog;
    if (!dialog || !utxFilePath) return;
    const groupName = dialog.group.trim();
    const textureName = dialog.name.trim();
    if (!groupName || !textureName) {
      notify("Informe o grupo e o nome da nova textura.", true);
      return;
    }
    if (utxDuplicateNameInUse) {
      notify("Já existe uma textura com esse nome no grupo informado.", true);
      return;
    }
    try {
      setBusy(true);
      await invoke<number>("utx_cached_duplicate_texture", {
        filePath: utxFilePath,
        sourceExportIndex: dialog.source.exportIndex,
        groupName,
        textureName,
      });
      await loadUtxPackage(utxFilePath, true, false);
      setUtxPackage(groupName);
      setUtxDuplicateDialog(null);
      notify(`Textura duplicada como ${groupName}.${textureName}.`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  function openUtxRename(entry: UtxEntry) {
    setUtxGalleryContextMenu(null);
    setUtxRenameDialog({ source: entry, name: utxFileStem(entry.name) });
  }

  async function renameUtxTexture() {
    const dialog = utxRenameDialog;
    if (!dialog || !utxFilePath) return;
    const textureName = dialog.name.trim();
    if (!textureName) {
      notify("Informe o novo nome da textura.", true);
      return;
    }
    if (utxRenameNameInUse) {
      notify("Já existe uma textura com esse nome no grupo atual.", true);
      return;
    }
    try {
      setBusy(true);
      await invoke("utx_cached_rename_texture", {
        filePath: utxFilePath,
        exportIndex: dialog.source.exportIndex,
        textureName,
      });
      await loadUtxPackage(utxFilePath, true, false);
      setUtxRenameDialog(null);
      notify(`Textura renomeada para ${textureName}.`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function openUtxProperties(entry: UtxEntry) {
    if (!utxFilePath) return;
    setUtxGalleryContextMenu(null);
    setUtxPropertiesDialog({ entry, form: emptyUtxPropertyForm(), loading: true });
    try {
      const properties = await invoke<UtxTextureProperties>("utx_cached_texture_properties", { filePath: utxFilePath, exportIndex: entry.exportIndex });
      setUtxPropertiesDialog({ entry, form: propertyFormFromState(properties, utxEntries), loading: false });
    } catch (error) {
      setUtxPropertiesDialog(null);
      notify(errorText(error), true);
    }
  }

  async function saveUtxProperties() {
    const dialog = utxPropertiesDialog;
    if (!dialog || !utxFilePath || dialog.loading) return;
    const { form } = dialog;
    const parseInteger = (value: string, label: string, fallback = 0) => {
      if (!value.trim()) return fallback;
      const parsed = Number(value);
      if (!Number.isInteger(parsed)) throw new Error(`${label} deve ser um número inteiro.`);
      return parsed;
    };
    const parseOptionalInteger = (value: string, label: string) => value.trim() ? parseInteger(value, label) : undefined;
    const parseFloatValue = (value: string, label: string) => {
      const parsed = Number(value || "0");
      if (!Number.isFinite(parsed)) throw new Error(`${label} deve ser um número válido.`);
      return parsed;
    };
    try {
      const nextEntry = form.animationEnabled
        ? utxEntries.find((entry) => entry.name === form.animNext.trim())
        : undefined;
      if (form.animationEnabled && !nextEntry) {
        notify("Selecione uma textura existente para AnimNext.", true);
        return;
      }
      const clamp = {
        uClamp: parseOptionalInteger(form.uClamp, "UClamp"),
        vClamp: parseOptionalInteger(form.vClamp, "VClamp"),
        uClampMode: parseOptionalInteger(form.uClampMode, "UClampMode"),
        vClampMode: parseOptionalInteger(form.vClampMode, "VClampMode"),
      };
      const hasClamp = Object.values(clamp).some((value) => value !== undefined);
      const edit = {
        alpha: form.alpha,
        masked: form.masked,
        clamp: hasClamp ? clamp : undefined,
        split9: {
          enabled: form.split9Enabled,
          x1: parseInteger(form.split9X1, "Split9X1"),
          x2: parseInteger(form.split9X2, "Split9X2"),
          x3: parseInteger(form.split9X3, "Split9X3"),
          y1: parseInteger(form.split9Y1, "Split9Y1"),
          y2: parseInteger(form.split9Y2, "Split9Y2"),
          y3: parseInteger(form.split9Y3, "Split9Y3"),
        },
        animation: {
          animNext: form.animationEnabled ? nextEntry!.exportIndex + 1 : 0,
          maxFrameRate: form.animationEnabled ? parseFloatValue(form.maxFrameRate, "MaxFrameRate") : 0,
          minFrameRate: form.animationEnabled ? parseFloatValue(form.minFrameRate, "MinFrameRate") : 0,
          oneTimeAnimLoop: form.animationEnabled && form.oneTimeAnimLoop,
          primeCount: form.animationEnabled ? parseInteger(form.primeCount, "PrimeCount") : 0,
          totalFrameNum: form.animationEnabled ? parseInteger(form.totalFrameNum, "TotalFrameNum") : 0,
        },
      };
      setBusy(true);
      await invoke("utx_cached_update_texture_properties", { filePath: utxFilePath, exportIndex: dialog.entry.exportIndex, edit });
      await loadUtxPackage(utxFilePath, true, false);
      setUtxPropertiesDialog(null);
      notify(`Propriedades atualizadas: ${dialog.entry.name}.`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function saveUtxBatchProperties() {
    const dialog = utxBatchPropertiesDialog;
    if (!dialog || !utxFilePath) return;
    const { form } = dialog;
    const parseInteger = (value: string, label: string) => {
      const parsed = Number(value);
      if (!Number.isInteger(parsed)) throw new Error(`${label} deve ser um número inteiro.`);
      return parsed;
    };
    const choiceValue = (choice: UtxBatchPropertyChoice) => choice === "keep" ? undefined : choice === "enabled";
    try {
      const alpha = choiceValue(form.alpha);
      const masked = choiceValue(form.masked);
      const split9 = form.updateSplit9 ? {
        enabled: form.split9Enabled,
        x1: parseInteger(form.split9X1, "Split9X1"),
        x2: parseInteger(form.split9X2, "Split9X2"),
        x3: parseInteger(form.split9X3, "Split9X3"),
        y1: parseInteger(form.split9Y1, "Split9Y1"),
        y2: parseInteger(form.split9Y2, "Split9Y2"),
        y3: parseInteger(form.split9Y3, "Split9Y3"),
      } : undefined;
      if (alpha === undefined && masked === undefined && !split9) {
        notify("Escolha ao menos uma propriedade para atualizar.", true);
        return;
      }
      setBusy(true);
      await invoke("utx_cached_update_texture_properties_batch", {
        filePath: utxFilePath,
        edits: dialog.entries.map((entry) => ({
          exportIndex: entry.exportIndex,
          edit: { alpha, masked, split9 },
        })),
      });
      await loadUtxPackage(utxFilePath, true, false);
      setUtxBatchPropertiesDialog(null);
      notify(`${plural(dialog.entries.length, "textura atualizada", "texturas atualizadas")}.`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function loadUtxPackage(path: string, preserveView = false, reopen = true) {
    setBusy(true);
    try {
      const listed = await invoke<UtxEntry[]>(reopen ? "utx_open_cached" : "utx_cached_list_entries", { filePath: path });
      const availablePackages = Array.from(new Set(listed.map(packageNameFor))).sort((left, right) => left.localeCompare(right));
      setUtxFilePath(path);
      setUtxRecentFiles((files) => [path, ...files.filter((file) => file.toLocaleLowerCase() !== path.toLocaleLowerCase())].slice(0, 10));
      setUtxEntries(listed);
      utxGalleryPreviewCache.current = {};
      setUtxGalleryPreviews({});
      setUtxGalleryPage(0);
      if (!preserveView) {
        setUtxFilter("Todos");
        setUtxQuery("");
        setUtxPackage(defaultUtxPackage(availablePackages));
      } else if (!listed.some((entry) => packageNameFor(entry) === utxPackage)) {
        setUtxPackage(defaultUtxPackage(availablePackages));
      }
      notify(`${listed.length} textura(s) carregada(s).`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function chooseUtxPackage() {
    const selected = await open({
      title: "Abrir pacote UTX",
      filters: [{ name: "Unreal Texture Package", extensions: ["utx"] }],
      multiple: false,
      defaultPath: rememberedDirectory(LAST_UTX_OPEN_DIRECTORY),
    });
    if (typeof selected === "string") {
      rememberDirectory(LAST_UTX_OPEN_DIRECTORY, directoryOf(selected));
      await loadUtxPackage(selected);
    }
  }

  function openNewUtxDialog() {
    setUtxNewName("");
    setUtxNewDirectory(rememberedDirectory(LAST_UTX_NEW_DIRECTORY) ?? rememberedDirectory(LAST_UTX_OPEN_DIRECTORY) ?? null);
    setUtxNewDialog(true);
  }

  async function chooseNewUtxDirectory() {
    const selected = await open({
      title: "Escolher onde salvar o novo UTX",
      directory: true,
      multiple: false,
      defaultPath: utxNewDirectory ?? rememberedDirectory(LAST_UTX_NEW_DIRECTORY) ?? rememberedDirectory(LAST_UTX_OPEN_DIRECTORY),
    });
    if (typeof selected === "string") {
      rememberDirectory(LAST_UTX_NEW_DIRECTORY, selected);
      setUtxNewDirectory(selected);
    }
  }

  async function createNewUtx() {
    const name = utxNewName.trim();
    const directory = utxNewDirectory;
    if (!name || !directory) return;
    const fileName = name.toLowerCase().endsWith(".utx") ? name : `${name}.utx`;
    const outputPath = fileInDirectory(directory, fileName);
    setBusy(true);
    try {
      await invoke("utx_create_new", { filePath: outputPath });
      rememberDirectory(LAST_UTX_OPEN_DIRECTORY, directory);
      rememberDirectory(LAST_UTX_NEW_DIRECTORY, directory);
      setUtxNewDialog(false);
      await loadUtxPackage(outputPath);
      notify("Novo UTX criado e aberto.");
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function exportOneUtx(entry: UtxEntry) {
    if (!utxFilePath) return;
    const extension = utxExtensionFor(entry);
    const outputPath = await save({
      title: `Exportar ${entry.format}`,
      defaultPath: fileInDirectory(rememberedDirectory(LAST_UTX_EXPORT_DIRECTORY), `${utxFileStem(entry.name)}.${extension}`),
      filters: [{ name: extension === "tga" ? "Targa Image" : extension === "dds" ? "DirectDraw Surface" : "Dados de textura", extensions: [extension] }],
    });
    if (!outputPath) return;
    rememberDirectory(LAST_UTX_EXPORT_DIRECTORY, directoryOf(outputPath));
    setBusy(true);
    try {
      await invoke("utx_cached_export_entry", { filePath: utxFilePath, exportIndex: entry.exportIndex, outputPath });
      notify(`Exportado: ${outputPath.split(/[\\/]/).pop()}`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function exportUtxEntries(entriesToExport: UtxEntry[]) {
    if (!utxFilePath || entriesToExport.length === 0) return;
    const outputDir = await open({
      title: "Selecionar pasta de exportação",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_UTX_EXPORT_DIRECTORY),
    });
    if (typeof outputDir !== "string") return;
    rememberDirectory(LAST_UTX_EXPORT_DIRECTORY, outputDir);
    const packageOutputDir = fileInDirectory(outputDir, fileStem(utxFilePath));
    setUtxExportProgress({ completed: 0, total: entriesToExport.length, fileName: "" });
    setBusy(true);
    try {
      const result = await invoke<{ exported: number; failed: number }>("utx_cached_export_entries", {
        filePath: utxFilePath,
        exportIndices: entriesToExport.map((entry) => entry.exportIndex),
        outputDir: packageOutputDir,
      });
      const exportedMessage = plural(result.exported, "textura exportada", "texturas exportadas");
      notify(result.failed ? `${exportedMessage}; ${plural(result.failed, "falha", "falhas")}.` : `${exportedMessage}.`, result.failed > 0);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setUtxExportProgress(null);
      setBusy(false);
    }
  }

  async function showUtxPreview(entry: UtxEntry) {
    if (!utxFilePath || !isPreviewableTexture(entry)) return;
    const requestId = ++previewRequest.current;
    setUtxViewer({ entry, loading: true });
    try {
      const preview = await invoke<UtxPreview>("utx_cached_preview_texture", { filePath: utxFilePath, exportIndex: entry.exportIndex });
      if (requestId === previewRequest.current) setUtxViewer({ entry, preview, loading: false });
    } catch (error) {
      if (requestId === previewRequest.current) {
        setUtxViewer(null);
        notify(errorText(error), true);
      }
    }
  }

  async function replaceUtxEntry(entry: UtxEntry) {
    if (!utxFilePath || !isPreviewableTexture(entry)) return;
    const extension = utxExtensionFor(entry);
    const selected = await open({
      title: `Selecionar ${entry.format} substituto`,
      filters: [{ name: extension === "tga" ? "Targa Image" : "DirectDraw Surface", extensions: [extension] }],
      multiple: false,
      defaultPath: rememberedDirectory(LAST_UTX_EXPORT_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    const accepted = await confirm(`Substituir "${entry.name}" no pacote atual? Esta operação altera o arquivo UTX.`, { title: "Confirmar substituição", kind: "warning", okLabel: "Substituir", cancelLabel: "Cancelar" });
    if (!accepted) return;
    setBusy(true);
    try {
      await invoke("utx_cached_replace_entry", { filePath: utxFilePath, exportIndex: entry.exportIndex, replacementPath: selected });
      await loadUtxPackage(utxFilePath, true, false);
      notify(`Substituída: ${entry.name}.`);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function importUtxEntries() {
    setUtxImportSourceDialog(true);
  }

  async function chooseUtxImportFiles() {
    if (!utxFilePath) return;
    setUtxImportSourceDialog(false);
    const selected = await open({
      title: "Selecionar texturas para importar",
      filters: [{ name: "Texturas suportadas", extensions: ["dds", "tga"] }],
      multiple: true,
      defaultPath: rememberedDirectory(LAST_UTX_EXPORT_DIRECTORY),
    });
    const files = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
    if (files.length === 0) return;
    setUtxImportFiles(files);
    setUtxImportGroup(utxPackage ?? "");
  }

  function notifyUtxImportSummary(result: UtxImportSummary) {
    const hasSuccess = result.replaced > 0 || result.added > 0;
    const parts = [
      result.replaced ? `${result.replaced} substituída(s)` : "",
      result.added ? `${result.added} adicionada(s)` : "",
      result.skipped ? `${result.skipped} ignorada(s)` : "",
      result.failed ? `${result.failed} falha(s)` : "",
    ].filter(Boolean);
    const detail = result.errors[0] ? ` ${hasSuccess ? "Detalhe" : "Erro"}: ${result.errors[0]}` : "";
    const logDetail =
      result.logPath && (result.failed > 0 || result.skipped > 0)
        ? ` Log: ${result.logPath}`
        : "";
    notify(
      `${parts.join(" · ") || "Nenhuma textura foi importada."}${detail}${logDetail}`,
      !hasSuccess && (result.failed > 0 || result.skipped > 0),
      hasSuccess && (result.failed > 0 || result.skipped > 0),
    );
  }

  async function chooseUtxImportDirectory() {
    if (!utxFilePath) return;
    setUtxImportSourceDialog(false);
    const selected = await open({
      title: "Selecionar pasta de texturas",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_UTX_EXPORT_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    rememberDirectory(LAST_UTX_EXPORT_DIRECTORY, selected);
    setUtxImportProgress({ completed: 0, total: 0, phase: "Lendo estrutura de pastas…", fileName: "" });
    setBusy(true);
    try {
      const result = await invoke<UtxImportSummary>("utx_cached_import_texture_directory", { filePath: utxFilePath, directory: selected });
      await loadUtxPackage(utxFilePath, true, false);
      notifyUtxImportSummary(result);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setUtxImportProgress(null);
      setBusy(false);
    }
  }

  async function confirmUtxImport() {
    const targetGroup = utxImportGroup.trim();
    const files = utxImportFiles;
    if (!utxFilePath || !files || files.length === 0 || !targetGroup) return;
    setUtxImportFiles(null);
    setUtxImportProgress({ completed: 0, total: files.length, phase: "Preparando texturas…", fileName: "" });
    setBusy(true);
    try {
      const result = await invoke<UtxImportSummary>("utx_cached_import_textures", { filePath: utxFilePath, packageName: targetGroup, texturePaths: files });
      await loadUtxPackage(utxFilePath, true, false);
      setUtxPackage(targetGroup);
      notifyUtxImportSummary(result);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setUtxImportProgress(null);
      setBusy(false);
    }
  }

  async function exportAllUtx() {
    setUtxExportScopeDialog(true);
  }

  async function exportUtxScope(scope: "group" | "all") {
    setUtxExportScopeDialog(false);
    await exportUtxEntries(scope === "group" ? utxEntriesForBulkExport : utxEntries);
  }

  async function exportSelectedUtx() {
    setUtxGalleryActionsOpen(false);
    await exportUtxEntries(selectedUtxGalleryEntries);
  }

  async function chooseUtxExtractFiles() {
    const selected = await open({
      title: "Selecionar arquivos UTX",
      filters: [{ name: "Unreal Texture Package", extensions: ["utx"] }],
      multiple: true,
      defaultPath: rememberedDirectory(LAST_UTX_EXTRACT_INPUT_DIRECTORY),
    });
    const files = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
    if (files.length === 0) return;
    rememberDirectory(LAST_UTX_EXTRACT_INPUT_DIRECTORY, directoryOf(files[0]));
    setUtxExtractFiles(files);
    setUtxExtractSummary(null);
  }

  async function chooseUtxExtractOutputDirectory() {
    const selected = await open({
      title: "Selecionar pasta de saída",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_UTX_EXTRACT_OUTPUT_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    rememberDirectory(LAST_UTX_EXTRACT_OUTPUT_DIRECTORY, selected);
    setUtxExtractOutputDirectory(selected);
    setUtxExtractSummary(null);
  }

  async function extractUtxPackages() {
    if (utxExtractFiles.length === 0) {
      notify("Selecione ao menos um arquivo UTX antes de extrair.", true);
      return;
    }
    if (!utxExtractOutputDirectory) {
      notify("Selecione a pasta de saída antes de extrair.", true);
      return;
    }
    setBusy(true);
    setUtxExtractSummary(null);
    setUtxExtractElapsedMs(null);
    setUtxExtractProgress({ completed: 0, total: 0, packageName: "", fileName: "Preparando pacotes…" });
    const startedAt = performance.now();
    try {
      const summary = await invoke<UtxExtractSummary>("utx_extract_packages", {
        filePaths: utxExtractFiles,
        outputDir: utxExtractOutputDirectory,
        mode: utxExtractMode,
      });
      setUtxExtractSummary(summary);
      setUtxExtractElapsedMs(performance.now() - startedAt);
      const hasSuccess = summary.exported > 0;
      const message = [
        plural(summary.exported, "textura extraída", "texturas extraídas"),
        summary.skipped ? plural(summary.skipped, "item ignorado", "itens ignorados") : "",
        summary.failed ? plural(summary.failed, "falha", "falhas") : "",
      ].filter(Boolean).join(" · ");
      notify(message, !hasSuccess && (summary.skipped > 0 || summary.failed > 0), hasSuccess && (summary.skipped > 0 || summary.failed > 0));
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setUtxExtractProgress(null);
      setBusy(false);
    }
  }

  async function chooseTextureResizeDirectory() {
    const selected = await open({
      title: "Selecionar pasta de ícones",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_TEXTURE_RESIZE_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    rememberDirectory(LAST_TEXTURE_RESIZE_DIRECTORY, selected);
    setTextureResizeDirectory(selected);
    setTextureResizeSummary(null);
  }

  async function resizeTextureDirectory() {
    if (!textureResizeDirectory) {
      notify("Selecione a pasta de ícones antes de iniciar.", true);
      return;
    }
    const sourceResolution = Number(textureResizeSourceResolution);
    const targetResolution = Number(textureResizeTargetResolution);
    if (
      !UE2_TEXTURE_RESOLUTIONS.includes(sourceResolution as typeof UE2_TEXTURE_RESOLUTIONS[number])
      || !UE2_TEXTURE_RESOLUTIONS.includes(targetResolution as typeof UE2_TEXTURE_RESOLUTIONS[number])
    ) {
      notify("Selecione dimensões compatíveis com Unreal Engine 2.", true);
      return;
    }
    setBusy(true);
    setTextureResizeSummary(null);
    setTextureResizeProgress({ completed: 0, total: 1, fileName: "Preparando arquivos…" });
    try {
      const summary = await invoke<TextureResizeSummary>("texture_resize_directory", {
        directory: textureResizeDirectory,
        sourceResolution,
        targetResolution,
      });
      setTextureResizeSummary(summary);
      const parts = [
        plural(summary.resizedFiles, "ícone redimensionado", "ícones redimensionados"),
        summary.preservedFiles ? plural(summary.preservedFiles, "arquivo copiado sem alteração", "arquivos copiados sem alteração") : "",
        summary.failedFiles ? `${plural(summary.failedFiles, "falha", "falhas")}` : "",
      ].filter(Boolean);
      notify(parts.join(" · "), summary.failedFiles > 0);
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setTextureResizeProgress(null);
      setBusy(false);
    }
  }

  async function chooseGeodataInputDirectory() {
    const selected = await open({
      title: "Selecionar pasta de geodata",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_GEODATA_INPUT_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    rememberDirectory(LAST_GEODATA_INPUT_DIRECTORY, selected);
    setGeodataInputDirectory(selected);
    setGeodataSummary(null);
  }

  async function chooseGeodataOutputDirectory() {
    const selected = await open({
      title: "Selecionar pasta de saída da geodata",
      directory: true,
      multiple: false,
      defaultPath: rememberedDirectory(LAST_GEODATA_OUTPUT_DIRECTORY),
    });
    if (typeof selected !== "string") return;
    rememberDirectory(LAST_GEODATA_OUTPUT_DIRECTORY, selected);
    setGeodataOutputDirectory(selected);
    setGeodataSummary(null);
  }

  async function convertGeodataDirectory() {
    if (!geodataInputDirectory) {
      notify("Selecione a pasta de entrada da geodata.", true);
      return;
    }
    if (!geodataOutputDirectory) {
      notify("Selecione a pasta de saída da geodata.", true);
      return;
    }
    setBusy(true);
    setGeodataSummary(null);
    setGeodataProgress({ completed: 0, total: 0, fileName: "Preparando arquivos…" });
    try {
      const summary = await invoke<GeodataSummary>("geodata_convert_directory", {
        inputDirectory: geodataInputDirectory,
        outputDirectory: geodataOutputDirectory,
        targetFormat: geodataOutputFormat,
      });
      setGeodataSummary(summary);
      const hasSuccess = summary.convertedFiles > 0 || summary.copiedFiles > 0;
      const message = [
        plural(summary.convertedFiles, "arquivo convertido", "arquivos convertidos"),
        summary.copiedFiles ? plural(summary.copiedFiles, "arquivo copiado", "arquivos copiados") : "",
        summary.skippedFiles ? plural(summary.skippedFiles, "item ignorado", "itens ignorados") : "",
        summary.failedFiles ? plural(summary.failedFiles, "falha", "falhas") : "",
      ].filter(Boolean).join(" · ");
      notify(message, !hasSuccess && (summary.skippedFiles > 0 || summary.failedFiles > 0), hasSuccess && (summary.skippedFiles > 0 || summary.failedFiles > 0));
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      setGeodataProgress(null);
      setBusy(false);
    }
  }

  async function copyToClipboard(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      notify(`${label} copiado.`);
    } catch {
      notify("Não foi possível copiar para a área de transferência.", true);
    }
  }

  function closeUtxViewer() {
    previewRequest.current += 1;
    setUtxViewer(null);
  }

  function moveUtxPreview(direction: -1 | 1) {
    if (!utxViewer || previewableUtxEntries.length < 2) return;
    const currentIndex = previewableUtxEntries.findIndex((entry) => entry.exportIndex === utxViewer.entry.exportIndex);
    const nextIndex = (currentIndex + direction + previewableUtxEntries.length) % previewableUtxEntries.length;
    void showUtxPreview(previewableUtxEntries[nextIndex]);
  }

  useEffect(() => {
    setUtxGalleryPage(0);
    setUtxGallerySelection(new Set());
    setUtxGallerySelectionAnchor(null);
    setUtxGalleryActionsOpen(false);
  }, [utxFilePath, utxPackage, utxFilter, utxQuery]);

  useEffect(() => {
    utxGalleryPreviewCache.current = {};
    setUtxGalleryPreviews({});
  }, [utxFilePath, utxViewMode, utxPackage, utxFilter, utxQuery, utxGalleryPage]);

  useEffect(() => {
    if (utxViewMode !== "gallery" || !utxFilePath) return;

    let cancelled = false;
    const pendingEntries = galleryUtxEntries.filter((entry) => isPreviewableTexture(entry) && !utxGalleryPreviewCache.current[entry.exportIndex]);
    let nextIndex = 0;

    async function loadNextPreview() {
      while (!cancelled) {
        const entry = pendingEntries[nextIndex++];
        if (!entry) return;

        try {
          const preview = await invoke<UtxPreview>("utx_cached_preview_texture", { filePath: utxFilePath, exportIndex: entry.exportIndex });
          if (cancelled) return;
          utxGalleryPreviewCache.current[entry.exportIndex] = preview;
          setUtxGalleryPreviews((current) => current[entry.exportIndex] ? current : { ...current, [entry.exportIndex]: preview });
        } catch {
          // A textura continua disponível na grade, mas sem miniatura quando o formato não puder ser decodificado.
        }
      }
    }

    void Promise.all(Array.from({ length: Math.min(3, pendingEntries.length) }, () => loadNextPreview()));
    return () => {
      cancelled = true;
    };
  }, [galleryUtxEntries, utxFilePath, utxViewMode]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<UtxImportProgress>("utx-import-progress", (event) => {
      setUtxImportProgress(event.payload);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error: unknown) => console.error("Não foi possível acompanhar a importação UTX:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<UtxExportProgress>("utx-export-progress", (event) => {
      setUtxExportProgress(event.payload);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error: unknown) => console.error("Não foi possível acompanhar a exportação UTX:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<UtxExtractProgress>("utx-extract-progress", (event) => {
      setUtxExtractProgress(event.payload);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error: unknown) => console.error("Não foi possível acompanhar a extração UTX:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<TextureResizeProgress>("texture-resize-progress", (event) => {
      setTextureResizeProgress(event.payload);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error: unknown) => console.error("Não foi possível acompanhar o redimensionamento:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<GeodataProgress>("geodata-convert-progress", (event) => {
      setGeodataProgress(event.payload);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error: unknown) => console.error("Não foi possível acompanhar a conversão geodata:", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!utxViewer && !utxPropertiesDialog && !utxBatchPropertiesDialog && !utxDuplicateDialog && !utxRenameDialog && !utxExportScopeDialog && !utxImportSourceDialog) return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        moveUtxPreview(-1);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        moveUtxPreview(1);
      } else if (event.key === "Escape") {
        event.preventDefault();
        if (utxImportSourceDialog) setUtxImportSourceDialog(false); else if (utxExportScopeDialog) setUtxExportScopeDialog(false); else if (utxRenameDialog) setUtxRenameDialog(null); else if (utxDuplicateDialog) setUtxDuplicateDialog(null); else if (utxBatchPropertiesDialog) setUtxBatchPropertiesDialog(null); else if (utxPropertiesDialog) setUtxPropertiesDialog(null); else closeUtxViewer();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [utxViewer, utxPropertiesDialog, utxBatchPropertiesDialog, utxDuplicateDialog, utxRenameDialog, utxExportScopeDialog, utxImportSourceDialog, previewableUtxEntries]);

  useEffect(() => {
    if (!utxGalleryContextMenu) return;

    function closeContextMenu() {
      setUtxGalleryContextMenu(null);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") closeContextMenu();
    }

    window.addEventListener("mousedown", closeContextMenu);
    window.addEventListener("scroll", closeContextMenu, true);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("mousedown", closeContextMenu);
      window.removeEventListener("scroll", closeContextMenu, true);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [utxGalleryContextMenu]);

  useEffect(() => {
    if (!utxGalleryActionsOpen) return;

    function closeActionsMenu() {
      setUtxGalleryActionsOpen(false);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") closeActionsMenu();
    }

    window.addEventListener("mousedown", closeActionsMenu);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("mousedown", closeActionsMenu);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [utxGalleryActionsOpen]);

  useEffect(() => {
    if (!utxRecentMenuOpen) return;

    function closeRecentMenu() {
      setUtxRecentMenuOpen(false);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") closeRecentMenu();
    }

    window.addEventListener("mousedown", closeRecentMenu);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("mousedown", closeRecentMenu);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [utxRecentMenuOpen]);

  useEffect(() => {
    let mounted = true;

    void invoke<AppSettings>("app_settings_load")
      .then((settings) => {
        const runtimeChanges = { ...persistedAppSettings };
        persistedAppSettings = { ...legacyAppSettings(), ...settings, ...runtimeChanges };
        appSettingsReady = true;
        queueAppSettingsSave();

        if (!mounted) return;
        setTextureResizeDirectory(rememberedDirectory(LAST_TEXTURE_RESIZE_DIRECTORY) ?? null);
        setGeodataInputDirectory(rememberedDirectory(LAST_GEODATA_INPUT_DIRECTORY) ?? null);
        setGeodataOutputDirectory(rememberedDirectory(LAST_GEODATA_OUTPUT_DIRECTORY) ?? null);
        setUtxExtractOutputDirectory(rememberedDirectory(LAST_UTX_EXTRACT_OUTPUT_DIRECTORY) ?? null);
        setUtxRecentFiles(storedUtxRecentFiles());
        setTheme(rememberedValue(APP_THEME) === "dark" ? "dark" : "light");
        setAppSettingsLoaded(true);
      })
      .catch((error) => console.error("Unable to load app settings.", error));

    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    if (!appSettingsLoaded) return;
    rememberValue(APP_THEME, theme);
  }, [theme, appSettingsLoaded]);

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => setAppVersion(null));
  }, []);

  useEffect(() => {
    if (!appSettingsLoaded) return;
    rememberValue(UTX_RECENT_FILES, JSON.stringify(utxRecentFiles));
  }, [utxRecentFiles, appSettingsLoaded]);

  useEffect(() => {
    function preventBrowserContextMenu(event: Event) {
      event.preventDefault();
    }

    window.addEventListener("contextmenu", preventBrowserContextMenu);
    return () => window.removeEventListener("contextmenu", preventBrowserContextMenu);
  }, []);

  useEffect(() => {
    function handleSelectionShortcuts(event: KeyboardEvent) {
      if (event.key === "F5") {
        event.preventDefault();
        return;
      }
      if (!event.ctrlKey || event.altKey) return;
      if (event.key.toLowerCase() === "a") {
        event.preventDefault();
      } else if (event.key.toLowerCase() === "x") {
        event.preventDefault();
        setUtxGallerySelection(new Set());
        setUtxGallerySelectionAnchor(null);
        setUtxGalleryActionsOpen(false);
      }
    }

    window.addEventListener("keydown", handleSelectionShortcuts, true);
    return () => window.removeEventListener("keydown", handleSelectionShortcuts, true);
  }, []);

  function clearUtxPackage() {
    setUtxFilePath(null);
    setUtxEntries([]);
    setUtxFilter("Todos");
    setUtxQuery("");
    setUtxPackage(null);
    utxGalleryPreviewCache.current = {};
    setUtxGalleryPreviews({});
    setUtxGalleryPage(0);
    setUtxGallerySelection(new Set());
    setUtxGallerySelectionAnchor(null);
    setUtxGalleryActionsOpen(false);
    setUtxGalleryContextMenu(null);
    setUtxPropertiesDialog(null);
    setUtxBatchPropertiesDialog(null);
    setUtxDuplicateDialog(null);
    setUtxRenameDialog(null);
    setUtxExportScopeDialog(false);
    setUtxImportSourceDialog(false);
    closeUtxViewer();
    void invoke("utx_clear_cache");
  }

  async function openAppSettingsDirectory() {
    try {
      const directory = await invoke<string>("app_settings_open_directory");
      notify(`Pasta de configurações aberta: ${directory}`);
    } catch (error) {
      notify(errorText(error), true);
    }
  }

  async function openAppLogsDirectory() {
    try {
      const directory = await invoke<string>("app_logs_open_directory");
      notify(`Pasta de logs aberta: ${directory}`);
    } catch (error) {
      notify(errorText(error), true);
    }
  }

  const isUtxPage = active === "utx";
  const isTextureResizePage = active === "resize";
  const isGeodataPage = active === "geodata";
  const isUtxExtractPage = active === "extract";
  const isSettingsPage = active === "settings";
  const textureResizeOutputHint = textureResizeDirectory
    ? `${textureResizeDirectory}${textureResizeDirectory.endsWith("/") || textureResizeDirectory.endsWith("\\") ? "" : "\\"}modified\\${textureResizeDirectory.split(/[\\/]/).filter(Boolean).pop() ?? "icons"}`
    : null;
  const headerTitle = isUtxPage ? "Texturas UTX" : isUtxExtractPage ? "Utx Extract" : isGeodataPage ? "Converter Geodata" : isTextureResizePage ? "Redimensionar" : "Configurações";
  const headerDescription = isUtxPage
    ? "Gerencie texturas, formatos DXT e dados Split9 sem sair do pacote."
    : isUtxExtractPage
      ? "Extraia texturas de vários pacotes UTX sem precisar abri-los no editor."
      : isGeodataPage
        ? "Converta regiões Lineage 2 entre L2J, CONV_DAT e L2G."
        : isTextureResizePage
          ? "Converta uma dimensão específica de DDS e TGA sem alterar os arquivos originais."
          : "Personalize a aparência e as preferências do aplicativo.";
  const headerEyebrow = isUtxExtractPage ? "EXTRAÇÃO DE PACOTES" : isGeodataPage ? "CONVERSÃO DE GEODATA" : isTextureResizePage ? "CONVERSÃO DE TEXTURAS" : isSettingsPage ? "PREFERÊNCIAS DO APP" : "FERRAMENTA DE PACOTES";
  const headerResourceLabel = isUtxPage
    ? utxFilePath ? utxPackage ? `${selectedUtxEntries.length} / ${plural(utxEntries.length, "textura", "texturas")}` : plural(utxEntries.length, "textura", "texturas") : "Aguardando arquivo"
    : isUtxExtractPage ? utxExtractFiles.length ? plural(utxExtractFiles.length, "arquivo UTX", "arquivos UTX") : "Aguardando arquivos" : isGeodataPage ? geodataSummary ? plural(geodataSummary.totalFiles, "região processada", "regiões processadas") : geodataInputDirectory ? "Pasta configurada" : "Aguardando pasta" : isTextureResizePage ? textureResizeSummary ? plural(textureResizeSummary.totalFiles, "ícone processado", "ícones processados") : textureResizeDirectory ? "Pasta configurada" : "Aguardando pasta" : theme === "dark" ? "Tema dark" : "Tema light";

  return (
    <main className="app-frame" data-theme={theme}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><Icon name="brand" size={20} /></div>
          <div className="brand-copy"><strong>Unreal Tools Lite</strong><span>Interface Kit</span></div>
        </div>

        <div className="workspace-label">WORKSPACE</div>
        <nav className="navigation" aria-label="Ferramentas">
          {navigation.map((item) => (
            <button className={`nav-item ${active === item.id ? "active" : ""}`} onClick={() => setActive(item.id)} key={item.id}>
              <span className="nav-icon"><Icon name={item.icon} size={18} /></span>
              <span className="nav-copy"><strong>{item.label}</strong><small>{item.hint}</small></span>
              {active === item.id && <Icon name="chevron" className="nav-chevron" size={15} />}
            </button>
          ))}
        </nav>

        <div className="sidebar-bottom">
          <div className="side-status"><span className="status-dot" />Sessão local</div>
          <button className={`settings-button ${isSettingsPage ? "active" : ""}`} onClick={() => setActive("settings")}>
            <span className="nav-icon"><Icon name="settings" size={18} /></span>
            <span className="nav-copy"><strong>Configurações</strong><small>Preferências do app</small></span>
            {isSettingsPage && <Icon name="chevron" className="nav-chevron" size={15} />}
          </button>
        </div>
      </aside>

      <section className="workspace">
        <header className="titlebar">
          <div className="drag-region" onPointerDown={startWindowDrag}>
            <div className="breadcrumb"><span>Unreal Tools Lite</span><Icon name="chevron" size={13} /><strong>{activeItem.label}</strong></div>
          </div>
          <div className="titlebar-actions">
            <button className="window-button" aria-label="Minimizar" onClick={() => void appWindow.minimize()}><Icon name="minimize" size={15} /></button>
            <button className="window-button" aria-label="Maximizar" onClick={() => void toggleMaximize()}><Icon name="maximize" size={14} /></button>
            <button className="window-button danger" aria-label="Fechar" onClick={closeWindow}><Icon name="close" size={16} /></button>
          </div>
        </header>
        <div className={`page-scroll ${isUtxPage ? "utx-page-scroll" : ""}`}>
          <section className="page-header">
            <div>
              <div className="eyebrow">{headerEyebrow}</div>
              <h1>{headerTitle}</h1>
              <p>{headerDescription}</p>
            </div>
            <div className="header-actions">
              {isUtxPage && <div className="utx-recent-menu">
                <button className="header-chip utx-recent-menu-trigger" disabled={utxRecentFiles.length === 0} onMouseDown={(event) => event.stopPropagation()} onClick={() => setUtxRecentMenuOpen((open) => !open)} aria-haspopup="menu" aria-expanded={utxRecentMenuOpen}><Icon name="folder" size={16} />Recentes<Icon className={utxRecentMenuOpen ? "open" : ""} name="chevron" size={14} /></button>
                {utxRecentMenuOpen && <div className="utx-recent-menu-popover" role="menu" aria-label="Arquivos UTX recentes" onMouseDown={(event) => event.stopPropagation()}>
                  <span className="utx-recent-menu-title">Arquivos recentes</span>
                  <div className="utx-recent-menu-list">
                    {utxRecentFiles.map((path) => <button className={utxFilePath?.toLocaleLowerCase() === path.toLocaleLowerCase() ? "active" : ""} role="menuitem" onClick={() => { setUtxRecentMenuOpen(false); void loadUtxPackage(path); }} disabled={busy} key={path} title={path}><Icon name="file" size={15} /><span><strong>{path.split(/[\\/]/).pop()}</strong><small>{path}</small></span></button>)}
                  </div>
                  <button className="utx-recent-menu-clear" role="menuitem" onClick={() => { setUtxRecentFiles([]); setUtxRecentMenuOpen(false); }} disabled={busy}><Icon name="close" size={14} />Limpar recentes</button>
                </div>}
              </div>}
              <div className="header-chip"><Icon name="grid" size={16} />{headerResourceLabel}</div>
            </div>
          </section>

          {isUtxPage ? (
            <>
              <section className="file-card utx-file-card">
                <button className="file-drop" onClick={() => void chooseUtxPackage()} disabled={busy}>
                  <span className="file-icon"><Icon name={utxFilePath ? "file" : "image"} size={22} /></span>
                  <span className="file-copy">
                    <strong>{utxFilePath ? utxFilePath.split(/[\\/]/).pop() : "Selecionar um pacote .utx"}</strong>
                    <small>{utxFilePath ?? "Escolha um pacote de texturas no explorador de arquivos"}</small>
                  </span>
                  <span className="browse-link">{busy ? "Carregando…" : "Procurar"} <Icon name="chevron" size={15} /></span>
                </button>
                <div className="file-actions">
                  <button className="secondary-action utx-bulk-action utx-export-action" disabled={!utxFilePath || busy || utxEntries.length === 0} onClick={() => void exportAllUtx()}><span className="utx-action-icon"><Icon name="download" size={17} /></span><span>Exportar todos</span></button>
                  <button className="primary-action utx-bulk-action utx-import-action" disabled={!utxFilePath || busy} onClick={() => void importUtxEntries()}><span className="utx-action-icon"><Icon name="upload" size={17} /></span><span>Importar lote</span></button>
                  <button className="row-action clear-package utx-clear-action" disabled={!utxFilePath || busy} onClick={clearUtxPackage} title="Fechar pacote" aria-label="Fechar pacote"><Icon name="close" size={17} /></button>
                </div>
              </section>

              <section className="toolbar utx-toolbar">
                <button className="secondary-action utx-new-action" disabled={busy} onClick={openNewUtxDialog}><Icon name="plus" size={16} />Novo</button>
                <button className="utx-view-toggle" disabled={!utxPackage} onClick={toggleUtxViewMode} title={utxViewMode === "list" ? "Alternar para visualização em galeria" : "Alternar para visualização em lista"}><Icon name={utxViewMode === "list" ? "grid" : "list"} size={15} /><span>{utxViewMode === "list" ? "Galeria" : "Lista"}</span></button>
                <label className="package-select">
                  <span>Pacote</span>
                  <select value={utxPackage ?? ""} disabled={!utxFilePath || busy} onChange={(event) => setUtxPackage(event.target.value || null)}><option value="" disabled>Selecionar…</option>{utxPackages.map((item) => <option key={item} value={item}>{item}</option>)}</select>
                </label>
                <label className="search-field"><Icon name="search" size={17} /><input value={utxQuery} disabled={!utxPackage} onChange={(event) => setUtxQuery(event.target.value)} placeholder="Filtrar por nome..." /></label>
                <div className="filters" role="group" aria-label="Filtrar formatos">
                  {["Todos", "RGBA8", "DXT"].map((item) => <button disabled={!utxPackage} onClick={() => setUtxFilter(item)} className={utxFilter === item ? "selected" : ""} key={item}>{item}</button>)}
                </div>
              </section>

              <section className={`content-panel utx-content-panel ${utxFilePath ? "has-content" : ""}`}>
                {utxFilePath ? (
                  utxViewMode === "gallery" ? (
                    utxPackage ? visibleUtxEntries.length > 0 ? (
                      <div className="utx-gallery-shell">
                        <div className="utx-gallery-grid">
                          {galleryUtxEntries.map((entry) => {
                            const preview = utxGalleryPreviews[entry.exportIndex];
                            const previewable = isPreviewableTexture(entry);
                            return (
                              <button className={`utx-gallery-card ${preview ? "has-preview" : ""} ${previewable ? "" : "unsupported"} ${utxGallerySelection.has(entry.exportIndex) ? "selected" : ""}`} key={`${entry.exportIndex}-${entry.name}`} disabled={busy} aria-pressed={utxGallerySelection.has(entry.exportIndex)} onClick={(event) => selectUtxGalleryEntry(entry, event.shiftKey, event.ctrlKey)} onContextMenu={(event) => { event.preventDefault(); openUtxGalleryContextMenu(entry, event.clientX, event.clientY); }} title="Clique seleciona ou desmarca uma textura; Ctrl alterna itens; Shift seleciona um intervalo. Botão direito para ações.">
                                <span className={`utx-gallery-art ${entry.hasAlpha ? "has-alpha" : "no-alpha"}`}>
                                  {preview ? <img src={preview.dataUrl} alt={entry.name} /> : <span className={`utx-gallery-placeholder ${previewable ? "loading" : "unsupported"}`}><Icon name="image" size={24} /><small>{previewable ? "Carregando…" : entry.format}</small></span>}
                                </span>
                                <span className="utx-gallery-copy"><strong>Texture {utxFileStem(entry.name)}</strong><small>{entry.width > 0 && entry.height > 0 ? `${entry.width} × ${entry.height}` : "Dimensões indisponíveis"} · {entry.format}{entry.hasSplit9 ? " · Split9" : ""}</small></span>
                              </button>
                            );
                          })}
                        </div>
                        <footer className="utx-gallery-pagination">
                          <span className="utx-gallery-page-status"><strong>Página {utxGalleryPage + 1} de {utxGalleryPageCount}</strong><small>{plural(visibleUtxEntries.length, "textura", "texturas")}{utxGallerySelectedCount > 0 && ` · ${plural(utxGallerySelectedCount, "selecionada", "selecionadas")}`}</small></span>
                          <div>
                            {utxGallerySelectedCount > 0 && <div className="utx-gallery-bulk-actions" onMouseDown={(event) => event.stopPropagation()}>
                              <button className="secondary-action utx-gallery-page-action" disabled={busy} onClick={() => setUtxGalleryActionsOpen((open) => !open)} aria-expanded={utxGalleryActionsOpen} aria-haspopup="menu"><Icon name="settings" size={15} />Ações ({utxGallerySelectedCount})</button>
                              {utxGalleryActionsOpen && <div className="utx-gallery-bulk-menu" role="menu" aria-label="Ações da seleção">
                                <button role="menuitem" disabled={busy} onClick={() => openUtxBatchProperties(selectedUtxGalleryEntries)}><Icon name="settings" size={16} />Propriedades</button>
                                <button role="menuitem" disabled={busy} onClick={() => void exportSelectedUtx()}><Icon name="download" size={16} />Exportar</button>
                                <button role="menuitem" disabled={busy} onClick={() => { setUtxGallerySelection(new Set()); setUtxGallerySelectionAnchor(null); setUtxGalleryActionsOpen(false); }}><Icon name="close" size={16} />Remover seleção</button>
                              </div>}
                            </div>}
                            <button className="secondary-action utx-gallery-page-action previous" disabled={utxGalleryPage === 0} onClick={() => setUtxGalleryPage((page) => Math.max(0, page - 1))}><Icon name="chevron" size={15} />Anterior</button>
                            <button className="secondary-action utx-gallery-page-action" disabled={utxGalleryPage >= utxGalleryPageCount - 1} onClick={() => setUtxGalleryPage((page) => Math.min(utxGalleryPageCount - 1, page + 1))}>Próxima<Icon name="chevron" size={15} /></button>
                          </div>
                        </footer>
                      </div>
                    ) : <div className="no-results">Nenhuma textura encontrada para este filtro.</div> : <div className="no-results">Selecione um pacote para carregar as texturas.</div>
                  ) : (
                  <>
                    <div className="list-header"><span aria-hidden="true" /><span>TEXTURA</span><span>FORMATO</span><span>AÇÕES</span></div>
                    <div className="entry-list">
                      {utxPackage && visibleUtxEntries.map((entry) => (
                        <article className="entry-row" key={`${entry.exportIndex}-${entry.name}`}>
                          <span className={`entry-preview ${isDxt(entry) ? "purple" : "cyan"}`}><Icon name="image" size={19} /></span>
                          <span className="entry-name"><strong>{entry.name}</strong><small>{entry.width > 0 && entry.height > 0 ? `${entry.width} × ${entry.height} px` : "Dimensões indisponíveis"}{entry.hasSplit9 ? " · Split9" : ""} · export #{entry.exportIndex + 1}</small></span>
                          <span className={`type-badge utx-badge ${entry.format.toLowerCase()}`}>{entry.format}</span>
                          <span className="entry-actions">
                            {isPreviewableTexture(entry) && <button className="row-action" disabled={busy} onClick={() => void openUtxProperties(entry)} title="Propriedades"><Icon name="settings" size={16} /></button>}
                            <button className="row-action" disabled={busy} onClick={() => void exportOneUtx(entry)} title="Exportar"><Icon name="download" size={16} /></button>
                            {isPreviewableTexture(entry) && <button className="row-action" disabled={busy} onClick={() => void showUtxPreview(entry)} title="Visualizar"><Icon name="eye" size={17} /></button>}
                            {isPreviewableTexture(entry) && <button className="row-action" disabled={busy} onClick={() => void replaceUtxEntry(entry)} title="Substituir"><Icon name="swap" size={17} /></button>}
                          </span>
                        </article>
                      ))}
                      {!utxPackage && <div className="no-results">Selecione um pacote para carregar as texturas.</div>}
                      {utxPackage && visibleUtxEntries.length === 0 && <div className="no-results">Nenhuma textura encontrada para este filtro.</div>}
                    </div>
                  </>
                  )
                ) : (
                  <div className="empty-state">
                    <div className="empty-orbit"><div className="empty-icon"><Icon name="image" size={34} /></div></div>
                    <h2>Suas texturas aparecem aqui</h2>
                    <p>Abra um arquivo UTX para listar, visualizar, exportar e substituir texturas com segurança.</p>
                    <button className="empty-action" onClick={() => void chooseUtxPackage()} disabled={busy}><Icon name="folder" size={17} />Abrir pacote UTX</button>
                    <div className="empty-note"><Icon name="info" size={14} />Compatível com pacotes Lineage 2 v111 e v121</div>
                  </div>
                )}
              </section>
              {utxFilePath && utxPackage && <div className="utx-footnote"><Icon name="info" size={14} />{utxStats.rgba8} RGBA8 · {utxStats.dxt} DXT · texturas com Split9 ou animação exportam um `.txt` complementar.</div>}
            </>
          ) : isUtxExtractPage ? (
            <section className="utx-extract-page">
              <section className="file-card utx-extract-file-card">
                <button className="file-drop" onClick={() => void chooseUtxExtractFiles()} disabled={busy}>
                  <span className="file-icon"><Icon name={utxExtractFiles.length ? "file" : "image"} size={22} /></span>
                  <span className="file-copy">
                    <strong>{utxExtractFiles.length ? plural(utxExtractFiles.length, "arquivo UTX selecionado", "arquivos UTX selecionados") : "Selecionar arquivos UTX"}</strong>
                    <small title={utxExtractFiles.join("\n")}>{utxExtractFiles.length === 0 ? "Selecione um ou mais pacotes .utx" : utxExtractFiles.length === 1 ? utxExtractFiles[0] : utxExtractFiles.map((path) => path.split(/[\\/]/).pop()).join(" · ")}</small>
                  </span>
                  <span className="browse-link">Procurar <Icon name="chevron" size={15} /></span>
                </button>
              </section>

              <section className="file-card utx-extract-file-card">
                <button className="file-drop" onClick={() => void chooseUtxExtractOutputDirectory()} disabled={busy}>
                  <span className="file-icon"><Icon name="folder" size={22} /></span>
                  <span className="file-copy">
                    <strong>Selecionar pasta de saída</strong>
                    <small>{utxExtractOutputDirectory ?? "Cada UTX será extraído em sua própria subpasta"}</small>
                  </span>
                  <span className="browse-link">Procurar <Icon name="chevron" size={15} /></span>
                </button>
              </section>

              <section className="utx-extract-controls">
                <div>
                  <span className="eyebrow">MODO DE EXPORTAÇÃO</span>
                  <h2>Extração em lote</h2>
                  <p>{utxExtractMode === "png" ? "Converte texturas compatíveis para PNG." : "Preserva o formato e os metadados originais."}</p>
                </div>
                <label className="utx-extract-mode">
                  <span>MODO</span>
                  <select value={utxExtractMode} disabled={busy} onChange={(event) => setUtxExtractMode(event.target.value as UtxExtractMode)}>
                    <option value="original">Original</option>
                    <option value="png">PNG</option>
                  </select>
                </label>
                <button className="primary-action utx-extract-run" disabled={utxExtractFiles.length === 0 || !utxExtractOutputDirectory || busy} onClick={() => void extractUtxPackages()}><Icon name="download" size={17} />Extrair</button>
              </section>

              <section className="utx-extract-result">
                {utxExtractSummary ? (
                  <>
                    <header><div><span className="eyebrow"><span className="pulse" />EXTRAÇÃO CONCLUÍDA</span><h2>{plural(utxExtractSummary.exported, "textura extraída", "texturas extraídas")}</h2><p>{plural(utxExtractSummary.packages, "pacote processado", "pacotes processados")}{utxExtractElapsedMs !== null ? ` · ${formatDuration(utxExtractElapsedMs)}` : ""}{utxExtractSummary.skipped ? ` · ${plural(utxExtractSummary.skipped, "item ignorado", "itens ignorados")}` : ""}{utxExtractSummary.failed ? ` · ${plural(utxExtractSummary.failed, "falha", "falhas")}` : ""}</p></div></header>
                    <div className="utx-extract-output"><Icon name="folder" size={17} /><div><span>SAÍDA</span><strong title={utxExtractSummary.outputDirectory}>{utxExtractSummary.outputDirectory}</strong></div></div>
                    {utxExtractSummary.errors.length > 0 && <div className="utx-extract-errors"><strong>Itens que precisam de atenção</strong>{utxExtractSummary.errors.map((error) => <p key={error}>{error}</p>)}</div>}
                  </>
                ) : (
                  <div className="empty-state utx-extract-empty">
                    <div className="empty-orbit"><div className="empty-icon"><Icon name="download" size={33} /></div></div>
                    <h2>Texturas extraídas aparecem aqui</h2>
                    <p>Selecione um ou mais arquivos UTX, defina a pasta de saída e escolha entre preservar o formato original ou converter para PNG.</p>
                  </div>
                )}
              </section>
            </section>
          ) : isTextureResizePage ? (
            <section className="texture-resize-page">
              <section className="file-card texture-resize-directory">
                <button className="file-drop" onClick={() => void chooseTextureResizeDirectory()} disabled={busy}>
                  <span className="file-icon"><Icon name={textureResizeDirectory ? "folder" : "resize"} size={22} /></span>
                  <span className="file-copy">
                    <strong>{textureResizeDirectory ? textureResizeDirectory.split(/[\\/]/).filter(Boolean).pop() : "Selecionar pasta de ícones"}</strong>
                    <small>{textureResizeDirectory ?? "A leitura inclui todas as subpastas com arquivos DDS ou TGA"}</small>
                  </span>
                  <span className="browse-link">Procurar <Icon name="chevron" size={15} /></span>
                </button>
              </section>

              <section className="texture-resize-controls">
                <div>
                  <span className="eyebrow">DIMENSÕES DE CONVERSÃO</span>
                  <h2>Converte o tamanho do documento</h2>
                  <p>Somente texturas de {textureResizeSourceResolution} × {textureResizeSourceResolution} px serão convertidas para {textureResizeTargetResolution} × {textureResizeTargetResolution} px. As demais serão copiadas.</p>
                </div>
                <label className="texture-resize-resolution">
                  <span>DE</span>
                  <select value={textureResizeSourceResolution} disabled={busy} onChange={(event) => setTextureResizeSourceResolution(event.target.value)}>
                    {UE2_TEXTURE_RESOLUTIONS.map((resolution) => <option value={resolution} key={resolution}>{resolution} × {resolution} px</option>)}
                  </select>
                </label>
                <label className="texture-resize-resolution">
                  <span>PARA</span>
                  <select value={textureResizeTargetResolution} disabled={busy} onChange={(event) => setTextureResizeTargetResolution(event.target.value)}>
                    {UE2_TEXTURE_RESOLUTIONS.map((resolution) => <option value={resolution} key={resolution}>{resolution} × {resolution} px</option>)}
                  </select>
                </label>
                <button className="primary-action texture-resize-run" disabled={!textureResizeDirectory || busy} onClick={() => void resizeTextureDirectory()}><Icon name="resize" size={17} />Redimensionar</button>
              </section>

              <section className="texture-resize-result">
                {textureResizeSummary ? (
                  <>
                    <header>
                      <div><span className="eyebrow"><span className="pulse" />CONVERSÃO CONCLUÍDA</span><h2>{plural(textureResizeSummary.totalFiles, "ícone processado", "ícones processados")}</h2><p>{plural(textureResizeSummary.resizedFiles, "arquivo redimensionado", "arquivos redimensionados")} · {plural(textureResizeSummary.preservedFiles, "arquivo copiado", "arquivos copiados")}{textureResizeSummary.copiedMetadata ? ` · ${plural(textureResizeSummary.copiedMetadata, "metadado atualizado", "metadados atualizados")}` : ""}</p></div>
                    </header>
                    <div className="texture-resize-output"><Icon name="folder" size={17} /><div><span>SAÍDA</span><strong title={textureResizeSummary.outputDirectory}>{textureResizeSummary.outputDirectory}</strong></div></div>
                    {textureResizeSummary.failedFiles > 0 && <div className="texture-resize-errors"><strong>{plural(textureResizeSummary.failedFiles, "arquivo não pôde ser convertido", "arquivos não puderam ser convertidos")}</strong>{textureResizeSummary.errors.map((error) => <p key={error}>{error}</p>)}</div>}
                  </>
                ) : (
                  <div className="empty-state texture-resize-empty">
                    <div className="empty-orbit"><div className="empty-icon"><Icon name="resize" size={33} /></div></div>
                    <h2>Ícones convertidos aparecem aqui</h2>
                    <p>Escolha a pasta e as dimensões de origem e destino. Somente o tamanho escolhido será convertido; os originais serão preservados.</p>
                    {textureResizeOutputHint && <div className="empty-note"><Icon name="folder" size={14} /><span title={textureResizeOutputHint}>Saída: {textureResizeOutputHint}</span></div>}
                  </div>
                )}
              </section>
            </section>
          ) : isGeodataPage ? (
            <section className="geodata-page">
              <section className="file-card geodata-directory">
                <button className="file-drop" onClick={() => void chooseGeodataInputDirectory()} disabled={busy}>
                  <span className="file-icon"><Icon name={geodataInputDirectory ? "folder" : "refresh"} size={22} /></span>
                  <span className="file-copy">
                    <strong>{geodataInputDirectory ? geodataInputDirectory.split(/[\\/]/).filter(Boolean).pop() : "Selecionar pasta de entrada"}</strong>
                    <small>{geodataInputDirectory ?? "A leitura inclui todas as subpastas com geodata suportada"}</small>
                  </span>
                  <span className="browse-link">Procurar <Icon name="chevron" size={15} /></span>
                </button>
              </section>

              <section className="file-card geodata-directory">
                <button className="file-drop" onClick={() => void chooseGeodataOutputDirectory()} disabled={busy}>
                  <span className="file-icon"><Icon name="folder" size={22} /></span>
                  <span className="file-copy">
                    <strong>Selecionar pasta de saída</strong>
                    <small>{geodataOutputDirectory ?? "A árvore de subpastas será preservada na conversão"}</small>
                  </span>
                  <span className="browse-link">Procurar <Icon name="chevron" size={15} /></span>
                </button>
              </section>

              <section className="geodata-controls">
                <div>
                  <span className="eyebrow">FORMATO DE SAÍDA</span>
                  <h2>Conversão de regiões</h2>
                  <p>Lê L2J, CONV_DAT, L2D, L2S, L2G, L2M, RP e PathTxt. Arquivos que já usam o formato escolhido são copiados.</p>
                </div>
                <label className="geodata-format">
                  <span>CONVERTER PARA</span>
                  <select value={geodataOutputFormat} disabled={busy} onChange={(event) => setGeodataOutputFormat(event.target.value as GeodataOutputFormat)}>
                    <option value="l2j">L2J (.l2j)</option>
                    <option value="convDat">CONV_DAT (_conv.dat)</option>
                    <option value="l2g">L2G (.l2g)</option>
                  </select>
                </label>
                <button className="primary-action geodata-run" disabled={!geodataInputDirectory || !geodataOutputDirectory || busy} onClick={() => void convertGeodataDirectory()}><Icon name="refresh" size={17} />Converter</button>
              </section>

              <section className="geodata-result">
                {geodataSummary ? (
                  <>
                    <header><div><span className="eyebrow"><span className="pulse" />CONVERSÃO CONCLUÍDA</span><h2>{plural(geodataSummary.totalFiles, "região processada", "regiões processadas")}</h2><p>{plural(geodataSummary.convertedFiles, "arquivo convertido", "arquivos convertidos")}{geodataSummary.copiedFiles ? ` · ${plural(geodataSummary.copiedFiles, "arquivo copiado", "arquivos copiados")}` : ""}{geodataSummary.skippedFiles ? ` · ${plural(geodataSummary.skippedFiles, "item ignorado", "itens ignorados")}` : ""}{geodataSummary.failedFiles ? ` · ${plural(geodataSummary.failedFiles, "falha", "falhas")}` : ""} · {plural(geodataSummary.workers, "worker usado", "workers usados")}</p></div></header>
                    <div className="geodata-output"><Icon name="folder" size={17} /><div><span>SAÍDA</span><strong title={geodataSummary.outputDirectory}>{geodataSummary.outputDirectory}</strong></div></div>
                    {geodataSummary.errors.length > 0 && <div className="geodata-errors"><strong>Itens que precisam de atenção</strong>{geodataSummary.errors.map((error) => <p key={error}>{error}</p>)}</div>}
                  </>
                ) : (
                  <div className="empty-state geodata-empty">
                    <div className="empty-orbit"><div className="empty-icon"><Icon name="refresh" size={33} /></div></div>
                    <h2>Regiões convertidas aparecem aqui</h2>
                    <p>Escolha as pastas de entrada e saída e defina o formato para converter os arquivos de geodata em lote.</p>
                  </div>
                )}
              </section>
            </section>
          ) : isSettingsPage ? (
            <section className="settings-page">
              <section className="settings-card">
                <header className="settings-card-header">
                  <span className="settings-card-icon"><Icon name="settings" size={21} /></span>
                  <div>
                    <h2>Aparência</h2>
                    <p>Escolha o tema usado em todas as áreas do Unreal Tools Lite.</p>
                  </div>
                </header>
                <div className="theme-options" role="radiogroup" aria-label="Tema do aplicativo">
                  <button className={`theme-option ${theme === "dark" ? "selected" : ""}`} role="radio" aria-checked={theme === "dark"} onClick={() => setTheme("dark")}>
                    <span className="theme-preview dark-preview"><i /><i /><i /></span>
                    <span className="theme-option-copy"><strong><Icon name="moon" size={17} />Dark</strong><small>Tema escuro para ambientes com pouca luz.</small></span>
                    <span className="theme-check">{theme === "dark" && "✓"}</span>
                  </button>
                  <button className={`theme-option ${theme === "light" ? "selected" : ""}`} role="radio" aria-checked={theme === "light"} onClick={() => setTheme("light")}>
                    <span className="theme-preview light-preview"><i /><i /><i /></span>
                    <span className="theme-option-copy"><strong><Icon name="sun" size={17} />Light</strong><small>Tema claro padrão, com sidebar escura para melhor foco.</small></span>
                    <span className="theme-check">{theme === "light" && "✓"}</span>
                  </button>
                </div>
                <footer className="settings-card-footer"><Icon name="info" size={15} />A preferência é salva automaticamente neste computador.</footer>
              </section>
              <section className="settings-card settings-storage-card">
                <header className="settings-card-header">
                  <span className="settings-card-icon"><Icon name="folder" size={21} /></span>
                  <div>
                    <h2>Dados do aplicativo</h2>
                    <p>Preferências, caminhos recentes e abas abertas são compartilhados entre as versões de desenvolvimento e produção.</p>
                  </div>
                </header>
                <div className="settings-storage-body">
                  <div>
                    <strong>Configurações locais</strong>
                    <small>Arquivo <code>settings.json</code> salvo na pasta AppData do Unreal Tools Lite.</small>
                  </div>
                  <button className="secondary-action settings-storage-open" onClick={() => void openAppSettingsDirectory()}><Icon name="folder" size={17} />Abrir pasta de configurações</button>
                </div>
                <div className="settings-storage-body">
                  <div>
                    <strong>Logs de operação</strong>
                    <small>Relatórios de importação e diagnóstico salvos temporariamente neste computador.</small>
                  </div>
                  <button className="secondary-action settings-storage-open" onClick={() => void openAppLogsDirectory()}><Icon name="folder" size={17} />Abrir pasta de logs</button>
                </div>
              </section>
            </section>
          ) : (
            <section className="coming-soon">
              <div className="coming-icon"><Icon name={activeItem.icon} size={31} /></div>
              <div><span className="eyebrow">EM CONSTRUÇÃO</span><h2>{activeItem.label}</h2><p>O shell e os componentes desta área já fazem parte da nova experiência Tauri.</p></div>
            </section>
          )}
        </div>

        <footer className="statusbar"><span>{busy ? isUtxExtractPage ? "Extraindo texturas…" : isTextureResizePage ? "Redimensionando ícones…" : isGeodataPage ? "Convertendo geodata…" : "Processando pacote…" : "© 2026 Unreal Tools Lite - By Mk"}</span>{appVersion && <span className="footer-version">v{appVersion}</span>}</footer>
      </section>
      {toast && <div className={`toast ${toast.isError ? "error" : toast.isInfo ? "info" : ""}`} role="status"><span className="toast-dot" />{toast.message}<button onClick={dismissToast} aria-label="Fechar aviso"><Icon name="close" size={15} /></button></div>}
      {utxGalleryContextMenu && <div className="utx-gallery-context-menu" role="menu" aria-label={`Ações para ${utxGalleryContextMenu.entry.name}`} style={{ left: utxGalleryContextMenu.x, top: utxGalleryContextMenu.y }} onMouseDown={(event) => event.stopPropagation()}>
        <span className="utx-gallery-context-title" title={utxGalleryContextMenu.entry.name}>{utxGalleryContextMenu.entry.name}</span>
        <button role="menuitem" disabled={busy || !isPreviewableTexture(utxGalleryContextMenu.entry)} onClick={() => { setUtxGalleryContextMenu(null); void showUtxPreview(utxGalleryContextMenu.entry); }}><Icon name="eye" size={16} />Preview</button>
        <button role="menuitem" disabled={busy} onClick={() => void openUtxProperties(utxGalleryContextMenu.entry)}><Icon name="settings" size={16} />Propriedades</button>
        <button role="menuitem" disabled={busy} onClick={() => openUtxDuplicate(utxGalleryContextMenu.entry)}><Icon name="copy" size={16} />Duplicar</button>
        <button role="menuitem" disabled={busy} onClick={() => openUtxRename(utxGalleryContextMenu.entry)}><Icon name="file" size={16} />Renomear</button>
        <button role="menuitem" onClick={() => { setUtxGalleryContextMenu(null); void copyUtxTexturePath(utxGalleryContextMenu.entry); }}><Icon name="copy" size={16} />Copiar path</button>
        <button role="menuitem" disabled={busy} onClick={() => { setUtxGalleryContextMenu(null); void exportOneUtx(utxGalleryContextMenu.entry); }}><Icon name="download" size={16} />Exportar</button>
        <button role="menuitem" disabled={busy || !isPreviewableTexture(utxGalleryContextMenu.entry)} onClick={() => { setUtxGalleryContextMenu(null); void replaceUtxEntry(utxGalleryContextMenu.entry); }}><Icon name="swap" size={16} />Substituir</button>
      </div>}
      {utxExportScopeDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => !busy && setUtxExportScopeDialog(false)}>
        <section className="utx-import-modal" role="dialog" aria-modal="true" aria-label="Escolher escopo da exportação UTX" onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">EXPORTAÇÃO UTX</span><h2>O que deseja exportar?</h2></div><button className="row-action" onClick={() => setUtxExportScopeDialog(false)} disabled={busy} aria-label="Cancelar exportação"><Icon name="close" size={18} /></button></header>
          <p>Grupo atual mantém os filtros de formato ativos. UTX inteiro exporta todas as texturas, em todos os grupos.</p>
          <div className="utx-export-scope-options">
            <button className="utx-export-scope-option" disabled={busy || utxEntriesForBulkExport.length === 0} onClick={() => void exportUtxScope("group")}><Icon name="folder" size={19} /><span><strong>Grupo atual</strong><small>{utxPackage ?? "Nenhum grupo selecionado"} · {plural(utxEntriesForBulkExport.length, "textura", "texturas")}</small></span><Icon name="chevron" size={16} /></button>
            <button className="utx-export-scope-option" disabled={busy || utxEntries.length === 0} onClick={() => void exportUtxScope("all")}><Icon name="image" size={19} /><span><strong>UTX inteiro</strong><small>{plural(utxEntries.length, "textura", "texturas")} em {plural(utxPackages.length, "grupo", "grupos")}</small></span><Icon name="chevron" size={16} /></button>
          </div>
          <footer><button className="secondary-action" onClick={() => setUtxExportScopeDialog(false)} disabled={busy}>Cancelar</button></footer>
        </section>
      </div>}
      {utxDuplicateDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => !busy && setUtxDuplicateDialog(null)}>
        <section className="utx-import-modal utx-duplicate-modal" role="dialog" aria-modal="true" aria-label={`Duplicar ${utxDuplicateDialog.source.name}`} onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">DUPLICAR TEXTURA</span><h2>{utxDuplicateDialog.source.name}</h2></div><button className="row-action" onClick={() => setUtxDuplicateDialog(null)} disabled={busy} aria-label="Cancelar duplicação"><Icon name="close" size={18} /></button></header>
          <p>Crie uma cópia idêntica da textura. O Package é mantido; escolha o Grupo e o novo Nome.</p>
          <label className="utx-import-group-field"><span>PACKAGE</span><input value={utxFilePath ? fileStem(utxFilePath) : "Pacote atual"} readOnly /></label>
          <label className="utx-import-group-field"><span>GRUPO</span><input autoFocus list="utx-duplicate-groups" value={utxDuplicateDialog.group} onChange={(event) => setUtxDuplicateDialog((current) => current ? { ...current, group: event.target.value } : current)} placeholder="Ex.: CandidateWnd" disabled={busy} /></label>
          <datalist id="utx-duplicate-groups">{utxPackages.map((item) => <option key={item} value={item} />)}</datalist>
          <label className="utx-import-group-field"><span>NOME</span><input value={utxDuplicateDialog.name} onChange={(event) => setUtxDuplicateDialog((current) => current ? { ...current, name: event.target.value } : current)} placeholder="Ex.: ButtonCopy" disabled={busy} /></label>
          <small className={`utx-import-group-note ${utxDuplicateNameInUse ? "error" : ""}`}>{utxDuplicateNameInUse ? "Esse nome já existe no grupo informado." : utxPackages.includes(utxDuplicateDialog.group.trim()) ? "O grupo existente receberá a cópia." : "Um novo grupo será criado para a cópia."}</small>
          <footer><button className="secondary-action" onClick={() => setUtxDuplicateDialog(null)} disabled={busy}>Cancelar</button><button className="primary-action" disabled={busy || !utxDuplicateDialog.group.trim() || !utxDuplicateDialog.name.trim() || utxDuplicateNameInUse} onClick={() => void duplicateUtxTexture()}><Icon name="copy" size={16} />Duplicar</button></footer>
        </section>
      </div>}
      {utxRenameDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => !busy && setUtxRenameDialog(null)}>
        <section className="utx-import-modal utx-rename-modal" role="dialog" aria-modal="true" aria-label={`Renomear ${utxRenameDialog.source.name}`} onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">RENOMEAR TEXTURA</span><h2>{utxRenameDialog.source.name}</h2></div><button className="row-action" onClick={() => setUtxRenameDialog(null)} disabled={busy} aria-label="Cancelar renomeação"><Icon name="close" size={18} /></button></header>
          <p>O nome será atualizado sem alterar a textura, o grupo ou suas referências internas.</p>
          <label className="utx-import-group-field"><span>PACKAGE</span><input value={utxFilePath ? fileStem(utxFilePath) : "Pacote atual"} readOnly /></label>
          <label className="utx-import-group-field"><span>GRUPO</span><input value={packageNameFor(utxRenameDialog.source)} readOnly /></label>
          <label className="utx-import-group-field"><span>NOVO NOME</span><input autoFocus value={utxRenameDialog.name} onChange={(event) => setUtxRenameDialog((current) => current ? { ...current, name: event.target.value } : current)} placeholder="Ex.: ButtonCopy" disabled={busy} /></label>
          <small className={`utx-import-group-note ${utxRenameNameInUse ? "error" : ""}`}>{utxRenameNameInUse ? "Esse nome já existe no grupo atual." : "O novo nome está disponível neste grupo."}</small>
          <footer><button className="secondary-action" onClick={() => setUtxRenameDialog(null)} disabled={busy}>Cancelar</button><button className="primary-action" disabled={busy || !utxRenameDialog.name.trim() || utxRenameNameInUse} onClick={() => void renameUtxTexture()}><Icon name="file" size={16} />Renomear</button></footer>
        </section>
      </div>}
      {utxNewDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => setUtxNewDialog(false)}>
        <section className="utx-import-modal utx-new-modal" role="dialog" aria-modal="true" aria-label="Criar novo pacote UTX" onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">NOVO UTX</span><h2>Criar pacote de texturas</h2></div><button className="row-action" onClick={() => setUtxNewDialog(false)} aria-label="Cancelar criação"><Icon name="close" size={18} /></button></header>
          <p>O arquivo será criado a partir do modelo interno do Unreal Editor e aberto em seguida para começar a importar texturas.</p>
          <label className="utx-import-group-field"><span>NOME DO PACOTE</span><input autoFocus value={utxNewName} onChange={(event) => setUtxNewName(event.target.value)} placeholder="Ex.: L2UI_Custom" /></label>
          <label className="utx-new-directory-field"><span>LOCAL DE SALVAMENTO</span><div><small title={utxNewDirectory ?? undefined}>{utxNewDirectory ?? "Selecione uma pasta para salvar o pacote"}</small><button className="secondary-action" onClick={() => void chooseNewUtxDirectory()} disabled={busy}><Icon name="folder" size={16} />Escolher pasta</button></div></label>
          <small className="utx-import-group-note">O nome interno do Package será igual ao nome do arquivo. Use apenas letras, números e sublinhado.</small>
          <footer><button className="secondary-action" onClick={() => setUtxNewDialog(false)} disabled={busy}>Cancelar</button><button className="primary-action" disabled={!utxNewName.trim() || !utxNewDirectory || busy} onClick={() => void createNewUtx()}><Icon name="plus" size={17} />Criar e abrir</button></footer>
        </section>
      </div>}
      {utxImportSourceDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => setUtxImportSourceDialog(false)}>
        <section className="utx-import-modal" role="dialog" aria-modal="true" aria-label="Escolher origem da importação UTX" onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">IMPORTAÇÃO UTX</span><h2>Como deseja importar?</h2></div><button className="row-action" onClick={() => setUtxImportSourceDialog(false)} aria-label="Cancelar importação"><Icon name="close" size={18} /></button></header>
          <p>Importe arquivos escolhidos manualmente ou use uma pasta exportada para restaurar seus grupos automaticamente.</p>
          <div className="utx-export-scope-options">
            <button className="utx-export-scope-option" onClick={() => void chooseUtxImportFiles()}><Icon name="image" size={19} /><span><strong>Selecionar arquivos</strong><small>Escolha texturas .tga ou .dds e defina o grupo de destino.</small></span><Icon name="chevron" size={16} /></button>
            <button className="utx-export-scope-option" onClick={() => void chooseUtxImportDirectory()}><Icon name="folder" size={19} /><span><strong>Importar uma pasta</strong><small>Raiz: Pacote principal. Cada subpasta: um grupo criado automaticamente.</small></span><Icon name="chevron" size={16} /></button>
          </div>
          <footer><button className="secondary-action" onClick={() => setUtxImportSourceDialog(false)}>Cancelar</button></footer>
        </section>
      </div>}
      {utxImportFiles && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => setUtxImportFiles(null)}>
        <section className="utx-import-modal" role="dialog" aria-modal="true" aria-label="Definir grupo de importação" onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">IMPORTAÇÃO UTX</span><h2>Escolha o grupo de destino</h2></div><button className="row-action" onClick={() => setUtxImportFiles(null)} aria-label="Cancelar importação"><Icon name="close" size={18} /></button></header>
          <p>{utxImportFiles.length} textura(s) selecionada(s). Use um grupo existente ou informe um nome para criar outro.</p>
          <label className="utx-import-group-field"><span>GRUPO</span><input autoFocus value={utxImportGroup} onChange={(event) => setUtxImportGroup(event.target.value)} placeholder="Ex.: CandidateWnd" /></label>
          <label className="utx-import-group-picker"><span>GRUPOS EXISTENTES</span><select value="" onChange={(event) => event.target.value && setUtxImportGroup(event.target.value)}><option value="">Selecionar da lista…</option>{utxPackages.map((item) => <option key={item} value={item}>{item}</option>)}</select></label>
          <small className="utx-import-group-note">{utxPackages.includes(utxImportGroup.trim()) ? "O grupo já existe: texturas com o mesmo nome serão substituídas." : "O grupo será criado e receberá as novas texturas."}</small>
          <footer><button className="secondary-action" onClick={() => setUtxImportFiles(null)}>Cancelar</button><button className="primary-action" disabled={!utxImportGroup.trim()} onClick={() => void confirmUtxImport()}><Icon name="upload" size={17} />Importar</button></footer>
        </section>
      </div>}
      {utxPropertiesDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => !busy && setUtxPropertiesDialog(null)}>
        <section className="utx-import-modal utx-properties-modal" role="dialog" aria-modal="true" aria-label={`Propriedades de ${utxPropertiesDialog.entry.name}`} onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">PROPRIEDADES DA TEXTURA</span><h2>{utxPropertiesDialog.entry.name}</h2><small>{utxPropertiesDialog.entry.width} × {utxPropertiesDialog.entry.height} px · {utxPropertiesDialog.entry.format}</small></div><button className="row-action" onClick={() => setUtxPropertiesDialog(null)} disabled={busy} aria-label="Fechar propriedades"><Icon name="close" size={18} /></button></header>
          {utxPropertiesDialog.loading ? <div className="utx-properties-loading"><Icon name="refresh" size={20} />Lendo propriedades da textura…</div> : <>
            <div className="utx-properties-body">
              <section className="utx-properties-section">
                <div><span className="eyebrow">SUPERFÍCIE</span><h3>Alpha e máscara</h3></div>
                <div className="utx-properties-switches">
                  <label><input type="checkbox" checked={utxPropertiesDialog.form.alpha} onChange={(event) => updateUtxPropertyForm({ alpha: event.target.checked })} disabled={busy} /><span>Alpha texture</span></label>
                  <label><input type="checkbox" checked={utxPropertiesDialog.form.masked} onChange={(event) => updateUtxPropertyForm({ masked: event.target.checked })} disabled={busy} /><span>Masked</span></label>
                </div>
              </section>
              <section className="utx-properties-section">
                <div><span className="eyebrow">ENDEREÇAMENTO</span><h3>Clamp</h3></div>
                <div className="utx-properties-grid">
                  <label><span>U Clamp</span><input type="number" value={utxPropertiesDialog.form.uClamp} onChange={(event) => updateUtxPropertyForm({ uClamp: event.target.value })} placeholder="Herdar" disabled={busy} /></label>
                  <label><span>V Clamp</span><input type="number" value={utxPropertiesDialog.form.vClamp} onChange={(event) => updateUtxPropertyForm({ vClamp: event.target.value })} placeholder="Herdar" disabled={busy} /></label>
                  <label><span>U Clamp mode</span><input type="number" value={utxPropertiesDialog.form.uClampMode} onChange={(event) => updateUtxPropertyForm({ uClampMode: event.target.value })} placeholder="Herdar" disabled={busy} /></label>
                  <label><span>V Clamp mode</span><input type="number" value={utxPropertiesDialog.form.vClampMode} onChange={(event) => updateUtxPropertyForm({ vClampMode: event.target.value })} placeholder="Herdar" disabled={busy} /></label>
                </div>
              </section>
              <section className="utx-properties-section">
                <div className="utx-properties-section-heading"><div><span className="eyebrow">SPLIT9</span><h3>Bordas escaláveis</h3></div><label className="utx-properties-toggle"><input type="checkbox" checked={utxPropertiesDialog.form.split9Enabled} onChange={(event) => updateUtxPropertyForm({ split9Enabled: event.target.checked })} disabled={busy} /><span>Ativar</span></label></div>
                <div className="utx-properties-split-grid" aria-disabled={!utxPropertiesDialog.form.split9Enabled}>
                  <label><span>X1</span><input type="number" value={utxPropertiesDialog.form.split9X1} onChange={(event) => updateUtxPropertyForm({ split9X1: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                  <label><span>X2</span><input type="number" value={utxPropertiesDialog.form.split9X2} onChange={(event) => updateUtxPropertyForm({ split9X2: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                  <label><span>X3</span><input type="number" value={utxPropertiesDialog.form.split9X3} onChange={(event) => updateUtxPropertyForm({ split9X3: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                  <label><span>Y1</span><input type="number" value={utxPropertiesDialog.form.split9Y1} onChange={(event) => updateUtxPropertyForm({ split9Y1: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                  <label><span>Y2</span><input type="number" value={utxPropertiesDialog.form.split9Y2} onChange={(event) => updateUtxPropertyForm({ split9Y2: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                  <label><span>Y3</span><input type="number" value={utxPropertiesDialog.form.split9Y3} onChange={(event) => updateUtxPropertyForm({ split9Y3: event.target.value })} disabled={busy || !utxPropertiesDialog.form.split9Enabled} /></label>
                </div>
              </section>
              <section className="utx-properties-section">
                <div className="utx-properties-section-heading"><div><span className="eyebrow">ANIMAÇÃO</span><h3>Sequência de frames</h3></div><label className="utx-properties-toggle"><input type="checkbox" checked={utxPropertiesDialog.form.animationEnabled} onChange={(event) => updateUtxPropertyForm({ animationEnabled: event.target.checked })} disabled={busy} /><span>Ativar</span></label></div>
                <label className="utx-properties-wide-field"><span>AnimNext</span><input list="utx-animation-targets" value={utxPropertiesDialog.form.animNext} onChange={(event) => updateUtxPropertyForm({ animNext: event.target.value })} placeholder="Selecionar próxima textura…" disabled={busy || !utxPropertiesDialog.form.animationEnabled} /></label>
                <datalist id="utx-animation-targets">{utxEntries.map((entry) => <option key={entry.exportIndex} value={entry.name}>{entry.name}</option>)}</datalist>
                <div className="utx-properties-grid">
                  <label><span>Max frame rate</span><input type="number" step="any" value={utxPropertiesDialog.form.maxFrameRate} onChange={(event) => updateUtxPropertyForm({ maxFrameRate: event.target.value })} disabled={busy || !utxPropertiesDialog.form.animationEnabled} /></label>
                  <label><span>Min frame rate</span><input type="number" step="any" value={utxPropertiesDialog.form.minFrameRate} onChange={(event) => updateUtxPropertyForm({ minFrameRate: event.target.value })} disabled={busy || !utxPropertiesDialog.form.animationEnabled} /></label>
                  <label><span>Prime count</span><input type="number" value={utxPropertiesDialog.form.primeCount} onChange={(event) => updateUtxPropertyForm({ primeCount: event.target.value })} disabled={busy || !utxPropertiesDialog.form.animationEnabled} /></label>
                  <label><span>Total frame num</span><input type="number" value={utxPropertiesDialog.form.totalFrameNum} onChange={(event) => updateUtxPropertyForm({ totalFrameNum: event.target.value })} disabled={busy || !utxPropertiesDialog.form.animationEnabled} /></label>
                </div>
                <label className="utx-properties-toggle utx-properties-loop"><input type="checkbox" checked={utxPropertiesDialog.form.oneTimeAnimLoop} onChange={(event) => updateUtxPropertyForm({ oneTimeAnimLoop: event.target.checked })} disabled={busy || !utxPropertiesDialog.form.animationEnabled} /><span>One time animation loop</span></label>
              </section>
            </div>
            <footer><button className="secondary-action" onClick={() => setUtxPropertiesDialog(null)} disabled={busy}>Cancelar</button><button className="primary-action" onClick={() => void saveUtxProperties()} disabled={busy}><Icon name="settings" size={16} />Salvar propriedades</button></footer>
          </>}
        </section>
      </div>}
      {utxBatchPropertiesDialog && <div className="utx-import-backdrop" role="presentation" onMouseDown={() => !busy && setUtxBatchPropertiesDialog(null)}>
        <section className="utx-import-modal utx-properties-modal utx-batch-properties-modal" role="dialog" aria-modal="true" aria-label="Propriedades das texturas selecionadas" onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">PROPRIEDADES EM LOTE</span><h2>{plural(utxBatchPropertiesDialog.entries.length, "textura selecionada", "texturas selecionadas")}</h2><small>Os valores escolhidos serão aplicados a toda a seleção.</small></div><button className="row-action" onClick={() => setUtxBatchPropertiesDialog(null)} disabled={busy} aria-label="Fechar propriedades em lote"><Icon name="close" size={18} /></button></header>
          <div className="utx-properties-body">
            <p className="utx-batch-properties-note"><Icon name="info" size={15} />Use “Não alterar” para preservar o valor atual de cada textura.</p>
            <section className="utx-properties-section">
              <div><span className="eyebrow">SUPERFÍCIE</span><h3>Alpha e máscara</h3></div>
              <div className="utx-properties-grid utx-batch-properties-grid">
                <label><span>Alpha texture</span><select value={utxBatchPropertiesDialog.form.alpha} onChange={(event) => updateUtxBatchPropertyForm({ alpha: event.target.value as UtxBatchPropertyChoice })} disabled={busy}><option value="keep">Não alterar</option><option value="enabled">Ativar</option><option value="disabled">Desativar</option></select></label>
                <label><span>Masked</span><select value={utxBatchPropertiesDialog.form.masked} onChange={(event) => updateUtxBatchPropertyForm({ masked: event.target.value as UtxBatchPropertyChoice })} disabled={busy}><option value="keep">Não alterar</option><option value="enabled">Ativar</option><option value="disabled">Desativar</option></select></label>
              </div>
            </section>
            <section className="utx-properties-section">
              <div className="utx-properties-section-heading"><div><span className="eyebrow">SPLIT9</span><h3>Bordas escaláveis</h3></div><label className="utx-properties-toggle"><input type="checkbox" checked={utxBatchPropertiesDialog.form.updateSplit9} onChange={(event) => updateUtxBatchPropertyForm({ updateSplit9: event.target.checked })} disabled={busy} /><span>Atualizar</span></label></div>
              <p className="utx-batch-properties-help">Ao atualizar, esta configuração substitui o Split9 de todas as texturas selecionadas.</p>
              <div className="utx-properties-section-heading utx-batch-split9-toggle"><span>Ativar Split9</span><label className="utx-properties-toggle"><input type="checkbox" checked={utxBatchPropertiesDialog.form.split9Enabled} onChange={(event) => updateUtxBatchPropertyForm({ split9Enabled: event.target.checked })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9} /><span>{utxBatchPropertiesDialog.form.split9Enabled ? "Sim" : "Não"}</span></label></div>
              <div className="utx-properties-split-grid" aria-disabled={!utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled}>
                <label><span>X1</span><input type="number" value={utxBatchPropertiesDialog.form.split9X1} onChange={(event) => updateUtxBatchPropertyForm({ split9X1: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
                <label><span>X2</span><input type="number" value={utxBatchPropertiesDialog.form.split9X2} onChange={(event) => updateUtxBatchPropertyForm({ split9X2: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
                <label><span>X3</span><input type="number" value={utxBatchPropertiesDialog.form.split9X3} onChange={(event) => updateUtxBatchPropertyForm({ split9X3: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
                <label><span>Y1</span><input type="number" value={utxBatchPropertiesDialog.form.split9Y1} onChange={(event) => updateUtxBatchPropertyForm({ split9Y1: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
                <label><span>Y2</span><input type="number" value={utxBatchPropertiesDialog.form.split9Y2} onChange={(event) => updateUtxBatchPropertyForm({ split9Y2: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
                <label><span>Y3</span><input type="number" value={utxBatchPropertiesDialog.form.split9Y3} onChange={(event) => updateUtxBatchPropertyForm({ split9Y3: event.target.value })} disabled={busy || !utxBatchPropertiesDialog.form.updateSplit9 || !utxBatchPropertiesDialog.form.split9Enabled} /></label>
              </div>
            </section>
          </div>
          <footer><button className="secondary-action" onClick={() => setUtxBatchPropertiesDialog(null)} disabled={busy}>Cancelar</button><button className="primary-action" onClick={() => void saveUtxBatchProperties()} disabled={busy}><Icon name="settings" size={16} />Aplicar a {utxBatchPropertiesDialog.entries.length}</button></footer>
        </section>
      </div>}
      {utxImportProgress && <div className="import-progress-backdrop" role="presentation">
        <section className="import-progress-modal" role="dialog" aria-modal="true" aria-label="Importando texturas UTX">
          <header>
            <div><span className="eyebrow">IMPORTAÇÃO UTX</span><h2>Processando texturas</h2></div>
            <strong>{utxImportPercent}%</strong>
          </header>
          <div
            className="import-progress-track"
            role="progressbar"
            aria-label="Progresso da importação"
            aria-valuemin={0}
            aria-valuemax={utxImportProgress.total}
            aria-valuenow={utxImportProgress.completed}
          ><span style={{ width: `${utxImportPercent}%` }} /></div>
          <p>{utxImportProgress.phase}</p>
          <small>{utxImportProgress.fileName || "Aguarde enquanto o pacote é atualizado."}<span>{utxImportProgress.completed} de {utxImportProgress.total}</span></small>
        </section>
      </div>}
      {utxExportProgress && <div className="import-progress-backdrop" role="presentation">
        <section className="import-progress-modal" role="dialog" aria-modal="true" aria-label="Exportando texturas UTX">
          <header>
            <div><span className="eyebrow">EXPORTAÇÃO UTX</span><h2>Exportando texturas</h2></div>
            <strong>{utxExportPercent}%</strong>
          </header>
          <div
            className="import-progress-track"
            role="progressbar"
            aria-label="Progresso da exportação"
            aria-valuemin={0}
            aria-valuemax={utxExportProgress.total}
            aria-valuenow={utxExportProgress.completed}
          ><span style={{ width: `${utxExportPercent}%` }} /></div>
          <p>Gravando texturas e os metadados complementares.</p>
          <small>{utxExportProgress.fileName || "Preparando arquivos…"}<span>{utxExportProgress.completed} de {utxExportProgress.total}</span></small>
        </section>
      </div>}
      {utxExtractProgress && <div className="import-progress-backdrop" role="presentation">
        <section className="import-progress-modal" role="dialog" aria-modal="true" aria-label="Extraindo texturas UTX">
          <header>
            <div><span className="eyebrow">EXTRAÇÃO UTX</span><h2>Extraindo texturas</h2></div>
            <strong>{utxExtractPercent}%</strong>
          </header>
          <div
            className="import-progress-track"
            role="progressbar"
            aria-label="Progresso da extração"
            aria-valuemin={0}
            aria-valuemax={utxExtractProgress.total}
            aria-valuenow={utxExtractProgress.completed}
          ><span style={{ width: `${utxExtractPercent}%` }} /></div>
          <p>{utxExtractMode === "png" ? "Convertendo texturas para PNG e gravando os metadados." : "Gravando texturas nos formatos originais e os metadados complementares."}</p>
          <small>{utxExtractProgress.fileName || "Preparando pacotes…"}<span>{utxExtractProgress.total > 0 ? `${utxExtractProgress.completed} de ${utxExtractProgress.total}` : utxExtractProgress.packageName || "Lendo estruturas"}</span></small>
        </section>
      </div>}
      {textureResizeProgress && <div className="import-progress-backdrop" role="presentation">
        <section className="import-progress-modal" role="dialog" aria-modal="true" aria-label="Redimensionando ícones">
          <header>
            <div><span className="eyebrow">CONVERSÃO DE ÍCONES</span><h2>Redimensionando texturas</h2></div>
            <strong>{textureResizePercent}%</strong>
          </header>
          <div
            className="import-progress-track"
            role="progressbar"
            aria-label="Progresso do redimensionamento"
            aria-valuemin={0}
            aria-valuemax={textureResizeProgress.total}
            aria-valuenow={textureResizeProgress.completed}
          ><span style={{ width: `${textureResizePercent}%` }} /></div>
          <p>Aplicando Lanczos3 e preservando o formato de cada textura.</p>
          <small>{textureResizeProgress.fileName || "Preparando arquivos…"}<span>{textureResizeProgress.completed} de {textureResizeProgress.total}</span></small>
        </section>
      </div>}
      {geodataProgress && <div className="import-progress-backdrop" role="presentation">
        <section className="import-progress-modal" role="dialog" aria-modal="true" aria-label="Convertendo geodata">
          <header>
            <div><span className="eyebrow">CONVERSÃO DE GEODATA</span><h2>Convertendo regiões</h2></div>
            <strong>{geodataPercent}%</strong>
          </header>
          <div
            className="import-progress-track"
            role="progressbar"
            aria-label="Progresso da conversão geodata"
            aria-valuemin={0}
            aria-valuemax={geodataProgress.total}
            aria-valuenow={geodataProgress.completed}
          ><span style={{ width: `${geodataPercent}%` }} /></div>
          <p>Processando arquivos em paralelo com um limite seguro para o computador.</p>
          <small>{geodataProgress.fileName || "Preparando arquivos…"}<span>{geodataProgress.completed} de {geodataProgress.total}</span></small>
        </section>
      </div>}
      {utxViewer && <div className="preview-backdrop" role="presentation" onMouseDown={closeUtxViewer}>
        <section className="preview-modal" role="dialog" aria-modal="true" aria-label={`Prévia de ${utxViewer.entry.name}`} onMouseDown={(event) => event.stopPropagation()}>
          <header><div><span className="eyebrow">PRÉ-VISUALIZAÇÃO {utxViewer.entry.format}</span><h2>{utxViewer.entry.name}</h2></div><button className="row-action" onClick={closeUtxViewer} aria-label="Fechar prévia"><Icon name="close" size={18} /></button></header>
          <div className="preview-stage">{utxViewer.preview && <img src={utxViewer.preview.dataUrl} alt={utxViewer.entry.name} />}</div>
          <footer>
            <span>{utxViewer.preview ? `${utxViewer.preview.width} × ${utxViewer.preview.height} px · ${utxViewer.entry.format}` : ""}</span>
            <div className="preview-navigation">
              <button className="row-action previous" disabled={utxViewer.loading || previewableUtxEntries.length < 2} onClick={() => moveUtxPreview(-1)} aria-label="Textura anterior"><Icon name="chevron" size={18} /></button>
              <span>{Math.max(0, previewableUtxEntries.findIndex((entry) => entry.exportIndex === utxViewer.entry.exportIndex) + 1)} de {previewableUtxEntries.length}</span>
              <button className="row-action" disabled={utxViewer.loading || previewableUtxEntries.length < 2} onClick={() => moveUtxPreview(1)} aria-label="Próxima textura"><Icon name="chevron" size={18} /></button>
            </div>
          </footer>
        </section>
      </div>}
    </main>
  );
}

export default App;
