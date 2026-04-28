import { invoke } from "@tauri-apps/api/core";
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
  ListFilter,
  Loader2,
  Play,
  RefreshCw,
  Search,
  Server,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import { StrictMode, useEffect, useMemo, useState } from "react";
import type { ChangeEvent, FormEvent, ReactElement, ReactNode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

type ConfigScope = "local" | "global";

type RuntimeState = "running" | "stale";

type RuntimeScope = "global" | "workspace";

type TunnelStatus = RuntimeState | "idle";

type StatusFilter = "all" | TunnelStatus;

type ScopeFilter = "all" | ConfigScope;

type AppView = "dashboard" | "add";

interface WorkspaceSelection {
  workspacePath: string;
  workspaceHistory: string[];
  localConfigPath: string;
  globalConfigPath: string;
  useGlobal: boolean;
  globalStatePath: string;
  workspaceStatePath: string;
}

interface WorkspaceSelectionInput {
  workspacePath: string;
  globalConfigPath: string;
  useGlobal: boolean;
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
  local: string;
  remote: string;
  ssh: string;
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

interface AppMessage {
  kind: "success" | "error" | "info";
  text: string;
}

interface TunnelFilters {
  query: string;
  status: StatusFilter;
  scope: ScopeFilter;
  tags: string[];
}

const initialPaths: WorkspaceSelection = {
  workspacePath: "",
  workspaceHistory: [],
  localConfigPath: "",
  globalConfigPath: "",
  useGlobal: true,
  globalStatePath: "",
  workspaceStatePath: "",
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
  const [settingsDraft, setSettingsDraft] = useState<WorkspaceSelection | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deleteTarget, setDeleteTarget] = useState<TunnelView | null>(null);
  const [message, setMessage] = useState<AppMessage | null>(null);
  const [isBusy, setIsBusy] = useState<boolean>(false);

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

  useEffect(() => {
    async function loadInitialDashboard(): Promise<void> {
      setIsBusy(true);

      try {
        const loaded = await invoke<DashboardState>("load_dashboard", {
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
      }
    }

    void loadInitialDashboard();
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setFilters((current) => {
        if (current.query === queryInput) {
          return current;
        }

        return { ...current, query: queryInput };
      });
    }, searchDebounceMilliseconds);

    return () => window.clearTimeout(timeoutId);
  }, [queryInput]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      if (isSettingsKeyboardShortcut(event)) {
        event.preventDefault();
        setSettingsDraft((current) => current ?? paths);
        return;
      }

      if (event.key === "Escape" && !isBusy) {
        setSettingsDraft(null);
      }
    }

    window.addEventListener("keydown", handleKeyDown);

    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isBusy, paths]);

  /**
   * 現在のパス設定に基づいてダッシュボードを再取得する
   */
  async function refreshDashboard(nextPaths: WorkspaceSelection = paths): Promise<boolean> {
    setIsBusy(true);

    try {
      const loaded = await invoke<DashboardState>("load_dashboard", {
        paths: normalizeWorkspaceSelection(nextPaths),
      });

      setDashboard(loaded);
      setPaths(loaded.paths);
      setSelectedIds((current) => keepExistingSelections(current, loaded.tunnels));
      setMessage(null);
      return true;
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
      return false;
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * 指定 ID のトンネルを開始する
   */
  async function startSelected(ids: string[]): Promise<void> {
    if (ids.length === 0) {
      setMessage({ kind: "info", text: "開始するトンネルを選択してください" });
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
      setMessage({ kind: "info", text: "停止するトンネルを選択してください" });
      return;
    }

    await runOperation(
      "stop_tunnels",
      ids.map((id) => ({ id })),
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
    command: "start_tunnels" | "stop_tunnels",
    targets: OperationTarget[],
  ): Promise<void> {
    setIsBusy(true);

    try {
      const report = await invoke<OperationReport>(command, {
        paths: normalizeWorkspaceSelection(paths),
        targets,
      });

      setMessage(operationMessage(report));
      await refreshDashboard(paths);
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
    } finally {
      setIsBusy(false);
    }
  }

  /**
   * 設定ファイルへトンネルを追加する
   */
  async function submitTunnel(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();

    if (form.scope === "local" && paths.workspacePath.trim().length === 0) {
      setMessage({
        kind: "error",
        text: "local 設定に追加するにはワークスペースを選択してください",
      });
      return;
    }

    let tunnel: TunnelInput;
    try {
      tunnel = formToTunnelInput(form);
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
      return;
    }

    setIsBusy(true);

    try {
      const loaded = await invoke<DashboardState>("add_tunnel_entry", {
        paths: normalizeWorkspaceSelection(paths),
        scope: form.scope,
        tunnel,
      });

      setDashboard(loaded);
      setPaths(loaded.paths);
      setForm({ ...initialForm, scope: form.scope });
      setMessage({ kind: "success", text: `${tunnel.id} を設定に追加しました` });
      setActiveView("dashboard");
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
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
      const loaded = await invoke<DashboardState>("remove_tunnel_entry", {
        paths: normalizeWorkspaceSelection(paths),
        scope: tunnel.source,
        id: tunnel.id,
      });

      setDashboard(loaded);
      setPaths(loaded.paths);
      setSelectedIds((current) => removeSelection(current, tunnel.id));
      setMessage({ kind: "success", text: `${tunnel.id} を設定から削除しました` });
    } catch (error) {
      setMessage({ kind: "error", text: stringifyError(error) });
    } finally {
      setIsBusy(false);
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

    const applied = await refreshDashboard(settingsDraft);
    if (applied) {
      closeSettings();
    }
  }

  /**
   * 追加フォームの変更を反映する
   */
  function updateForm(field: keyof TunnelFormState, value: string): void {
    setForm((current) => ({ ...current, [field]: value }));
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
    setFilters((current) => ({ ...current, [field]: value }));
  }

  /**
   * 検索入力値を即時反映し、一覧への適用は遅延させる
   */
  function updateQueryInput(value: string): void {
    setQueryInput(value);
  }

  /**
   * タグ絞り込みの選択状態を切り替える
   */
  function toggleTagFilter(tag: string): void {
    setFilters((current) => ({ ...current, tags: toggleTag(current.tags, tag) }));
  }

  /**
   * 一覧の絞り込み条件を初期状態へ戻す
   */
  function resetFilters(): void {
    setQueryInput(initialFilters.query);
    setFilters(initialFilters);
  }

  return (
    <main className="app-shell min-h-screen text-base-content">
      <div className="mx-auto flex w-full max-w-[90rem] flex-col gap-5 px-4 py-5 sm:px-6 lg:px-8">
        <AppHeader
          stats={stats}
          activeView={activeView}
          isBusy={isBusy}
          onViewChange={setActiveView}
          onOpenSettings={openSettings}
          onRefresh={() => void refreshDashboard()}
        />

        <WorkspaceContextPanel paths={paths} isBusy={isBusy} onOpenSettings={openSettings} />

        <MessagePanel message={message} />

        {activeView === "dashboard" ? (
          <DashboardView
            dashboard={dashboard}
            filteredTunnels={filteredTunnels}
            hasActiveFilters={hasActiveFilters}
            selectedIds={selectedIds}
            selectedCount={selectedIdList.length}
            selectedVisibleCount={selectedVisibleCount}
            availableTags={availableTags}
            isBusy={isBusy}
            queryInput={queryInput}
            filters={filters}
            onQueryInputChange={updateQueryInput}
            onFilterChange={updateFilter}
            onToggleTag={toggleTagFilter}
            onResetFilters={resetFilters}
            onClearSelection={() => setSelectedIds(new Set())}
            onSelectVisible={selectVisibleTunnels}
            onDeselectVisible={deselectVisibleTunnels}
            onToggleSelection={toggleSelection}
            onStartSelected={() => void startSelected(selectedIdList)}
            onStopSelected={() => void stopSelected(selectedIdList)}
            onStartTunnel={(id) => void startSelected([id])}
            onStopTunnel={(id) => void stopSelected([id])}
            onStopTracked={(target) => void stopTracked(target)}
            onRemoveTunnel={setDeleteTarget}
            onAddTunnel={() => setActiveView("add")}
          />
        ) : activeView === "add" ? (
          <AddTunnelView
            form={form}
            canUseLocal={paths.workspacePath.trim().length > 0}
            isBusy={isBusy}
            onChange={updateForm}
            onSubmit={(event) => void submitTunnel(event)}
            onOpenSettings={openSettings}
            onBrowseIdentityFile={() => void browseIdentityFile()}
          />
        ) : null}
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
      />
      <ConfirmRemoveModal
        tunnel={deleteTarget}
        isBusy={isBusy}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={(tunnel) => void removeTunnel(tunnel)}
      />
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
  activeView: AppView;
  isBusy: boolean;
  onViewChange: (view: AppView) => void;
  onOpenSettings: () => void;
  onRefresh: () => void;
}

/**
 * アプリ全体の操作状況と再読み込み導線を表示する
 */
function AppHeader({
  stats,
  activeView,
  isBusy,
  onViewChange,
  onOpenSettings,
  onRefresh,
}: AppHeaderProps): ReactElement {
  return (
    <header className="overflow-hidden rounded-lg border border-base-300 bg-base-100 shadow-sm">
      <div className="flex flex-col gap-4 px-5 py-4 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0">
          <span className="text-xs font-bold uppercase tracking-wide text-primary">fwd-deck</span>
          <h1 className="mt-1 truncate text-2xl leading-tight font-bold sm:text-3xl">
            Port Forwarding Deck
          </h1>
          <p className="mt-1 text-sm text-base-content/60">
            SSH tunnel operations for local development
          </p>
        </div>
        <div className="flex flex-col gap-3">
          <div className="join w-full self-start lg:w-auto lg:self-end">
            <button
              type="button"
              className={`btn btn-sm join-item flex-1 lg:flex-none ${
                activeView === "dashboard" ? "btn-primary" : "btn-outline"
              }`}
              onClick={() => onViewChange("dashboard")}
            >
              <ListFilter size={15} />
              Dashboard
            </button>
            <button
              type="button"
              className={`btn btn-sm join-item flex-1 lg:flex-none ${
                activeView === "add" ? "btn-primary" : "btn-outline"
              }`}
              onClick={() => onViewChange("add")}
            >
              <CirclePlus size={15} />
              Add tunnel
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm join-item flex-1 lg:flex-none"
              onClick={onOpenSettings}
              aria-label="Settings"
              title="Settings (Cmd/Ctrl + ,)"
            >
              <Settings2 size={15} />
              Settings
            </button>
          </div>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-[repeat(3,minmax(7rem,1fr))_auto] lg:w-auto">
            <StatusMetric label="Configured" value={stats.configured} icon={<Gauge size={17} />} />
            <StatusMetric
              label="Running"
              value={stats.running}
              tone="success"
              icon={<Activity size={17} />}
            />
            <StatusMetric
              label="Stale"
              value={stats.stale}
              tone="warning"
              icon={<Clock3 size={17} />}
            />
            <IconButton
              label="再読み込み"
              className="btn btn-square btn-ghost h-full min-h-16 w-full border border-base-300 sm:w-16"
              onClick={onRefresh}
              disabled={isBusy}
            >
              {isBusy ? <Loader2 className="animate-spin" size={18} /> : <RefreshCw size={18} />}
            </IconButton>
          </div>
        </div>
      </div>
    </header>
  );
}

interface WorkspaceContextPanelProps {
  paths: WorkspaceSelection;
  isBusy: boolean;
  onOpenSettings: () => void;
}

/**
 * 現在の作業対象ワークスペースを表示する
 */
function WorkspaceContextPanel({
  paths,
  isBusy,
  onOpenSettings,
}: WorkspaceContextPanelProps): ReactElement {
  const workspacePath = paths.workspacePath.trim();
  const hasWorkspace = workspacePath.length > 0;

  return (
    <section className="rounded-lg border border-base-300 bg-base-100 shadow-sm">
      <div className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-3">
          <span
            className={`rounded-md p-2 ${
              hasWorkspace ? "bg-primary/10 text-primary" : "bg-warning/10 text-warning"
            }`}
          >
            <FolderOpen size={17} />
          </span>
          <div className="min-w-0">
            <div className="text-xs font-bold uppercase tracking-wide text-base-content/50">
              Workspace
            </div>
            <div
              className={`mt-0.5 truncate font-mono text-xs ${
                hasWorkspace ? "text-base-content/85" : "text-warning"
              }`}
              title={workspacePath || "Not selected"}
            >
              {workspacePath || "Not selected"}
            </div>
          </div>
        </div>
        <button
          type="button"
          className="btn btn-outline btn-sm self-start sm:self-auto"
          onClick={onOpenSettings}
          disabled={isBusy}
        >
          <Settings2 size={15} />
          Settings
        </button>
      </div>
    </section>
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
    <div className="min-w-0 rounded-md border border-base-300 bg-base-200/50 px-3 py-2">
      <div className="flex items-center gap-2 text-xs font-semibold text-base-content/60">
        <span className={textColor}>{icon}</span>
        <span className="truncate">{label}</span>
      </div>
      <div className={`mt-1 text-2xl leading-none font-bold ${textColor}`}>{value}</div>
    </div>
  );
}

interface IconButtonProps {
  label: string;
  className: string;
  disabled?: boolean;
  children: ReactNode;
  onClick: () => void;
}

/**
 * Tooltip 付きアイコンボタンを表示する
 */
function IconButton({
  label,
  className,
  disabled = false,
  children,
  onClick,
}: IconButtonProps): ReactElement {
  return (
    <div className="tooltip tooltip-left" data-tip={label}>
      <button
        className={className}
        type="button"
        onClick={onClick}
        disabled={disabled}
        aria-label={label}
      >
        {children}
      </button>
    </div>
  );
}

interface DashboardViewProps {
  dashboard: DashboardState | null;
  filteredTunnels: TunnelView[];
  hasActiveFilters: boolean;
  selectedIds: Set<string>;
  selectedCount: number;
  selectedVisibleCount: number;
  availableTags: string[];
  queryInput: string;
  filters: TunnelFilters;
  isBusy: boolean;
  onQueryInputChange: (value: string) => void;
  onFilterChange: <K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]) => void;
  onToggleTag: (tag: string) => void;
  onResetFilters: () => void;
  onClearSelection: () => void;
  onSelectVisible: () => void;
  onDeselectVisible: () => void;
  onToggleSelection: (id: string) => void;
  onStartSelected: () => void;
  onStopSelected: () => void;
  onStartTunnel: (id: string) => void;
  onStopTunnel: (id: string) => void;
  onStopTracked: (target: OperationTarget) => void;
  onRemoveTunnel: (tunnel: TunnelView) => void;
  onAddTunnel: () => void;
}

/**
 * 運用対象の一覧と実行操作を表示する
 */
function DashboardView({
  dashboard,
  filteredTunnels,
  hasActiveFilters,
  selectedIds,
  selectedCount,
  selectedVisibleCount,
  availableTags,
  queryInput,
  filters,
  isBusy,
  onQueryInputChange,
  onFilterChange,
  onToggleTag,
  onResetFilters,
  onClearSelection,
  onSelectVisible,
  onDeselectVisible,
  onToggleSelection,
  onStartSelected,
  onStopSelected,
  onStartTunnel,
  onStopTunnel,
  onStopTracked,
  onRemoveTunnel,
  onAddTunnel,
}: DashboardViewProps): ReactElement {
  const [isTrackedPanelCollapsed, setIsTrackedPanelCollapsed] = useState<boolean>(true);
  const hasTrackedRuntime = (dashboard?.trackedTunnels.length ?? 0) > 0;

  return (
    <section
      className={`flex min-w-0 flex-col gap-4 ${hasTrackedRuntime ? (isTrackedPanelCollapsed ? "pb-20" : "pb-56") : ""}`}
    >
      <ValidationPanel dashboard={dashboard} />
      <TunnelOperationsPanel
        selectedCount={selectedCount}
        totalCount={dashboard?.tunnels.length ?? 0}
        visibleCount={filteredTunnels.length}
        selectedVisibleCount={selectedVisibleCount}
        availableTags={availableTags}
        queryInput={queryInput}
        filters={filters}
        hasActiveFilters={hasActiveFilters}
        isBusy={isBusy}
        onQueryInputChange={onQueryInputChange}
        onFilterChange={onFilterChange}
        onToggleTag={onToggleTag}
        onResetFilters={onResetFilters}
        onSelectVisible={onSelectVisible}
        onDeselectVisible={onDeselectVisible}
        onStart={onStartSelected}
        onStop={onStopSelected}
        onClear={onClearSelection}
      />
      <TunnelDeck
        dashboard={dashboard}
        tunnels={filteredTunnels}
        hasActiveFilters={hasActiveFilters}
        selectedIds={selectedIds}
        isBusy={isBusy}
        onToggle={onToggleSelection}
        onStart={onStartTunnel}
        onStop={onStopTunnel}
        onRemove={onRemoveTunnel}
        onAddTunnel={onAddTunnel}
      />
      <TrackedPanel
        dashboard={dashboard}
        isCollapsed={isTrackedPanelCollapsed}
        isBusy={isBusy}
        onToggleCollapsed={() => setIsTrackedPanelCollapsed((current) => !current)}
        onStop={onStopTracked}
      />
    </section>
  );
}

interface AddTunnelViewProps {
  form: TunnelFormState;
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
  canUseLocal,
  isBusy,
  onChange,
  onSubmit,
  onOpenSettings,
  onBrowseIdentityFile,
}: AddTunnelViewProps): ReactElement {
  return (
    <section className="mx-auto flex w-full max-w-6xl flex-col gap-4">
      <TunnelForm
        form={form}
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
}: SettingsModalProps): ReactElement | null {
  if (!isOpen) {
    return null;
  }

  return (
    <div
      className="modal modal-open"
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-title"
    >
      <div className="modal-box max-h-[calc(100vh-2rem)] w-11/12 max-w-5xl overflow-hidden p-0">
        <div className="flex items-start justify-between gap-4 border-b border-base-300 px-5 py-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Settings2 className="text-primary" size={18} />
              <h2 id="settings-title" className="text-lg font-bold">
                Settings
              </h2>
            </div>
          </div>
          <IconButton
            label="設定を閉じる"
            className="btn btn-square btn-ghost btn-sm"
            onClick={onCancel}
            disabled={isBusy}
          >
            <X size={17} />
          </IconButton>
        </div>

        <div className="max-h-[calc(100vh-13rem)] overflow-y-auto px-5 py-4">
          <PathPanel
            paths={paths}
            isBusy={isBusy}
            onChange={onChange}
            onBrowseWorkspace={onBrowseWorkspace}
            onBrowseGlobalConfig={onBrowseGlobalConfig}
            onSelectWorkspace={onSelectWorkspace}
          />
        </div>

        <div className="flex justify-end gap-2 border-t border-base-300 px-5 py-4">
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={onCancel}
            disabled={isBusy}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={onApply}
            disabled={isBusy}
          >
            {isBusy ? <Loader2 className="animate-spin" size={16} /> : <RefreshCw size={16} />}
            Apply changes
          </button>
        </div>
      </div>
      <button className="modal-backdrop" type="button" onClick={onCancel} disabled={isBusy}>
        close
      </button>
    </div>
  );
}

interface PathPanelProps {
  paths: WorkspaceSelection;
  isBusy: boolean;
  onChange: (field: keyof WorkspaceSelection, value: string | boolean) => void;
  onBrowseWorkspace: () => void;
  onBrowseGlobalConfig: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
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
}: PathPanelProps): ReactElement {
  return (
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <section className="flex flex-col gap-3 rounded-lg border border-base-300 bg-base-200/30 p-4">
        <div className="flex items-center gap-2">
          <FolderOpen className="text-primary" size={17} />
          <h3 className="text-sm font-bold">Workspace</h3>
        </div>
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
          <TextField
            label="Workspace directory"
            value={paths.workspacePath}
            onChange={(value) => onChange("workspacePath", value)}
          />
          <button
            type="button"
            className="btn btn-outline btn-sm mb-0"
            onClick={onBrowseWorkspace}
            disabled={isBusy}
          >
            <FolderOpen size={15} />
            Browse
          </button>
        </div>
        {paths.workspaceHistory.length > 0 ? (
          <div className="rounded-md border border-base-300 bg-base-100 px-3 py-2">
            <div className="mb-2 text-xs font-bold uppercase tracking-wide text-base-content/50">
              Recent workspaces
            </div>
            <div className="flex max-h-36 flex-col gap-1 overflow-y-auto">
              {paths.workspaceHistory.map((workspacePath) => (
                <button
                  key={workspacePath}
                  type="button"
                  className="btn btn-ghost btn-xs min-h-8 justify-start font-mono text-xs"
                  onClick={() => onSelectWorkspace(workspacePath)}
                  disabled={isBusy}
                  title={workspacePath}
                >
                  <span className="truncate">{workspacePath}</span>
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </section>

      <section className="flex flex-col gap-3 rounded-lg border border-base-300 bg-base-200/30 p-4">
        <div className="flex items-center gap-2">
          <Settings2 className="text-primary" size={17} />
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
          <button
            type="button"
            className="btn btn-outline btn-sm mb-0"
            onClick={onBrowseGlobalConfig}
            disabled={isBusy || !paths.useGlobal}
          >
            <FolderOpen size={15} />
            Browse
          </button>
        </div>
        <div className="rounded-md border border-base-300 bg-base-100 px-3 py-2">
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span className="text-sm font-semibold">Use global config</span>
            <input
              type="checkbox"
              className="toggle toggle-primary toggle-sm"
              checked={paths.useGlobal}
              onChange={(event) => onChange("useGlobal", event.target.checked)}
            />
          </label>
        </div>
      </section>

      <section className="flex flex-col gap-3 rounded-lg border border-base-300 bg-base-200/30 p-4 lg:col-span-2">
        <div className="flex items-center gap-2">
          <Activity className="text-primary" size={17} />
          <h3 className="text-sm font-bold">Runtime state</h3>
        </div>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          <PathValue label="Global state" value={paths.globalStatePath} />
          <PathValue label="Workspace state" value={paths.workspaceStatePath} />
        </div>
      </section>
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
    <div className="rounded-md border border-base-300 bg-base-200/40 px-3 py-2">
      <div className="text-xs font-semibold text-base-content/60">{label}</div>
      <div className="mt-1 min-h-5 truncate font-mono text-xs text-base-content/85" title={value}>
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
    return <AlertMessage kind="success">設定は有効です。</AlertMessage>;
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
  const iconClassName =
    kind === "success"
      ? "text-[#047857]"
      : kind === "warning"
        ? "text-[#b45309]"
        : kind === "error"
          ? "text-[#b91c1c]"
          : "text-[#1d4ed8]";
  const toneClassName =
    kind === "success"
      ? "border-[#86efac] bg-[#ecfdf3]"
      : kind === "warning"
        ? "border-[#f59e0b] bg-[#fff7dc]"
        : kind === "error"
          ? "border-[#fca5a5] bg-[#fef2f2]"
          : "border-[#93c5fd] bg-[#eff6ff]";
  const icon =
    kind === "success" ? (
      <CheckCircle2 className={iconClassName} size={18} />
    ) : (
      <AlertTriangle className={iconClassName} size={18} />
    );

  return (
    <div
      className={`flex items-center gap-3 rounded-lg border px-4 py-3 text-sm text-base-content shadow-sm ${toneClassName}`}
      role={kind === "error" ? "alert" : "status"}
    >
      <span className="shrink-0">{icon}</span>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
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
  selectedCount: number;
  totalCount: number;
  visibleCount: number;
  selectedVisibleCount: number;
  availableTags: string[];
  queryInput: string;
  filters: TunnelFilters;
  hasActiveFilters: boolean;
  isBusy: boolean;
  onQueryInputChange: (value: string) => void;
  onFilterChange: <K extends keyof TunnelFilters>(field: K, value: TunnelFilters[K]) => void;
  onToggleTag: (tag: string) => void;
  onResetFilters: () => void;
  onSelectVisible: () => void;
  onDeselectVisible: () => void;
  onStart: () => void;
  onStop: () => void;
  onClear: () => void;
}

/**
 * 一覧の絞り込みと複数選択操作を表示する
 */
function TunnelOperationsPanel({
  selectedCount,
  totalCount,
  visibleCount,
  selectedVisibleCount,
  availableTags,
  queryInput,
  filters,
  hasActiveFilters,
  isBusy,
  onQueryInputChange,
  onFilterChange,
  onToggleTag,
  onResetFilters,
  onSelectVisible,
  onDeselectVisible,
  onStart,
  onStop,
  onClear,
}: TunnelOperationsPanelProps): ReactElement {
  return (
    <section className="rounded-lg border border-base-300 bg-base-100 shadow-sm">
      <div className="flex flex-col gap-4 p-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <ListFilter className="text-primary" size={18} />
              <h2 className="text-base leading-6 font-bold">Tunnels</h2>
              <span className="badge badge-neutral badge-sm">
                {visibleCount} / {totalCount}
              </span>
            </div>
            <p className="mt-1 text-sm text-base-content/60">
              {selectedCount} selected across the current dashboard
            </p>
          </div>
          <div className="flex flex-wrap gap-2 lg:justify-end">
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onSelectVisible}
              disabled={isBusy || visibleCount === 0 || selectedVisibleCount === visibleCount}
            >
              Select visible
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={onDeselectVisible}
              disabled={isBusy || selectedVisibleCount === 0}
            >
              Deselect visible
            </button>
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={onStart}
              disabled={isBusy || selectedCount === 0}
            >
              <Play size={16} />
              Start
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onStop}
              disabled={isBusy || selectedCount === 0}
            >
              <CircleStop size={16} />
              Stop
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={onClear}
              disabled={isBusy || selectedCount === 0}
            >
              Clear
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-3 lg:grid-cols-[minmax(14rem,1fr)_auto_auto_auto] lg:items-center">
          <div className="relative">
            <Search
              className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-base-content/40"
              size={16}
            />
            <input
              className="input input-bordered input-sm w-full pr-9 pl-9"
              value={queryInput}
              onChange={(event: ChangeEvent<HTMLInputElement>) =>
                onQueryInputChange(event.target.value)
              }
              placeholder="Search ID, tag, endpoint"
              aria-label="Search tunnels"
            />
            {queryInput.length > 0 ? (
              <button
                type="button"
                className="btn btn-square btn-ghost btn-xs absolute top-1/2 right-1 -translate-y-1/2"
                onClick={() => onQueryInputChange("")}
                aria-label="検索条件を消去"
                title="検索条件を消去"
              >
                <X size={14} />
              </button>
            ) : null}
          </div>

          <div className="join">
            {statusFilterOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`btn btn-sm join-item ${
                  filters.status === option.value ? "btn-neutral" : "btn-outline"
                }`}
                onClick={() => onFilterChange("status", option.value)}
                aria-pressed={filters.status === option.value}
              >
                {option.label}
              </button>
            ))}
          </div>

          <div className="join">
            {scopeFilterOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`btn btn-sm join-item ${
                  filters.scope === option.value ? "btn-neutral" : "btn-outline"
                }`}
                onClick={() => onFilterChange("scope", option.value)}
                aria-pressed={filters.scope === option.value}
              >
                {option.label}
              </button>
            ))}
          </div>

          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={onResetFilters}
            disabled={!hasActiveFilters}
          >
            Reset
          </button>
        </div>

        {availableTags.length > 0 ? (
          <div className="flex flex-wrap items-center gap-2 border-t border-base-300 pt-3">
            <span className="text-xs font-bold uppercase tracking-wide text-base-content/50">
              Tags
            </span>
            {availableTags.map((tag) => {
              const selected = filters.tags.includes(tag);
              return (
                <button
                  key={tag}
                  type="button"
                  className={`btn btn-xs rounded-full ${
                    selected ? "btn-primary" : "btn-outline tag-outline"
                  }`}
                  onClick={() => onToggleTag(tag)}
                  aria-pressed={selected}
                >
                  {tag}
                </button>
              );
            })}
          </div>
        ) : null}
      </div>
    </section>
  );
}

interface MessagePanelProps {
  message: AppMessage | null;
}

/**
 * 操作結果メッセージを表示する
 */
function MessagePanel({ message }: MessagePanelProps): ReactElement | null {
  if (message === null) {
    return null;
  }

  const kind = message.kind === "error" ? "error" : message.kind === "success" ? "success" : "info";
  return <AlertMessage kind={kind}>{message.text}</AlertMessage>;
}

interface TunnelDeckProps {
  dashboard: DashboardState | null;
  tunnels: TunnelView[];
  hasActiveFilters: boolean;
  selectedIds: Set<string>;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onRemove: (tunnel: TunnelView) => void;
  onAddTunnel: () => void;
}

/**
 * 設定済みトンネルのカード一覧を表示する
 */
function TunnelDeck({
  dashboard,
  tunnels,
  hasActiveFilters,
  selectedIds,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onRemove,
  onAddTunnel,
}: TunnelDeckProps): ReactElement {
  if (dashboard === null) {
    return <EmptyState title="Loading tunnels">設定と実行状態を読み込んでいます。</EmptyState>;
  }

  if (dashboard.tunnels.length === 0) {
    return (
      <EmptyState
        title="No configured tunnels"
        action={
          <button className="btn btn-primary btn-sm" type="button" onClick={onAddTunnel}>
            <CirclePlus size={16} />
            Add tunnel
          </button>
        }
      >
        Add tunnel から新しい接続を追加できます。
      </EmptyState>
    );
  }

  if (tunnels.length === 0 && hasActiveFilters) {
    return (
      <EmptyState title="No matching tunnels">
        検索条件またはフィルターを変更してください。
      </EmptyState>
    );
  }

  return (
    <section className="grid grid-cols-1 gap-4 lg:grid-cols-2">
      {tunnels.map((tunnel) => (
        <TunnelCard
          key={tunnel.id}
          tunnel={tunnel}
          checked={selectedIds.has(tunnel.id)}
          isBusy={isBusy}
          onToggle={onToggle}
          onStart={onStart}
          onStop={onStop}
          onRemove={onRemove}
        />
      ))}
    </section>
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
    <section className="rounded-lg border border-dashed border-base-300 bg-base-100/75 shadow-sm">
      <div className="flex min-h-40 flex-col items-center justify-center gap-2 px-5 py-8 text-center">
        <div className="rounded-full bg-base-200 p-3 text-base-content/50">
          <ListFilter size={22} />
        </div>
        <h2 className="text-base font-bold">{title}</h2>
        <p className="max-w-md text-sm text-base-content/60">{children}</p>
        {action ? <div className="mt-2">{action}</div> : null}
      </div>
    </section>
  );
}

interface TunnelCardProps {
  tunnel: TunnelView;
  checked: boolean;
  isBusy: boolean;
  onToggle: (id: string) => void;
  onStart: (id: string) => void;
  onStop: (id: string) => void;
  onRemove: (tunnel: TunnelView) => void;
}

/**
 * トンネル 1 件の操作カードを表示する
 */
function TunnelCard({
  tunnel,
  checked,
  isBusy,
  onToggle,
  onStart,
  onStop,
  onRemove,
}: TunnelCardProps): ReactElement {
  const running = tunnel.status?.state === "running";
  const status = tunnel.status?.state ?? "idle";

  return (
    <article
      className={`flex h-full flex-col rounded-lg border bg-base-100 shadow-sm transition ${
        checked
          ? "border-primary ring-2 ring-primary/20"
          : "border-base-300 hover:border-base-content/20"
      }`}
    >
      <div className="flex h-full flex-col gap-4 p-5">
        <div className="flex items-start justify-between gap-3">
          <label className="flex min-w-0 cursor-pointer items-start gap-3">
            <input
              type="checkbox"
              className="checkbox checkbox-primary checkbox-sm mt-1"
              checked={checked}
              onChange={() => onToggle(tunnel.id)}
            />
            <span className="min-w-0">
              <span className="block truncate text-base leading-6 font-bold">{tunnel.id}</span>
              <span className="mt-0.5 block truncate text-xs text-base-content/50">
                {tunnel.sourcePath}
              </span>
            </span>
          </label>
          <StatusBadge status={status} />
        </div>

        <p className="min-h-10 text-sm leading-5 text-base-content/60">
          {tunnel.description ?? "No description"}
        </p>

        <TagList tags={tunnel.tags} />
        <EndpointList tunnel={tunnel} />

        <div className="grid grid-cols-2 gap-2 text-xs">
          <div className="rounded-md border border-base-300 bg-base-200/40 px-3 py-2">
            <div className="font-semibold text-base-content/50">Source</div>
            <div className="mt-1 font-mono text-base-content/80">{tunnel.source}</div>
          </div>
          <div className="rounded-md border border-base-300 bg-base-200/40 px-3 py-2">
            <div className="font-semibold text-base-content/50">Runtime</div>
            <div className="mt-1 font-mono text-base-content/80">
              {tunnel.status ? `pid ${tunnel.status.pid}` : "not tracked"}
            </div>
          </div>
        </div>

        <div className="flex flex-wrap gap-2 text-xs text-base-content/60">
          <span className="badge badge-ghost badge-sm">
            {tunnel.timeouts.connectTimeoutSeconds}s connect
          </span>
          <span className="badge badge-ghost badge-sm">
            {tunnel.timeouts.startGraceMilliseconds}ms grace
          </span>
        </div>

        <div className="mt-auto flex items-center justify-end gap-2 pt-1">
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={() => onStart(tunnel.id)}
            disabled={isBusy || running}
          >
            <Play size={15} />
            Start
          </button>
          <button
            type="button"
            className="btn btn-outline btn-sm"
            onClick={() => onStop(tunnel.id)}
            disabled={isBusy || tunnel.status === null}
          >
            <CircleStop size={15} />
            Stop
          </button>
          <IconButton
            label="設定から削除"
            className="btn btn-square btn-ghost btn-sm text-error"
            onClick={() => onRemove(tunnel)}
            disabled={isBusy}
          >
            <Trash2 size={16} />
          </IconButton>
        </div>
      </div>
    </article>
  );
}

interface StatusBadgeProps {
  status: TunnelStatus;
}

/**
 * トンネル状態の badge を表示する
 */
function StatusBadge({ status }: StatusBadgeProps): ReactElement {
  const className =
    status === "running"
      ? "badge badge-success badge-sm"
      : status === "stale"
        ? "badge badge-warning badge-sm"
        : "badge badge-ghost badge-sm";

  return <span className={className}>{status}</span>;
}

interface TagListProps {
  tags: string[];
}

/**
 * タグ一覧を表示する
 */
function TagList({ tags }: TagListProps): ReactElement {
  if (tags.length === 0) {
    return <div className="min-h-6 text-xs leading-6 text-base-content/50">No tags</div>;
  }

  return (
    <div className="flex min-h-6 flex-wrap items-center gap-1">
      {tags.map((tag) => (
        <span className="badge badge-primary badge-outline badge-sm tag-outline" key={tag}>
          {tag}
        </span>
      ))}
    </div>
  );
}

interface EndpointListProps {
  tunnel: TunnelView;
}

/**
 * 接続先情報を表示する
 */
function EndpointList({ tunnel }: EndpointListProps): ReactElement {
  return (
    <div className="rounded-lg border border-base-300 bg-base-200/40 p-3">
      <div className="grid gap-2">
        <EndpointRow icon={<Server size={15} />} label="Local" value={tunnel.local} />
        <div className="ml-3 h-3 border-l border-base-content/20" aria-hidden="true" />
        <EndpointRow icon={<ArrowRight size={15} />} label="Remote" value={tunnel.remote} />
        <div className="ml-3 h-3 border-l border-base-content/20" aria-hidden="true" />
        <EndpointRow icon={<KeyRound size={15} />} label="SSH" value={tunnel.ssh} />
      </div>
    </div>
  );
}

interface EndpointRowProps {
  icon: ReactNode;
  label: string;
  value: string;
}

/**
 * 接続先情報の 1 行を表示する
 */
function EndpointRow({ icon, label, value }: EndpointRowProps): ReactElement {
  return (
    <div className="grid grid-cols-[1.5rem_4.25rem_minmax(0,1fr)] items-center gap-2 text-sm">
      <span className="text-base-content/50">{icon}</span>
      <span className="font-semibold text-base-content/60">{label}</span>
      <span className="truncate font-mono text-xs text-base-content/90" title={value}>
        {value}
      </span>
    </div>
  );
}

interface TrackedPanelProps {
  dashboard: DashboardState | null;
  isCollapsed: boolean;
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
  isBusy,
  onToggleCollapsed,
  onStop,
}: TrackedPanelProps): ReactElement | null {
  if (dashboard === null || dashboard.trackedTunnels.length === 0) {
    return null;
  }

  return (
    <section className="pointer-events-none fixed right-4 bottom-4 left-4 z-40 sm:left-1/2 sm:right-auto sm:-translate-x-1/2">
      <div className="tracked-runtime-shell pointer-events-auto w-full overflow-hidden rounded-xl bg-base-100 sm:w-[42rem]">
        <button
          type="button"
          className="flex w-full items-center justify-between gap-3 bg-primary/5 px-3.5 py-2.5 text-left"
          onClick={onToggleCollapsed}
          aria-expanded={!isCollapsed}
        >
          <span className="flex min-w-0 items-center gap-2">
            <span className="rounded-md bg-primary/10 p-1.5 text-primary">
              <Activity className="shrink-0" size={18} />
            </span>
            <span className="truncate text-sm font-bold sm:text-base">Tracked runtime</span>
            <span className="badge badge-primary badge-outline badge-sm">
              {dashboard.trackedTunnels.length}
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-2 text-xs text-base-content/60">
            {isCollapsed ? "Show" : "Hide"}
            {isCollapsed ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
          </span>
        </button>

        {!isCollapsed ? (
          <div className="max-h-44 overflow-auto border-t border-base-200">
            <table className="table table-xs table-pin-rows">
              <thead>
                <tr>
                  <th className="bg-base-100 text-xs text-base-content/55">ID</th>
                  <th className="bg-base-100 text-xs text-base-content/55">Endpoint</th>
                  <th className="bg-base-100 text-xs text-base-content/55">Status</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {dashboard.trackedTunnels.map((tracked) => (
                  <tr key={tracked.runtimeKey}>
                    <td className="font-bold">
                      <div>{tracked.id}</div>
                      <div className="text-[0.65rem] font-normal text-base-content/50">
                        {tracked.runtimeScope}
                      </div>
                    </td>
                    <td className="max-w-md truncate font-mono text-xs">
                      {tracked.local} {" -> "} {tracked.remote}
                    </td>
                    <td>
                      <StatusBadge status={tracked.status.state} />
                    </td>
                    <td className="text-right">
                      <button
                        type="button"
                        className="btn btn-outline btn-xs"
                        onClick={() =>
                          onStop({ id: tracked.id, runtimeScope: tracked.runtimeScope })
                        }
                        disabled={isBusy}
                      >
                        <CircleStop size={13} />
                        Stop
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>
    </section>
  );
}

interface TunnelFormProps {
  form: TunnelFormState;
  canUseLocal: boolean;
  isBusy: boolean;
  onChange: (field: keyof TunnelFormState, value: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onOpenSettings: () => void;
  onBrowseIdentityFile: () => void;
}

/**
 * 設定追加フォームを表示する
 */
function TunnelForm({
  form,
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
      className="overflow-hidden rounded-lg border border-base-300 bg-base-100 shadow-sm"
      onSubmit={onSubmit}
    >
      <div className="flex flex-col gap-4 border-b border-base-300 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <CirclePlus className="text-primary" size={18} />
          <h2 className="text-base font-bold">Add tunnel</h2>
        </div>

        <div className="join w-full rounded-md bg-base-200/60 p-1 sm:w-72">
          <button
            type="button"
            className={`btn btn-sm join-item flex-1 ${
              form.scope === "local" ? "btn-primary" : "btn-ghost"
            }`}
            onClick={() => onChange("scope", "local")}
            disabled={localUnavailable}
          >
            Local
          </button>
          <button
            type="button"
            className={`btn btn-sm join-item flex-1 ${
              form.scope === "global" ? "btn-primary" : "btn-ghost"
            }`}
            onClick={() => onChange("scope", "global")}
          >
            Global
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-5 p-5 xl:grid-cols-3">
        {localUnavailable && form.scope === "local" ? (
          <div className="xl:col-span-3">
            <AlertMessage kind="warning">
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                <span>local 設定に追加するには Settings でワークスペースを選択してください。</span>
                <button
                  type="button"
                  className="btn btn-outline btn-xs"
                  onClick={onOpenSettings}
                  disabled={isBusy}
                >
                  <Settings2 size={13} />
                  Settings
                </button>
              </div>
            </AlertMessage>
          </div>
        ) : null}
        <section className="flex flex-col gap-3">
          <h3 className="text-xs font-bold uppercase tracking-wide text-base-content/50">
            Identity
          </h3>
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

        <section className="flex flex-col gap-3 border-t border-base-300 pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-5">
          <h3 className="text-xs font-bold uppercase tracking-wide text-base-content/50">
            Routing
          </h3>
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

        <section className="flex flex-col gap-3 border-t border-base-300 pt-4 xl:border-t-0 xl:border-l xl:pt-0 xl:pl-5">
          <h3 className="text-xs font-bold uppercase tracking-wide text-base-content/50">SSH</h3>
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
            <button
              type="button"
              className="btn btn-outline btn-sm mb-0"
              onClick={onBrowseIdentityFile}
              disabled={isBusy}
            >
              <FolderOpen size={15} />
              Browse
            </button>
          </div>
        </section>
      </div>

      <div className="flex justify-end border-t border-base-300 px-5 py-4">
        <button
          className="btn btn-primary btn-sm"
          type="submit"
          disabled={isBusy || (localUnavailable && form.scope === "local")}
        >
          <CirclePlus size={16} />
          Add tunnel
        </button>
      </div>
    </form>
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
    <label className="form-control w-full">
      <div className="label py-1">
        <span className="label-text text-xs font-semibold">{label}</span>
      </div>
      <input
        className="input input-bordered input-sm w-full text-sm"
        value={value}
        onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(event.target.value)}
        placeholder={placeholder}
        inputMode={inputMode}
        required={required}
        disabled={disabled}
      />
    </label>
  );
}

interface ConfirmRemoveModalProps {
  tunnel: TunnelView | null;
  isBusy: boolean;
  onCancel: () => void;
  onConfirm: (tunnel: TunnelView) => void;
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
    <div className="modal modal-open" role="dialog" aria-modal="true">
      <div className="modal-box">
        <h3 className="text-lg font-bold">Remove tunnel</h3>
        <p className="py-4">
          {tunnel.id} を {tunnel.source} 設定から削除します。この操作は設定ファイルを書き換えます。
        </p>
        <div className="modal-action">
          <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={isBusy}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-error"
            onClick={() => onConfirm(tunnel)}
            disabled={isBusy}
          >
            Remove
          </button>
        </div>
      </div>
      <button className="modal-backdrop" type="button" onClick={onCancel} disabled={isBusy}>
        close
      </button>
    </div>
  );
}

/**
 * トンネル一覧を画面上の絞り込み条件で抽出する
 */
function filterTunnels(tunnels: TunnelView[], filters: TunnelFilters): TunnelView[] {
  const query = filters.query.trim().toLowerCase();

  return tunnels.filter((tunnel) => {
    const status = tunnelStatus(tunnel);
    const matchesStatus = filters.status === "all" || filters.status === status;
    const matchesScope = filters.scope === "all" || filters.scope === tunnel.source;
    const matchesTags = filters.tags.every((tag) => tunnel.tags.includes(tag));
    const matchesQuery = query.length === 0 || tunnelContainsQuery(tunnel, query);

    return matchesStatus && matchesScope && matchesTags && matchesQuery;
  });
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
  const fields = [
    tunnel.id,
    tunnel.description ?? "",
    tunnel.local,
    tunnel.remote,
    tunnel.ssh,
    tunnel.source,
    tunnel.sourcePath,
    ...tunnel.tags,
  ];

  return fields.some((field) => field.toLowerCase().includes(query));
}

/**
 * ダッシュボードの集計値を算出する
 */
function calculateStats(dashboard: DashboardState | null): DashboardStats {
  if (dashboard === null) {
    return { configured: 0, running: 0, stale: 0 };
  }

  return {
    configured: dashboard.tunnels.length,
    running: dashboard.trackedTunnels.filter((tracked) => tracked.status.state === "running")
      .length,
    stale: dashboard.trackedTunnels.filter((tracked) => tracked.status.state === "stale").length,
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
  };
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
 * 現在存在するトンネルだけを選択状態として残す
 */
function keepExistingSelections(current: Set<string>, tunnels: TunnelView[]): Set<string> {
  const ids = new Set(tunnels.map((tunnel) => tunnel.id));
  return new Set(Array.from(current).filter((id) => ids.has(id)));
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
function operationMessage(report: OperationReport): AppMessage {
  const successCount = report.succeeded.length;
  const failureCount = report.failed.length;

  if (failureCount === 0) {
    return {
      kind: "success",
      text: `${successCount} 件の操作が完了しました`,
    };
  }

  const failed = report.failed.map((failure) => `${failure.id}: ${failure.message}`).join(" / ");
  return {
    kind: successCount > 0 ? "info" : "error",
    text: `${successCount} 件成功、${failureCount} 件失敗しました。${failed}`,
  };
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
