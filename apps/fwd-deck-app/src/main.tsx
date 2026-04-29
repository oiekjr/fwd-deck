import {
  Alert,
  Button as HeroButton,
  Card,
  Checkbox,
  Chip,
  Dropdown,
  Input as HeroInput,
  Label as HeroLabel,
  Modal,
  ProgressBar,
  Switch,
  Table,
  TextField as HeroTextField,
  Tooltip,
} from "./components/ui/shadcn-compat";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { homeDir, join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  CirclePlus,
  CircleStop,
  Clock3,
  FolderOpen,
  Gauge,
  KeyRound,
  LayoutGrid,
  ListFilter,
  Loader2,
  Minus,
  Pencil,
  Play,
  RefreshCw,
  Rows3,
  Search,
  Server,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import {
  StrictMode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ChangeEvent, FormEvent, ReactElement, ReactNode, RefObject } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

type ConfigScope = "local" | "global";

type RuntimeState = "running" | "stale";

type RuntimeScope = "global" | "workspace";

type TunnelStatus = RuntimeState | "idle";

type StatusFilter = "all" | TunnelStatus;

type ScopeFilter = "all" | ConfigScope;

type AppView = "dashboard" | "add";

type TunnelDisplayMode = "card" | "slim";

type HeroButtonVariant =
  | "danger"
  | "danger-soft"
  | "ghost"
  | "outline"
  | "primary"
  | "secondary"
  | "tertiary";

type HeroButtonSize = "lg" | "md" | "sm";

type AlertStatus = "accent" | "danger" | "success" | "warning";

type AppCommand =
  | "load_dashboard"
  | "switch_workspace"
  | "start_tunnels"
  | "stop_tunnels"
  | "add_tunnel_entry"
  | "update_tunnel_entry"
  | "remove_tunnel_entry"
  | "remove_workspace_history_entry"
  | "refresh_tray_menu";

interface WorkspaceSelection {
  workspacePath: string;
  workspaceHistory: string[];
  localConfigPath: string;
  globalConfigPath: string;
  useGlobal: boolean;
  globalStatePath: string;
  workspaceStatePath: string;
  hideDockIconWhenWindowHidden: boolean;
  autoStopTunnelsOnQuit: boolean;
}

interface WorkspaceSelectionInput {
  workspacePath: string;
  globalConfigPath: string;
  useGlobal: boolean;
  hideDockIconWhenWindowHidden: boolean;
  autoStopTunnelsOnQuit: boolean;
}

interface OperationTarget {
  id: string;
  runtimeScope?: RuntimeScope;
}

interface DashboardState {
  paths: WorkspaceSelection;
  hasConfig: boolean;
  validation: ValidationView;
  tunnels: TunnelView[];
  trackedTunnels: TrackedTunnelView[];
}

interface WorkspaceSwitchResult {
  dashboard: DashboardState;
  stoppedCount: number;
}

interface ValidationView {
  isValid: boolean;
  errors: ValidationIssueView[];
  warnings: ValidationIssueView[];
}

interface ValidationIssueView {
  source: string;
  path: string;
  tunnelId: string | null;
  message: string;
}

interface TunnelView {
  id: string;
  description: string | null;
  tags: string[];
  localHost: string;
  localPort: number;
  local: string;
  remoteHost: string;
  remotePort: number;
  remote: string;
  sshUser: string;
  sshHost: string;
  sshPort: number | null;
  ssh: string;
  identityFile: string | null;
  source: ConfigScope;
  sourcePath: string;
  timeouts: TimeoutView;
  status: RuntimeStatusView | null;
}

interface TrackedTunnelView {
  id: string;
  runtimeScope: RuntimeScope;
  runtimeKey: string;
  local: string;
  remote: string;
  ssh: string;
  status: RuntimeStatusView;
}

interface RuntimeStatusView {
  runtimeScope: RuntimeScope;
  runtimeKey: string;
  pid: number;
  state: RuntimeState;
  source: ConfigScope;
  sourcePath: string;
  startedAtUnixSeconds: number;
}

interface TimeoutView {
  connectTimeoutSeconds: number;
  serverAliveIntervalSeconds: number;
  serverAliveCountMax: number;
  startGraceMilliseconds: number;
}

interface OperationReport {
  succeeded: OperationSuccessView[];
  failed: OperationFailureView[];
}

interface OperationSuccessView {
  id: string;
  message: string;
}

interface OperationFailureView {
  id: string;
  message: string;
}

interface TunnelFormState {
  scope: ConfigScope;
  id: string;
  description: string;
  tags: string;
  localHost: string;
  localPort: string;
  remoteHost: string;
  remotePort: string;
  sshUser: string;
  sshHost: string;
  sshPort: string;
  identityFile: string;
}

interface TunnelInput {
  id: string;
  description: string | null;
  tags: string[];
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  sshUser: string;
  sshHost: string;
  sshPort: number | null;
  identityFile: string | null;
}

type AppMessageKind = "success" | "error" | "info";

interface AppMessage {
  kind: AppMessageKind;
  text: string;
}

interface OperationToastMessage {
  id: number;
  kind: AppMessageKind;
  summary: string;
  detail?: string;
}

type OperationToastInput = Omit<OperationToastMessage, "id">;

type TunnelOperationCommand = "start_tunnels" | "stop_tunnels";

interface OperationProgress {
  operationId: string;
  command: TunnelOperationCommand;
  completedCount: number;
  totalCount: number;
}

interface OperationProgressEventPayload {
  operationId: string;
  completedCount: number;
  totalCount: number;
}

interface TrayOperationResultPayload {
  kind: AppMessageKind;
  summary: string;
  detail: string | null;
}

interface RefreshDashboardOptions {
  silent?: boolean;
  persistPaths?: boolean;
}

interface PathSelectionApplyResult {
  dashboard: DashboardState;
  stoppedCount: number;
}

interface TauriRuntimeWindow extends Window {
  __TAURI_INTERNALS__?: {
    invoke?: unknown;
  };
}

interface TunnelFilters {
  query: string;
  status: StatusFilter;
  scope: ScopeFilter;
  tags: string[];
}

interface HighlightedTextPart {
  text: string;
  isMatch: boolean;
}

interface ViewportScrollSnapshot {
  left: number;
  top: number;
}

const initialPaths: WorkspaceSelection = {
  workspacePath: "",
  workspaceHistory: [],
  localConfigPath: "",
  globalConfigPath: "",
  useGlobal: true,
  globalStatePath: "",
  workspaceStatePath: "",
  hideDockIconWhenWindowHidden: false,
  autoStopTunnelsOnQuit: false,
};

const initialForm: TunnelFormState = {
  scope: "local",
  id: "",
  description: "",
  tags: "",
  localHost: "127.0.0.1",
  localPort: "",
  remoteHost: "",
  remotePort: "",
  sshUser: "",
  sshHost: "",
  sshPort: "22",
  identityFile: "",
};

const initialFilters: TunnelFilters = {
  query: "",
  status: "all",
  scope: "all",
  tags: [],
};

const searchDebounceMilliseconds = 200;
const autoRefreshIntervalMilliseconds = 2_000;
const operationToastDismissMilliseconds = 4_000;
const operationProgressEventName = "operation-progress";
const trayOperationResultEventName = "tray-operation-result";
const openSettingsEventName = "open-settings";
const missingTauriRuntimeMessage =
  "Tauri 実行環境が見つかりません。アプリの操作確認は npm run tauri dev またはビルド済みアプリから実行してください";

const statusFilterOptions: ReadonlyArray<{ value: StatusFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "stale", label: "Stale" },
  { value: "idle", label: "Idle" },
];

const scopeFilterOptions: ReadonlyArray<{ value: ScopeFilter; label: string }> = [
  { value: "all", label: "All scopes" },
  { value: "local", label: "Local" },
  { value: "global", label: "Global" },
];

/**
 * アプリ全体の UI を描画する
 */
function App(): ReactElement {
  const [dashboard, setDashboard] = useState<DashboardState | null>(null);
  const [paths, setPaths] = useState<WorkspaceSelection>(initialPaths);
  const [form, setForm] = useState<TunnelFormState>(initialForm);
  const [filters, setFilters] = useState<TunnelFilters>(initialFilters);
  const [queryInput, setQueryInput] = useState<string>(initialFilters.query);
  const [activeView, setActiveView] = useState<AppView>("dashboard");
  const [tunnelDisplayMode, setTunnelDisplayMode] = useState<TunnelDisplayMode>("slim");
  const [settingsDraft, setSettingsDraft] = useState<WorkspaceSelection | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deleteTarget, setDeleteTarget] = useState<TunnelView | null>(null);
  const [editTarget, setEditTarget] = useState<TunnelView | null>(null);
  const [editForm, setEditForm] = useState<TunnelFormState>(initialForm);
  const [formFeedback, setFormFeedback] = useState<AppMessage | null>(null);
  const [editFormFeedback, setEditFormFeedback] = useState<AppMessage | null>(null);
  const [message, setMessage] = useState<AppMessage | null>(null);
  const [operationToast, setOperationToast] = useState<OperationToastMessage | null>(null);
  const [operationProgress, setOperationProgress] = useState<OperationProgress | null>(null);
  const [isBusy, setIsBusy] = useState<boolean>(false);
  const [hasCompletedInitialLoad, setHasCompletedInitialLoad] = useState<boolean>(false);
  const autoRefreshInFlightRef = useRef<boolean>(false);
  const operationInFlightRef = useRef<boolean>(false);
  const activeOperationIdRef = useRef<string | null>(null);
  const operationSequenceRef = useRef<number>(0);
  const operationToastIdRef = useRef<number>(0);
  const resultScrollSnapshotRef = useRef<ViewportScrollSnapshot | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  const stats = useMemo<DashboardStats>(() => calculateStats(dashboard), [dashboard]);
  const selectedIdList = useMemo<string[]>(() => Array.from(selectedIds), [selectedIds]);
  const availableTags = useMemo<string[]>(
    () => collectAvailableTags(dashboard?.tunnels ?? []),
    [dashboard],
  );
  const filteredTunnels = useMemo<TunnelView[]>(
    () => filterTunnels(dashboard?.tunnels ?? [], filters),
    [dashboard, filters],
  );
  const filteredTunnelIds = useMemo<string[]>(
    () => filteredTunnels.map((tunnel) => tunnel.id),
    [filteredTunnels],
  );
  const selectedVisibleCount = useMemo<number>(
    () => filteredTunnelIds.filter((id) => selectedIds.has(id)).length,
    [filteredTunnelIds, selectedIds],
  );
  const hasActiveFilters = useMemo<boolean>(
    () => hasActiveTunnelFilters(filters) || queryInput.trim().length > 0,
    [filters, queryInput],
  );

  /**
   * 操作結果トーストを表示する
   */
  const showOperationToast = useCallback((message: OperationToastInput): void => {
    operationToastIdRef.current += 1;
    setMessage(null);
    setOperationToast({ ...message, id: operationToastIdRef.current });
  }, []);

  /**
   * 表示中の操作結果トーストを閉じる
   */
  const dismissOperationToast = useCallback((): void => {
    setOperationToast(null);
  }, []);

  const captureResultScrollPosition = useCallback((): void => {
    resultScrollSnapshotRef.current = createViewportScrollSnapshot();
  }, []);

  /**
   * 読み込んだダッシュボード状態を画面へ反映する
   */
  const applyLoadedDashboard = useCallback(
    (loaded: DashboardState): void => {
      if (hasActiveTunnelFilters(filters)) {
        captureResultScrollPosition();
      }

      setDashboard(loaded);
      setPaths(loaded.paths);
      setSelectedIds((current) => keepExistingSelections(current, loaded.tunnels));
    },
    [captureResultScrollPosition, filters],
  );

  useLayoutEffect(() => {
    const snapshot = resultScrollSnapshotRef.current;
    if (snapshot === null) {
      return;
    }

    resultScrollSnapshotRef.current = null;

    if (activeView !== "dashboard") {
      return;
    }

    restoreViewportScroll(snapshot);
  });

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      if (filters.query === queryInput) {
        return;
      }

      captureResultScrollPosition();

      setFilters((current) => {
        if (current.query === queryInput) {
          return current;
        }

        return { ...current, query: queryInput };
      });
    }, searchDebounceMilliseconds);

    return () => window.clearTimeout(timeoutId);
  }, [captureResultScrollPosition, filters.query, queryInput]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      if (isSettingsKeyboardShortcut(event)) {
        event.preventDefault();
        setSettingsDraft((current) => current ?? paths);
        return;
      }

      if (
        isSearchKeyboardShortcut(event) &&
        activeView === "dashboard" &&
        settingsDraft === null &&
        deleteTarget === null &&
        editTarget === null
      ) {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }

      if (event.key === "Escape" && !isBusy) {
        setSettingsDraft(null);
      }
    }

    window.addEventListener("keydown", handleKeyDown);

    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeView, deleteTarget, editTarget, isBusy, paths, settingsDraft]);

  useEffect(() => {
    if (operationToast === null) {
      return;
    }

    const toastId = operationToast.id;
    const timeoutId = window.setTimeout(() => {
      setOperationToast((current) => (current?.id === toastId ? null : current));
    }, operationToastDismissMilliseconds);

    return () => window.clearTimeout(timeoutId);
  }, [operationToast]);

  useEffect(() => {
    if (!isTauriRuntimeAvailable()) {
      return;
    }

    let isDisposed = false;
    let unlisten: (() => void) | null = null;

    void listen<OperationProgressEventPayload>(operationProgressEventName, (event) => {
      const payload = event.payload;

      if (payload.operationId !== activeOperationIdRef.current) {
        return;
      }

      setOperationProgress((current) => {
        if (current === null || current.operationId !== payload.operationId) {
          return current;
        }

        return {
          ...current,
          completedCount: clampCompletedCount(payload.completedCount, payload.totalCount),
          totalCount: payload.totalCount,
        };
      });
    })
      .then((nextUnlisten) => {
        if (isDisposed) {
          nextUnlisten();
          return;
        }

        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      isDisposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntimeAvailable()) {
      return;
    }

    let isDisposed = false;
    let unlisten: (() => void) | null = null;

    void listen<void>(openSettingsEventName, () => {
      setSettingsDraft((current) => current ?? paths);
    })
      .then((nextUnlisten) => {
        if (isDisposed) {
          nextUnlisten();
          return;
        }

        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      isDisposed = true;
      unlisten?.();
    };
  }, [paths]);

  /**
   * 現在のパス設定に基づいてダッシュボードを再取得する
   */
  const refreshDashboard = useCallback(
    async (
      nextPaths: WorkspaceSelection = paths,
      options: RefreshDashboardOptions = {},
    ): Promise<boolean> => {
      const isSilent = options.silent === true;
      const shouldPersistPaths = options.persistPaths !== false;

      if (!isSilent) {
        setIsBusy(true);
      }

      try {
        const loaded = await invokeCommand<DashboardState>("load_dashboard", {
          paths: shouldPersistPaths ? normalizeWorkspaceSelection(nextPaths) : null,
        });

        applyLoadedDashboard(loaded);

        if (!isSilent) {
          setMessage(null);
        }

        return true;
      } catch (error) {
        if (!isSilent) {
          setMessage({ kind: "error", text: stringifyError(error) });
        }

        return false;
      } finally {
        if (!isSilent) {
          setIsBusy(false);
        }
      }
    },
    [applyLoadedDashboard, paths],
  );

  useEffect(() => {
    if (!isTauriRuntimeAvailable()) {
      return;
    }

    let isDisposed = false;
    let unlisten: (() => void) | null = null;

    void listen<TrayOperationResultPayload>(trayOperationResultEventName, (event) => {
      const payload = event.payload;

      void refreshDashboard(paths, { persistPaths: false, silent: true });
      showOperationToast({
        kind: payload.kind,
        summary: payload.summary,
        detail: payload.detail ?? undefined,
      });

      if (payload.kind === "error") {
        setActiveView("dashboard");
      }
    })
      .then((nextUnlisten) => {
        if (isDisposed) {
          nextUnlisten();
          return;
        }

        unlisten = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      isDisposed = true;
      unlisten?.();
    };
  }, [paths, refreshDashboard, showOperationToast]);

  useEffect(() => {
    async function loadInitialDashboard(): Promise<void> {
      setIsBusy(true);

      try {
        const loaded = await invokeCommand<DashboardState>("load_dashboard", {
          paths: null,
        });

        setDashboard(loaded);
        setPaths(loaded.paths);
        setSelectedIds((current) => keepExistingSelections(current, loaded.tunnels));
        setMessage(null);
      } catch (error) {
        setMessage({ kind: "error", text: stringifyError(error) });
      } finally {
        setIsBusy(false);
        setHasCompletedInitialLoad(true);
      }
    }

    void loadInitialDashboard();
  }, []);

  useEffect(() => {
    if (!hasCompletedInitialLoad) {
      return;
    }

    function runAutoRefresh(): void {
      if (autoRefreshInFlightRef.current || operationInFlightRef.current) {
        return;
      }

      autoRefreshInFlightRef.current = true;
      void refreshDashboard(paths, { persistPaths: false, silent: true }).finally(() => {
        autoRefreshInFlightRef.current = false;
      });
    }

    function refreshWhenVisible(): void {
      if (document.visibilityState === "visible") {
        runAutoRefresh();
      }
    }

    const intervalId = window.setInterval(runAutoRefresh, autoRefreshIntervalMilliseconds);

    window.addEventListener("focus", runAutoRefresh);
    document.addEventListener("visibilitychange", refreshWhenVisible);

    return () => {
      window.clearInterval(intervalId);
      window.removeEventListener("focus", runAutoRefresh);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
  }, [hasCompletedInitialLoad, paths, refreshDashboard]);

  /**
   * 指定 ID のトンネルを開始する
   */
  async function startSelected(ids: string[]): Promise<void> {
    if (ids.length === 0) {
      showOperationToast({ kind: "info", summary: "開始するトンネルを選択してください" });
      return;
    }

    await runOperation(
      "start_tunnels",
      ids.map((id) => ({ id })),
    );
  }

  /**
   * 指定 ID のトンネルを停止する
   */
  async function stopSelected(ids: string[]): Promise<void> {
    if (ids.length === 0) {
      showOperationToast({ kind: "info", summary: "停止するトンネルを選択してください" });
      return;
    }

    await runOperation(
      "stop_tunnels",
      ids.map((id) => operationTargetForStop(id, dashboard)),
    );
  }

  /**
   * 追跡中 runtime のトンネルを停止する
   */
  async function stopTracked(target: OperationTarget): Promise<void> {
    await runOperation("stop_tunnels", [target]);
  }

  /**
   * トンネル操作を実行して結果を反映する
   */
  async function runOperation(
    command: TunnelOperationCommand,
    targets: OperationTarget[],
  ): Promise<void> {
    operationSequenceRef.current += 1;
    const operationId = `operation-${operationSequenceRef.current}`;

    operationInFlightRef.current = true;
    activeOperationIdRef.current = operationId;
    setOperationProgress({
      operationId,
      command,
      completedCount: 0,
      totalCount: targets.length,
    });
    setIsBusy(true);
    await waitForNextPaint();

    try {
      const report = await invokeCommand<OperationReport>(command, {
        paths: normalizeWorkspaceSelection(paths),
        targets,
        operationId,
      });

      await refreshDashboard(paths, { silent: true });
      const message = operationMessage(report);
      if (message === null) {
        dismissOperationToast();
        return;
      }

      showOperationToast(message);
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    } finally {
      operationInFlightRef.current = false;
      activeOperationIdRef.current = null;
      setOperationProgress(null);
      setIsBusy(false);
    }
  }

  /**
   * 設定ファイルへトンネルを追加する
   */
  async function submitTunnel(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();

    if (form.scope === "local" && paths.workspacePath.trim().length === 0) {
      setFormFeedback({
        kind: "error",
        text: "local 設定に追加するにはワークスペースを選択してください",
      });
      return;
    }

    let tunnel: TunnelInput;
    try {
      tunnel = formToTunnelInput(form);
    } catch (error) {
      setFormFeedback({ kind: "error", text: stringifyError(error) });
      return;
    }

    setFormFeedback(null);
    setIsBusy(true);

    try {
      const loaded = await invokeCommand<DashboardState>("add_tunnel_entry", {
        paths: normalizeWorkspaceSelection(paths),
        scope: form.scope,
        tunnel,
      });

      setDashboard(loaded);
      setPaths(loaded.paths);
      setForm({ ...initialForm, scope: form.scope });
      setFormFeedback(null);
      showOperationToast({ kind: "success", summary: `${tunnel.id} を設定に追加しました` });
      setActiveView("dashboard");
    } catch (error) {
      setFormFeedback({ kind: "error", text: stringifyError(error) });
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * 設定ファイル内の既存トンネルを更新する
   */
  async function submitEditedTunnel(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();

    if (editTarget === null) {
      return;
    }

    let tunnel: TunnelInput;
    try {
      tunnel = formToTunnelInput(editForm);
    } catch (error) {
      setEditFormFeedback({ kind: "error", text: stringifyError(error) });
      return;
    }

    setEditFormFeedback(null);
    setIsBusy(true);

    try {
      const loaded = await invokeCommand<DashboardState>("update_tunnel_entry", {
        paths: normalizeWorkspaceSelection(paths),
        scope: editTarget.source,
        id: editTarget.id,
        tunnel,
      });

      if (hasActiveTunnelFilters(filters)) {
        captureResultScrollPosition();
      }

      setDashboard(loaded);
      setPaths(loaded.paths);
      setSelectedIds((current) => keepExistingSelections(current, loaded.tunnels));
      setEditTarget(null);
      setEditFormFeedback(null);
      showOperationToast({ kind: "success", summary: `${tunnel.id} を設定に反映しました` });
    } catch (error) {
      setEditFormFeedback({ kind: "error", text: stringifyError(error) });
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * 設定ファイルからトンネルを削除する
   */
  async function removeTunnel(tunnel: TunnelView): Promise<void> {
    setDeleteTarget(null);
    setIsBusy(true);

    try {
      const loaded = await invokeCommand<DashboardState>("remove_tunnel_entry", {
        paths: normalizeWorkspaceSelection(paths),
        scope: tunnel.source,
        id: tunnel.id,
      });

      if (hasActiveTunnelFilters(filters)) {
        captureResultScrollPosition();
      }

      setDashboard(loaded);
      setPaths(loaded.paths);
      setSelectedIds((current) => removeSelection(current, tunnel.id));
      showOperationToast({ kind: "success", summary: `${tunnel.id} を設定から削除しました` });
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * ワークスペース履歴の指定行を永続設定から削除する
   */
  async function removeWorkspaceHistoryEntry(workspacePath: string): Promise<void> {
    setIsBusy(true);

    try {
      const nextPaths = await invokeCommand<WorkspaceSelection>("remove_workspace_history_entry", {
        workspacePath,
      });
      const nextHistory = nextPaths.workspaceHistory;

      setPaths((current) => ({ ...current, workspaceHistory: nextHistory }));
      setSettingsDraft((current) =>
        current === null ? current : { ...current, workspaceHistory: nextHistory },
      );
      showOperationToast({ kind: "success", summary: "Recent workspaces から削除しました" });
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * トレイメニューへ現在の設定状態を反映する
   */
  async function refreshTrayMenuFromUi(): Promise<void> {
    try {
      await invokeCommand<void>("refresh_tray_menu", {});
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    }
  }

  /**
   * ツールバーから変更したパス設定を即時保存する
   */
  async function applyToolbarPathSelection(
    nextPaths: WorkspaceSelection,
    successSummary: string,
  ): Promise<void> {
    const result = await applyPathSelectionToDashboard(nextPaths);
    if (result === null) {
      return;
    }

    setSettingsDraft((current) =>
      current === null ? current : { ...current, ...result.dashboard.paths },
    );
    await refreshTrayMenuFromUi();
    showOperationToast({
      kind: "success",
      summary: workspaceSwitchSuccessSummary(successSummary, result.stoppedCount),
    });
  }

  /**
   * パス設定を保存してダッシュボードへ反映する
   */
  async function applyPathSelectionToDashboard(
    nextPaths: WorkspaceSelection,
  ): Promise<PathSelectionApplyResult | null> {
    const shouldSwitchWorkspace = workspacePathHasChanged(paths, nextPaths);
    setIsBusy(true);

    try {
      const result = shouldSwitchWorkspace
        ? await invokeCommand<WorkspaceSwitchResult>("switch_workspace", {
            paths: normalizeWorkspaceSelection(nextPaths),
          })
        : {
            dashboard: await invokeCommand<DashboardState>("load_dashboard", {
              paths: normalizeWorkspaceSelection(nextPaths),
            }),
            stoppedCount: 0,
          };

      applyLoadedDashboard(result.dashboard);
      setMessage(null);

      return result;
    } catch (error) {
      if (shouldSwitchWorkspace) {
        showOperationToast({ kind: "error", summary: stringifyError(error) });
      } else {
        setMessage({ kind: "error", text: stringifyError(error) });
      }

      return null;
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * 履歴から選択したワークスペースを即時適用する
   */
  async function switchWorkspaceFromToolbar(workspacePath: string): Promise<void> {
    const nextWorkspacePath = workspacePath.trim();
    if (nextWorkspacePath.length === 0 || nextWorkspacePath === paths.workspacePath.trim()) {
      return;
    }

    await applyToolbarPathSelection(
      { ...paths, workspacePath: nextWorkspacePath },
      "Workspace を切り替えました",
    );
  }

  /**
   * フォルダ選択ダイアログからワークスペースを即時適用する
   */
  async function browseWorkspaceFromToolbar(): Promise<void> {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== "string") {
        return;
      }

      await applyToolbarPathSelection(
        { ...paths, workspacePath: selected },
        "Workspace を切り替えました",
      );
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    }
  }

  /**
   * 設定モーダルを現在の適用済み設定で開く
   */
  function openSettings(): void {
    setSettingsDraft((current) => current ?? paths);
  }

  /**
   * 設定モーダルを閉じて未適用の変更を破棄する
   */
  function closeSettings(): void {
    setSettingsDraft(null);
  }

  /**
   * 設定モーダルの未適用入力を更新する
   */
  function updateSettingsDraft(field: keyof WorkspaceSelection, value: string | boolean): void {
    setSettingsDraft((current) => {
      if (current === null) {
        return current;
      }

      return { ...current, [field]: value };
    });
  }

  /**
   * フォルダ選択ダイアログの結果を未適用ワークスペースへ反映する
   */
  async function browseWorkspace(): Promise<void> {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== "string") {
        return;
      }

      updateSettingsDraft("workspacePath", selected);
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
    }
  }

  /**
   * ファイル選択ダイアログの結果を未適用グローバル設定へ反映する
   */
  async function browseGlobalConfig(): Promise<void> {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: "TOML", extensions: ["toml"] }],
      });

      if (typeof selected !== "string") {
        return;
      }

      updateSettingsDraft("globalConfigPath", selected);
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
    }
  }

  /**
   * ファイル選択ダイアログの結果を identity_file 入力へ反映する
   */
  async function browseIdentityFile(): Promise<void> {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        defaultPath: await identityFileDialogDefaultPath(),
      });

      if (typeof selected !== "string") {
        return;
      }

      updateForm("identityFile", selected);
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
    }
  }

  /**
   * ファイル選択ダイアログの結果を編集フォームの identity_file 入力へ反映する
   */
  async function browseEditIdentityFile(): Promise<void> {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        defaultPath: await identityFileDialogDefaultPath(),
      });

      if (typeof selected !== "string") {
        return;
      }

      updateEditForm("identityFile", selected);
    } catch (error) {
      showOperationToast({ kind: "error", summary: stringifyError(error) });
    }
  }

  /**
   * 履歴から選択したワークスペースを未適用入力へ反映する
   */
  function selectWorkspaceFromHistory(workspacePath: string): void {
    updateSettingsDraft("workspacePath", workspacePath);
  }

  /**
   * 設定モーダルの入力を適用してダッシュボードを再読み込みする
   */
  async function applySettings(): Promise<void> {
    if (settingsDraft === null) {
      return;
    }

    const result = await applyPathSelectionToDashboard(settingsDraft);
    if (result !== null) {
      await refreshTrayMenuFromUi();

      closeSettings();
      if (result.stoppedCount > 0) {
        showOperationToast({
          kind: "success",
          summary: workspaceSwitchSuccessSummary("Workspace を切り替えました", result.stoppedCount),
        });
      }
    }
  }

  /**
   * 追加フォームの変更を反映する
   */
  function updateForm(field: keyof TunnelFormState, value: string): void {
    setFormFeedback(null);
    setForm((current) => ({ ...current, [field]: value }));
  }

  /**
   * 編集フォームの変更を反映する
   */
  function updateEditForm(field: keyof TunnelFormState, value: string): void {
    setEditFormFeedback(null);
    setEditForm((current) => ({ ...current, [field]: value }));
  }

  /**
   * 指定トンネルを編集対象として開く
   */
  function openEditTunnel(tunnel: TunnelView): void {
    setEditTarget(tunnel);
    setEditForm(formFromTunnel(tunnel));
    setEditFormFeedback(null);
  }

  /**
   * トンネル選択状態を切り替える
   */
  function toggleSelection(id: string): void {
    setSelectedIds((current) => toggleId(current, id));
  }

  /**
   * 表示中のトンネルを選択状態へ追加する
   */
  function selectVisibleTunnels(): void {
    setSelectedIds((current) => addSelections(current, filteredTunnelIds));
  }

  /**
   * 表示中のトンネルを選択状態から除外する
   */
  function deselectVisibleTunnels(): void {
    setSelectedIds((current) => removeSelections(current, filteredTunnelIds));
  }

  /**
   * 一覧の絞り込み条件を反映する
   */
  function updateFilter<K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]): void {
    captureResultScrollPosition();
    setFilters((current) => ({ ...current, [field]: value }));
  }

  /**
   * 検索入力値を即時反映し、一覧への適用は遅延させる
   */
  function updateQueryInput(value: string): void {
    captureResultScrollPosition();
    setQueryInput(value);
  }

  /**
   * タグ絞り込みの選択状態を切り替える
   */
  function toggleTagFilter(tag: string): void {
    captureResultScrollPosition();
    setFilters((current) => ({ ...current, tags: toggleTag(current.tags, tag) }));
  }

  /**
   * 一覧の絞り込み条件を初期状態へ戻す
   */
  function resetFilters(): void {
    captureResultScrollPosition();
    setQueryInput(initialFilters.query);
    setFilters(initialFilters);
  }

  return (
    <main className="min-h-screen bg-muted/45 text-foreground">
      <div className="flex min-h-screen flex-col">
        <AppHeader
          stats={stats}
          paths={paths}
          activeView={activeView}
          isBusy={isBusy}
          onViewChange={setActiveView}
          onOpenSettings={openSettings}
          onBrowseWorkspace={() => void browseWorkspaceFromToolbar()}
          onSelectWorkspace={(workspacePath) => void switchWorkspaceFromToolbar(workspacePath)}
          onRefresh={() => void refreshDashboard()}
        />

        <div className="mx-auto flex w-full max-w-[96rem] flex-1 flex-col gap-3 px-3 py-3 sm:px-4 lg:px-5">
          <MessagePanel message={message} />

          {activeView === "dashboard" ? (
            <DashboardView
              dashboard={dashboard}
              hasCompletedInitialLoad={hasCompletedInitialLoad}
              filteredTunnels={filteredTunnels}
              hasActiveFilters={hasActiveFilters}
              selectedIds={selectedIds}
              selectedCount={selectedIdList.length}
              selectedVisibleCount={selectedVisibleCount}
              availableTags={availableTags}
              operationProgress={operationProgress}
              isBusy={isBusy}
              queryInput={queryInput}
              searchInputRef={searchInputRef}
              filters={filters}
              displayMode={tunnelDisplayMode}
              onQueryInputChange={updateQueryInput}
              onFilterChange={updateFilter}
              onToggleTag={toggleTagFilter}
              onResetFilters={resetFilters}
              onDisplayModeChange={setTunnelDisplayMode}
              onClearSelection={() => setSelectedIds(new Set())}
              onSelectVisible={selectVisibleTunnels}
              onDeselectVisible={deselectVisibleTunnels}
              onToggleSelection={toggleSelection}
              onStartSelected={() => void startSelected(selectedIdList)}
              onStopSelected={() => void stopSelected(selectedIdList)}
              onStartTunnel={(id) => void startSelected([id])}
              onStopTunnel={(id) => void stopSelected([id])}
              onStopTracked={(target) => void stopTracked(target)}
              onEditTunnel={openEditTunnel}
              onRemoveTunnel={setDeleteTarget}
              onAddTunnel={() => setActiveView("add")}
            />
          ) : activeView === "add" ? (
            <AddTunnelView
              form={form}
              feedback={formFeedback}
              canUseLocal={paths.workspacePath.trim().length > 0}
              isBusy={isBusy}
              onChange={updateForm}
              onSubmit={(event) => void submitTunnel(event)}
              onOpenSettings={openSettings}
              onBrowseIdentityFile={() => void browseIdentityFile()}
            />
          ) : null}
        </div>
      </div>
      <SettingsModal
        isOpen={settingsDraft !== null}
        paths={settingsDraft ?? paths}
        isBusy={isBusy}
        onCancel={closeSettings}
        onApply={() => void applySettings()}
        onChange={updateSettingsDraft}
        onBrowseWorkspace={() => void browseWorkspace()}
        onBrowseGlobalConfig={() => void browseGlobalConfig()}
        onSelectWorkspace={selectWorkspaceFromHistory}
        onRemoveWorkspace={(workspacePath) => void removeWorkspaceHistoryEntry(workspacePath)}
      />
      <ConfirmRemoveModal
        tunnel={deleteTarget}
        isBusy={isBusy}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={(tunnel) => void removeTunnel(tunnel)}
      />
      <EditTunnelModal
        tunnel={editTarget}
        form={editForm}
        feedback={editFormFeedback}
        isBusy={isBusy}
        onChange={updateEditForm}
        onCancel={() => {
          setEditTarget(null);
          setEditFormFeedback(null);
        }}
        onSubmit={(event) => void submitEditedTunnel(event)}
        onBrowseIdentityFile={() => void browseEditIdentityFile()}
      />
      <ToastViewport toast={operationToast} onDismiss={dismissOperationToast} />
    </main>
  );
}

interface DashboardStats {
  configured: number;
  running: number;
  stale: number;
}

interface AppHeaderProps {
  stats: DashboardStats;
  paths: WorkspaceSelection;
  activeView: AppView;
  isBusy: boolean;
  onViewChange: (view: AppView) => void;
  onOpenSettings: () => void;
  onBrowseWorkspace: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onRefresh: () => void;
}

/**
 * アプリ全体の操作状況と再読み込み導線を表示する
 */
function AppHeader({
  stats,
  paths,
  activeView,
  isBusy,
  onViewChange,
  onOpenSettings,
  onBrowseWorkspace,
  onSelectWorkspace,
  onRefresh,
}: AppHeaderProps): ReactElement {
  return (
    <header className="sticky top-0 z-30 border-b border-border bg-card/90 backdrop-blur supports-[backdrop-filter]:bg-card/80">
      <div className="mx-auto grid w-full max-w-[96rem] gap-2 px-3 py-2 sm:px-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center lg:px-5">
        <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-center">
          <div className="flex min-w-0 items-center gap-2">
            <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground shadow-sm">
              <Server size={17} />
            </span>
            <div className="min-w-0">
              <div className="truncate text-sm leading-5 font-semibold">Fwd Deck</div>
              <div className="truncate text-[0.7rem] leading-4 text-muted-foreground">
                Port Forwarding Deck
              </div>
            </div>
          </div>
          <WorkspacePill
            paths={paths}
            isBusy={isBusy}
            onOpenSettings={onOpenSettings}
            onBrowseWorkspace={onBrowseWorkspace}
            onSelectWorkspace={onSelectWorkspace}
          />
        </div>

        <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-center md:justify-end">
          <nav className="grid w-full grid-cols-2 gap-0.5 rounded-lg border border-border bg-muted p-0.5 md:w-72">
            <HeroButton
              type="button"
              variant={activeView === "dashboard" ? "primary" : "ghost"}
              size="sm"
              fullWidth
              onPress={() => onViewChange("dashboard")}
              className="min-w-0 justify-center"
            >
              <ListFilter size={15} />
              Dashboard
            </HeroButton>
            <HeroButton
              type="button"
              variant={activeView === "add" ? "primary" : "ghost"}
              size="sm"
              fullWidth
              onPress={() => onViewChange("add")}
              className="min-w-0 justify-center"
            >
              <CirclePlus size={15} />
              Add tunnel
            </HeroButton>
          </nav>

          <div className="grid grid-cols-[repeat(3,minmax(0,1fr))_auto_auto] gap-1 md:flex md:items-center">
            <StatusMetric label="Configured" value={stats.configured} icon={<Gauge size={15} />} />
            <StatusMetric
              label="Running"
              value={stats.running}
              tone="success"
              icon={<Activity size={15} />}
            />
            <StatusMetric
              label="Stale"
              value={stats.stale}
              tone="warning"
              icon={<Clock3 size={15} />}
            />
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              isIconOnly
              onPress={onOpenSettings}
              aria-label="Settings"
            >
              <Settings2 size={15} />
            </HeroButton>
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              isIconOnly
              onPress={onRefresh}
              isDisabled={isBusy}
              aria-label="再読み込み"
            >
              {isBusy ? <Loader2 className="animate-spin" size={15} /> : <RefreshCw size={15} />}
            </HeroButton>
          </div>
        </div>
      </div>
    </header>
  );
}

interface WorkspacePillProps {
  paths: WorkspaceSelection;
  isBusy: boolean;
  onOpenSettings: () => void;
  onBrowseWorkspace: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
}

/**
 * 現在の作業対象ワークスペースをヘッダー内へ表示する
 */
function WorkspacePill({
  paths,
  isBusy,
  onOpenSettings,
  onBrowseWorkspace,
  onSelectWorkspace,
}: WorkspacePillProps): ReactElement {
  const workspacePath = paths.workspacePath.trim();
  const hasWorkspace = workspacePath.length > 0;
  const recentWorkspaces = paths.workspaceHistory.filter(
    (historyPath) => historyPath.trim() !== "",
  );

  return (
    <div
      className={`grid h-8 min-w-0 grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-1.5 rounded-lg border px-2 shadow-sm md:w-[24rem] ${
        hasWorkspace ? "border-border bg-card/90" : "border-warning/25 bg-card"
      }`}
    >
      <span
        className={`flex size-5 items-center justify-center rounded-sm ${
          hasWorkspace ? "text-foreground/70" : "text-warning"
        }`}
      >
        <FolderOpen size={14} />
      </span>
      <div className="min-w-0">
        <div className="sr-only">Workspace</div>
        <div
          className={`truncate font-mono text-xs ${hasWorkspace ? "text-foreground/85" : "text-warning"}`}
          title={workspacePath || "Not selected"}
        >
          {workspacePath || "Not selected"}
        </div>
      </div>
      <Dropdown>
        <Dropdown.Trigger
          type="button"
          className="size-7 rounded-sm"
          isDisabled={isBusy}
          aria-label="ワークスペースを切り替え"
        >
          <ChevronDown size={15} />
        </Dropdown.Trigger>
        <Dropdown.Popover placement="bottom end" className="w-[min(24rem,calc(100vw-2rem))]">
          <Dropdown.Menu
            onAction={(key) => {
              if (key === "__browse") {
                onBrowseWorkspace();
                return;
              }

              onSelectWorkspace(String(key));
            }}
          >
            <Dropdown.Item id="__browse" textValue="Browse workspace">
              <FolderOpen size={14} />
              <HeroLabel>Browse workspace...</HeroLabel>
            </Dropdown.Item>
            {recentWorkspaces.length > 0 ? (
              recentWorkspaces.map((historyPath) => (
                <Dropdown.Item
                  id={historyPath}
                  key={historyPath}
                  isDisabled={historyPath === workspacePath}
                  textValue={historyPath}
                >
                  <HeroLabel className="max-w-full truncate font-mono text-xs">
                    {historyPath}
                  </HeroLabel>
                </Dropdown.Item>
              ))
            ) : (
              <Dropdown.Item id="__empty" isDisabled textValue="No recent workspaces">
                <HeroLabel>No recent workspaces</HeroLabel>
              </Dropdown.Item>
            )}
          </Dropdown.Menu>
        </Dropdown.Popover>
      </Dropdown>
      <HeroButton
        type="button"
        variant="ghost"
        size="sm"
        isIconOnly
        className="size-7 rounded-sm"
        onPress={onOpenSettings}
        isDisabled={isBusy}
        aria-label="ワークスペース設定"
      >
        <Settings2 size={15} />
      </HeroButton>
    </div>
  );
}

interface StatusMetricProps {
  label: string;
  value: number;
  icon: ReactNode;
  tone?: "success" | "warning";
}

/**
 * 上部の集計値を表示する
 */
function StatusMetric({ label, value, icon, tone }: StatusMetricProps): ReactElement {
  const textColor = tone === "success" ? "text-success" : tone === "warning" ? "text-warning" : "";

  return (
    <div className="flex h-8 min-w-0 items-center gap-1.5 rounded-lg border border-border bg-card/90 px-2 text-xs shadow-sm">
      <div className="flex min-w-0 items-center gap-1.5 text-muted-foreground">
        <span className={textColor}>{icon}</span>
        <span className="truncate">{label}</span>
      </div>
      <div className={`shrink-0 font-semibold tabular-nums ${textColor}`}>{value}</div>
    </div>
  );
}

interface IconButtonProps {
  label: string;
  variant?: HeroButtonVariant;
  size?: HeroButtonSize;
  disabled?: boolean;
  children: ReactNode;
  onPress: () => void;
}

/**
 * Tooltip 付きアイコンボタンを表示する
 */
function IconButton({
  label,
  variant = "ghost",
  size = "sm",
  disabled = false,
  children,
  onPress,
}: IconButtonProps): ReactElement {
  return (
    <Tooltip>
      <HeroButton
        type="button"
        variant={variant}
        size={size}
        isIconOnly
        onPress={onPress}
        isDisabled={disabled}
        aria-label={label}
      >
        {children}
      </HeroButton>
      <Tooltip.Content placement="left" showArrow>
        {label}
      </Tooltip.Content>
    </Tooltip>
  );
}

interface SelectionCheckboxProps {
  label: string;
  isSelected: boolean;
  isDisabled?: boolean;
  onChange: () => void;
  className?: string;
}

/**
 * 一覧選択用のチェックボックスを表示する
 */
function SelectionCheckbox({
  label,
  isSelected,
  isDisabled = false,
  onChange,
  className,
}: SelectionCheckboxProps): ReactElement {
  return (
    <Checkbox
      aria-label={label}
      className={className}
      isDisabled={isDisabled}
      isSelected={isSelected}
      onChange={onChange}
    >
      <Checkbox.Control>
        <Checkbox.Indicator />
      </Checkbox.Control>
    </Checkbox>
  );
}

/**
 * 有効化された要素の表示高さを監視する
 */
function useMeasuredElementHeight<T extends HTMLElement>(
  isEnabled: boolean,
): [RefObject<T | null>, number] {
  const elementRef = useRef<T | null>(null);
  const [height, setHeight] = useState<number>(0);

  useEffect(() => {
    if (!isEnabled || elementRef.current === null) {
      setHeight(0);
      return;
    }

    const element = elementRef.current;
    const updateHeight = (): void => {
      const nextHeight = Math.ceil(element.getBoundingClientRect().height);
      setHeight((current) => (current === nextHeight ? current : nextHeight));
    };

    updateHeight();

    const resizeObserver = new ResizeObserver(updateHeight);
    resizeObserver.observe(element);
    window.addEventListener("resize", updateHeight);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateHeight);
    };
  }, [isEnabled]);

  return [elementRef, height];
}

/**
 * 現在のビューポートスクロール位置を記録する
 */
function createViewportScrollSnapshot(): ViewportScrollSnapshot {
  return {
    left: window.scrollX,
    top: window.scrollY,
  };
}

/**
 * 記録済みのビューポートスクロール位置へ復元する
 */
function restoreViewportScroll(snapshot: ViewportScrollSnapshot): void {
  window.scrollTo(snapshot.left, Math.min(snapshot.top, maximumViewportScrollTop()));
}

/**
 * 現在のドキュメントで指定可能な最大スクロール位置を算出する
 */
function maximumViewportScrollTop(): number {
  const scrollHeight = Math.max(document.documentElement.scrollHeight, document.body.scrollHeight);

  return Math.max(0, scrollHeight - window.innerHeight);
}

interface DashboardViewProps {
  dashboard: DashboardState | null;
  hasCompletedInitialLoad: boolean;
  filteredTunnels: TunnelView[];
  hasActiveFilters: boolean;
  selectedIds: Set<string>;
  selectedCount: number;
  selectedVisibleCount: number;
  availableTags: string[];
  operationProgress: OperationProgress | null;
  queryInput: string;
  searchInputRef: RefObject<HTMLInputElement | null>;
  filters: TunnelFilters;
  displayMode: TunnelDisplayMode;
  isBusy: boolean;
  onQueryInputChange: (value: string) => void;
  onFilterChange: <K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]) => void;
  onToggleTag: (tag: string) => void;
  onResetFilters: () => void;
  onDisplayModeChange: (mode: TunnelDisplayMode) => void;
  onClearSelection: () => void;
  onSelectVisible: () => void;
  onDeselectVisible: () => void;
  onToggleSelection: (id: string) => void;
  onStartSelected: () => void;
  onStopSelected: () => void;
  onStartTunnel: (id: string) => void;
  onStopTunnel: (id: string) => void;
  onStopTracked: (target: OperationTarget) => void;
  onEditTunnel: (tunnel: TunnelView) => void;
  onRemoveTunnel: (tunnel: TunnelView) => void;
  onAddTunnel: () => void;
}

/**
 * 運用対象の一覧と実行操作を表示する
 */
function DashboardView({
  dashboard,
  hasCompletedInitialLoad,
  filteredTunnels,
  hasActiveFilters,
  selectedIds,
  selectedCount,
  selectedVisibleCount,
  availableTags,
  operationProgress,
  queryInput,
  searchInputRef,
  filters,
  displayMode,
  isBusy,
  onQueryInputChange,
  onFilterChange,
  onToggleTag,
  onResetFilters,
  onDisplayModeChange,
  onClearSelection,
  onSelectVisible,
  onDeselectVisible,
  onToggleSelection,
  onStartSelected,
  onStopSelected,
  onStartTunnel,
  onStopTunnel,
  onStopTracked,
  onEditTunnel,
  onRemoveTunnel,
  onAddTunnel,
}: DashboardViewProps): ReactElement {
  const [isTrackedPanelCollapsed, setIsTrackedPanelCollapsed] = useState<boolean>(true);
  const hasTrackedRuntime = (dashboard?.trackedTunnels.length ?? 0) > 0;
  const hasSelection = selectedCount > 0;
  const shouldShowSelectionActionBar = hasSelection || filteredTunnels.length > 0;
  const [selectionBarRef, selectionBarHeight] = useMeasuredElementHeight<HTMLDivElement>(
    shouldShowSelectionActionBar,
  );
  const [trackedPanelRef, trackedPanelHeight] =
    useMeasuredElementHeight<HTMLDivElement>(hasTrackedRuntime);
  const bottomPaddingPixels = dashboardBottomPaddingPixels(trackedPanelHeight, selectionBarHeight);

  return (
    <section className="flex min-w-0 flex-col gap-3" style={{ paddingBottom: bottomPaddingPixels }}>
      <ValidationPanel dashboard={dashboard} />
      <TunnelOperationsPanel
        totalCount={dashboard?.tunnels.length ?? 0}
        visibleCount={filteredTunnels.length}
        availableTags={availableTags}
        queryInput={queryInput}
        searchInputRef={searchInputRef}
        filters={filters}
        displayMode={displayMode}
        hasActiveFilters={hasActiveFilters}
        onQueryInputChange={onQueryInputChange}
        onFilterChange={onFilterChange}
        onToggleTag={onToggleTag}
        onResetFilters={onResetFilters}
        onDisplayModeChange={onDisplayModeChange}
      />
      <SelectionActionBar
        isVisible={shouldShowSelectionActionBar}
        selectedCount={selectedCount}
        visibleCount={filteredTunnels.length}
        selectedVisibleCount={selectedVisibleCount}
        trackedPanelHeight={trackedPanelHeight}
        panelRef={selectionBarRef}
        operationProgress={operationProgress}
        isBusy={isBusy}
        onSelectVisible={onSelectVisible}
        onDeselectVisible={onDeselectVisible}
        onStart={onStartSelected}
        onStop={onStopSelected}
        onClear={onClearSelection}
      />
      <TunnelDeck
        dashboard={dashboard}
        hasCompletedInitialLoad={hasCompletedInitialLoad}
        tunnels={filteredTunnels}
        filters={filters}
        displayMode={displayMode}
        hasActiveFilters={hasActiveFilters}
        selectedIds={selectedIds}
        isBusy={isBusy}
        onToggle={onToggleSelection}
        onStart={onStartTunnel}
        onStop={onStopTunnel}
        onEdit={onEditTunnel}
        onRemove={onRemoveTunnel}
        onAddTunnel={onAddTunnel}
        onResetFilters={onResetFilters}
      />
      <TrackedPanel
        dashboard={dashboard}
        isCollapsed={isTrackedPanelCollapsed}
        panelRef={trackedPanelRef}
        isBusy={isBusy}
        onToggleCollapsed={() => setIsTrackedPanelCollapsed((current) => !current)}
        onStop={onStopTracked}
      />
    </section>
  );
}

interface AddTunnelViewProps {
  form: TunnelFormState;
  feedback: AppMessage | null;
  canUseLocal: boolean;
  isBusy: boolean;
  onChange: (field: keyof TunnelFormState, value: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onOpenSettings: () => void;
  onBrowseIdentityFile: () => void;
}

/**
 * トンネル追加専用の入力画面を表示する
 */
function AddTunnelView({
  form,
  feedback,
  canUseLocal,
  isBusy,
  onChange,
  onSubmit,
  onOpenSettings,
  onBrowseIdentityFile,
}: AddTunnelViewProps): ReactElement {
  return (
    <section className="mx-auto flex w-full max-w-6xl flex-col gap-3">
      <TunnelForm
        form={form}
        feedback={feedback}
        canUseLocal={canUseLocal}
        isBusy={isBusy}
        onChange={onChange}
        onSubmit={onSubmit}
        onOpenSettings={onOpenSettings}
        onBrowseIdentityFile={onBrowseIdentityFile}
      />
    </section>
  );
}

interface SettingsModalProps {
  isOpen: boolean;
  paths: WorkspaceSelection;
  isBusy: boolean;
  onChange: (field: keyof WorkspaceSelection, value: string | boolean) => void;
  onApply: () => void;
  onCancel: () => void;
  onBrowseWorkspace: () => void;
  onBrowseGlobalConfig: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onRemoveWorkspace: (workspacePath: string) => void;
}

/**
 * 設定ファイルと状態ファイルの参照先をモーダルで編集する
 */
function SettingsModal({
  isOpen,
  paths,
  isBusy,
  onChange,
  onApply,
  onCancel,
  onBrowseWorkspace,
  onBrowseGlobalConfig,
  onSelectWorkspace,
  onRemoveWorkspace,
}: SettingsModalProps): ReactElement | null {
  if (!isOpen) {
    return null;
  }

  return (
    <Modal
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open && !isBusy) {
          onCancel();
        }
      }}
    >
      <Modal.Backdrop variant="blur" isDismissable={!isBusy}>
        <Modal.Container placement="center" scroll="inside" size="lg" className="px-4 py-4 sm:px-6">
          <Modal.Dialog className="w-full max-w-5xl overflow-hidden p-0">
            <div className="flex items-center justify-between gap-4 border-b border-border px-5 py-4">
              <div className="flex min-w-0 items-center gap-2">
                <span className="text-foreground/70">
                  <Settings2 size={18} />
                </span>
                <h2 className="text-base font-semibold">Settings</h2>
              </div>
              <HeroButton
                type="button"
                slot="close"
                variant="ghost"
                size="sm"
                isIconOnly
                isDisabled={isBusy}
                aria-label="設定を閉じる"
              >
                <X size={17} />
              </HeroButton>
            </div>

            <div className="max-h-[calc(100vh-13rem)] overflow-y-auto bg-muted/25 px-5 py-4">
              <PathPanel
                paths={paths}
                isBusy={isBusy}
                onChange={onChange}
                onBrowseWorkspace={onBrowseWorkspace}
                onBrowseGlobalConfig={onBrowseGlobalConfig}
                onSelectWorkspace={onSelectWorkspace}
                onRemoveWorkspace={onRemoveWorkspace}
              />
            </div>

            <div className="flex justify-end gap-2 border-t border-border px-5 py-4">
              <HeroButton type="button" slot="close" variant="ghost" size="sm" isDisabled={isBusy}>
                Cancel
              </HeroButton>
              <HeroButton
                type="button"
                variant="primary"
                size="sm"
                onPress={onApply}
                isDisabled={isBusy}
              >
                {isBusy ? <Loader2 className="animate-spin" size={16} /> : <RefreshCw size={16} />}
                Apply changes
              </HeroButton>
            </div>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}

interface PathPanelProps {
  paths: WorkspaceSelection;
  isBusy: boolean;
  onChange: (field: keyof WorkspaceSelection, value: string | boolean) => void;
  onBrowseWorkspace: () => void;
  onBrowseGlobalConfig: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onRemoveWorkspace: (workspacePath: string) => void;
}

/**
 * 設定ファイルと状態ファイルの参照先を表示する
 */
function PathPanel({
  paths,
  isBusy,
  onChange,
  onBrowseWorkspace,
  onBrowseGlobalConfig,
  onSelectWorkspace,
  onRemoveWorkspace,
}: PathPanelProps): ReactElement {
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <Card variant="secondary" className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <FolderOpen className="text-foreground/70" size={17} />
          <h3 className="text-sm font-bold">Workspace</h3>
        </div>
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
          <TextField
            label="Workspace directory"
            value={paths.workspacePath}
            onChange={(value) => onChange("workspacePath", value)}
          />
          <HeroButton
            type="button"
            variant="outline"
            size="sm"
            className="mb-0"
            onPress={onBrowseWorkspace}
            isDisabled={isBusy}
          >
            <FolderOpen size={15} />
            Browse
          </HeroButton>
        </div>
        {paths.workspaceHistory.length > 0 ? (
          <div className="rounded-lg border border-border bg-muted/35 px-3 py-2">
            <div className="mb-2 text-xs font-medium text-muted-foreground">Recent workspaces</div>
            <div className="flex max-h-36 flex-col gap-1 overflow-y-auto">
              {paths.workspaceHistory.map((workspacePath) => (
                <div
                  key={workspacePath}
                  className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1"
                >
                  <HeroButton
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="min-h-8 min-w-0 justify-start font-mono text-xs"
                    onPress={() => onSelectWorkspace(workspacePath)}
                    isDisabled={isBusy}
                  >
                    <span className="truncate">{workspacePath}</span>
                  </HeroButton>
                  <HeroButton
                    type="button"
                    variant="danger-soft"
                    size="sm"
                    isIconOnly
                    onPress={() => onRemoveWorkspace(workspacePath)}
                    isDisabled={isBusy}
                    aria-label="履歴から削除"
                  >
                    <Trash2 size={14} />
                  </HeroButton>
                </div>
              ))}
            </div>
          </div>
        ) : null}
      </Card>

      <Card variant="secondary" className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <Settings2 className="text-foreground/70" size={17} />
          <h3 className="text-sm font-bold">Configuration files</h3>
        </div>
        <PathValue label="Local config" value={paths.localConfigPath} />
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
          <TextField
            label="Global config"
            value={paths.globalConfigPath}
            onChange={(value) => onChange("globalConfigPath", value)}
            disabled={!paths.useGlobal}
          />
          <HeroButton
            type="button"
            variant="outline"
            size="sm"
            className="mb-0"
            onPress={onBrowseGlobalConfig}
            isDisabled={isBusy || !paths.useGlobal}
          >
            <FolderOpen size={15} />
            Browse
          </HeroButton>
        </div>
        <div className="rounded-lg border border-border bg-muted/35 px-3 py-2">
          <Switch
            size="sm"
            isSelected={paths.useGlobal}
            onChange={(selected) => onChange("useGlobal", selected)}
            className="w-full justify-between"
          >
            <Switch.Content>
              <span className="text-sm font-semibold">Use global config</span>
            </Switch.Content>
            <Switch.Control>
              <Switch.Thumb />
            </Switch.Control>
          </Switch>
        </div>
      </Card>

      <Card variant="secondary" className="flex flex-col gap-3 p-4 lg:col-span-2">
        <div className="flex items-center gap-2">
          <Settings2 className="text-foreground/70" size={17} />
          <h3 className="text-sm font-bold">Application</h3>
        </div>
        <div className="rounded-lg border border-border bg-muted/35 px-3 py-2">
          <Switch
            size="sm"
            isSelected={paths.hideDockIconWhenWindowHidden}
            onChange={(selected) => onChange("hideDockIconWhenWindowHidden", selected)}
            className="w-full justify-between"
          >
            <Switch.Content>
              <span className="text-sm font-semibold">Hide Dock icon while window is hidden</span>
            </Switch.Content>
            <Switch.Control>
              <Switch.Thumb />
            </Switch.Control>
          </Switch>
        </div>
        <div className="rounded-lg border border-border bg-muted/35 px-3 py-2">
          <Switch
            size="sm"
            isSelected={paths.autoStopTunnelsOnQuit}
            onChange={(selected) => onChange("autoStopTunnelsOnQuit", selected)}
            className="w-full justify-between"
          >
            <Switch.Content>
              <span className="text-sm font-semibold">Auto-stop tunnels on quit</span>
            </Switch.Content>
            <Switch.Control>
              <Switch.Thumb />
            </Switch.Control>
          </Switch>
        </div>
      </Card>

      <Card variant="secondary" className="flex flex-col gap-3 p-4 lg:col-span-2">
        <div className="flex items-center gap-2">
          <Activity className="text-foreground/70" size={17} />
          <h3 className="text-sm font-bold">Runtime state</h3>
        </div>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          <PathValue label="Global state" value={paths.globalStatePath} />
          <PathValue label="Workspace state" value={paths.workspaceStatePath} />
        </div>
      </Card>
    </div>
  );
}

interface PathValueProps {
  label: string;
  value: string;
}

/**
 * 読み取り専用パスを表示する
 */
function PathValue({ label, value }: PathValueProps): ReactElement {
  return (
    <div className="rounded-lg border border-border bg-muted/35 px-3 py-2">
      <div className="text-xs font-semibold text-foreground/60">{label}</div>
      <div className="mt-1 min-h-5 truncate font-mono text-xs text-foreground/85" title={value}>
        {value || "Not selected"}
      </div>
    </div>
  );
}

interface ValidationPanelProps {
  dashboard: DashboardState | null;
}

/**
 * 設定検証の結果を表示する
 */
function ValidationPanel({ dashboard }: ValidationPanelProps): ReactElement | null {
  if (dashboard === null) {
    return null;
  }

  if (!dashboard.hasConfig) {
    return (
      <AlertMessage kind="warning">
        設定ファイルが見つかりません。Add tunnel から local または global 設定を作成できます。
      </AlertMessage>
    );
  }

  if (dashboard.validation.errors.length === 0 && dashboard.validation.warnings.length === 0) {
    return null;
  }

  return (
    <section className="flex flex-col gap-2">
      {dashboard.validation.errors.map((issue) => (
        <IssueRow
          key={`${issue.path}:${issue.tunnelId ?? "file"}:${issue.message}`}
          issue={issue}
          kind="error"
        />
      ))}
      {dashboard.validation.warnings.map((issue) => (
        <IssueRow
          key={`${issue.path}:${issue.tunnelId ?? "file"}:${issue.message}`}
          issue={issue}
          kind="warning"
        />
      ))}
    </section>
  );
}

interface AlertMessageProps {
  kind: "success" | "warning" | "error" | "info";
  children: ReactNode;
}

/**
 * 通知メッセージを状態別の視認性で表示する
 */
function AlertMessage({ kind, children }: AlertMessageProps): ReactElement {
  return (
    <Alert
      status={alertStatus(kind)}
      className="rounded-md"
      role={kind === "error" ? "alert" : "status"}
    >
      <Alert.Indicator>{alertIcon(kind, 18)}</Alert.Indicator>
      <Alert.Content>{children}</Alert.Content>
    </Alert>
  );
}

interface IssueRowProps {
  issue: ValidationIssueView;
  kind: "error" | "warning";
}

/**
 * 設定検証の 1 件を表示する
 */
function IssueRow({ issue, kind }: IssueRowProps): ReactElement {
  return (
    <AlertMessage kind={kind}>
      {issue.tunnelId ? `${issue.tunnelId}: ` : ""}
      {issue.message}
    </AlertMessage>
  );
}

interface TunnelOperationsPanelProps {
  totalCount: number;
  visibleCount: number;
  availableTags: string[];
  queryInput: string;
  searchInputRef: RefObject<HTMLInputElement | null>;
  filters: TunnelFilters;
  displayMode: TunnelDisplayMode;
  hasActiveFilters: boolean;
  onQueryInputChange: (value: string) => void;
  onFilterChange: <K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]) => void;
  onToggleTag: (tag: string) => void;
  onResetFilters: () => void;
  onDisplayModeChange: (mode: TunnelDisplayMode) => void;
}

/**
 * 一覧の絞り込み条件を表示する
 */
function TunnelOperationsPanel({
  totalCount,
  visibleCount,
  availableTags,
  queryInput,
  searchInputRef,
  filters,
  displayMode,
  hasActiveFilters,
  onQueryInputChange,
  onFilterChange,
  onToggleTag,
  onResetFilters,
  onDisplayModeChange,
}: TunnelOperationsPanelProps): ReactElement {
  const visibleTags = orderTagsBySelection(availableTags, filters.tags);

  return (
    <section className="rounded-xl border border-border bg-card shadow-sm">
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex flex-col gap-2 border-b border-border px-3 py-2.5 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <ListFilter className="text-foreground/70" size={16} />
              <h2 className="text-sm leading-5 font-semibold">Tunnels</h2>
              <Chip size="sm" variant="secondary">
                {visibleCount} / {totalCount}
              </Chip>
            </div>
          </div>
          <div className="flex flex-col gap-2 self-start sm:flex-row sm:items-center lg:self-auto">
            <TunnelDisplayModeControl
              displayMode={displayMode}
              onDisplayModeChange={onDisplayModeChange}
            />
            <HeroButton
              type="button"
              variant="ghost"
              size="sm"
              onPress={onResetFilters}
              isDisabled={!hasActiveFilters}
            >
              Reset filters
            </HeroButton>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-2 px-3 pb-3 lg:grid-cols-[minmax(16rem,1fr)_auto_auto] lg:items-center">
          <HeroTextField className="w-full" variant="secondary">
            <HeroLabel className="sr-only">Search tunnels</HeroLabel>
            <div className="relative">
              <Search
                className="pointer-events-none absolute top-1/2 left-3 z-10 -translate-y-1/2 text-foreground/40"
                size={16}
              />
              <HeroInput
                ref={searchInputRef}
                fullWidth
                className="h-9 w-full pr-9 pl-9"
                variant="secondary"
                value={queryInput}
                onChange={(event: ChangeEvent<HTMLInputElement>) =>
                  onQueryInputChange(event.target.value)
                }
                placeholder="Search ID, tag, endpoint"
                aria-label="Search tunnels"
              />
              {queryInput.length > 0 ? (
                <HeroButton
                  type="button"
                  variant="ghost"
                  size="sm"
                  isIconOnly
                  className="absolute top-1/2 right-1 z-10 -translate-y-1/2"
                  onPress={() => onQueryInputChange("")}
                  aria-label="検索条件を消去"
                >
                  <X size={14} />
                </HeroButton>
              ) : null}
            </div>
          </HeroTextField>

          <div className="grid grid-cols-4 gap-0.5 rounded-lg border border-border bg-muted p-0.5">
            {statusFilterOptions.map((option) => (
              <HeroButton
                key={option.value}
                type="button"
                variant={filters.status === option.value ? "primary" : "ghost"}
                size="sm"
                fullWidth
                onPress={() => onFilterChange("status", option.value)}
                aria-pressed={filters.status === option.value}
                className="min-w-0 justify-center"
              >
                {option.label}
              </HeroButton>
            ))}
          </div>

          <div className="grid grid-cols-3 gap-0.5 rounded-lg border border-border bg-muted p-0.5">
            {scopeFilterOptions.map((option) => (
              <HeroButton
                key={option.value}
                type="button"
                variant={filters.scope === option.value ? "primary" : "ghost"}
                size="sm"
                fullWidth
                onPress={() => onFilterChange("scope", option.value)}
                aria-pressed={filters.scope === option.value}
                className="min-w-0 justify-center"
              >
                {option.label}
              </HeroButton>
            ))}
          </div>
        </div>

        <ActiveFilterChips
          queryInput={queryInput}
          filters={filters}
          onQueryInputChange={onQueryInputChange}
          onFilterChange={onFilterChange}
          onToggleTag={onToggleTag}
        />

        {availableTags.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5 border-t border-border px-3 py-2">
            <span className="text-xs font-medium text-muted-foreground">Tags</span>
            {visibleTags.map((tag) => {
              const selected = filters.tags.includes(tag);
              return (
                <HeroButton
                  key={tag}
                  type="button"
                  variant={selected ? "primary" : "outline"}
                  size="sm"
                  onPress={() => onToggleTag(tag)}
                  aria-pressed={selected}
                  className="min-h-7 rounded-full px-3"
                >
                  {tag}
                </HeroButton>
              );
            })}
          </div>
        ) : null}
      </div>
    </section>
  );
}

interface TunnelDisplayModeControlProps {
  displayMode: TunnelDisplayMode;
  onDisplayModeChange: (mode: TunnelDisplayMode) => void;
}

/**
 * 一覧表示モードの切り替え操作を表示する
 */
function TunnelDisplayModeControl({
  displayMode,
  onDisplayModeChange,
}: TunnelDisplayModeControlProps): ReactElement {
  return (
    <div
      className="grid w-full grid-cols-2 gap-0.5 rounded-lg border border-border bg-muted p-0.5 sm:w-auto"
      aria-label="トンネル一覧の表示形式"
    >
      <HeroButton
        type="button"
        variant={displayMode === "slim" ? "primary" : "ghost"}
        size="sm"
        fullWidth
        onPress={() => onDisplayModeChange("slim")}
        aria-pressed={displayMode === "slim"}
        className="min-w-0 justify-center"
      >
        <Rows3 size={14} />
        Slim
      </HeroButton>
      <HeroButton
        type="button"
        variant={displayMode === "card" ? "primary" : "ghost"}
        size="sm"
        fullWidth
        onPress={() => onDisplayModeChange("card")}
        aria-pressed={displayMode === "card"}
        className="min-w-0 justify-center"
      >
        <LayoutGrid size={14} />
        Cards
      </HeroButton>
    </div>
  );
}

interface ActiveFilterChipsProps {
  queryInput: string;
  filters: TunnelFilters;
  onQueryInputChange: (value: string) => void;
  onFilterChange: <K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]) => void;
  onToggleTag: (tag: string) => void;
}

/**
 * 有効な絞り込み条件を解除可能なチップとして表示する
 */
function ActiveFilterChips({
  queryInput,
  filters,
  onQueryInputChange,
  onFilterChange,
  onToggleTag,
}: ActiveFilterChipsProps): ReactElement | null {
  const query = queryInput.trim();
  const hasStatusFilter = filters.status !== initialFilters.status;
  const hasScopeFilter = filters.scope !== initialFilters.scope;
  const hasTagFilters = filters.tags.length > 0;

  if (query.length === 0 && !hasStatusFilter && !hasScopeFilter && !hasTagFilters) {
    return null;
  }

  return (
    <div className="mx-3 flex flex-wrap items-center gap-1.5 rounded-lg border border-border bg-muted/50 px-2 py-1.5">
      <span className="text-xs font-medium text-muted-foreground">Active</span>
      {query.length > 0 ? (
        <FilterChip label={`query: ${query}`} onRemove={() => onQueryInputChange("")} />
      ) : null}
      {hasStatusFilter ? (
        <FilterChip
          label={`status: ${filters.status}`}
          onRemove={() => onFilterChange("status", initialFilters.status)}
        />
      ) : null}
      {hasScopeFilter ? (
        <FilterChip
          label={`scope: ${filters.scope}`}
          onRemove={() => onFilterChange("scope", initialFilters.scope)}
        />
      ) : null}
      {filters.tags.map((tag) => (
        <FilterChip key={tag} label={`tag: ${tag}`} onRemove={() => onToggleTag(tag)} />
      ))}
    </div>
  );
}

interface FilterChipProps {
  label: string;
  onRemove: () => void;
}

/**
 * 個別解除できる絞り込み条件を表示する
 */
function FilterChip({ label, onRemove }: FilterChipProps): ReactElement {
  return (
    <HeroButton
      type="button"
      variant="outline"
      size="sm"
      className="max-w-full rounded-full"
      onPress={onRemove}
      aria-label={`${label} を解除`}
    >
      <span className="max-w-52 truncate">{label}</span>
      <X size={12} />
    </HeroButton>
  );
}

interface SelectionActionBarProps {
  isVisible: boolean;
  selectedCount: number;
  visibleCount: number;
  selectedVisibleCount: number;
  trackedPanelHeight: number;
  panelRef: RefObject<HTMLDivElement | null>;
  operationProgress: OperationProgress | null;
  isBusy: boolean;
  onSelectVisible: () => void;
  onDeselectVisible: () => void;
  onStart: () => void;
  onStop: () => void;
  onClear: () => void;
}

/**
 * 表示中または選択中のトンネルに対する一括操作を横長バーで表示する
 */
function SelectionActionBar({
  isVisible,
  selectedCount,
  visibleCount,
  selectedVisibleCount,
  trackedPanelHeight,
  panelRef,
  operationProgress,
  isBusy,
  onSelectVisible,
  onDeselectVisible,
  onStart,
  onStop,
  onClear,
}: SelectionActionBarProps): ReactElement | null {
  if (!isVisible) {
    return null;
  }

  const hiddenSelectedCount = selectedCount - selectedVisibleCount;
  const bottomPixels = selectionActionBarBottomPixels(trackedPanelHeight);
  const selectedInViewLabel =
    selectedCount === 0
      ? `${visibleCount} visible results`
      : `${selectedVisibleCount} in current view`;

  return (
    <section
      className="pointer-events-none fixed right-4 left-4 z-50"
      style={{ bottom: bottomPixels }}
      aria-live="polite"
    >
      <div
        ref={panelRef}
        className="pointer-events-auto mx-auto flex max-h-[min(16rem,calc(100vh-2rem))] w-full max-w-[96rem] flex-col gap-2 overflow-auto rounded-xl border border-border bg-card/95 px-3 py-2 shadow-2xl backdrop-blur xl:flex-row xl:items-center xl:justify-between"
      >
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <CheckCircle2 className="text-foreground/70" size={16} />
            <h2 className="text-sm leading-6 font-bold">Bulk actions</h2>
            <Chip color="accent" size="sm" variant="primary">
              {selectedCount} total selected
            </Chip>
            <Chip size="sm" variant="secondary">
              {selectedInViewLabel}
            </Chip>
            {hiddenSelectedCount > 0 ? (
              <Chip color="warning" size="sm" variant="soft">
                {hiddenSelectedCount} hidden by filters included
              </Chip>
            ) : null}
          </div>
        </div>

        <div className="flex flex-col gap-2 xl:flex-row xl:items-center xl:justify-end">
          <SelectionOperationProgress progress={operationProgress} />

          <div className="grid grid-cols-2 gap-2 xl:grid-cols-[repeat(2,max-content)]">
            <HeroButton
              type="button"
              variant="primary"
              size="sm"
              onPress={onStart}
              isDisabled={isBusy || selectedCount === 0}
            >
              <Play size={16} />
              Start selected
            </HeroButton>
            <HeroButton
              type="button"
              variant="danger"
              size="sm"
              onPress={onStop}
              isDisabled={isBusy || selectedCount === 0}
            >
              <CircleStop size={16} />
              Stop selected
            </HeroButton>
          </div>

          <div className="grid grid-cols-1 gap-2 sm:grid-cols-3 xl:grid-cols-[repeat(3,max-content)]">
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              onPress={onSelectVisible}
              isDisabled={isBusy || visibleCount === 0 || selectedVisibleCount === visibleCount}
            >
              <CheckCircle2 size={16} />
              Select all visible
            </HeroButton>
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              onPress={onDeselectVisible}
              isDisabled={isBusy || selectedVisibleCount === 0}
            >
              <Minus size={16} />
              Deselect visible
            </HeroButton>
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              onPress={onClear}
              isDisabled={isBusy || selectedCount === 0}
            >
              <X size={16} />
              Clear all
            </HeroButton>
          </div>
        </div>
      </div>
    </section>
  );
}

/**
 * 固定表示される下部パネルに応じた一覧末尾の余白を算出する
 */
function dashboardBottomPaddingPixels(
  trackedPanelHeight: number,
  selectionBarHeight: number,
): number {
  const viewportMarginPixels = 16;
  const floatingPanelGapPixels = 12;
  const contentMarginPixels = 24;
  const hasTrackedPanel = trackedPanelHeight > 0;
  const hasSelectionBar = selectionBarHeight > 0;
  const interPanelGapPixels = hasTrackedPanel && hasSelectionBar ? floatingPanelGapPixels : 0;
  const floatingPanelsHeight = trackedPanelHeight + selectionBarHeight + interPanelGapPixels;

  if (floatingPanelsHeight === 0) {
    return 0;
  }

  return viewportMarginPixels + floatingPanelsHeight + contentMarginPixels;
}

/**
 * tracked runtime パネルとの重なりを避けるための固定位置を算出する
 */
function selectionActionBarBottomPixels(trackedPanelHeight: number): number {
  const viewportMarginPixels = 16;
  const floatingPanelGapPixels = 12;

  if (trackedPanelHeight === 0) {
    return viewportMarginPixels;
  }

  return viewportMarginPixels + trackedPanelHeight + floatingPanelGapPixels;
}

interface MessagePanelProps {
  message: AppMessage | null;
}

/**
 * ページ全体の状態メッセージを表示する
 */
function MessagePanel({ message }: MessagePanelProps): ReactElement | null {
  if (message === null) {
    return null;
  }

  const kind = message.kind === "error" ? "error" : message.kind === "success" ? "success" : "info";
  return <AlertMessage kind={kind}>{message.text}</AlertMessage>;
}

interface SelectionOperationProgressProps {
  progress: OperationProgress | null;
}

/**
 * Selection バー内に一括操作の進行中状態を表示する
 */
function SelectionOperationProgress({
  progress,
}: SelectionOperationProgressProps): ReactElement | null {
  if (progress === null) {
    return null;
  }

  const label = operationProgressLabel(progress);

  return (
    <div className="min-w-0 rounded-lg border border-border bg-muted/50 px-3 py-2 xl:w-64">
      <div className="flex min-w-0 items-center gap-2">
        <Loader2 className="shrink-0 animate-spin text-primary" size={16} />
        <span className="truncate text-xs font-semibold text-foreground">{label}</span>
      </div>
      <ProgressBar
        aria-label={label}
        className="mt-2 w-full"
        color="accent"
        maxValue={progress.totalCount}
        size="sm"
        value={progress.completedCount}
      >
        <ProgressBar.Track>
          <ProgressBar.Fill />
        </ProgressBar.Track>
      </ProgressBar>
    </div>
  );
}

interface ToastViewportProps {
  toast: OperationToastMessage | null;
  onDismiss: () => void;
}

/**
 * 操作結果トーストの固定表示領域を描画する
 */
function ToastViewport({ toast, onDismiss }: ToastViewportProps): ReactElement | null {
  if (toast === null) {
    return null;
  }

  return (
    <section className="pointer-events-none fixed top-4 right-4 left-4 z-[60] sm:left-auto sm:w-[30rem]">
      <OperationToast toast={toast} onDismiss={onDismiss} />
    </section>
  );
}

interface OperationToastProps {
  toast: OperationToastMessage;
  onDismiss: () => void;
}

/**
 * 操作結果の要約と詳細をトーストとして表示する
 */
function OperationToast({ toast, onDismiss }: OperationToastProps): ReactElement {
  return (
    <Alert
      status={alertStatus(toast.kind)}
      className="pointer-events-auto max-h-[min(22rem,calc(100vh-2rem))] w-full overflow-hidden shadow-lg"
      role={toast.kind === "error" ? "alert" : "status"}
    >
      <Alert.Indicator>{alertIcon(toast.kind, 20)}</Alert.Indicator>
      <Alert.Content className="min-w-0 flex-1">
        <p className="[overflow-wrap:anywhere] leading-6 font-semibold">{toast.summary}</p>
        {toast.detail ? (
          <p className="mt-1 max-h-52 overflow-auto [overflow-wrap:anywhere] leading-6 whitespace-pre-wrap text-foreground/80">
            {toast.detail}
          </p>
        ) : null}
      </Alert.Content>
      <HeroButton
        type="button"
        variant="ghost"
        size="sm"
        isIconOnly
        className="-mt-1 -mr-2 shrink-0"
        onPress={onDismiss}
        aria-label="通知を閉じる"
      >
        <X size={14} />
      </HeroButton>
    </Alert>
  );
}

interface TunnelDeckProps {
  dashboard: DashboardState | null;
  hasCompletedInitialLoad: boolean;
  tunnels: TunnelView[];
  filters: TunnelFilters;
  displayMode: TunnelDisplayMode;
  hasActiveFilters: boolean;
  selectedIds: Set<string>;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onEdit: (tunnel: TunnelView) => void;
  onRemove: (tunnel: TunnelView) => void;
  onAddTunnel: () => void;
  onResetFilters: () => void;
}

/**
 * 設定済みトンネルのカード一覧を表示する
 */
function TunnelDeck({
  dashboard,
  hasCompletedInitialLoad,
  tunnels,
  filters,
  displayMode,
  hasActiveFilters,
  selectedIds,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onEdit,
  onRemove,
  onAddTunnel,
  onResetFilters,
}: TunnelDeckProps): ReactElement {
  if (dashboard === null) {
    if (hasCompletedInitialLoad) {
      return (
        <EmptyState title="Dashboard unavailable">
          アプリ実行環境または設定を確認してから再読み込みしてください。
        </EmptyState>
      );
    }

    return <EmptyState title="Loading tunnels">設定と実行状態を読み込んでいます。</EmptyState>;
  }

  if (dashboard.tunnels.length === 0) {
    return (
      <EmptyState
        title="No configured tunnels"
        action={
          <HeroButton type="button" variant="primary" size="sm" onPress={onAddTunnel}>
            <CirclePlus size={16} />
            Add tunnel
          </HeroButton>
        }
      >
        Add tunnel から新しい接続を追加できます。
      </EmptyState>
    );
  }

  if (tunnels.length === 0 && hasActiveFilters) {
    return (
      <EmptyState
        title="No matching tunnels"
        action={
          <HeroButton type="button" variant="outline" size="sm" onPress={onResetFilters}>
            <X size={16} />
            Reset filters
          </HeroButton>
        }
      >
        検索条件またはフィルターを変更してください。
      </EmptyState>
    );
  }

  if (displayMode === "slim") {
    return (
      <TunnelSlimList
        tunnels={tunnels}
        query={filters.query}
        selectedIds={selectedIds}
        isBusy={isBusy}
        onToggle={onToggle}
        onStart={onStart}
        onStop={onStop}
        onEdit={onEdit}
        onRemove={onRemove}
      />
    );
  }

  return (
    <section className="grid grid-cols-1 gap-4 lg:grid-cols-2">
      {tunnels.map((tunnel) => (
        <TunnelCard
          key={tunnel.id}
          tunnel={tunnel}
          query={filters.query}
          checked={selectedIds.has(tunnel.id)}
          isBusy={isBusy}
          onToggle={onToggle}
          onStart={onStart}
          onStop={onStop}
          onEdit={onEdit}
          onRemove={onRemove}
        />
      ))}
    </section>
  );
}

interface TunnelSlimListProps {
  tunnels: TunnelView[];
  query: string;
  selectedIds: Set<string>;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onEdit: (tunnel: TunnelView) => void;
  onRemove: (tunnel: TunnelView) => void;
}

/**
 * 設定済みトンネルを行単位のスリム一覧で表示する
 */
function TunnelSlimList({
  tunnels,
  query,
  selectedIds,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onEdit,
  onRemove,
}: TunnelSlimListProps): ReactElement {
  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm">
      <Table variant="secondary">
        <Table.ScrollContainer>
          <Table.Content aria-label="Configured tunnels" className="min-w-[64rem]">
            <Table.Header>
              <Table.Column className="w-12">Select</Table.Column>
              <Table.Column isRowHeader>ID</Table.Column>
              <Table.Column>Status</Table.Column>
              <Table.Column>Local</Table.Column>
              <Table.Column>Remote</Table.Column>
              <Table.Column>SSH</Table.Column>
              <Table.Column>Source</Table.Column>
              <Table.Column className="text-right">Actions</Table.Column>
            </Table.Header>
            <Table.Body>
              {tunnels.map((tunnel) => (
                <TunnelSlimRow
                  key={tunnel.id}
                  tunnel={tunnel}
                  query={query}
                  checked={selectedIds.has(tunnel.id)}
                  isBusy={isBusy}
                  onToggle={onToggle}
                  onStart={onStart}
                  onStop={onStop}
                  onEdit={onEdit}
                  onRemove={onRemove}
                />
              ))}
            </Table.Body>
          </Table.Content>
        </Table.ScrollContainer>
      </Table>
    </section>
  );
}

interface TunnelSlimRowProps {
  tunnel: TunnelView;
  query: string;
  checked: boolean;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onEdit: (tunnel: TunnelView) => void;
  onRemove: (tunnel: TunnelView) => void;
}

/**
 * スリム一覧内のトンネル 1 件を表示する
 */
function TunnelSlimRow({
  tunnel,
  query,
  checked,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onEdit,
  onRemove,
}: TunnelSlimRowProps): ReactElement {
  const running = tunnel.status?.state === "running";
  const status = tunnel.status?.state ?? "idle";
  const highlightQuery = query.trim();

  return (
    <Table.Row className={checked ? "bg-primary/5" : undefined} id={tunnel.id}>
      <Table.Cell>
        <SelectionCheckbox
          label={`${tunnel.id} を選択`}
          isSelected={checked}
          onChange={() => onToggle(tunnel.id)}
        />
      </Table.Cell>
      <Table.Cell className="max-w-56">
        <div className="truncate text-sm font-bold" title={tunnel.id}>
          <HighlightedText text={tunnel.id} query={highlightQuery} />
        </div>
      </Table.Cell>
      <Table.Cell>
        <StatusBadge status={status} />
      </Table.Cell>
      <Table.Cell className="max-w-44 truncate font-mono text-xs">
        <span title={tunnel.local}>
          <HighlightedText text={tunnel.local} query={highlightQuery} />
        </span>
      </Table.Cell>
      <Table.Cell className="max-w-44 truncate font-mono text-xs">
        <span title={tunnel.remote}>
          <HighlightedText text={tunnel.remote} query={highlightQuery} />
        </span>
      </Table.Cell>
      <Table.Cell className="max-w-52 truncate font-mono text-xs">
        <span title={tunnel.ssh}>
          <HighlightedText text={tunnel.ssh} query={highlightQuery} />
        </span>
      </Table.Cell>
      <Table.Cell className="font-semibold text-foreground/70">
        <HighlightedText text={tunnel.source} query={highlightQuery} />
      </Table.Cell>
      <Table.Cell>
        <div className="flex min-w-max items-center justify-end gap-1">
          <HeroButton
            type="button"
            variant={running ? "ghost" : "primary"}
            size="sm"
            onPress={() => onStart(tunnel.id)}
            isDisabled={isBusy || running}
          >
            <Play size={13} />
            Start
          </HeroButton>
          <HeroButton
            type="button"
            variant={running ? "danger" : "outline"}
            size="sm"
            onPress={() => onStop(tunnel.id)}
            isDisabled={isBusy || tunnel.status === null}
          >
            <CircleStop size={13} />
            Stop
          </HeroButton>
          <IconButton
            label="設定を編集"
            variant="ghost"
            onPress={() => onEdit(tunnel)}
            disabled={isBusy}
          >
            <Pencil size={14} />
          </IconButton>
          <IconButton
            label="設定から削除"
            variant="danger-soft"
            onPress={() => onRemove(tunnel)}
            disabled={isBusy}
          >
            <Trash2 size={14} />
          </IconButton>
        </div>
      </Table.Cell>
    </Table.Row>
  );
}

interface EmptyStateProps {
  title: string;
  children: ReactNode;
  action?: ReactNode;
}

/**
 * 空状態を表示する
 */
function EmptyState({ title, children, action }: EmptyStateProps): ReactElement {
  return (
    <section className="rounded-xl border border-dashed border-border bg-card/70 shadow-sm">
      <div className="flex min-h-40 flex-col items-center justify-center gap-2 px-5 py-8 text-center">
        <div className="rounded-lg bg-muted p-3 text-foreground/50">
          <ListFilter size={22} />
        </div>
        <h2 className="text-base font-bold">{title}</h2>
        <p className="max-w-md text-sm text-foreground/60">{children}</p>
        {action ? <div className="mt-2">{action}</div> : null}
      </div>
    </section>
  );
}

interface TunnelCardProps {
  tunnel: TunnelView;
  query: string;
  checked: boolean;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onEdit: (tunnel: TunnelView) => void;
  onRemove: (tunnel: TunnelView) => void;
}

/**
 * トンネル 1 件の操作カードを表示する
 */
function TunnelCard({
  tunnel,
  query,
  checked,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onEdit,
  onRemove,
}: TunnelCardProps): ReactElement {
  const running = tunnel.status?.state === "running";
  const status = tunnel.status?.state ?? "idle";
  const highlightQuery = query.trim();
  const statusBorderClassName =
    status === "running"
      ? "border-l-success"
      : status === "stale"
        ? "border-l-warning"
        : "border-l-default";

  return (
    <Card
      variant="secondary"
      className={`flex h-full flex-col border-l-4 transition ${statusBorderClassName} ${
        checked ? "bg-primary/5" : "bg-card"
      }`}
    >
      <div className="flex h-full flex-col gap-3 p-4">
        <div className="flex items-start justify-between gap-3">
          <Checkbox
            className="min-w-0 flex-1 items-start gap-3"
            isSelected={checked}
            onChange={() => onToggle(tunnel.id)}
          >
            <Checkbox.Control className="mt-1">
              <Checkbox.Indicator />
            </Checkbox.Control>
            <Checkbox.Content className="min-w-0">
              <span className="block truncate text-base leading-6 font-bold">
                <HighlightedText text={tunnel.id} query={highlightQuery} />
              </span>
              <span className="mt-0.5 block truncate text-xs text-foreground/50">
                <HighlightedText text={tunnel.sourcePath} query={highlightQuery} />
              </span>
            </Checkbox.Content>
          </Checkbox>
          <StatusBadge status={status} />
        </div>

        <p className="min-h-10 text-sm leading-5 text-foreground/60">
          <HighlightedText text={tunnel.description ?? "No description"} query={highlightQuery} />
        </p>

        <TagList tags={tunnel.tags} query={highlightQuery} />
        <EndpointList tunnel={tunnel} query={highlightQuery} />

        <div className="grid grid-cols-2 gap-2 text-xs xl:grid-cols-4">
          <MetaItem label="Source" value={tunnel.source} query={highlightQuery} />
          <MetaItem label="Runtime" value={tunnel.status ? `pid ${tunnel.status.pid}` : "none"} />
          <MetaItem label="Connect" value={`${tunnel.timeouts.connectTimeoutSeconds}s`} />
          <MetaItem label="Grace" value={`${tunnel.timeouts.startGraceMilliseconds}ms`} />
        </div>

        <div className="mt-auto flex items-center justify-end gap-2 pt-1">
          <HeroButton
            type="button"
            variant={running ? "ghost" : "primary"}
            size="sm"
            onPress={() => onStart(tunnel.id)}
            isDisabled={isBusy || running}
          >
            <Play size={15} />
            Start
          </HeroButton>
          <HeroButton
            type="button"
            variant={running ? "danger" : "outline"}
            size="sm"
            onPress={() => onStop(tunnel.id)}
            isDisabled={isBusy || tunnel.status === null}
          >
            <CircleStop size={15} />
            Stop
          </HeroButton>
          <IconButton
            label="設定を編集"
            variant="ghost"
            onPress={() => onEdit(tunnel)}
            disabled={isBusy}
          >
            <Pencil size={16} />
          </IconButton>
          <IconButton
            label="設定から削除"
            variant="danger-soft"
            onPress={() => onRemove(tunnel)}
            disabled={isBusy}
          >
            <Trash2 size={16} />
          </IconButton>
        </div>
      </div>
    </Card>
  );
}

interface MetaItemProps {
  label: string;
  value: string;
  query?: string;
}

/**
 * トンネルカード内の補助情報を一定幅で表示する
 */
function MetaItem({ label, value, query = "" }: MetaItemProps): ReactElement {
  return (
    <div className="min-w-0 rounded-lg border border-border bg-muted/35 px-2.5 py-2">
      <div className="font-semibold text-foreground/50">{label}</div>
      <div className="mt-1 truncate font-mono text-foreground/80" title={value}>
        <HighlightedText text={value} query={query} />
      </div>
    </div>
  );
}

interface StatusBadgeProps {
  status: TunnelStatus;
}

/**
 * トンネル状態のチップを表示する
 */
function StatusBadge({ status }: StatusBadgeProps): ReactElement {
  const color = status === "running" ? "success" : status === "stale" ? "warning" : "default";

  return (
    <Chip color={color} size="sm" variant={status === "idle" ? "secondary" : "soft"}>
      {status}
    </Chip>
  );
}

interface TagListProps {
  tags: string[];
  query: string;
}

/**
 * タグ一覧を表示する
 */
function TagList({ tags, query }: TagListProps): ReactElement {
  if (tags.length === 0) {
    return <div className="min-h-6 text-xs leading-6 text-foreground/50">No tags</div>;
  }

  return (
    <div className="flex min-h-6 flex-wrap items-center gap-1">
      {tags.map((tag) => (
        <Chip color="accent" key={tag} size="sm" variant="soft">
          <HighlightedText text={tag} query={query} />
        </Chip>
      ))}
    </div>
  );
}

interface EndpointListProps {
  tunnel: TunnelView;
  query: string;
}

/**
 * 接続先情報を表示する
 */
function EndpointList({ tunnel, query }: EndpointListProps): ReactElement {
  return (
    <div className="rounded-lg border border-border bg-muted/35 p-2.5">
      <div className="grid gap-2 xl:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto_minmax(0,1fr)] xl:items-center">
        <EndpointNode
          icon={<Server size={15} />}
          label="Local"
          value={tunnel.local}
          query={query}
        />
        <RouteConnector />
        <EndpointNode
          icon={<ArrowRight size={15} />}
          label="Remote"
          value={tunnel.remote}
          query={query}
        />
        <RouteConnector />
        <EndpointNode icon={<KeyRound size={15} />} label="SSH" value={tunnel.ssh} query={query} />
      </div>
    </div>
  );
}

interface EndpointNodeProps {
  icon: ReactNode;
  label: string;
  value: string;
  query?: string;
}

/**
 * 接続先情報の 1 区間を表示する
 */
function EndpointNode({ icon, label, value, query = "" }: EndpointNodeProps): ReactElement {
  return (
    <div className="min-w-0 rounded-lg border border-border bg-card px-2.5 py-2">
      <div className="flex items-center gap-2 text-xs font-semibold text-foreground/55">
        <span>{icon}</span>
        <span>{label}</span>
      </div>
      <div className="mt-1 truncate font-mono text-xs text-foreground/90" title={value}>
        <HighlightedText text={value} query={query} />
      </div>
    </div>
  );
}

interface HighlightedTextProps {
  text: string;
  query: string;
}

/**
 * 検索語に一致する部分へ強調表示を適用する
 */
function HighlightedText({ text, query }: HighlightedTextProps): ReactElement {
  const normalizedQuery = query.trim().toLowerCase();

  if (normalizedQuery.length === 0) {
    return <>{text}</>;
  }

  return (
    <>
      {splitTextBySearchQuery(text, normalizedQuery).map((part, index) =>
        part.isMatch ? (
          <mark
            key={`${part.text}:${index}`}
            className="rounded bg-warning/25 px-0.5 text-foreground"
          >
            {part.text}
          </mark>
        ) : (
          <span key={`${part.text}:${index}`}>{part.text}</span>
        ),
      )}
    </>
  );
}

interface RouteConnectorProps {
  horizontalAt?: "lg" | "xl";
}

/**
 * 接続経路の方向を表示する
 */
function RouteConnector({ horizontalAt = "xl" }: RouteConnectorProps): ReactElement {
  const verticalClassName = horizontalAt === "lg" ? "lg:hidden" : "xl:hidden";
  const horizontalClassName = horizontalAt === "lg" ? "hidden lg:block" : "hidden xl:block";

  return (
    <div className="flex items-center justify-center text-foreground/35 xl:w-5" aria-hidden="true">
      <div className={`h-3 border-l border-foreground/20 ${verticalClassName}`} />
      <ArrowRight className={horizontalClassName} size={15} />
    </div>
  );
}

interface TrackedPanelProps {
  dashboard: DashboardState | null;
  isCollapsed: boolean;
  panelRef: RefObject<HTMLDivElement | null>;
  isBusy: boolean;
  onToggleCollapsed: () => void;
  onStop: (target: OperationTarget) => void;
}

/**
 * 状態ファイルで追跡中のトンネルを表示する
 */
function TrackedPanel({
  dashboard,
  isCollapsed,
  panelRef,
  isBusy,
  onToggleCollapsed,
  onStop,
}: TrackedPanelProps): ReactElement | null {
  if (dashboard === null || dashboard.trackedTunnels.length === 0) {
    return null;
  }

  return (
    <section className="pointer-events-none fixed right-4 bottom-4 left-4 z-40">
      <div
        ref={panelRef}
        className="pointer-events-auto mx-auto w-full max-w-[96rem] overflow-hidden rounded-xl border border-border bg-card/95 shadow-2xl backdrop-blur"
      >
        <HeroButton
          type="button"
          variant="ghost"
          fullWidth
          className="h-auto justify-between gap-3 rounded-none bg-muted/35 px-3 py-2 text-left"
          onPress={onToggleCollapsed}
          aria-expanded={!isCollapsed}
        >
          <span className="flex min-w-0 items-center gap-2">
            <span className="rounded-md bg-muted p-1.5 text-foreground/70">
              <Activity className="shrink-0" size={16} />
            </span>
            <span className="truncate text-sm font-semibold">Tracked runtime</span>
            <Chip color="accent" size="sm" variant="soft">
              {dashboard.trackedTunnels.length}
            </Chip>
          </span>
          <span className="flex shrink-0 items-center gap-2 text-xs text-foreground/60">
            {isCollapsed ? "Show" : "Hide"}
            {isCollapsed ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
          </span>
        </HeroButton>

        {!isCollapsed ? (
          <div className="max-h-44 overflow-auto border-t border-border bg-card">
            <Table variant="secondary">
              <Table.ScrollContainer>
                <Table.Content aria-label="Tracked runtime tunnels">
                  <Table.Header>
                    <Table.Column isRowHeader>ID</Table.Column>
                    <Table.Column>Endpoint</Table.Column>
                    <Table.Column>Status</Table.Column>
                    <Table.Column className="text-right">Actions</Table.Column>
                  </Table.Header>
                  <Table.Body>
                    {dashboard.trackedTunnels.map((tracked) => (
                      <Table.Row id={tracked.runtimeKey} key={tracked.runtimeKey}>
                        <Table.Cell className="font-bold">
                          <div>{tracked.id}</div>
                          <div className="text-[0.65rem] font-normal text-foreground/50">
                            {tracked.runtimeScope}
                          </div>
                        </Table.Cell>
                        <Table.Cell className="max-w-md truncate font-mono text-xs">
                          {tracked.local} {" -> "} {tracked.remote}
                        </Table.Cell>
                        <Table.Cell>
                          <StatusBadge status={tracked.status.state} />
                        </Table.Cell>
                        <Table.Cell className="text-right">
                          <HeroButton
                            type="button"
                            variant="outline"
                            size="sm"
                            onPress={() =>
                              onStop({ id: tracked.id, runtimeScope: tracked.runtimeScope })
                            }
                            isDisabled={isBusy}
                          >
                            <CircleStop size={13} />
                            Stop
                          </HeroButton>
                        </Table.Cell>
                      </Table.Row>
                    ))}
                  </Table.Body>
                </Table.Content>
              </Table.ScrollContainer>
            </Table>
          </div>
        ) : null}
      </div>
    </section>
  );
}

interface TunnelFormProps {
  form: TunnelFormState;
  feedback: AppMessage | null;
  canUseLocal: boolean;
  isBusy: boolean;
  onChange: (field: keyof TunnelFormState, value: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onOpenSettings: () => void;
  onBrowseIdentityFile: () => void;
}

interface FormFeedbackProps {
  feedback: AppMessage | null;
}

/**
 * フォーム送信に紐づくフィードバックを表示する
 */
function FormFeedback({ feedback }: FormFeedbackProps): ReactElement | null {
  if (feedback === null) {
    return null;
  }

  return (
    <div className="xl:col-span-3">
      <AlertMessage kind={feedback.kind}>{feedback.text}</AlertMessage>
    </div>
  );
}

/**
 * 設定追加フォームを表示する
 */
function TunnelForm({
  form,
  feedback,
  canUseLocal,
  isBusy,
  onChange,
  onSubmit,
  onOpenSettings,
  onBrowseIdentityFile,
}: TunnelFormProps): ReactElement {
  const localUnavailable = !canUseLocal;

  return (
    <form
      className="overflow-hidden rounded-xl border border-border bg-card shadow-sm"
      onSubmit={onSubmit}
    >
      <div className="flex flex-col gap-3 border-b border-border px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <CirclePlus className="text-foreground/70" size={16} />
          <h2 className="text-sm font-semibold">Add tunnel</h2>
        </div>

        <div className="grid w-full grid-cols-2 gap-0.5 rounded-lg border border-border bg-muted p-0.5 sm:w-72">
          <HeroButton
            type="button"
            variant={form.scope === "local" ? "primary" : "ghost"}
            size="sm"
            fullWidth
            onPress={() => onChange("scope", "local")}
            isDisabled={localUnavailable}
            className="justify-center"
          >
            Local
          </HeroButton>
          <HeroButton
            type="button"
            variant={form.scope === "global" ? "primary" : "ghost"}
            size="sm"
            fullWidth
            onPress={() => onChange("scope", "global")}
            className="justify-center"
          >
            Global
          </HeroButton>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 bg-muted/25 p-4 xl:grid-cols-3">
        {localUnavailable && form.scope === "local" ? (
          <div className="xl:col-span-3">
            <AlertMessage kind="warning">
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                <span>local 設定に追加するには Settings でワークスペースを選択してください。</span>
                <HeroButton
                  type="button"
                  variant="outline"
                  size="sm"
                  onPress={onOpenSettings}
                  isDisabled={isBusy}
                >
                  <Settings2 size={13} />
                  Settings
                </HeroButton>
              </div>
            </AlertMessage>
          </div>
        ) : null}
        <FormFeedback feedback={feedback} />
        <TunnelDraftSummary form={form} />
        <section className="flex flex-col gap-3">
          <h3 className="text-xs font-medium text-muted-foreground">Identity</h3>
          <TextField
            label="ID"
            value={form.id}
            onChange={(value) => onChange("id", value)}
            required
          />
          <TextField
            label="Description"
            value={form.description}
            onChange={(value) => onChange("description", value)}
          />
          <TextField
            label="Tags"
            value={form.tags}
            onChange={(value) => onChange("tags", value)}
            placeholder="dev,project-a"
          />
        </section>

        <section className="flex flex-col gap-3 border-t border-border pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-4">
          <h3 className="text-xs font-medium text-muted-foreground">Routing</h3>
          <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
            <TextField
              label="Local host"
              value={form.localHost}
              onChange={(value) => onChange("localHost", value)}
              required
            />
            <TextField
              label="Local port"
              value={form.localPort}
              onChange={(value) => onChange("localPort", value)}
              inputMode="numeric"
              required
            />
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
            <TextField
              label="Remote host"
              value={form.remoteHost}
              onChange={(value) => onChange("remoteHost", value)}
              required
            />
            <TextField
              label="Remote port"
              value={form.remotePort}
              onChange={(value) => onChange("remotePort", value)}
              inputMode="numeric"
              required
            />
          </div>
        </section>

        <section className="flex flex-col gap-3 border-t border-border pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-4">
          <h3 className="text-xs font-medium text-muted-foreground">SSH</h3>
          <TextField
            label="SSH user"
            value={form.sshUser}
            onChange={(value) => onChange("sshUser", value)}
            required
          />
          <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
            <TextField
              label="SSH host"
              value={form.sshHost}
              onChange={(value) => onChange("sshHost", value)}
              required
            />
            <TextField
              label="SSH port"
              value={form.sshPort}
              onChange={(value) => onChange("sshPort", value)}
              inputMode="numeric"
            />
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
            <TextField
              label="Identity file"
              value={form.identityFile}
              onChange={(value) => onChange("identityFile", value)}
              placeholder="~/.ssh/id_ed25519"
            />
            <HeroButton
              type="button"
              variant="outline"
              size="sm"
              className="mb-0"
              onPress={onBrowseIdentityFile}
              isDisabled={isBusy}
            >
              <FolderOpen size={15} />
              Browse
            </HeroButton>
          </div>
        </section>
      </div>

      <div className="flex justify-end border-t border-border px-4 py-3">
        <HeroButton
          type="submit"
          variant="primary"
          size="sm"
          isDisabled={isBusy || (localUnavailable && form.scope === "local")}
        >
          <CirclePlus size={16} />
          Add tunnel
        </HeroButton>
      </div>
    </form>
  );
}

interface TunnelDraftSummaryProps {
  form: TunnelFormState;
}

/**
 * 追加フォームの入力内容を接続経路として要約する
 */
function TunnelDraftSummary({ form }: TunnelDraftSummaryProps): ReactElement {
  return (
    <section className="rounded-lg border border-border bg-card p-3 xl:col-span-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium text-muted-foreground">Draft route</span>
            <Chip size="sm" variant="soft">
              {form.scope}
            </Chip>
          </div>
          <h3 className="mt-1 truncate text-sm font-semibold">
            {form.id.trim() || "Untitled tunnel"}
          </h3>
        </div>
        <div className="grid min-w-0 flex-1 gap-2 lg:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto_minmax(0,1fr)] lg:items-center">
          <EndpointNode
            icon={<Server size={15} />}
            label="Local"
            value={formatDraftEndpoint(form.localHost, form.localPort, "local host:port")}
          />
          <RouteConnector horizontalAt="lg" />
          <EndpointNode
            icon={<ArrowRight size={15} />}
            label="Remote"
            value={formatDraftEndpoint(form.remoteHost, form.remotePort, "remote host:port")}
          />
          <RouteConnector horizontalAt="lg" />
          <EndpointNode
            icon={<KeyRound size={15} />}
            label="SSH"
            value={formatDraftEndpoint(form.sshHost, form.sshPort, "ssh host:port")}
          />
        </div>
      </div>
    </section>
  );
}

interface TextFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  inputMode?: "text" | "numeric";
  required?: boolean;
  disabled?: boolean;
}

/**
 * ラベル付き入力欄を表示する
 */
function TextField({
  label,
  value,
  onChange,
  placeholder,
  inputMode = "text",
  required = false,
  disabled = false,
}: TextFieldProps): ReactElement {
  return (
    <HeroTextField
      className="w-full gap-1"
      isDisabled={disabled}
      isRequired={required}
      variant="secondary"
    >
      <HeroLabel className="text-xs font-semibold text-foreground/70">{label}</HeroLabel>
      <HeroInput
        className="h-9 w-full"
        fullWidth
        variant="secondary"
        value={value}
        onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(event.target.value)}
        placeholder={placeholder}
        inputMode={inputMode}
        required={required}
        disabled={disabled}
      />
    </HeroTextField>
  );
}

interface ConfirmRemoveModalProps {
  tunnel: TunnelView | null;
  isBusy: boolean;
  onCancel: () => void;
  onConfirm: (tunnel: TunnelView) => void;
}

interface EditTunnelModalProps {
  tunnel: TunnelView | null;
  form: TunnelFormState;
  feedback: AppMessage | null;
  isBusy: boolean;
  onChange: (field: keyof TunnelFormState, value: string) => void;
  onCancel: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onBrowseIdentityFile: () => void;
}

/**
 * 既存トンネル設定の編集モーダルを表示する
 */
function EditTunnelModal({
  tunnel,
  form,
  feedback,
  isBusy,
  onChange,
  onCancel,
  onSubmit,
  onBrowseIdentityFile,
}: EditTunnelModalProps): ReactElement | null {
  if (tunnel === null) {
    return null;
  }

  return (
    <Modal
      isOpen
      onOpenChange={(open) => {
        if (!open && !isBusy) {
          onCancel();
        }
      }}
    >
      <Modal.Backdrop variant="blur" isDismissable={!isBusy}>
        <Modal.Container placement="center" scroll="inside" size="lg" className="px-4 py-4 sm:px-6">
          <Modal.Dialog className="w-full max-w-5xl overflow-hidden p-0">
            <form onSubmit={onSubmit}>
              <div className="flex flex-col gap-3 border-b border-border px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <Pencil className="text-foreground/70" size={18} />
                    <h3 className="text-base font-semibold">Edit tunnel</h3>
                    <Chip color="accent" size="sm" variant="soft">
                      {tunnel.source}
                    </Chip>
                  </div>
                  <p className="mt-1 truncate font-mono text-xs text-foreground/55">
                    {tunnel.sourcePath}
                  </p>
                </div>
              </div>

              <div className="max-h-[calc(100vh-13rem)] overflow-y-auto bg-muted/25 p-4">
                <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
                  <FormFeedback feedback={feedback} />
                  <TunnelDraftSummary form={form} />
                  <section className="flex flex-col gap-3">
                    <h4 className="text-xs font-medium text-muted-foreground">Identity</h4>
                    <TextField
                      label="ID"
                      value={form.id}
                      onChange={(value) => onChange("id", value)}
                      required
                    />
                    <TextField
                      label="Description"
                      value={form.description}
                      onChange={(value) => onChange("description", value)}
                    />
                    <TextField
                      label="Tags"
                      value={form.tags}
                      onChange={(value) => onChange("tags", value)}
                      placeholder="dev,project-a"
                    />
                  </section>

                  <section className="flex flex-col gap-3 border-t border-border pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-4">
                    <h4 className="text-xs font-medium text-muted-foreground">Routing</h4>
                    <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
                      <TextField
                        label="Local host"
                        value={form.localHost}
                        onChange={(value) => onChange("localHost", value)}
                        required
                      />
                      <TextField
                        label="Local port"
                        value={form.localPort}
                        onChange={(value) => onChange("localPort", value)}
                        inputMode="numeric"
                        required
                      />
                    </div>
                    <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
                      <TextField
                        label="Remote host"
                        value={form.remoteHost}
                        onChange={(value) => onChange("remoteHost", value)}
                        required
                      />
                      <TextField
                        label="Remote port"
                        value={form.remotePort}
                        onChange={(value) => onChange("remotePort", value)}
                        inputMode="numeric"
                        required
                      />
                    </div>
                  </section>

                  <section className="flex flex-col gap-3 border-t border-border pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-4">
                    <h4 className="text-xs font-medium text-muted-foreground">SSH</h4>
                    <TextField
                      label="SSH user"
                      value={form.sshUser}
                      onChange={(value) => onChange("sshUser", value)}
                      required
                    />
                    <div className="grid grid-cols-[minmax(0,1fr)_7.5rem] gap-2">
                      <TextField
                        label="SSH host"
                        value={form.sshHost}
                        onChange={(value) => onChange("sshHost", value)}
                        required
                      />
                      <TextField
                        label="SSH port"
                        value={form.sshPort}
                        onChange={(value) => onChange("sshPort", value)}
                        inputMode="numeric"
                      />
                    </div>
                    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
                      <TextField
                        label="Identity file"
                        value={form.identityFile}
                        onChange={(value) => onChange("identityFile", value)}
                        placeholder="~/.ssh/id_ed25519"
                      />
                      <HeroButton
                        type="button"
                        variant="outline"
                        size="sm"
                        className="mb-0"
                        onPress={onBrowseIdentityFile}
                        isDisabled={isBusy}
                      >
                        <FolderOpen size={15} />
                        Browse
                      </HeroButton>
                    </div>
                  </section>
                </div>
              </div>

              <div className="flex justify-end gap-2 border-t border-border px-5 py-4">
                <HeroButton
                  type="button"
                  variant="ghost"
                  size="sm"
                  onPress={onCancel}
                  isDisabled={isBusy}
                >
                  Cancel
                </HeroButton>
                <HeroButton type="submit" variant="primary" size="sm" isDisabled={isBusy}>
                  <Pencil size={16} />
                  Save
                </HeroButton>
              </div>
            </form>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}

/**
 * トンネル設定削除の確認モーダルを表示する
 */
function ConfirmRemoveModal({
  tunnel,
  isBusy,
  onCancel,
  onConfirm,
}: ConfirmRemoveModalProps): ReactElement | null {
  if (tunnel === null) {
    return null;
  }

  return (
    <Modal
      isOpen
      onOpenChange={(open) => {
        if (!open && !isBusy) {
          onCancel();
        }
      }}
    >
      <Modal.Backdrop variant="blur" isDismissable={!isBusy}>
        <Modal.Container placement="center" size="sm">
          <Modal.Dialog className="w-full max-w-md overflow-hidden p-0">
            <div className="border-b border-border px-5 py-4">
              <h3 className="text-base font-semibold">Remove tunnel</h3>
            </div>
            <div className="px-5 py-4">
              <p className="text-sm leading-6 text-foreground/70">
                {tunnel.id} を {tunnel.source}{" "}
                設定から削除します。この操作は設定ファイルを書き換えます。
              </p>
            </div>
            <div className="flex justify-end gap-2 border-t border-border px-5 py-4">
              <HeroButton type="button" slot="close" variant="ghost" size="sm" isDisabled={isBusy}>
                Cancel
              </HeroButton>
              <HeroButton
                type="button"
                variant="danger"
                size="sm"
                onPress={() => onConfirm(tunnel)}
                isDisabled={isBusy}
              >
                Remove
              </HeroButton>
            </div>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}

/**
 * トンネル一覧を画面上の絞り込み条件で抽出する
 */
function filterTunnels(tunnels: TunnelView[], filters: TunnelFilters): TunnelView[] {
  const query = filters.query.trim().toLowerCase();
  const requiredTags = new Set(filters.tags);

  return tunnels.filter((tunnel) => {
    const status = tunnelStatus(tunnel);

    if (filters.status !== "all" && filters.status !== status) {
      return false;
    }

    if (filters.scope !== "all" && filters.scope !== tunnel.source) {
      return false;
    }

    if (!tunnelMatchesRequiredTags(tunnel, requiredTags)) {
      return false;
    }

    return query.length === 0 || tunnelContainsQuery(tunnel, query);
  });
}

/**
 * トンネルが選択済みタグをすべて持つか判定する
 */
function tunnelMatchesRequiredTags(tunnel: TunnelView, requiredTags: Set<string>): boolean {
  if (requiredTags.size === 0) {
    return true;
  }

  if (tunnel.tags.length < requiredTags.size) {
    return false;
  }

  for (const tag of requiredTags) {
    if (!tunnel.tags.includes(tag)) {
      return false;
    }
  }

  return true;
}

/**
 * 設定済みトンネルから利用可能なタグ一覧を抽出する
 */
function collectAvailableTags(tunnels: TunnelView[]): string[] {
  const tags = new Set<string>();

  tunnels.forEach((tunnel) => {
    tunnel.tags.forEach((tag) => tags.add(tag));
  });

  return Array.from(tags).sort((left, right) => left.localeCompare(right));
}

/**
 * 選択中タグを先頭へ寄せてタグ一覧を並べ替える
 */
function orderTagsBySelection(tags: string[], selectedTags: string[]): string[] {
  const selected = new Set(selectedTags);

  return [...tags].sort((left, right) => {
    const leftSelected = selected.has(left);
    const rightSelected = selected.has(right);

    if (leftSelected !== rightSelected) {
      return leftSelected ? -1 : 1;
    }

    return left.localeCompare(right);
  });
}

/**
 * 絞り込み条件が初期状態から変更されているか判定する
 */
function hasActiveTunnelFilters(filters: TunnelFilters): boolean {
  return (
    filters.query.trim().length > 0 ||
    filters.status !== initialFilters.status ||
    filters.scope !== initialFilters.scope ||
    filters.tags.length > 0
  );
}

/**
 * トンネルの表示用状態を統一する
 */
function tunnelStatus(tunnel: TunnelView): TunnelStatus {
  return tunnel.status?.state ?? "idle";
}

/**
 * トンネルが検索語を含むか判定する
 */
function tunnelContainsQuery(tunnel: TunnelView, query: string): boolean {
  return (
    stringContainsQuery(tunnel.id, query) ||
    stringContainsQuery(tunnel.description ?? "", query) ||
    stringContainsQuery(tunnel.local, query) ||
    stringContainsQuery(tunnel.remote, query) ||
    stringContainsQuery(tunnel.ssh, query) ||
    stringContainsQuery(tunnel.source, query) ||
    stringContainsQuery(tunnel.sourcePath, query) ||
    tunnel.tags.some((tag) => stringContainsQuery(tag, query))
  );
}

/**
 * 文字列が正規化済み検索語を含むか判定する
 */
function stringContainsQuery(value: string, query: string): boolean {
  return value.toLowerCase().includes(query);
}

/**
 * 検索語との一致有無に応じて表示テキストを分割する
 */
function splitTextBySearchQuery(text: string, normalizedQuery: string): HighlightedTextPart[] {
  const normalizedText = text.toLowerCase();
  const parts: HighlightedTextPart[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    const matchIndex = normalizedText.indexOf(normalizedQuery, cursor);

    if (matchIndex === -1) {
      parts.push({ text: text.slice(cursor), isMatch: false });
      break;
    }

    if (matchIndex > cursor) {
      parts.push({ text: text.slice(cursor, matchIndex), isMatch: false });
    }

    const matchEnd = matchIndex + normalizedQuery.length;
    parts.push({ text: text.slice(matchIndex, matchEnd), isMatch: true });
    cursor = matchEnd;
  }

  return parts.length > 0 ? parts : [{ text, isMatch: false }];
}

/**
 * ダッシュボードの集計値を算出する
 */
function calculateStats(dashboard: DashboardState | null): DashboardStats {
  if (dashboard === null) {
    return { configured: 0, running: 0, stale: 0 };
  }

  let running = 0;
  let stale = 0;

  for (const tracked of dashboard.trackedTunnels) {
    if (tracked.status.state === "running") {
      running += 1;
    } else if (tracked.status.state === "stale") {
      stale += 1;
    }
  }

  return {
    configured: dashboard.tunnels.length,
    running,
    stale,
  };
}

/**
 * identity_file 用ダイアログの開始パスを解決する
 */
async function identityFileDialogDefaultPath(): Promise<string | undefined> {
  try {
    return await join(await homeDir(), ".ssh");
  } catch {
    return undefined;
  }
}

/**
 * Tauri command を実行環境の有無を確認して呼び出す
 */
async function invokeCommand<T>(command: AppCommand, args: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntimeAvailable()) {
    throw new Error(missingTauriRuntimeMessage);
  }

  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (isMissingTauriRuntimeError(error)) {
      throw new Error(missingTauriRuntimeMessage);
    }

    throw error;
  }
}

/**
 * 現在の window が Tauri API を公開しているか判定する
 */
function isTauriRuntimeAvailable(): boolean {
  return typeof (window as TauriRuntimeWindow).__TAURI_INTERNALS__?.invoke === "function";
}

/**
 * Tauri API 未提供時の実行時エラーか判定する
 */
function isMissingTauriRuntimeError(error: unknown): boolean {
  const message = stringifyError(error);

  return (
    message.includes("reading 'invoke'") ||
    message.includes('reading "invoke"') ||
    message.includes("__TAURI_INTERNALS__")
  );
}

/**
 * フォーム入力を command 入力へ変換する
 */
function formToTunnelInput(form: TunnelFormState): TunnelInput {
  return {
    id: requireText(form.id, "ID"),
    description: optionalText(form.description),
    tags: parseTags(form.tags),
    localHost: requireText(form.localHost, "Local host"),
    localPort: parsePort(form.localPort, "Local port", true),
    remoteHost: requireText(form.remoteHost, "Remote host"),
    remotePort: parsePort(form.remotePort, "Remote port", true),
    sshUser: requireText(form.sshUser, "SSH user"),
    sshHost: requireText(form.sshHost, "SSH host"),
    sshPort: parsePort(form.sshPort, "SSH port", false),
    identityFile: optionalText(form.identityFile),
  };
}

/**
 * 表示用トンネルを編集フォームの初期値へ変換する
 */
function formFromTunnel(tunnel: TunnelView): TunnelFormState {
  return {
    scope: tunnel.source,
    id: tunnel.id,
    description: tunnel.description ?? "",
    tags: tunnel.tags.join(","),
    localHost: tunnel.localHost,
    localPort: tunnel.localPort.toString(),
    remoteHost: tunnel.remoteHost,
    remotePort: tunnel.remotePort.toString(),
    sshUser: tunnel.sshUser,
    sshHost: tunnel.sshHost,
    sshPort: tunnel.sshPort?.toString() ?? "",
    identityFile: tunnel.identityFile ?? "",
  };
}

/**
 * 必須文字列を検証して返す
 */
function requireText(value: string, label: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    throw new Error(`${label} は必須です`);
  }

  return trimmed;
}

/**
 * 任意文字列を空値なら null へ変換する
 */
function optionalText(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}

/**
 * カンマ区切りタグ入力を配列へ変換する
 */
function parseTags(value: string): string[] {
  if (value.trim().length === 0) {
    return [];
  }

  return value
    .split(",")
    .map((tag) => tag.trim().toLowerCase())
    .filter((tag) => tag.length > 0);
}

/**
 * 入力途中の host と port を経路プレビュー用に整形する
 */
function formatDraftEndpoint(host: string, port: string, placeholder: string): string {
  const trimmedHost = host.trim();
  const trimmedPort = port.trim();

  if (trimmedHost.length > 0 && trimmedPort.length > 0) {
    return `${trimmedHost}:${trimmedPort}`;
  }

  if (trimmedHost.length > 0) {
    return `${trimmedHost}:port`;
  }

  if (trimmedPort.length > 0) {
    return `host:${trimmedPort}`;
  }

  return placeholder;
}

/**
 * ポート番号入力を数値へ変換する
 */
function parsePort(value: string, label: string, required: true): number;
function parsePort(value: string, label: string, required: false): number | null;
function parsePort(value: string, label: string, required: boolean): number | null {
  const trimmed = value.trim();
  if (trimmed.length === 0 && !required) {
    return null;
  }

  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
    throw new Error(`${label} は 1 から 65535 の数値で入力してください`);
  }

  return parsed;
}

/**
 * ワークスペース入力を command 入力へ正規化する
 */
function normalizeWorkspaceSelection(paths: WorkspaceSelection): WorkspaceSelectionInput {
  return {
    workspacePath: paths.workspacePath.trim(),
    globalConfigPath: paths.globalConfigPath.trim(),
    useGlobal: paths.useGlobal,
    hideDockIconWhenWindowHidden: paths.hideDockIconWhenWindowHidden,
    autoStopTunnelsOnQuit: paths.autoStopTunnelsOnQuit,
  };
}

/**
 * ワークスペースパスの変更有無を判定する
 */
function workspacePathHasChanged(current: WorkspaceSelection, next: WorkspaceSelection): boolean {
  return current.workspacePath.trim() !== next.workspacePath.trim();
}

/**
 * ワークスペース切り替え時の成功通知文を生成する
 */
function workspaceSwitchSuccessSummary(defaultSummary: string, stoppedCount: number): string {
  if (stoppedCount > 0) {
    return "旧 Workspace のポートフォワーディングを停止して切り替えました";
  }

  return defaultSummary;
}

/**
 * 停止対象の runtime scope を現在の表示状態から取得する
 */
function operationTargetForStop(id: string, dashboard: DashboardState | null): OperationTarget {
  const runtimeScope = dashboard?.tunnels.find((tunnel) => tunnel.id === id)?.status?.runtimeScope;

  if (runtimeScope === undefined) {
    return { id };
  }

  return { id, runtimeScope };
}

/**
 * 設定モーダルを開くショートカット入力か判定する
 */
function isSettingsKeyboardShortcut(event: KeyboardEvent): boolean {
  const hasPrimaryModifier = event.metaKey || event.ctrlKey;
  const isCommaKey = event.key === "," || event.code === "Comma";

  return hasPrimaryModifier && isCommaKey && !event.altKey && !event.shiftKey;
}

/**
 * Dashboard の検索欄へ移動するショートカット入力か判定する
 */
function isSearchKeyboardShortcut(event: KeyboardEvent): boolean {
  const hasPrimaryModifier = event.metaKey || event.ctrlKey;
  const isKKey = event.key.toLowerCase() === "k" || event.code === "KeyK";

  return hasPrimaryModifier && isKKey && !event.altKey && !event.shiftKey;
}

/**
 * 現在存在するトンネルだけを選択状態として残す
 */
function keepExistingSelections(current: Set<string>, tunnels: TunnelView[]): Set<string> {
  const ids = new Set(tunnels.map((tunnel) => tunnel.id));
  const next = new Set<string>();
  let hasRemovedSelection = false;

  current.forEach((id) => {
    if (ids.has(id)) {
      next.add(id);
      return;
    }

    hasRemovedSelection = true;
  });

  return hasRemovedSelection ? next : current;
}

/**
 * 指定 ID の選択状態を切り替える
 */
function toggleId(current: Set<string>, id: string): Set<string> {
  const next = new Set(current);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }

  return next;
}

/**
 * 指定 ID 群を選択状態へ追加する
 */
function addSelections(current: Set<string>, ids: string[]): Set<string> {
  const next = new Set(current);
  ids.forEach((id) => next.add(id));
  return next;
}

/**
 * 指定 ID を選択状態から除外する
 */
function removeSelection(current: Set<string>, id: string): Set<string> {
  const next = new Set(current);
  next.delete(id);
  return next;
}

/**
 * 指定 ID 群を選択状態から除外する
 */
function removeSelections(current: Set<string>, ids: string[]): Set<string> {
  const next = new Set(current);
  ids.forEach((id) => next.delete(id));
  return next;
}

/**
 * タグ絞り込みの選択状態を切り替える
 */
function toggleTag(current: string[], tag: string): string[] {
  if (current.includes(tag)) {
    return current.filter((currentTag) => currentTag !== tag);
  }

  return [...current, tag];
}

/**
 * 操作結果を通知文へ変換する
 */
function operationMessage(report: OperationReport): OperationToastInput | null {
  const successCount = report.succeeded.length;
  const failureCount = report.failed.length;

  if (successCount === 0 && failureCount === 0) {
    return null;
  }

  if (failureCount === 0) {
    return {
      kind: "success",
      summary: `${successCount} 件の操作が完了しました`,
    };
  }

  const failed = report.failed.map((failure) => `${failure.id}: ${failure.message}`).join("\n");
  return {
    kind: successCount > 0 ? "info" : "error",
    summary: `${successCount} 件成功、${failureCount} 件失敗しました`,
    detail: failed,
  };
}

/**
 * 実行中操作の表示ラベルを生成する
 */
function operationProgressLabel(progress: OperationProgress): string {
  const operation = progress.command === "start_tunnels" ? "開始" : "停止";
  const completedCount = clampCompletedCount(progress.completedCount, progress.totalCount);

  return `${completedCount} / ${progress.totalCount} 件を${operation}中`;
}

/**
 * 完了件数を総件数の範囲内へ正規化する
 */
function clampCompletedCount(completedCount: number, totalCount: number): number {
  return Math.max(0, Math.min(completedCount, totalCount));
}

/**
 * 操作開始表示が画面へ反映される描画機会まで待機する
 */
function waitForNextPaint(): Promise<void> {
  return new Promise((resolve) => {
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => resolve());
    });
  });
}

/**
 * 通知種別を shadcn/ui 互換の状態値へ変換する
 */
function alertStatus(kind: AlertMessageProps["kind"]): AlertStatus {
  if (kind === "success") {
    return "success";
  }

  if (kind === "warning") {
    return "warning";
  }

  if (kind === "error") {
    return "danger";
  }

  return "accent";
}

/**
 * 通知種別に対応するアイコンを生成する
 */
function alertIcon(kind: AlertMessageProps["kind"], size: number): ReactElement {
  const iconClassName =
    kind === "success"
      ? "text-success"
      : kind === "warning"
        ? "text-warning"
        : kind === "error"
          ? "text-danger"
          : "text-primary";

  if (kind === "success") {
    return <CheckCircle2 className={iconClassName} size={size} />;
  }

  return <AlertTriangle className={iconClassName} size={size} />;
}

/**
 * unknown のエラー値を表示文字列へ変換する
 */
function stringifyError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "予期しないエラーが発生しました";
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
