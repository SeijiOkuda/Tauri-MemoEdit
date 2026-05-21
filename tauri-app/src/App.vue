<script setup lang="ts">
import { onMounted, onUnmounted, ref, computed, nextTick, watch } from "vue";
import { open, save, ask } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readTextFile } from "@tauri-apps/plugin-fs";
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { exit } from '@tauri-apps/plugin-process';
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from '@tauri-apps/plugin-opener';

type DiffLineType = 'same' | 'local-only' | 'drive-only';
interface DiffLine { text: string; type: DiffLineType; }
interface ConflictDialogData {
  tab: Tab;
  localContent: string;
  driveContent: string;
  resolve: (choice: 'local' | 'drive') => void;
}

interface AuthUser {
  name: string;
  email: string;
  picture: string | null;
}

interface Tab {
  id: string;
  name: string;
  text: string;
  textSaved: string;
  isDirty?: boolean;
  path: string | null;
  charCode: string;
  driveFileId: string | null;
  cloudSync: boolean;
  cloudStatus: 'synced' | 'syncing' | 'error' | 'none';
  isLoading?: boolean;
  loadingError?: string | null;
}

let nextId = 1;
function createTab(): Tab {
  return { id: String(nextId++), name: "新しいファイル", text: "", textSaved: "", path: null, charCode: "utf-8", driveFileId: null, cloudSync: false, cloudStatus: 'none' };
}

const tabs = ref<Tab[]>([createTab()]);
const activeTabId = ref(tabs.value[0].id);
const activeTab = computed(() => tabs.value.find(t => t.id === activeTabId.value)!);
const activeTabText = computed({
  get: () => activeTab.value.text,
  set: (value: string) => {
    const tab = activeTab.value;
    tab.text = value;
    tab.isDirty = value !== tab.textSaved;
  },
});

const isMenuFile = ref(false);
const isMenuEncoding = ref(false);
const isMenuAuth = ref(false);
const menuRef = ref<HTMLElement | null>(null);
const menuEncodingRef = ref<HTMLElement | null>(null);
const menuAuthRef = ref<HTMLElement | null>(null);
const textarea = ref<HTMLTextAreaElement | null>(null);
const authUser = ref<AuthUser | null>(null);

const tabContextMenu = ref<{ tab: Tab; x: number; y: number } | null>(null);
const conflictDialog = ref<ConflictDialogData | null>(null);
const diffResult = computed(() => {
  if (!conflictDialog.value) return { local: [] as DiffLine[], drive: [] as DiffLine[] };
  return computeDiff(conflictDialog.value.localContent, conflictDialog.value.driveContent);
});

// セッション永続化
const SESSION_KEY = 'memo-edit-tab-session';
const DRIVE_SESSION_FILENAME = '__memo_edit_session__';
const DRIVE_SESSION_FILE_KEY = 'memo-edit-drive-session-file-id';
const APP_FOLDER_KEY = 'memo-edit-drive-folder-id';
const ZOOM_KEY = 'memo-edit-zoom';
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 3;
const ZOOM_STEP = 0.1;
interface SavedTab { path: string; charCode: string; }
interface SavedTabEntry { path?: string; charCode?: string; driveFileId?: string; name?: string; }
interface TabSession { tabs?: SavedTabEntry[]; localTabs?: SavedTab[]; activeTabPath: string | null; activeTabDriveId: string | null; }
function saveTabSession() {
  const savedTabs: SavedTabEntry[] = tabs.value
    .filter(t => t.path !== null || (!t.path && !!t.driveFileId && t.cloudSync))
    .map(t => t.path
      ? { path: t.path!, charCode: t.charCode, name: t.name, ...(t.driveFileId ? { driveFileId: t.driveFileId } : {}) }
      : { driveFileId: t.driveFileId!, name: t.name }
    );
  const active = activeTab.value;
  const session: TabSession = {
    tabs: savedTabs,
    activeTabPath: active?.path ?? null,
    activeTabDriveId: (!active?.path && active?.driveFileId) ? active.driveFileId : null,
  };
  localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

function clampZoom(value: number) {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(value * 100) / 100));
}

function readSavedZoom() {
  const saved = Number(localStorage.getItem(ZOOM_KEY));
  return Number.isFinite(saved) ? clampZoom(saved) : 1;
}

const appZoom = ref(readSavedZoom());
const zoomPercent = computed(() => `${Math.round(appZoom.value * 100)}%`);

function applyZoom() {
  const zoom = clampZoom(appZoom.value);
  appZoom.value = zoom;
  localStorage.setItem(ZOOM_KEY, String(zoom));
}

function changeZoom(delta: number) {
  appZoom.value = clampZoom(appZoom.value + delta);
  applyZoom();
}

function resetZoom() {
  appZoom.value = 1;
  applyZoom();
}

async function getOrCreateAppFolder(token: string): Promise<string> {
  const cached = localStorage.getItem(APP_FOLDER_KEY);
  if (cached) return cached;

  // 既存フォルダを検索（別端末からの初回アクセス等）
  const folders = await invoke<{ id: string; name: string }[]>('drive_list_files', {
    query: `mimeType = 'application/vnd.google-apps.folder' and name contains 'Tauri-Memo_' and trashed = false`,
    accessToken: token,
  });
  if (folders.length > 0) {
    localStorage.setItem(APP_FOLDER_KEY, folders[0].id);
    return folders[0].id;
  }

  // 新規作成
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  const ts = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  const folder = await invoke<{ id: string }>('drive_create_folder', {
    name: `Tauri-Memo_${ts}`,
    accessToken: token,
  });
  localStorage.setItem(APP_FOLDER_KEY, folder.id);
  return folder.id;
}

async function saveDriveSession(token: string) {
  try {
    // DriveセッションはdriveFileIdとnameのみ（pathは端末依存なので除外）
    const driveTabs = tabs.value
      .filter(t => !!t.driveFileId && t.cloudSync)
      .map(t => ({ driveFileId: t.driveFileId!, name: t.name }));
    const content = JSON.stringify({ tabs: driveTabs });

    const folderId = await getOrCreateAppFolder(token);
    let fileId = localStorage.getItem(DRIVE_SESSION_FILE_KEY);
    if (fileId) {
      await invoke('drive_update_file', { fileId, content, accessToken: token });
    } else {
      const files = await invoke<{ id: string; name: string }[]>('drive_list_files', {
        query: `'${folderId}' in parents and name = '${DRIVE_SESSION_FILENAME}' and trashed = false`,
        accessToken: token,
      });
      fileId = files[0]?.id ?? null;
      if (fileId) {
        await invoke('drive_update_file', { fileId, content, accessToken: token });
      } else {
        const created = await invoke<{ id: string }>('drive_create_file', {
          name: DRIVE_SESSION_FILENAME,
          content,
          accessToken: token,
          parentId: folderId,
        });
        fileId = created.id;
      }
      localStorage.setItem(DRIVE_SESSION_FILE_KEY, fileId!);
    }
  } catch (err) {
    console.error('Drive session save failed:', err);
  }
}

// セットアップウィザード
const showSetupWizard = ref(false);
const setupStep = ref(1);
const setupClientId = ref('');
const setupClientSecret = ref('');
const setupSaving = ref(false);
const setupError = ref('');

function tabName(tab: Tab) {
  return tab.name;
}

function cloudIconTitle(tab: Tab) {
  const status = tab.cloudStatus === 'synced' ? '同期済み' : tab.cloudStatus === 'syncing' ? '同期中...' : '同期エラー';
  const type = tab.path ? 'ローカル+クラウド' : 'クラウド専用';
  return `${type} (${status})`;
}

function isUnsaved(tab: Tab) {
  return !!tab.isDirty;
}

function needsLocalSaveConfirm(tab: Tab) {
  return !!tab.isDirty && !!tab.path;
}

async function confirmIfUnsaved(action: 'exit' | 'reopen' | 'close', tab?: Tab): Promise<boolean> {
  const hasUnsaved = action === 'exit'
    ? tabs.value.some(needsLocalSaveConfirm)
    : isUnsaved(tab ?? activeTab.value);
  if (!hasUnsaved) return true;
  const message = action === 'exit'
    ? "変更が保存されていません。終了しますか？"
    : action === 'reopen'
    ? "変更が保存されていません。保存せずに再度開き直しますか？"
    : "変更が保存されていません。タブを閉じますか？";
  return await ask(message, {
    title: "確認", kind: 'warning', okLabel: "はい", cancelLabel: "いいえ"
  });
}

async function loadFileIntoTab(tab: Tab, filePath: string, encoding: string = "utf-8") {
  tab.isLoading = true;
  tab.loadingError = null;
  try {
    const content = await readTextFile(filePath, { encoding });
    tab.text = content;
    tab.textSaved = content;
    tab.isDirty = false;
  } catch (err) {
    tab.loadingError = String(err);
    throw err;
  } finally {
    tab.isLoading = false;
  }
}

function prepareLocalTab(tab: Tab, filePath: string, encoding: string = "utf-8") {
  tab.path = filePath;
  tab.name = filePath.split(/[\\/]/).pop() ?? filePath;
  tab.charCode = encoding;
  tab.text = "";
  tab.textSaved = "";
  tab.isDirty = false;
  tab.loadingError = null;
  tab.isLoading = true;
}

function reusableBlankTab() {
  return tabs.value.find(t => !t.path && !t.driveFileId && t.text === '' && !t.cloudSync);
}

async function reconcileLocalDrive(tab: Tab, filePath: string, token: string | null, knownDriveId?: string) {
  const driveId = knownDriveId ?? (await invoke<{ local: Record<string, string> }>('mapping_get_all')).local[filePath];
  if (!driveId) return;

  tab.driveFileId = driveId;
  tab.cloudSync = true;
  tab.cloudStatus = 'syncing';
  try {
    if (token) {
      const folderId = await getOrCreateAppFolder(token);
      await invoke('drive_move_to_folder', { fileId: driveId, folderId, accessToken: token }).catch(() => {});
      const driveContent = await invoke<string>('drive_get_file_content', { fileId: driveId, accessToken: token });
      if (driveContent !== tab.text) {
        const choice = await showConflictDialog(tab, tab.text, driveContent);
        if (choice === 'drive') {
          tab.text = driveContent;
          tab.textSaved = driveContent;
          tab.isDirty = false;
        } else {
          tab.isDirty = tab.text !== tab.textSaved;
        }
      }
    }
    tab.cloudStatus = 'synced';
  } catch (err) {
    tab.cloudStatus = 'error';
    console.error('drive reconcile failed', err);
  }
}

async function loadLocalTabInBackground(tab: Tab, entry: SavedTabEntry, token: string | null) {
  if (!entry.path) return;
  try {
    await loadFileIntoTab(tab, entry.path, entry.charCode ?? "utf-8");
    if (entry.charCode) tab.charCode = entry.charCode;
    await reconcileLocalDrive(tab, entry.path, token, entry.driveFileId);
    saveTabSession();
  } catch (err) {
    console.error("file load failed", err);
    if (entry.driveFileId && token) {
      await loadDriveTabInBackground(tab, entry, token);
    }
  }
}

async function loadDriveTabInBackground(tab: Tab, entry: SavedTabEntry, token: string) {
  if (!entry.driveFileId) return;
  tab.isLoading = true;
  tab.loadingError = null;
  tab.cloudStatus = 'syncing';
  try {
    const folderId = await getOrCreateAppFolder(token);
    await invoke('drive_move_to_folder', { fileId: entry.driveFileId, folderId, accessToken: token }).catch(() => {});
    const content = await invoke<string>('drive_get_file_content', { fileId: entry.driveFileId, accessToken: token });
    tab.name = entry.name ?? tab.name;
    tab.text = content;
    tab.textSaved = content;
    tab.isDirty = false;
    tab.path = null;
    tab.driveFileId = entry.driveFileId;
    tab.cloudSync = true;
    tab.cloudStatus = 'synced';
    saveTabSession();
  } catch (err) {
    tab.loadingError = String(err);
    tab.cloudStatus = 'error';
    await invoke('mapping_remove_cloud_only', { driveFileId: entry.driveFileId }).catch(() => {});
  } finally {
    tab.isLoading = false;
  }
}

let unlistenAuth: UnlistenFn | null = null;
let unlistenAuthExpired: UnlistenFn | null = null;

async function restoreTabSession(token: string | null, clearExisting = false) {
  // セッションデータを読み込み（ログイン時はDriveを優先、なければlocalStorage）
  let savedSession: TabSession | null = null;
  let fromDrive = false;
  if (token) {
    try {
      let driveSessionFileId = localStorage.getItem(DRIVE_SESSION_FILE_KEY);
      if (!driveSessionFileId) {
        const folderId = await getOrCreateAppFolder(token);
        const files = await invoke<{ id: string; name: string }[]>('drive_list_files', {
          query: `'${folderId}' in parents and name = '${DRIVE_SESSION_FILENAME}' and trashed = false`,
          accessToken: token,
        });
        driveSessionFileId = files[0]?.id ?? null;
        if (driveSessionFileId) localStorage.setItem(DRIVE_SESSION_FILE_KEY, driveSessionFileId);
      }
      if (driveSessionFileId) {
        const content = await invoke<string>('drive_get_file_content', {
          fileId: driveSessionFileId,
          accessToken: token,
        });
        savedSession = JSON.parse(content);
        fromDrive = true;
      }
    } catch {}
  }
  if (!savedSession) {
    const sessionStr = localStorage.getItem(SESSION_KEY);
    if (sessionStr) {
      try { savedSession = JSON.parse(sessionStr); } catch {}
    }
  }

  // Driveセッション復元時: ローカルマッピングを逆引きしてpathを補完
  if (fromDrive && savedSession?.tabs) {
    try {
      const mappings = await invoke<{ local: Record<string, string> }>('mapping_get_all');
      const driveIdToPath: Record<string, string> = {};
      for (const [path, driveId] of Object.entries(mappings.local)) {
        driveIdToPath[driveId] = path;
      }
      for (const entry of savedSession.tabs) {
        if (entry.driveFileId && !entry.path) {
          const localPath = driveIdToPath[entry.driveFileId];
          if (localPath) entry.path = localPath;
        }
      }
    } catch {}
  }

  // ログイン後復元の場合、既存の空タブをリセット
  if (clearExisting && savedSession) {
    tabs.value = [createTab()];
    activeTabId.value = tabs.value[0].id;
  }

  // タブをセッション順に復元（旧形式 localTabs にも対応）
  const sessionTabs: SavedTabEntry[] = savedSession?.tabs
    ?? (savedSession?.localTabs?.map(t => ({ path: t.path, charCode: t.charCode })) ?? []);
  if (sessionTabs.length) {
    const backgroundLoads: Promise<void>[] = [];
    for (const entry of sessionTabs) {
      if (entry.path) {
        const blankTab = reusableBlankTab();
        const tab = blankTab ?? createTab();
        if (!blankTab) tabs.value.push(tab);
        prepareLocalTab(tab, entry.path, entry.charCode ?? "utf-8");
        if (entry.driveFileId) {
          tab.driveFileId = entry.driveFileId;
          tab.cloudSync = true;
          tab.cloudStatus = 'syncing';
        }
        backgroundLoads.push(loadLocalTabInBackground(tab, entry, token));
      } else if (entry.driveFileId && token) {
        const blankTab = reusableBlankTab();
        const tab = blankTab ?? createTab();
        if (!blankTab) tabs.value.push(tab);
        tab.name = entry.name ?? tab.name;
        tab.text = "";
        tab.textSaved = "";
        tab.isDirty = false;
        tab.path = null;
        tab.driveFileId = entry.driveFileId;
        tab.cloudSync = true;
        tab.cloudStatus = 'syncing';
        tab.isLoading = true;
        tab.loadingError = null;
        backgroundLoads.push(loadDriveTabInBackground(tab, entry, token));
      }
    }
    void Promise.allSettled(backgroundLoads);
  }

  // アクティブタブを復元
  if (savedSession) {
    if (savedSession.activeTabPath) {
      const restoredActive = tabs.value.find(t => t.path === savedSession!.activeTabPath);
      if (restoredActive) activeTabId.value = restoredActive.id;
    } else if (savedSession.activeTabDriveId) {
      const restoredActive = tabs.value.find(t => t.driveFileId === savedSession!.activeTabDriveId);
      if (restoredActive) activeTabId.value = restoredActive.id;
    }
    saveTabSession();
  }
}

onMounted(async () => {
  await listen('open-file', async (event: { payload: string }) => {
    await openFileInTab(event.payload);
  });

  await listen("app-close-requested", async () => {
    await closeToTray();
  });

  await listen("app-quit-requested", async () => {
    await exitApp();
  });

  // 保存済みの認証状態を復元
  const user = await invoke<AuthUser | null>('get_auth_user');
  if (user) authUser.value = user;

  // セッション復元
  const sessionToken = user ? await invoke<string | null>('get_access_token') : null;
  await restoreTabSession(sessionToken);

  // 認証イベントを購読
  unlistenAuth = await listen<AuthUser>('auth-complete', async (event) => {
    authUser.value = event.payload;
    const token = await invoke<string | null>('get_access_token').catch(() => null);
    await restoreTabSession(token, true);
  });
  unlistenAuthExpired = await listen('auth-expired', () => {
    authUser.value = null;
  });

  window.addEventListener("keydown", handleKeyDown);
  window.addEventListener("wheel", handleWheel, { passive: false });
  window.addEventListener("click", handleClickOutside);

  applyZoom();
  invoke("frontend_ready");
});

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeyDown);
  window.removeEventListener('wheel', handleWheel);
  window.removeEventListener('click', handleClickOutside);
  unlistenAuth?.();
  unlistenAuthExpired?.();
});

function handleClickOutside(e: MouseEvent) {
  if (menuRef.value && !menuRef.value.contains(e.target as Node)) {
    isMenuFile.value = false;
  }
  if (menuEncodingRef.value && !menuEncodingRef.value.contains(e.target as Node)) {
    isMenuEncoding.value = false;
  }
  if (menuAuthRef.value && !menuAuthRef.value.contains(e.target as Node)) {
    isMenuAuth.value = false;
  }
  tabContextMenu.value = null;
}

// -----------------------------------------------------------------------
// LCSベースの行差分計算
// -----------------------------------------------------------------------
function computeDiff(localText: string, driveText: string): { local: DiffLine[]; drive: DiffLine[] } {
  const a = localText.split('\n');
  const b = driveText.split('\n');
  const m = a.length;
  const n = b.length;

  // LCS DPテーブル
  const dp: number[][] = Array.from({ length: m + 1 }, () => new Array(n + 1).fill(0));
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] = a[i-1] === b[j-1] ? dp[i-1][j-1] + 1 : Math.max(dp[i-1][j], dp[i][j-1]);
    }
  }

  // バックトラック
  const localLines: DiffLine[] = [];
  const driveLines: DiffLine[] = [];
  let i = m, j = n;
  while (i > 0 || j > 0) {
    if (i > 0 && j > 0 && a[i-1] === b[j-1]) {
      localLines.unshift({ text: a[i-1], type: 'same' });
      driveLines.unshift({ text: b[j-1], type: 'same' });
      i--; j--;
    } else if (j > 0 && (i === 0 || dp[i][j-1] >= dp[i-1][j])) {
      driveLines.unshift({ text: b[j-1], type: 'drive-only' });
      j--;
    } else {
      localLines.unshift({ text: a[i-1], type: 'local-only' });
      i--;
    }
  }
  return { local: localLines, drive: driveLines };
}

function showConflictDialog(tab: Tab, localContent: string, driveContent: string): Promise<'local' | 'drive'> {
  return new Promise((resolve) => {
    conflictDialog.value = { tab, localContent, driveContent, resolve };
  });
}

function resolveConflict(choice: 'local' | 'drive') {
  if (!conflictDialog.value) return;
  conflictDialog.value.resolve(choice);
  conflictDialog.value = null;
}

// -----------------------------------------------------------------------
function generateTimestampFilename(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}.txt`;
}

async function initCloudForTab(tab: Tab) {
  try {
    const token = await invoke<string | null>('get_access_token');
    if (!token) return;
    tab.cloudStatus = 'syncing';
    const name = generateTimestampFilename();
    const folderId = await getOrCreateAppFolder(token);
    const file = await invoke<{ id: string }>('drive_create_file', {
      name,
      content: tab.text,
      accessToken: token,
      parentId: folderId,
    });
    tab.driveFileId = file.id;
    tab.cloudSync = true;
    tab.textSaved = tab.text;
    tab.isDirty = false;
    tab.cloudStatus = 'synced';

    // マッピングを保存
    if (tab.path) {
      await invoke('mapping_set_local', { localPath: tab.path, driveFileId: file.id });
    } else {
      await invoke('mapping_add_cloud_only', { driveFileId: file.id, name });
    }
  } catch (err) {
    tab.cloudStatus = 'error';
    console.error('Drive file creation failed:', err);
  }
}

async function syncTabToCloud(tab: Tab) {
  if (!tab.driveFileId || !tab.cloudSync) return;
  try {
    const token = await invoke<string | null>('get_access_token');
    if (!token) { tab.cloudStatus = 'error'; return; }
    await invoke('drive_update_file', {
      fileId: tab.driveFileId,
      content: tab.text,
      accessToken: token,
    });
    tab.textSaved = tab.text;
    tab.isDirty = false;
    tab.cloudStatus = 'synced';
  } catch (err) {
    tab.cloudStatus = 'error';
    console.error('Cloud sync failed:', err);
  }
}

function showTabContextMenu(tab: Tab, event: MouseEvent) {
  tabContextMenu.value = { tab, x: event.clientX, y: event.clientY };
}

async function disableCloudSync(tab: Tab) {
  tabContextMenu.value = null;
  if (tab.driveFileId) {
    try {
      const token = await invoke<string | null>('get_access_token');
      if (token) await invoke('drive_delete_file', { fileId: tab.driveFileId, accessToken: token });
    } catch (err) {
      console.error('Drive delete failed:', err);
    }
    // マッピングを削除
    if (tab.path) {
      await invoke('mapping_remove_local', { localPath: tab.path });
    } else {
      await invoke('mapping_remove_cloud_only', { driveFileId: tab.driveFileId });
    }
  }
  tab.driveFileId = null;
  tab.cloudSync = false;
  tab.cloudStatus = 'none';
}

async function enableCloudSync(tab: Tab) {
  tabContextMenu.value = null;
  await initCloudForTab(tab);
}

async function loginGoogle() {
  const config = await invoke<{ client_id: string; client_secret: string }>('get_oauth_config');
  if (!config.client_id) {
    setupClientId.value = '';
    setupClientSecret.value = '';
    setupError.value = '';
    setupStep.value = 1;
    showSetupWizard.value = true;
    return;
  }
  try {
    await invoke('start_google_auth');
  } catch (err) {
    await ask(String(err), { title: "ログインエラー", kind: 'error', okLabel: "OK", cancelLabel: "OK" });
  }
}

async function logoutGoogle() {
  await invoke('sign_out');
  authUser.value = null;
  isMenuAuth.value = false;
}

async function saveOAuthSetup() {
  setupError.value = '';
  if (!setupClientId.value.trim() || !setupClientSecret.value.trim()) {
    setupError.value = 'クライアントIDとシークレットを両方入力してください。';
    return;
  }
  if (!setupClientId.value.includes('googleusercontent.com')) {
    setupError.value = 'クライアントIDの形式が正しくありません。\n（例: 123456789-xxx.apps.googleusercontent.com）';
    return;
  }
  setupSaving.value = true;
  try {
    await invoke('save_oauth_config', {
      clientId: setupClientId.value.trim(),
      clientSecret: setupClientSecret.value.trim(),
    });
    showSetupWizard.value = false;
    await invoke('start_google_auth');
  } catch (err) {
    setupError.value = String(err);
  } finally {
    setupSaving.value = false;
  }
}

async function openFileInTab(filePath: string) {
  const cur = activeTab.value;
  const isBlank = !cur.path && cur.text === "" && !isUnsaved(cur);
  const tab = isBlank ? cur : createTab();
  if (!isBlank) tabs.value.push(tab);

  // 空タブをローカルファイルタブに置き換える際、cloud_onlyマッピングを削除する（Uの操作）
  if (isBlank && tab.driveFileId) {
    try { await invoke('mapping_remove_cloud_only', { driveFileId: tab.driveFileId }); } catch {}
    tab.driveFileId = null;
    tab.cloudSync = false;
    tab.cloudStatus = 'none';
  }

  tab.path = filePath;
  tab.name = filePath.split(/[\\/]/).pop() ?? filePath;
  tab.charCode = "utf-8";
  tab.text = "";
  tab.textSaved = "";
  tab.isDirty = false;
  tab.loadingError = null;
  activeTabId.value = tab.id;
  nextTick(() => textarea.value?.focus());
  void (async () => {
    try {
      await loadFileIntoTab(tab, filePath);
      const token = await invoke<string | null>('get_access_token').catch(() => null);
      await reconcileLocalDrive(tab, filePath, token);
      saveTabSession();
    } catch (err) {
      console.error("file load failed", err);
    }
  })();
  return true;
  try {
    await loadFileIntoTab(tab, filePath);

    // マッピングからDriveの紐づけを復元し、競合チェック
    const mappings = await invoke<{ local: Record<string, string>; cloud_only: { drive_file_id: string; name: string }[] }>('mapping_get_all');
    const driveId = mappings.local[filePath];
    if (driveId) {
      try {
        const token = await invoke<string | null>('get_access_token');
        if (token) {
          // フォルダ外のファイルを移動（既存マッピングの移行）
          const folderId = await getOrCreateAppFolder(token as string);
          await invoke('drive_move_to_folder', { fileId: driveId, folderId, accessToken: token }).catch(() => {});

          const driveContent = await invoke<string>('drive_get_file_content', {
            fileId: driveId,
            accessToken: token,
          });
          if (driveContent !== tab.text) {
            // 差分あり → 競合ダイアログ
            const choice = await showConflictDialog(tab, tab.text, driveContent);
            if (choice === 'drive') {
              tab.text = driveContent;
              tab.textSaved = driveContent;
              tab.isDirty = false;
            }
          }
        }
      } catch (err) {
        console.error('競合チェック失敗:', err);
      }
      tab.driveFileId = driveId;
      tab.cloudSync = true;
      tab.cloudStatus = 'synced';
    }
    saveTabSession();
    return true;
  } catch (err) {
    console.error("❌ ファイル読み込み失敗:", err);
    if (!isBlank) {
      tabs.value = tabs.value.filter(t => t.id !== tab.id);
      if (activeTabId.value === tab.id) {
        activeTabId.value = tabs.value.find(t => t.id === cur.id)?.id ?? tabs.value[0]?.id ?? '';
      }
    } else {
      tab.path = null;
      tab.charCode = 'utf-8';
    }
    return false;
  }
}

async function saveFile() {
  const tab = activeTab.value;

  // クラウド専用タブ（ローカルパスなし）はDriveへ直接保存
  if (!tab.path && tab.cloudSync && tab.driveFileId) {
    await syncTabToCloud(tab);
    return;
  }

  const wasCloudOnly = tab.cloudSync && !tab.path;
  if (!tab.path) {
    const newPath = await save({
      filters: [{ name: 'Text Files', extensions: ['txt'] }],
      defaultPath: 'memo.txt',
    });
    if (!newPath) return;
    tab.path = newPath;
    tab.name = newPath.split(/[\\/]/).pop() ?? newPath;
    // クラウド専用→ローカル+クラウドに昇格: マッピングを更新
    if (wasCloudOnly && tab.driveFileId) {
      await invoke('mapping_set_local', { localPath: newPath, driveFileId: tab.driveFileId });
      await invoke('mapping_remove_cloud_only', { driveFileId: tab.driveFileId });
    }
  }

  if (tab.charCode !== "utf-8") {
    const ok = await ask(`現在のエンコードは${tab.charCode}です。utf-8で保存しますか？`, {
      title: "確認", kind: 'warning', okLabel: "はい", cancelLabel: "いいえ"
    });
    if (!ok) return;
  }

  try {
    await writeTextFile(tab.path, tab.text);
    tab.textSaved = tab.text;
    tab.isDirty = false;
    tab.charCode = "utf-8";
    // ローカル保存と同時にクラウドへも同期
    if (tab.cloudSync && tab.driveFileId) {
      await syncTabToCloud(tab);
    }
  } catch (err) {
    console.error("❌ ファイル保存失敗:", err);
  }
}

async function downloadCloudTab(tab: Tab) {
  const newPath = await save({
    filters: [{ name: 'Text Files', extensions: ['txt'] }],
    defaultPath: 'memo.txt',
  });
  if (!newPath) return;

  const wasCloudOnly = tab.cloudSync && !tab.path;
  tab.path = newPath;
  tab.name = newPath.split(/[\\/]/).pop() ?? newPath;
  if (wasCloudOnly && tab.driveFileId) {
    await invoke('mapping_set_local', { localPath: newPath, driveFileId: tab.driveFileId });
    await invoke('mapping_remove_cloud_only', { driveFileId: tab.driveFileId });
  }

  try {
    await writeTextFile(tab.path, tab.text);
    tab.textSaved = tab.text;
    tab.isDirty = false;
    tab.charCode = "utf-8";
    if (tab.cloudSync && tab.driveFileId) {
      await syncTabToCloud(tab);
    }
  } catch (err) {
    console.error("❌ ファイル保存失敗:", err);
  }
}

function handleKeyDown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault();
    saveFile();
    return;
  }
  if (e.ctrlKey || e.metaKey) {
    if (e.key === '+' || e.key === '=' || e.code === 'NumpadAdd') {
      e.preventDefault();
      changeZoom(ZOOM_STEP);
    } else if (e.key === '-' || e.key === '_' || e.code === 'Minus' || e.code === 'NumpadSubtract') {
      e.preventDefault();
      changeZoom(-ZOOM_STEP);
    } else if (e.key === '0') {
      e.preventDefault();
      resetZoom();
    }
  }
}

function handleWheel(e: WheelEvent) {
  if (!e.ctrlKey && !e.metaKey) return;
  e.preventDefault();
  const delta = Math.abs(e.deltaY) >= Math.abs(e.deltaX) ? e.deltaY : e.deltaX;
  if (delta < 0) {
    changeZoom(ZOOM_STEP);
  } else if (delta > 0) {
    changeZoom(-ZOOM_STEP);
  }
}

function onFileClick() {
  isMenuFile.value = !isMenuFile.value;
}

function onEncodingClick() {
  isMenuEncoding.value = !isMenuEncoding.value;
}

async function openFileDialog() {
  const filePath = await open({
    filters: [{ name: 'Text Files', extensions: ['txt'] }],
  });
  if (filePath) {
    await openFileInTab(filePath as string);
  }
}

async function reOpenFile(tab: Tab, encoding: string) {
  if (!tab.path) return;
  if (!await confirmIfUnsaved('reopen', tab)) return;
  tab.charCode = encoding;
  try {
    await loadFileIntoTab(tab, tab.path, encoding);
  } catch (err) {
    console.error("❌ ファイル再読み込み失敗:", err);
  }
}

async function exitApp() {
  if (!await confirmIfUnsaved('exit')) return;
  await runBackgroundCloseTasks();
  await exit().catch(err => console.error("❌ アプリ終了失敗:", err));
}

async function closeToTray() {
  if (!await confirmIfUnsaved('exit')) return;
  await invoke('hide_main_window').catch(err => console.error("hide window failed", err));
  void runBackgroundCloseTasks();
}

async function runBackgroundCloseTasks() {
  saveTabSession();
  const cloudOnlyDirtyTabs = tabs.value.filter(t => !t.path && t.cloudSync && t.driveFileId && isUnsaved(t));
  await Promise.allSettled(cloudOnlyDirtyTabs.map(t => syncTabToCloud(t)));
  if (authUser.value) {
    const token = await invoke<string | null>('get_access_token');
    if (token) await saveDriveSession(token);
  }
}

async function newTab() {
  const tab = createTab();
  tabs.value.push(tab);
  activeTabId.value = tab.id;
  nextTick(() => textarea.value?.focus());
  if (authUser.value) {
    void initCloudForTab(tab);
  }
}

async function closeTab(tabId: string) {
  const tab = tabs.value.find(t => t.id === tabId);
  if (!tab) return;

  // タブを閉じる前にクラウド同期
  if (tab.cloudSync && tab.driveFileId) {
    await syncTabToCloud(tab);
  }

  if (!await confirmIfUnsaved('close', tab)) return;

  const idx = tabs.value.findIndex(t => t.id === tabId);
  tabs.value.splice(idx, 1);

  if (tabs.value.length === 0) {
    const newT = createTab();
    tabs.value.push(newT);
    activeTabId.value = newT.id;
    if (authUser.value) await initCloudForTab(newT);
  } else if (activeTabId.value === tabId) {
    activeTabId.value = tabs.value[Math.min(idx, tabs.value.length - 1)].id;
  }
  saveTabSession();
}

// タブ構造（パス・アクティブタブ）が変わるたびにセッションを保存
watch(
  () => tabs.value.map(t => `${t.path}|${t.charCode}|${t.driveFileId}`).join(',') + '::' + activeTabId.value,
  saveTabSession
);

const insertTab = (e: KeyboardEvent) => {
  if (textarea.value) {
    const start = textarea.value.selectionStart;
    const end = textarea.value.selectionEnd;
    const value = textarea.value.value;

    activeTabText.value = value.substring(0, start) + "\t" + value.substring(end);

    nextTick(() => {
      if (textarea.value) {
        textarea.value.selectionStart = textarea.value.selectionEnd = start + 1;
      }
    });

    e.preventDefault();
  }
};
</script>

<template>
  <nav class="menu-bar">
    <div class="menu-item" @click="onFileClick" ref="menuRef">
      ファイル(F)
      <div v-if="isMenuFile" class="dropdown">
        <div class="dropdown-item" @click="newTab">新しいタブ</div>
        <div class="dropdown-item" @click="openFileDialog">開く</div>
        <div class="dropdown-item" @click="saveFile">保存</div>
        <div class="dropdown-item" @click="closeToTray">終了</div>
      </div>
    </div>
    <div class="menu-item">編集(E)</div>
    <div class="menu-item">ヘルプ(H)</div>
    <div class="menu-spacer"></div>
    <div class="zoom-control" title="拡大率">
      <button class="zoom-button" @click="changeZoom(-ZOOM_STEP)" :disabled="appZoom <= MIN_ZOOM" title="縮小">-</button>
      <button class="zoom-percent" @click="resetZoom" title="100%に戻す">{{ zoomPercent }}</button>
      <button class="zoom-button" @click="changeZoom(ZOOM_STEP)" :disabled="appZoom >= MAX_ZOOM" title="拡大">+</button>
    </div>
    <div class="menu-auth" ref="menuAuthRef">
      <button v-if="!authUser" class="auth-login-btn" @click="loginGoogle" title="Googleでログイン">
        <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 12c2.7 0 4.8-2.1 4.8-4.8S14.7 2.4 12 2.4 7.2 4.5 7.2 7.2 9.3 12 12 12zm0 2.4c-3.2 0-9.6 1.6-9.6 4.8v2.4h19.2v-2.4c0-3.2-6.4-4.8-9.6-4.8z"/>
        </svg>
        ログイン
      </button>
      <div v-else class="auth-user-btn" @click="isMenuAuth = !isMenuAuth" title="アカウント">
        <img v-if="authUser.picture" :src="authUser.picture" class="auth-avatar" referrerpolicy="no-referrer" />
        <span v-else class="auth-initial">{{ authUser.name.charAt(0) }}</span>
        <div v-if="isMenuAuth" class="dropdown-auth">
          <div class="auth-info-name">{{ authUser.name }}</div>
          <div class="auth-info-email">{{ authUser.email }}</div>
          <div class="dropdown-divider"></div>
          <div class="dropdown-item" @click.stop="logoutGoogle">ログアウト</div>
        </div>
      </div>
    </div>
  </nav>
  <div class="tab-bar">
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="tab"
      :class="{ active: tab.id === activeTabId, loading: tab.isLoading, error: tab.loadingError }"
      @click="activeTabId = tab.id"
      @contextmenu.prevent="showTabContextMenu(tab, $event)"
    >
      <span v-if="tab.isLoading" class="tab-loading-icon" title="読み込み中"></span>
      <span v-else-if="tab.cloudStatus !== 'none'" class="tab-cloud-icon" :class="['cloud-' + tab.cloudStatus, tab.path ? 'cloud-local' : 'cloud-only']" :title="cloudIconTitle(tab)">
        <svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 24 24" fill="currentColor">
          <path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z"/>
        </svg>
        <svg v-if="tab.path" xmlns="http://www.w3.org/2000/svg" width="9" height="9" viewBox="0 0 24 24" fill="currentColor" style="margin-left:1px">
          <path d="M19 2H5C3.9 2 3 2.9 3 4v16c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-7 14c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm5-8H7V5h10v3z"/>
        </svg>
      </span>
      <span class="tab-name">{{ tabName(tab) }}{{ isUnsaved(tab) ? ' ●' : '' }}</span>
      <span class="tab-close" @click.stop="closeTab(tab.id)">×</span>
    </div>
    <div class="tab-new" @click="newTab">+</div>
  </div>
  <main class="fullscreen-container" :style="{ '--app-zoom': appZoom }">
    <textarea
      ref="textarea"
      v-model="activeTabText"
      class="cool-textarea"
      :class="{ loading: activeTab.isLoading }"
      :readonly="activeTab.isLoading"
      placeholder="ここにメモを入力..."
      @keydown.tab.prevent="insertTab"
      spellcheck="false"
      autofocus
    ></textarea>
    <div v-if="activeTab.isLoading || activeTab.loadingError" class="editor-status" :class="{ error: activeTab.loadingError }">
      {{ activeTab.loadingError ? 'ファイルを読み込めませんでした' : 'ファイルを読み込み中...' }}
    </div>
  </main>
  <!-- 競合ダイアログ -->
  <div v-if="conflictDialog" class="conflict-overlay">
    <div class="conflict-modal">
      <div class="conflict-header">
        <h2 class="conflict-title">競合が検出されました</h2>
        <p class="conflict-subtitle">{{ conflictDialog.tab.path ?? 'クラウドファイル' }} のローカル版とDrive版の内容が異なります。使用するバージョンを選んでください。</p>
        <div class="conflict-legend">
          <span class="legend-local">■ ローカルのみの行</span>
          <span class="legend-drive">■ Driveのみの行</span>
        </div>
      </div>
      <div class="conflict-panels">
        <div class="conflict-panel">
          <div class="conflict-panel-header">ローカル版 <span class="conflict-line-count">{{ diffResult.local.length }}行</span></div>
          <div class="diff-view">
            <div v-for="(line, idx) in diffResult.local" :key="idx" :class="['diff-line', 'diff-' + line.type]">
              <span class="diff-ln">{{ idx + 1 }}</span>
              <span class="diff-text">{{ line.text }}</span>
            </div>
          </div>
        </div>
        <div class="conflict-panel">
          <div class="conflict-panel-header">Drive版 <span class="conflict-line-count">{{ diffResult.drive.length }}行</span></div>
          <div class="diff-view">
            <div v-for="(line, idx) in diffResult.drive" :key="idx" :class="['diff-line', 'diff-' + line.type]">
              <span class="diff-ln">{{ idx + 1 }}</span>
              <span class="diff-text">{{ line.text }}</span>
            </div>
          </div>
        </div>
      </div>
      <div class="conflict-actions">
        <button class="conflict-btn conflict-btn-local" @click="resolveConflict('local')">ローカル版を使用</button>
        <button class="conflict-btn conflict-btn-drive" @click="resolveConflict('drive')">Drive版を使用</button>
      </div>
    </div>
  </div>

  <!-- タブ右クリックメニュー -->
  <div v-if="tabContextMenu" class="tab-context-menu" :style="{ left: tabContextMenu.x + 'px', top: tabContextMenu.y + 'px' }">
    <div v-if="tabContextMenu.tab.cloudSync" class="context-item" @click.stop="disableCloudSync(tabContextMenu.tab)">クラウド同期を解除</div>
    <div v-else-if="authUser" class="context-item" @click.stop="enableCloudSync(tabContextMenu.tab)">クラウドに同期する</div>
    <div v-if="tabContextMenu.tab.cloudSync && !tabContextMenu.tab.path" class="context-item" @click.stop="downloadCloudTab(tabContextMenu.tab); tabContextMenu = null">ダウンロード</div>
    <div class="context-item context-item-danger" @click.stop="closeTab(tabContextMenu.tab.id); tabContextMenu = null">タブを閉じる</div>
  </div>

  <!-- セットアップウィザード -->
  <div v-if="showSetupWizard" class="wizard-overlay">
    <div class="wizard-modal">

      <!-- ステップインジケーター -->
      <div class="wizard-steps">
        <div v-for="i in 5" :key="i" :class="['wizard-step-dot', { active: setupStep === i, done: setupStep > i }]"></div>
      </div>
      <div class="wizard-step-label">ステップ {{ setupStep }} / 5</div>

      <!-- ステップ 1: はじめに -->
      <div v-if="setupStep === 1" class="wizard-content">
        <div class="wizard-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="#4285f4">
            <path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z"/>
          </svg>
        </div>
        <h2 class="wizard-title">Google Drive 連携の設定</h2>
        <p class="wizard-desc">このアプリでメモをGoogle Driveに自動保存するには、Googleの開発者設定が一度だけ必要です。</p>
        <p class="wizard-desc">5つのステップで完了します。所要時間は約5〜10分です。</p>
        <ul class="wizard-checklist">
          <li>Google Cloudプロジェクトの作成</li>
          <li>Google Drive APIの有効化</li>
          <li>OAuth同意画面の設定</li>
          <li>認証情報の作成</li>
          <li>クライアントIDの入力</li>
        </ul>
      </div>

      <!-- ステップ 2: プロジェクト作成 -->
      <div v-if="setupStep === 2" class="wizard-content">
        <h2 class="wizard-title">Google Cloudプロジェクトを作成する</h2>
        <p class="wizard-desc">Googleのサービスを利用するために、無料の「Google Cloudプロジェクト」が必要です。</p>
        <ol class="wizard-steps-list">
          <li>下のボタンをクリックしてGoogle Cloud Consoleを開く</li>
          <li>ページ上部の「プロジェクトを選択」をクリック</li>
          <li>右上の「新しいプロジェクト」をクリック</li>
          <li>プロジェクト名を入力（例: MemoEdit）して「作成」をクリック</li>
          <li>作成したプロジェクトが選択されていることを確認</li>
        </ol>
        <button class="wizard-link-btn" @click="openUrl('https://console.cloud.google.com/')">
          Google Cloud Consoleを開く
          <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
        </button>
      </div>

      <!-- ステップ 3: Drive API有効化 -->
      <div v-if="setupStep === 3" class="wizard-content">
        <h2 class="wizard-title">Google Drive APIを有効にする</h2>
        <p class="wizard-desc">メモをDriveに保存するために、Google Drive APIを有効にします。</p>
        <ol class="wizard-steps-list">
          <li>下のボタンをクリックしてGoogle Drive APIのページを開く</li>
          <li>青い「有効にする」ボタンをクリック</li>
          <li>「APIが有効になりました」と表示されれば完了</li>
        </ol>
        <button class="wizard-link-btn" @click="openUrl('https://console.cloud.google.com/apis/library/drive.googleapis.com')">
          Google Drive APIを有効にする
          <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
        </button>
      </div>

      <!-- ステップ 4: 認証情報作成 -->
      <div v-if="setupStep === 4" class="wizard-content">
        <h2 class="wizard-title">OAuth認証情報を作成する</h2>

        <p class="wizard-section-title">4-1. 同意画面を設定する</p>
        <ol class="wizard-steps-list">
          <li>下のボタンから「OAuth同意画面」ページを開く</li>
          <li>「外部」を選択して「作成」をクリック</li>
          <li>アプリ名（例: MemoEdit）とサポートメールを入力→「保存して次へ」</li>
          <li>スコープ・省略可能の情報はそのまま「保存して次へ」を2回クリック</li>
          <li>「テストユーザー」セクションで「+ ADD USERS」をクリック</li>
          <li>ログインに使うGmailアドレスを入力して「追加」→「保存して次へ」</li>
        </ol>
        <div class="wizard-alert">
          テストユーザーへの追加を忘れると「アクセスをブロック」エラーが発生します。
          必ず自分のGmailアドレスを追加してください。
        </div>
        <button class="wizard-link-btn" @click="openUrl('https://console.cloud.google.com/apis/auth/consent')">
          OAuth同意画面を開く
          <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
        </button>

        <p class="wizard-section-title" style="margin-top:12px">4-2. 認証情報（クライアントID）を作成する</p>
        <ol class="wizard-steps-list">
          <li>下のボタンから「認証情報」ページを開く</li>
          <li>「認証情報を作成」→「OAuthクライアントID」をクリック</li>
          <li>アプリの種類:「デスクトップ アプリ」を選択</li>
          <li>名前は任意（例: MemoEdit）→「作成」をクリック</li>
          <li>表示された「クライアントID」と「クライアントシークレット」をコピーしておく</li>
        </ol>
        <button class="wizard-link-btn" @click="openUrl('https://console.cloud.google.com/apis/credentials')">
          認証情報ページを開く
          <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>
        </button>
      </div>

      <!-- ステップ 5: 情報入力 -->
      <div v-if="setupStep === 5" class="wizard-content">
        <h2 class="wizard-title">クライアント情報を入力する</h2>
        <p class="wizard-desc">前のステップで作成した認証情報を入力してください。</p>
        <div class="wizard-form">
          <label class="wizard-label">
            クライアントID
            <input
              v-model="setupClientId"
              class="wizard-input"
              placeholder="123456789-xxx.apps.googleusercontent.com"
              spellcheck="false"
            />
          </label>
          <label class="wizard-label">
            クライアントシークレット
            <input
              v-model="setupClientSecret"
              class="wizard-input"
              type="password"
              placeholder="GOCSPX-..."
              spellcheck="false"
            />
          </label>
          <p v-if="setupError" class="wizard-error">{{ setupError }}</p>
        </div>
      </div>

      <!-- ナビゲーション -->
      <div class="wizard-nav">
        <button class="wizard-btn-secondary" @click="showSetupWizard = false">キャンセル</button>
        <div class="wizard-nav-right">
          <button v-if="setupStep > 1" class="wizard-btn-secondary" @click="setupStep--">戻る</button>
          <button
            v-if="setupStep < 5"
            class="wizard-btn-primary"
            @click="setupStep++"
          >次へ</button>
          <button
            v-else
            class="wizard-btn-primary"
            @click="saveOAuthSetup"
            :disabled="setupSaving"
          >{{ setupSaving ? '保存中...' : 'ログインする' }}</button>
        </div>
      </div>

    </div>
  </div>

  <nav class="footer">
    <div class="char-code" @click="onEncodingClick" ref="menuEncodingRef">
      {{ activeTab.charCode }}
      <div v-if="isMenuEncoding" class="dropdown-encoding">
        <div class="dropdown-item-encoding" @click="reOpenFile(activeTab, 'utf-8')">utf-8</div>
        <div class="dropdown-item-encoding" @click="reOpenFile(activeTab, 'shift-jis')">shift-jis</div>
      </div>
    </div>
    <div class="path">{{ activeTab.path }}</div>
  </nav>
</template>

<style scoped>

.fullscreen-container {
  position: relative;
  width: 100vw;
  height: calc(100vh - 70px); /* menu-bar 20px + tab-bar 30px + footer 20px */
  overflow: hidden;
  background: linear-gradient(135deg, #18181a 0%, #23232b 100%);
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  padding: 0;
  border-radius: 0;
  overflow-y: scroll;
}

.cool-textarea {
  width: 100%;
  height: 100%;
  background: #18181a;
  color: #f6f6f6;
  border: none;
  font-size: calc(1.25rem * var(--app-zoom, 1));
  font-family: 'Fira Mono', 'Consolas', 'Menlo', monospace;
  padding: calc(1rem * var(--app-zoom, 1));
  box-sizing: border-box;
  resize: none;
  outline: none;
  border-radius: 0;
}

.cool-textarea.loading {
  color: #9ca3af;
}

.editor-status {
  position: absolute;
  top: 12px;
  right: 16px;
  padding: 6px 10px;
  background: rgba(37, 37, 38, 0.92);
  color: #d4d4d4;
  border: 1px solid #3c3c3c;
  border-radius: 4px;
  font-size: 12px;
  pointer-events: none;
}

.editor-status.error {
  color: #fca5a5;
  border-color: #7f1d1d;
}

.menu-bar {
  display: flex;
  flex-direction: row;
  width: 100vw;
  height: 20px;
  background-color: #505050;
  padding: 0;
  margin: 0;
  gap: 5px;
}

.menu-item {
  color: #f6f6f6;
  user-select: none;
  text-align: center;
  font-size: small;
  width: 70px;
  margin: 0 0 0 5px;
}

.dropdown {
  position: absolute;
  top: 30px;
  left: 0;
  background: #252526;
  border: 1px solid #3c3c3c;
  display: inline-block;
  z-index: 100;
}

.dropdown-item {
  padding: 5px 12px;
  color: #ccc;
  white-space: nowrap;
  text-align: left;
}

.dropdown-item:hover {
  background-color: #555;
  color: #ffffff;
}

.zoom-control {
  display: flex;
  align-items: center;
  height: 100%;
  gap: 2px;
  color: #f6f6f6;
  user-select: none;
}

.zoom-button,
.zoom-percent {
  height: 16px;
  min-width: 22px;
  border: 1px solid #777;
  background: #3f3f3f;
  color: #f6f6f6;
  font-size: 11px;
  line-height: 1;
  padding: 0 5px;
  border-radius: 3px;
  cursor: pointer;
}

.zoom-percent {
  min-width: 46px;
}

.zoom-button:hover:not(:disabled),
.zoom-percent:hover {
  background: #5a5a5a;
}

.zoom-button:disabled {
  opacity: 0.45;
  cursor: default;
}

/* Tab bar */
.tab-bar {
  display: flex;
  flex-direction: row;
  width: 100vw;
  height: 30px;
  background-color: #2d2d2d;
  padding: 0;
  margin: 0;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: thin;
}

.tab-bar::-webkit-scrollbar {
  height: 3px;
}

.tab {
  display: flex;
  align-items: center;
  gap: 4px;
  height: 100%;
  padding: 0 8px 0 12px;
  background-color: #3c3c3c;
  color: #aaa;
  font-size: small;
  cursor: pointer;
  user-select: none;
  border-right: 1px solid #1e1e1e;
  white-space: nowrap;
  flex-shrink: 0;
  box-sizing: border-box;
}

.tab:hover {
  background-color: #464646;
  color: #ddd;
}

.tab.active {
  background-color: #18181a;
  color: #f6f6f6;
  border-top: 2px solid #0078d4;
}

.tab.loading .tab-name::after {
  content: " 読込中";
  color: #7dd3fc;
  font-size: 11px;
}

.tab.error .tab-name::after {
  content: " エラー";
  color: #fca5a5;
  font-size: 11px;
}

.tab-name {
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tab-close {
  font-size: 14px;
  line-height: 1;
  color: #888;
  padding: 1px 3px;
  border-radius: 3px;
  flex-shrink: 0;
}

.tab-close:hover {
  background-color: #666;
  color: #fff;
}

.tab-new {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 100%;
  color: #aaa;
  font-size: 18px;
  cursor: pointer;
  user-select: none;
  flex-shrink: 0;
}

.tab-new:hover {
  background-color: #464646;
  color: #fff;
}

::-webkit-scrollbar {
  width: 8px;
  background: #23232b;
}
::-webkit-scrollbar-thumb {
  background: #33334a;
  border-radius: 4px;
}

.footer {
  display: flex;
  flex-direction: row;
  width: 100vw;
  height: 20px;
  background-color: #505050;
  padding: 0;
  margin: 0;
  gap: 5px;
}

.char-code {
  width: 100px;
  color: #f6f6f6;
  user-select: none;
  text-align: center;
  font-size: small;
  box-sizing: border-box;
  border-right: 1px solid #ccc;
  white-space: nowrap;
  padding: 0;
  margin: 0;
}

.dropdown-encoding {
  width: 100px;
  position: absolute;
  bottom: 20px;
  left: 0;
  background: #252526;
  border: 1px solid #3c3c3c;
  display: inline-block;
  z-index: 100;
}

.dropdown-item-encoding {
  width: 100px;
  color: #ccc;
  text-align: center;
}

.dropdown-item-encoding:hover {
  background-color: #555;
  color: #ffffff;
}

.path {
  flex: 1;
  color: #f6f6f6;
  user-select: none;
  font-size: small;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0 0 0 10px;
  margin: 0;
}

/* Conflict dialog */
.conflict-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.8);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.conflict-modal {
  background: #1e1e2e;
  border: 1px solid #3c3c5c;
  border-radius: 8px;
  width: 90vw;
  max-width: 1000px;
  height: 80vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.conflict-header {
  padding: 16px 20px 12px;
  border-bottom: 1px solid #333;
  flex-shrink: 0;
}

.conflict-title {
  color: #f6f6f6;
  font-size: 1rem;
  margin: 0 0 4px;
}

.conflict-subtitle {
  color: #aaa;
  font-size: 0.82rem;
  margin: 0 0 8px;
}

.conflict-legend {
  display: flex;
  gap: 16px;
  font-size: 0.78rem;
}

.legend-local { color: #e07070; }
.legend-drive { color: #70c070; }

.conflict-panels {
  display: flex;
  flex: 1;
  overflow: hidden;
  gap: 1px;
  background: #333;
}

.conflict-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: #1e1e2e;
}

.conflict-panel-header {
  padding: 6px 12px;
  background: #252536;
  color: #ccc;
  font-size: 0.82rem;
  font-weight: bold;
  flex-shrink: 0;
  border-bottom: 1px solid #333;
}

.conflict-line-count {
  color: #888;
  font-weight: normal;
  margin-left: 6px;
}

.diff-view {
  flex: 1;
  overflow-y: auto;
  font-family: 'Fira Mono', 'Consolas', monospace;
  font-size: 0.8rem;
}

.diff-line {
  display: flex;
  align-items: baseline;
  min-height: 1.4em;
  padding: 1px 0;
}

.diff-line.diff-local-only {
  background: rgba(200, 60, 60, 0.25);
}

.diff-line.diff-drive-only {
  background: rgba(60, 180, 60, 0.2);
}

.diff-ln {
  width: 40px;
  min-width: 40px;
  text-align: right;
  padding-right: 10px;
  color: #555;
  user-select: none;
  flex-shrink: 0;
}

.diff-text {
  color: #d4d4d4;
  white-space: pre;
  word-break: break-all;
}

.diff-line.diff-local-only .diff-text { color: #f08080; }
.diff-line.diff-drive-only .diff-text { color: #80d080; }

.conflict-actions {
  display: flex;
  justify-content: center;
  gap: 16px;
  padding: 14px 20px;
  border-top: 1px solid #333;
  flex-shrink: 0;
}

.conflict-btn {
  padding: 8px 28px;
  border: none;
  border-radius: 4px;
  font-size: 0.88rem;
  cursor: pointer;
  font-weight: bold;
}

.conflict-btn-local {
  background: #5a2020;
  color: #f08080;
  border: 1px solid #8a3030;
}

.conflict-btn-local:hover { background: #6e2626; }

.conflict-btn-drive {
  background: #1a4a1a;
  color: #80d080;
  border: 1px solid #2a6a2a;
}

.conflict-btn-drive:hover { background: #1e5a1e; }

/* Cloud sync status icons */
.tab-cloud-icon {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  gap: 1px;
}

.cloud-only {
  opacity: 0.85;
}

.cloud-local {
  opacity: 1;
}

.cloud-synced {
  color: #5cb85c;
}

.cloud-syncing {
  color: #4da6ff;
  animation: cloud-pulse 1.4s ease-in-out infinite;
}

.cloud-error {
  color: #e05555;
}

.tab.loading .tab-name::after {
  content: none;
}

.tab-loading-icon {
  width: 12px;
  height: 12px;
  border: 2px solid #4b5563;
  border-top-color: #7dd3fc;
  border-radius: 50%;
  flex-shrink: 0;
  box-sizing: border-box;
  animation: tab-loading-spin 0.8s linear infinite;
}

@keyframes tab-loading-spin {
  to { transform: rotate(360deg); }
}

@keyframes cloud-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

/* Tab context menu */
.tab-context-menu {
  position: fixed;
  background: #252526;
  border: 1px solid #3c3c3c;
  z-index: 500;
  min-width: 160px;
  box-shadow: 0 4px 12px rgba(0,0,0,0.5);
}

.context-item {
  padding: 7px 14px;
  color: #ccc;
  font-size: small;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}

.context-item:hover {
  background: #094771;
  color: #fff;
}

.context-item-danger:hover {
  background: #6e1c1c;
  color: #fff;
}

/* Setup Wizard */
.wizard-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.75);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.wizard-modal {
  background: #1e1e2e;
  border: 1px solid #3c3c5c;
  border-radius: 8px;
  width: 520px;
  max-width: 90vw;
  max-height: 85vh;
  display: flex;
  flex-direction: column;
  padding: 28px 32px 24px;
  gap: 16px;
  overflow-y: auto;
}

.wizard-steps {
  display: flex;
  gap: 8px;
  justify-content: center;
}

.wizard-step-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #444;
  transition: background 0.2s;
}

.wizard-step-dot.active {
  background: #0078d4;
}

.wizard-step-dot.done {
  background: #0a8a3a;
}

.wizard-step-label {
  text-align: center;
  font-size: 11px;
  color: #888;
  margin-top: -8px;
}

.wizard-content {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.wizard-icon {
  text-align: center;
}

.wizard-title {
  color: #f6f6f6;
  font-size: 1.1rem;
  margin: 0;
}

.wizard-desc {
  color: #bbb;
  font-size: 0.88rem;
  margin: 0;
  line-height: 1.6;
}

.wizard-checklist {
  color: #ccc;
  font-size: 0.85rem;
  padding-left: 1.4rem;
  margin: 0;
  line-height: 2;
}

.wizard-steps-list {
  color: #ccc;
  font-size: 0.85rem;
  padding-left: 1.4rem;
  margin: 0;
  line-height: 2;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.wizard-note {
  background: #2a2a1e;
  border-left: 3px solid #c8a000;
  padding: 8px 12px;
  color: #e0c060;
  font-size: 0.82rem;
  line-height: 1.7;
  list-style: none;
  margin-left: -1.4rem;
}

.wizard-section-title {
  color: #f6f6f6;
  font-size: 0.88rem;
  font-weight: bold;
  margin: 0;
}

.wizard-alert {
  background: #2a1e1e;
  border-left: 3px solid #e05555;
  padding: 10px 14px;
  color: #f08080;
  font-size: 0.83rem;
  line-height: 1.6;
}

.wizard-link-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: #0078d4;
  color: #fff;
  border: none;
  border-radius: 4px;
  padding: 8px 16px;
  font-size: 0.85rem;
  cursor: pointer;
  align-self: flex-start;
}

.wizard-link-btn:hover {
  background: #006bb5;
}

.wizard-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.wizard-label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: #ccc;
  font-size: 0.85rem;
}

.wizard-input {
  background: #2a2a3a;
  border: 1px solid #444;
  border-radius: 4px;
  color: #f6f6f6;
  font-size: 0.85rem;
  padding: 8px 10px;
  outline: none;
  font-family: 'Fira Mono', 'Consolas', monospace;
}

.wizard-input:focus {
  border-color: #0078d4;
}

.wizard-error {
  color: #e05555;
  font-size: 0.82rem;
  margin: 0;
  white-space: pre-wrap;
}

.wizard-nav {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-top: 8px;
  border-top: 1px solid #333;
}

.wizard-nav-right {
  display: flex;
  gap: 8px;
}

.wizard-btn-primary {
  background: #0078d4;
  color: #fff;
  border: none;
  border-radius: 4px;
  padding: 7px 20px;
  font-size: 0.85rem;
  cursor: pointer;
}

.wizard-btn-primary:hover:not(:disabled) {
  background: #006bb5;
}

.wizard-btn-primary:disabled {
  opacity: 0.5;
  cursor: default;
}

.wizard-btn-secondary {
  background: none;
  color: #aaa;
  border: 1px solid #555;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 0.85rem;
  cursor: pointer;
}

.wizard-btn-secondary:hover {
  background: #333;
  color: #eee;
}

/* Auth */
.menu-spacer {
  flex: 1;
}

.menu-auth {
  display: flex;
  align-items: center;
  height: 100%;
  padding: 0 6px;
  position: relative;
}

.auth-login-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  background: none;
  border: 1px solid #888;
  border-radius: 3px;
  color: #f6f6f6;
  font-size: 11px;
  padding: 1px 6px;
  cursor: pointer;
  height: 16px;
  line-height: 1;
}

.auth-login-btn:hover {
  background: #666;
}

.auth-user-btn {
  display: flex;
  align-items: center;
  cursor: pointer;
  position: relative;
}

.auth-avatar {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  object-fit: cover;
}

.auth-initial {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #0078d4;
  color: #fff;
  font-size: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.dropdown-auth {
  position: absolute;
  top: 20px;
  right: 0;
  background: #252526;
  border: 1px solid #3c3c3c;
  min-width: 180px;
  z-index: 200;
}

.auth-info-name {
  padding: 8px 12px 2px;
  color: #f6f6f6;
  font-size: small;
  font-weight: bold;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.auth-info-email {
  padding: 0 12px 8px;
  color: #aaa;
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dropdown-divider {
  border-top: 1px solid #3c3c3c;
  margin: 0;
}
</style>
