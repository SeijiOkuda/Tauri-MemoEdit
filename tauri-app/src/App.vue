<script setup lang="ts">
import { onMounted, onUnmounted, ref, computed, nextTick } from "vue";
import { open, save, ask } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readTextFile } from "@tauri-apps/plugin-fs";
import { listen } from '@tauri-apps/api/event';
import { exit } from '@tauri-apps/plugin-process';
import { invoke } from "@tauri-apps/api/core";

interface Tab {
  id: string;
  text: string;
  textSaved: string;
  path: string | null;
  charCode: string;
}

let nextId = 1;
function createTab(): Tab {
  return { id: String(nextId++), text: "", textSaved: "", path: null, charCode: "utf-8" };
}

const tabs = ref<Tab[]>([createTab()]);
const activeTabId = ref(tabs.value[0].id);
const activeTab = computed(() => tabs.value.find(t => t.id === activeTabId.value)!);

const isMenuFile = ref(false);
const isMenuEncoding = ref(false);
const menuRef = ref<HTMLElement | null>(null);
const menuEncodingRef = ref<HTMLElement | null>(null);
const textarea = ref<HTMLTextAreaElement | null>(null);

function tabName(tab: Tab) {
  if (tab.path) return tab.path.split(/[\\/]/).pop() ?? tab.path;
  return "新しいファイル";
}

function isUnsaved(tab: Tab) {
  return tab.text !== tab.textSaved;
}

onMounted(async () => {
  await listen('open-file', async (event: { payload: string }) => {
    await openFileInTab(event.payload);
  });

  await listen("app-close-requested", async () => {
    exitApp();
  });

  window.addEventListener("keydown", handleKeyDown);
  window.addEventListener("click", handleClickOutside);

  invoke("frontend_ready");
});

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeyDown);
  window.removeEventListener('click', handleClickOutside);
});

function handleClickOutside(e: MouseEvent) {
  if (menuRef.value && !menuRef.value.contains(e.target as Node)) {
    isMenuFile.value = false;
  }
  if (menuEncodingRef.value && !menuEncodingRef.value.contains(e.target as Node)) {
    isMenuEncoding.value = false;
  }
}

async function openFileInTab(filePath: string) {
  const cur = activeTab.value;
  const isBlank = !cur.path && cur.text === "" && !isUnsaved(cur);
  const tab = isBlank ? cur : createTab();
  if (!isBlank) tabs.value.push(tab);

  tab.path = filePath;
  tab.charCode = "utf-8";
  try {
    const content = await readTextFile(filePath, { encoding: "utf-8" });
    tab.text = content;
    tab.textSaved = content;
    activeTabId.value = tab.id;
  } catch (err) {
    console.error("❌ ファイル読み込み失敗:", err);
    if (!isBlank) tabs.value = tabs.value.filter(t => t.id !== tab.id);
  }
}

async function saveFile() {
  const tab = activeTab.value;
  if (!tab.path) {
    const newPath = await save({
      filters: [{ name: 'Text Files', extensions: ['txt'] }],
      defaultPath: 'memo.txt',
    });
    if (!newPath) return;
    tab.path = newPath;
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
    tab.charCode = "utf-8";
  } catch (err) {
    console.error("❌ ファイル保存失敗:", err);
  }
}

function handleKeyDown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault();
    saveFile();
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

async function reOpenFile(encoding: string) {
  const tab = activeTab.value;
  if (!tab.path) return;

  if (isUnsaved(tab)) {
    const ok = await ask("変更が保存されていません。保存せずに再度開き直しますか？", {
      title: "確認", kind: 'warning', okLabel: "はい", cancelLabel: "いいえ"
    });
    if (!ok) return;
  }

  tab.charCode = encoding;
  try {
    const content = await readTextFile(tab.path, { encoding });
    tab.text = content;
    tab.textSaved = content;
  } catch (err) {
    console.error("❌ ファイル再読み込み失敗:", err);
  }
}

async function exitApp() {
  const hasUnsaved = tabs.value.some(isUnsaved);
  if (hasUnsaved) {
    const ok = await ask("変更が保存されていません。終了しますか？", {
      title: "確認", kind: 'warning', okLabel: "はい", cancelLabel: "いいえ"
    });
    if (!ok) return;
  }
  await exit().catch(err => console.error("❌ アプリ終了失敗:", err));
}

function newTab() {
  const tab = createTab();
  tabs.value.push(tab);
  activeTabId.value = tab.id;
  nextTick(() => textarea.value?.focus());
}

async function closeTab(tabId: string) {
  const tab = tabs.value.find(t => t.id === tabId);
  if (!tab) return;

  if (isUnsaved(tab)) {
    const ok = await ask("変更が保存されていません。タブを閉じますか？", {
      title: "確認", kind: 'warning', okLabel: "はい", cancelLabel: "いいえ"
    });
    if (!ok) return;
  }

  const idx = tabs.value.findIndex(t => t.id === tabId);
  tabs.value.splice(idx, 1);

  if (tabs.value.length === 0) {
    const newT = createTab();
    tabs.value.push(newT);
    activeTabId.value = newT.id;
  } else if (activeTabId.value === tabId) {
    activeTabId.value = tabs.value[Math.min(idx, tabs.value.length - 1)].id;
  }
}

const insertTab = (e: KeyboardEvent) => {
  if (textarea.value) {
    const start = textarea.value.selectionStart;
    const end = textarea.value.selectionEnd;
    const value = textarea.value.value;

    activeTab.value.text = value.substring(0, start) + "\t" + value.substring(end);

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
        <div class="dropdown-item" @click="exitApp">終了</div>
      </div>
    </div>
    <div class="menu-item">編集(E)</div>
    <div class="menu-item">ヘルプ(H)</div>
  </nav>
  <div class="tab-bar">
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="tab"
      :class="{ active: tab.id === activeTabId }"
      @click="activeTabId = tab.id"
    >
      <span class="tab-name">{{ tabName(tab) }}{{ isUnsaved(tab) ? ' ●' : '' }}</span>
      <span class="tab-close" @click.stop="closeTab(tab.id)">×</span>
    </div>
    <div class="tab-new" @click="newTab">+</div>
  </div>
  <main class="fullscreen-container">
    <textarea
      ref="textarea"
      v-model="activeTab.text"
      class="cool-textarea"
      placeholder="ここにメモを入力..."
      @keydown.tab.prevent="insertTab"
      spellcheck="false"
      autofocus
    ></textarea>
  </main>
  <nav class="footer">
    <div class="char-code" @click="onEncodingClick" ref="menuEncodingRef">
      {{ activeTab.charCode }}
      <div v-if="isMenuEncoding" class="dropdown-encoding">
        <div class="dropdown-item-encoding" @click="reOpenFile('utf-8')">utf-8</div>
        <div class="dropdown-item-encoding" @click="reOpenFile('shift-jis')">shift-jis</div>
      </div>
    </div>
    <div class="path">{{ activeTab.path }}</div>
  </nav>
</template>

<style scoped>

.fullscreen-container {
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
  font-size: 1.25rem;
  font-family: 'Fira Mono', 'Consolas', 'Menlo', monospace;
  padding: 1rem;
  box-sizing: border-box;
  resize: none;
  outline: none;
  border-radius: 0;
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
</style>