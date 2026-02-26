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
const isMenuEncoding = ref<boolean>(false);
const path = ref<string | null>(null);
const charCode = ref<string | null>("utf-8");

const menuRef = ref<HTMLElement | null>(null);
const menuEncodingRef = ref<HTMLElement | null>(null);
const textarea = ref<HTMLTextAreaElement | null>(null);

onMounted(async () => {
  await listen('open-file', async (event: { payload: string }) => {
    console.log("📂 外部起動ファイルイベント受信:", event.payload);
    const filePath = event.payload;
    path.value = filePath;

    if (path.value) {
      await openFile(path.value);
    } else {
      console.log("❌ ファイル選択キャンセル");
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
  if (menuEncodingRef.value && !menuEncodingRef.value.contains(e.target as Node)) {
    isMenuEncoding.value = false;
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
    if(charCode.value && charCode.value !== "utf-8") {
      const message = `現在のエンコードは${charCode.value}です。utf-8で保存しますか？`;
      const okToSave = await ask(message, {
        title: "確認",
        kind: 'warning',
        okLabel: "はい",
        cancelLabel: "いいえ"
      });
      if (!okToSave) {
        console.log("❌ 保存キャンセル");
        return;
      }
    }
    try {
      await writeTextFile(path.value, text.value);
      textSaved.value = text.value;
      charCode.value = "utf-8";
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

function onEncodingClick() {
  isMenuEncoding.value = !isMenuEncoding.value;
}

async function openFileDialog() {
  path.value = await open({
    filters: [{ name: 'Text Files', extensions: ['txt'] }],
  });
  if (path.value) {
    await openFile(path.value);
  } else {
    console.log("❌ ファイル選択キャンセル");
  }
}

async function openFile(path: string) {
  try {
    const fileContent = await readTextFile(path, { encoding: charCode.value? charCode.value : "utf-8" });
    text.value = fileContent;
    textSaved.value = fileContent;
    console.log("✅ ファイル読み込み成功:", path);
  } catch (err) {
    console.error("❌ ファイル読み込み失敗:", err);
  }
}

async function reOpenFile(path: string | null, encoding: string) {
  try {
    const okToReopen = await confirmIfUnsaved('reopen');
    if (!okToReopen) return;
    charCode.value = encoding;
    if (!path) {
      console.log("❌ ファイルが開かれていません");
      return;
    }
    await openFile(path);
    console.log("✅ ファイル再読み込み成功:", path, "エンコード:", encoding);
  } catch (err) {
    console.error("❌ ファイル再読み込み失敗:", err);
  }
}

async function exitApp() {
  const okToExit = await confirmIfUnsaved('exit');
  if (!okToExit) return;

  await exit()
  .catch(err => {
    console.error("❌ アプリ終了失敗:", err);
  });
}

async function confirmIfUnsaved(action: 'exit' | 'reopen'): Promise<boolean> {
  if (text.value !== textSaved.value) {
    const message = action === 'exit' 
      ? "変更が保存されていません。終了しますか？" 
      : "変更が保存されていません。保存せずに再度開き直しますか？";
    return await ask(message, {
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
        <div class="dropdown-item" @click="openFileDialog">開く</div>
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
    <div class="char-code" @click="onEncodingClick" ref="menuEncodingRef">
      {{ charCode }}
      <div v-if="isMenuEncoding" class="dropdown-encoding">
          <div class="dropdown-item-encoding" @click="reOpenFile(path, 'utf-8')">
            utf-8
          </div>
          <div class="dropdown-item-encoding" @click="reOpenFile(path, 'shift-jis')">
            shift-jis
          </div>
      </div>
    </div>
    <div class="path">{{ path }}</div>
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

.char-code {
  width: 100px;
  color: #f6f6f6;
  user-select: none;
  text-align: center;
  font-size: small;
  padding: 5px 10px;
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
  padding: 5px 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0 0 0 10px;
  margin: 0;
}

</style>