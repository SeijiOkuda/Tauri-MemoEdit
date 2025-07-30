<script setup lang="ts">
import { onMounted, onUnmounted, ref, nextTick } from "vue";
import { open, save, ask } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readTextFile } from "@tauri-apps/plugin-fs";
import { listen } from '@tauri-apps/api/event';
import { exit } from '@tauri-apps/plugin-process';
import { invoke } from "@tauri-apps/api/core";

const text = ref("");
const textSaved = ref("");
const isMenuFile = ref<boolean>(false);
const path = ref<string | null>(null);
const menuRef = ref<HTMLElement | null>(null);
const textarea = ref<HTMLTextAreaElement | null>(null);

onMounted(async () => {
  await listen('open-file', async (event: { payload: string }) => {
    console.log("📂 外部起動ファイルイベント受信:", event.payload);
    const filePath = event.payload;
    path.value = filePath;

    try {
      const fileContent = await readTextFile(filePath);
      text.value = fileContent;
      textSaved.value = fileContent;
      console.log("📂 外部起動ファイル読み込み成功:", filePath);
    } catch (err) {
      console.error("❌ 外部ファイル読み込み失敗:", err);
    }
  });

  await listen("app-close-requested", async () => {
    console.log("❌ アプリ閉じるリクエストを受信");
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
}

async function saveFile() {
  if (!path.value) {
    path.value = await save({
      filters: [{ name: 'Text Files', extensions: ['txt'] }],
      defaultPath: 'memo.txt',
    });
  }
  
  if (path.value) {
    try {
      await writeTextFile(path.value, text.value);
      textSaved.value = text.value;
      console.log("✅ ファイル保存成功:", path.value);
    } catch (err) {
      console.error("❌ ファイル保存失敗:", err);
    }
  } else {
    console.log("❌ 保存キャンセル");
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

async function openFile() {
  path.value = await open({
    filters: [{ name: 'Text Files', extensions: ['txt'] }],
  });

  if (path.value) {
    try {
      const fileContent = await readTextFile(path.value);
      text.value = fileContent;
      textSaved.value = fileContent;
      console.log("✅ ファイル読み込み成功:", path.value);
    } catch (err) {
      console.error("❌ ファイル読み込み失敗:", err);
    }
  } else {
    console.log("❌ 開くキャンセル");
  }
}

async function exitApp() {
  const okToExit = await confirmExitIfUnsaved();
  if (!okToExit) return;

  await exit()
  .catch(err => {
    console.error("❌ アプリ終了失敗:", err);
  });
}

async function confirmExitIfUnsaved(): Promise<boolean> {
  if (text.value !== textSaved.value) {
    return await ask("変更が保存されていません。終了しますか？", {
      title: "確認",
      kind: 'warning',
      okLabel: "はい",
      cancelLabel: "いいえ"
    });
  }
  return true;
}

function onEditClick() {
  console.log("編集メニューがクリックされました");
}

function onHelpClick() {
  console.log("ヘルプメニューがクリックされました");
}

const insertTab = (e: KeyboardEvent) => {
  if (textarea.value) {
    const start = textarea.value.selectionStart;
    const end = textarea.value.selectionEnd;
    const value = textarea.value.value;

    text.value = value.substring(0, start) + "\t" + value.substring(end);

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
        <div class="dropdown-item">新しいファイル</div>
        <div class="dropdown-item" @click="openFile">開く</div>
        <div class="dropdown-item" @click="saveFile">保存</div>
        <div class="dropdown-item" @click="exitApp">終了</div>
      </div>
    </div>
    <div class="menu-item" @click="onEditClick">編集(E)</div>
    <div class="menu-item" @click="onHelpClick">ヘルプ(H)</div>
  </nav>
  <main class="fullscreen-container">
    <textarea
      ref="textarea"
      v-model="text"
      class="cool-textarea"
      placeholder="ここにメモを入力..."
      @keydown.tab.prevent="insertTab"
      spellcheck="false"
      autofocus
    ></textarea>
  </main>
  <nav class="footer">
  </nav>
</template>

<style scoped>

.fullscreen-container {
  width: 100vw;
  height: calc(100vh - 40px); /* Adjust for menu bar height and footer height */
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

</style>