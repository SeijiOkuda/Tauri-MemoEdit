<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";

const text = ref("");

async function saveFile() {
  const path = await save({
    filters: [{ name: 'Text Files', extensions: ['txt'] }],
    defaultPath: 'memo.txt',
  });
  
  if (path) {
    try {
      await writeTextFile(path, text.value);
      console.log("✅ ファイル保存成功:", path);
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

onMounted(() => window.addEventListener('keydown', handleKeyDown));
onUnmounted(() => window.removeEventListener('keydown', handleKeyDown));

</script>

<template>
  <main class="fullscreen-container">
    <textarea
      v-model="text"
      class="cool-textarea"
      placeholder="ここにメモを入力..."
      spellcheck="false"
      autofocus
    ></textarea>
  </main>
</template>

<style scoped>

.fullscreen-container {
  width: 100vw;
  height: 100vh;
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

::-webkit-scrollbar {
  width: 8px;
  background: #23232b;
}
::-webkit-scrollbar-thumb {
  background: #33334a;
  border-radius: 4px;
}

</style>