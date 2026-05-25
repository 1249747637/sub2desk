<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

type Platform = "openai" | "anthropic";
type Theme = "light" | "dark";

interface ModelItem {
  id: string;
  enabled: boolean;
}

interface Profile {
  id: string;
  name: string;
  accountName: string;
  platform: Platform;
  poolMode: boolean;
  poolModeRetryCount: number;
  priority: number;
  groupId: number | null;
  baseUrl: string;
  apiKey: string;
  models: ModelItem[];
  lastFetchedAt: string | null;
}

interface Settings {
  backendBaseUrl: string;
  adminApiKey: string;
  theme: Theme;
}

interface AppState {
  profiles: Profile[];
  activeProfileId: string;
  settings: Settings;
}

interface GroupOption {
  id: number;
  name: string;
  platform?: string;
  status?: string;
}

interface Toast {
  id: number;
  kind: "success" | "error" | "info";
  text: string;
}

const isTauri = Boolean((window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
const STORAGE_KEY = "sub2desk-browser-state";

const state = ref<AppState>(createDefaultState());
const groups = ref<GroupOption[]>([]);
const loading = reactive({
  boot: true,
  saving: false,
  groups: false,
  models: false,
  submit: false,
  testingModel: "",
});
const ui = reactive({
  settingsOpen: false,
  confirmOpen: false,
  renameProfileId: "",
  renameDraft: "",
});
const toasts = ref<Toast[]>([]);
const saveTimer = ref<number | null>(null);
let loaded = false;
let toastId = 1;

const activeProfile = computed<Profile>(() => {
  if (state.value.profiles.length === 0) {
    const profile = createProfile("Profile 1");
    state.value.profiles.push(profile);
    state.value.activeProfileId = profile.id;
    return profile;
  }
  return (
    state.value.profiles.find((profile) => profile.id === state.value.activeProfileId) ??
    state.value.profiles[0]
  );
});

const enabledModels = computed(() =>
  activeProfile.value.models.filter((model) => model.enabled),
);

const disabledModels = computed(() =>
  activeProfile.value.models.filter((model) => !model.enabled),
);

const canSubmit = computed(() => {
  const profile = activeProfile.value;
  return Boolean(
    profile?.name.trim() &&
      profile?.accountName !== undefined &&
      profile?.baseUrl.trim() &&
      profile?.apiKey.trim() &&
      profile?.groupId &&
      enabledModels.value.length > 0 &&
      state.value.settings.backendBaseUrl.trim() &&
      state.value.settings.adminApiKey.trim(),
  );
});

const platformLabel = computed(() =>
  activeProfile.value.platform === "openai" ? "OpenAI" : "Anthropic",
);

onMounted(async () => {
  try {
    if (isTauri) {
      state.value = normalizeState(await invoke<AppState>("load_state"));
    } else {
      const cached = window.localStorage.getItem(STORAGE_KEY);
      state.value = cached ? normalizeState(JSON.parse(cached)) : createDefaultState();
      pushToast("info", "浏览器预览模式：网络请求需在 Tauri 桌面环境中执行");
    }
    document.documentElement.dataset.theme = state.value.settings.theme;
    await refreshGroups(false);
  } catch (error) {
    pushToast("error", toErrorMessage(error));
  } finally {
    loaded = true;
    loading.boot = false;
  }
});

watch(
  state,
  () => {
    if (!loaded) return;
    document.documentElement.dataset.theme = state.value.settings.theme;
    scheduleSave();
  },
  { deep: true },
);

watch(
  () => activeProfile.value?.platform,
  async () => {
    if (!loaded) return;
    activeProfile.value.groupId = null;
    groups.value = [];
    await refreshGroups(false);
  },
);

function createDefaultState(): AppState {
  const profiles = [1, 2, 3].map((idx) => createProfile(`Profile ${idx}`));
  return {
    profiles,
    activeProfileId: profiles[0].id,
    settings: {
      backendBaseUrl: "http://localhost:8080",
      adminApiKey: "",
      theme: "light",
    },
  };
}

function createProfile(name: string): Profile {
  return {
    id: crypto.randomUUID(),
    name,
    accountName: "",
    platform: "openai",
    poolMode: false,
    poolModeRetryCount: 3,
    priority: 1,
    groupId: null,
    baseUrl: "https://api.openai.com",
    apiKey: "",
    models: [],
    lastFetchedAt: null,
  };
}

function normalizeState(nextState: AppState): AppState {
  const normalized = nextState ?? createDefaultState();
  if (!Array.isArray(normalized.profiles) || normalized.profiles.length === 0) {
    normalized.profiles = createDefaultState().profiles;
  }
  normalized.profiles = normalized.profiles.map((profile, idx) => ({
    ...createProfile(`Profile ${idx + 1}`),
    ...profile,
    id: profile.id || crypto.randomUUID(),
    name: profile.name?.trim() || `Profile ${idx + 1}`,
    accountName: profile.accountName?.trim() || "",
    priority: Number(profile.priority) > 0 ? Number(profile.priority) : 1,
    poolModeRetryCount:
      Number(profile.poolModeRetryCount) > 0 ? Number(profile.poolModeRetryCount) : 3,
    groupId: profile.groupId ?? null,
    models: Array.isArray(profile.models) ? profile.models : [],
  }));
  if (!normalized.profiles.some((profile) => profile.id === normalized.activeProfileId)) {
    normalized.activeProfileId = normalized.profiles[0].id;
  }
  normalized.settings = {
    backendBaseUrl: normalized.settings?.backendBaseUrl || "http://localhost:8080",
    adminApiKey: normalized.settings?.adminApiKey || "",
    theme: normalized.settings?.theme === "dark" ? "dark" : "light",
  };
  return normalized;
}

function selectProfile(id: string) {
  state.value.activeProfileId = id;
}

function addProfile() {
  const profile = createProfile(`Profile ${state.value.profiles.length + 1}`);
  state.value.profiles.push(profile);
  state.value.activeProfileId = profile.id;
  beginRename(profile);
}

function deleteProfile(id: string) {
  if (state.value.profiles.length <= 1) {
    pushToast("error", "至少保留一个 Profile");
    return;
  }
  const idx = state.value.profiles.findIndex((profile) => profile.id === id);
  state.value.profiles = state.value.profiles.filter((profile) => profile.id !== id);
  if (state.value.activeProfileId === id) {
    state.value.activeProfileId = state.value.profiles[Math.max(0, idx - 1)]?.id ?? state.value.profiles[0].id;
  }
}

function beginRename(profile: Profile) {
  ui.renameProfileId = profile.id;
  ui.renameDraft = profile.name;
  nextTick(() => {
    const input = document.querySelector<HTMLInputElement>("[data-rename-input='true']");
    input?.focus();
    input?.select();
  });
}

function commitRename() {
  const profile = state.value.profiles.find((item) => item.id === ui.renameProfileId);
  const name = ui.renameDraft.trim();
  if (profile && name) {
    profile.name = name;
  }
  ui.renameProfileId = "";
  ui.renameDraft = "";
}

function setPlatform(platform: Platform) {
  const profile = activeProfile.value;
  if (profile.platform === platform) return;
  profile.platform = platform;
  profile.groupId = null;
  profile.models = [];
  profile.baseUrl = platform === "openai" ? "https://api.openai.com" : "https://api.anthropic.com";
}

async function refreshGroups(showSuccess = true) {
  if (!activeProfile.value || !state.value.settings.backendBaseUrl.trim()) return;
  if (!isTauri) return;
  loading.groups = true;
  try {
    groups.value = await invoke<GroupOption[]>("fetch_groups", {
      settings: state.value.settings,
      platform: activeProfile.value.platform,
    });
    if (showSuccess) pushToast("success", `已加载 ${groups.value.length} 个分组`);
  } catch (error) {
    pushToast("error", toErrorMessage(error));
  } finally {
    loading.groups = false;
  }
}

async function fetchModelList() {
  const profile = activeProfile.value;
  loading.models = true;
  try {
    if (!isTauri) throw new Error("请在 Tauri 桌面环境中获取模型列表");
    const fetched = await invoke<ModelItem[]>("fetch_models", { profile });
    const previous = new Map(profile.models.map((model) => [model.id, model.enabled]));
    profile.models = fetched.map((model) => ({
      id: model.id,
      enabled: previous.has(model.id) ? Boolean(previous.get(model.id)) : true,
    }));
    profile.lastFetchedAt = new Date().toISOString();
    pushToast("success", `已获取 ${profile.models.length} 个模型`);
  } catch (error) {
    pushToast("error", toErrorMessage(error));
  } finally {
    loading.models = false;
  }
}

async function testModel(model: ModelItem) {
  loading.testingModel = model.id;
  try {
    if (!isTauri) throw new Error("请在 Tauri 桌面环境中测试模型");
    const result = await invoke<{ message: string }>("test_model", {
      profile: activeProfile.value,
      modelId: model.id,
    });
    pushToast("success", `${model.id}: ${result.message}`);
  } catch (error) {
    pushToast("error", `${model.id}: ${toErrorMessage(error)}`);
  } finally {
    loading.testingModel = "";
  }
}

function toggleModel(model: ModelItem) {
  model.enabled = !model.enabled;
}

function openSubmitConfirm() {
  if (!canSubmit.value) {
    pushToast("error", "请补全后端地址、管理员 Key、分组、上游 Key，并至少选择一个模型");
    return;
  }
  ui.confirmOpen = true;
}

async function submitProfile() {
  loading.submit = true;
  try {
    if (!isTauri) throw new Error("请在 Tauri 桌面环境中提交");
    const result = await invoke<{ message: string }>("submit_profile", {
      settings: state.value.settings,
      profile: activeProfile.value,
    });
    ui.confirmOpen = false;
    pushToast("success", result.message);
  } catch (error) {
    pushToast("error", toErrorMessage(error));
  } finally {
    loading.submit = false;
  }
}

function scheduleSave() {
  if (saveTimer.value) window.clearTimeout(saveTimer.value);
  saveTimer.value = window.setTimeout(saveState, 350);
}

async function saveState() {
  loading.saving = true;
  try {
    if (isTauri) {
      await invoke("save_state", { state: state.value });
    } else {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state.value));
    }
  } catch (error) {
    pushToast("error", toErrorMessage(error));
  } finally {
    loading.saving = false;
  }
}

function pushToast(kind: Toast["kind"], text: string) {
  const id = toastId++;
  toasts.value.push({ id, kind, text });
  window.setTimeout(() => {
    toasts.value = toasts.value.filter((toast) => toast.id !== id);
  }, kind === "error" ? 6500 : 4200);
}

function toErrorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}
</script>

<template>
  <main class="app-shell">
    <header class="topbar">
      <div class="brand">
        <span class="brand-mark">S2</span>
        <div>
          <h1>Sub2Desk</h1>
          <p>{{ loading.saving ? "正在保存" : "本地已保存" }}</p>
        </div>
      </div>
      <button class="icon-button" type="button" title="设置" @click="ui.settingsOpen = true">
        ⚙
      </button>
    </header>

    <nav class="profile-strip" aria-label="Profiles">
      <button
        v-for="profile in state.profiles"
        :key="profile.id"
        class="profile-tab"
        :class="{ active: profile.id === state.activeProfileId }"
        type="button"
        @click="selectProfile(profile.id)"
        @dblclick="beginRename(profile)"
      >
        <input
          v-if="ui.renameProfileId === profile.id"
          v-model="ui.renameDraft"
          data-rename-input="true"
          @blur="commitRename"
          @keydown.enter.prevent="commitRename"
          @keydown.esc.prevent="ui.renameProfileId = ''"
        />
        <span v-else>{{ profile.name }}</span>
      </button>
      <button class="icon-button compact" type="button" title="新增 Profile" @click="addProfile">
        +
      </button>
      <button
        class="icon-button compact danger"
        type="button"
        title="删除当前 Profile"
        @click="deleteProfile(activeProfile.id)"
      >
        -
      </button>
      <button
        class="text-button"
        type="button"
        @click="beginRename(activeProfile)"
      >
        改名
      </button>
    </nav>

    <section v-if="!loading.boot" class="workspace">
      <section class="panel profile-panel">
        <div class="section-head">
          <div>
            <h2>账号配置</h2>
            <p>{{ platformLabel }} upstream profile</p>
          </div>
          <div class="segmented" aria-label="账号平台">
            <button
              type="button"
              :class="{ active: activeProfile.platform === 'openai' }"
              @click="setPlatform('openai')"
            >
              OpenAI
            </button>
            <button
              type="button"
              :class="{ active: activeProfile.platform === 'anthropic' }"
              @click="setPlatform('anthropic')"
            >
              Anthropic
            </button>
          </div>
        </div>

        <div class="form-grid">
          <label>
            <span>账号名称</span>
            <input
              v-model.trim="activeProfile.accountName"
              placeholder="留空则发送时自动使用时间"
            />
          </label>
          <label>
            <span>Base URL</span>
            <input v-model.trim="activeProfile.baseUrl" placeholder="https://api.openai.com" />
          </label>
          <label>
            <span>API Key</span>
            <input v-model.trim="activeProfile.apiKey" type="password" placeholder="上游账号 Key" />
          </label>
          <label>
            <span>分组</span>
            <div class="inline-control">
              <select v-model="activeProfile.groupId">
                <option :value="null">选择分组</option>
                <option v-for="group in groups" :key="group.id" :value="group.id">
                  {{ group.name }}
                </option>
              </select>
              <button type="button" @click="refreshGroups()" :disabled="loading.groups">
                {{ loading.groups ? "加载中" : "刷新" }}
              </button>
            </div>
          </label>
          <label>
            <span>优先级</span>
            <input v-model.number="activeProfile.priority" type="number" min="1" step="1" />
          </label>
          <label class="switch-row">
            <span>池模式</span>
            <input v-model="activeProfile.poolMode" type="checkbox" />
          </label>
          <label>
            <span>池重试次数</span>
            <input
              v-model.number="activeProfile.poolModeRetryCount"
              type="number"
              min="1"
              step="1"
            />
          </label>
        </div>

        <div class="actions-row">
          <button class="primary" type="button" @click="fetchModelList" :disabled="loading.models">
            {{ loading.models ? "正在获取" : "获取模型列表" }}
          </button>
          <span v-if="activeProfile.lastFetchedAt" class="muted">
            {{ new Date(activeProfile.lastFetchedAt).toLocaleString() }}
          </span>
        </div>
      </section>

      <section class="panel models-panel">
        <div class="section-head">
          <div>
            <h2>模型</h2>
            <p>{{ enabledModels.length }} 已选，{{ disabledModels.length }} 已禁用</p>
          </div>
          <button
            class="text-button"
            type="button"
            @click="activeProfile.models.forEach((model) => (model.enabled = true))"
            :disabled="activeProfile.models.length === 0"
          >
            全选
          </button>
        </div>

        <div class="model-list" :class="{ empty: activeProfile.models.length === 0 }">
          <p v-if="activeProfile.models.length === 0" class="empty-text">
            点击“获取模型列表”后，模型会出现在这里并默认全选。
          </p>
          <div
            v-for="model in activeProfile.models"
            :key="model.id"
            class="model-row"
            :class="{ disabled: !model.enabled }"
          >
            <label class="model-check">
              <input v-model="model.enabled" type="checkbox" />
              <span>{{ model.id }}</span>
            </label>
            <div class="model-actions">
              <button
                type="button"
                @click="testModel(model)"
                :disabled="loading.testingModel === model.id"
              >
                {{ loading.testingModel === model.id ? "测试中" : "测试" }}
              </button>
              <button type="button" @click="toggleModel(model)">
                {{ model.enabled ? "禁用" : "启用" }}
              </button>
            </div>
          </div>
        </div>
      </section>
    </section>

    <footer class="submit-bar">
      <div>
        <strong>{{ activeProfile.name }}</strong>
        <span>
          {{ platformLabel }} · 账号
          {{ activeProfile.accountName || "自动时间命名" }} · 分组
          {{ groups.find((group) => group.id === activeProfile.groupId)?.name ?? "未选择" }} ·
          {{ enabledModels.length }} 个模型 · 优先级 {{ activeProfile.priority || 1 }}
        </span>
      </div>
      <button class="primary" type="button" @click="openSubmitConfirm" :disabled="loading.submit">
        发送
      </button>
    </footer>

    <aside v-if="ui.settingsOpen" class="modal-backdrop" @click.self="ui.settingsOpen = false">
      <section class="modal">
        <div class="section-head">
          <div>
            <h2>设置</h2>
            <p>后端地址只保存 origin，程序内部统一拼接 /api/v1。</p>
          </div>
          <button class="icon-button compact" type="button" @click="ui.settingsOpen = false">
            ×
          </button>
        </div>
        <label>
          <span>后端管理地址</span>
          <input v-model.trim="state.settings.backendBaseUrl" placeholder="http://localhost:8080" />
        </label>
        <label>
          <span>管理员 API Key</span>
          <input
            v-model.trim="state.settings.adminApiKey"
            type="password"
            placeholder="x-api-key"
          />
        </label>
        <label>
          <span>主题</span>
          <select v-model="state.settings.theme">
            <option value="light">日间模式</option>
            <option value="dark">夜间模式</option>
          </select>
        </label>
        <div class="actions-row end">
          <button type="button" @click="refreshGroups()">测试并刷新分组</button>
          <button class="primary" type="button" @click="ui.settingsOpen = false">完成</button>
        </div>
      </section>
    </aside>

    <aside v-if="ui.confirmOpen" class="modal-backdrop" @click.self="ui.confirmOpen = false">
      <section class="modal confirm">
        <div class="section-head">
          <div>
            <h2>确认发送</h2>
            <p>将创建一个 sub2api apikey 账号。</p>
          </div>
        </div>
        <dl class="summary-list">
          <div>
            <dt>Profile</dt>
            <dd>{{ activeProfile.name }}</dd>
          </div>
          <div>
            <dt>账号名称</dt>
            <dd>{{ activeProfile.accountName || "自动使用发送时间" }}</dd>
          </div>
          <div>
            <dt>平台</dt>
            <dd>{{ platformLabel }}</dd>
          </div>
          <div>
            <dt>分组</dt>
            <dd>{{ groups.find((group) => group.id === activeProfile.groupId)?.name }}</dd>
          </div>
          <div>
            <dt>模型</dt>
            <dd>{{ enabledModels.length }} 个已选择</dd>
          </div>
          <div>
            <dt>优先级</dt>
            <dd>{{ activeProfile.priority || 1 }}</dd>
          </div>
          <div>
            <dt>池模式</dt>
            <dd>{{ activeProfile.poolMode ? "开启" : "关闭" }}</dd>
          </div>
        </dl>
        <div class="actions-row end">
          <button type="button" @click="ui.confirmOpen = false">取消</button>
          <button class="primary" type="button" @click="submitProfile" :disabled="loading.submit">
            {{ loading.submit ? "发送中" : "确认发送" }}
          </button>
        </div>
      </section>
    </aside>

    <div class="toast-stack" aria-live="polite">
      <div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.kind">
        {{ toast.text }}
      </div>
    </div>
  </main>
</template>

<style>
:root {
  color-scheme: light;
  font-family:
    Inter, "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", Arial, sans-serif;
  line-height: 1.45;
  font-weight: 400;
  color: #20242a;
  background: #eef1f4;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  --bg: #eef1f4;
  --surface: #ffffff;
  --surface-soft: #f7f8fa;
  --border: #d9dee6;
  --text: #20242a;
  --muted: #67717f;
  --accent: #1f7a6d;
  --accent-strong: #166356;
  --danger: #b14343;
  --shadow: 0 16px 42px rgba(27, 37, 51, 0.11);
}

:root[data-theme="dark"] {
  color-scheme: dark;
  --bg: #17191d;
  --surface: #23262b;
  --surface-soft: #1d2025;
  --border: #373d46;
  --text: #edf0f3;
  --muted: #a0a8b3;
  --accent: #40b8a4;
  --accent-strong: #67cdbc;
  --danger: #ef7777;
  --shadow: 0 18px 48px rgba(0, 0, 0, 0.28);
  color: var(--text);
  background: var(--bg);
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  min-width: 720px;
  min-height: 100vh;
  background: var(--bg);
}

button,
input,
select {
  font: inherit;
}

button {
  min-height: 34px;
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  background: var(--surface);
  cursor: pointer;
}

button:hover:not(:disabled) {
  border-color: var(--accent);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

input,
select {
  width: 100%;
  min-height: 36px;
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0 10px;
  color: var(--text);
  background: var(--surface);
  outline: none;
}

input:focus,
select:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
}

label {
  display: grid;
  gap: 7px;
  color: var(--muted);
  font-size: 13px;
}

label span {
  color: var(--muted);
}

.app-shell {
  display: grid;
  grid-template-rows: auto auto 1fr auto;
  min-height: 100vh;
}

.topbar,
.profile-strip,
.submit-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}

.topbar {
  justify-content: space-between;
  padding: 14px 18px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 12px;
}

.brand-mark {
  display: inline-grid;
  place-items: center;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  color: #ffffff;
  background: var(--accent);
  font-weight: 700;
}

h1,
h2,
p {
  margin: 0;
}

h1 {
  font-size: 18px;
  letter-spacing: 0;
}

h2 {
  font-size: 17px;
  letter-spacing: 0;
}

p,
.muted {
  color: var(--muted);
  font-size: 13px;
}

.profile-strip {
  padding: 10px 18px;
  overflow-x: auto;
}

.profile-tab {
  flex: 0 0 auto;
  min-width: 122px;
  max-width: 210px;
  padding: 0 12px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.profile-tab.active {
  border-color: var(--accent);
  color: var(--accent-strong);
  background: color-mix(in srgb, var(--accent) 10%, var(--surface));
}

.profile-tab input {
  min-height: 28px;
  padding: 0 6px;
}

.icon-button {
  display: inline-grid;
  place-items: center;
  width: 38px;
  min-width: 38px;
  padding: 0;
  font-size: 18px;
}

.icon-button.compact {
  width: 34px;
  min-width: 34px;
}

.icon-button.danger {
  color: var(--danger);
}

.text-button {
  padding: 0 12px;
}

.primary {
  border-color: var(--accent);
  color: #ffffff;
  background: var(--accent);
  padding: 0 16px;
}

.primary:hover:not(:disabled) {
  background: var(--accent-strong);
}

.workspace {
  display: grid;
  grid-template-columns: minmax(320px, 420px) 1fr;
  gap: 16px;
  padding: 16px 18px 92px;
  min-height: 0;
}

.panel {
  min-width: 0;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: var(--shadow);
}

.profile-panel,
.models-panel {
  padding: 16px;
}

.section-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 16px;
}

.segmented {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 2px;
  background: var(--surface-soft);
}

.segmented button {
  min-height: 30px;
  border: 0;
  border-radius: 5px;
  padding: 0 10px;
  background: transparent;
}

.segmented button.active {
  color: #ffffff;
  background: var(--accent);
}

.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
}

.form-grid label:nth-child(1),
.form-grid label:nth-child(2),
.form-grid label:nth-child(3),
.form-grid label:nth-child(4) {
  grid-column: 1 / -1;
}

.inline-control {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 8px;
}

.inline-control button {
  padding: 0 12px;
}

.switch-row {
  align-content: start;
}

.switch-row input {
  width: 38px;
  height: 22px;
  min-height: 22px;
  accent-color: var(--accent);
}

.actions-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 16px;
}

.actions-row.end {
  justify-content: flex-end;
}

.model-list {
  display: grid;
  align-content: start;
  gap: 8px;
  max-height: calc(100vh - 270px);
  min-height: 320px;
  overflow: auto;
  padding-right: 4px;
}

.model-list.empty {
  place-items: center;
  border: 1px dashed var(--border);
  border-radius: 8px;
  background: var(--surface-soft);
}

.empty-text {
  max-width: 360px;
  text-align: center;
}

.model-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 12px;
  min-height: 46px;
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 6px 8px;
  background: var(--surface-soft);
}

.model-row.disabled {
  opacity: 0.58;
}

.model-check {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  color: var(--text);
}

.model-check input {
  width: 16px;
  min-width: 16px;
  min-height: 16px;
  accent-color: var(--accent);
}

.model-check span {
  overflow: hidden;
  color: var(--text);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-actions {
  display: flex;
  gap: 8px;
}

.model-actions button {
  padding: 0 10px;
}

.submit-bar {
  position: fixed;
  right: 0;
  bottom: 0;
  left: 0;
  justify-content: space-between;
  border-top: 1px solid var(--border);
  padding: 12px 18px;
  box-shadow: 0 -12px 32px rgba(25, 31, 39, 0.1);
}

.submit-bar div {
  display: grid;
  gap: 2px;
}

.submit-bar span {
  color: var(--muted);
  font-size: 13px;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 20;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.42);
}

.modal {
  display: grid;
  gap: 14px;
  width: min(560px, 100%);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 18px;
  background: var(--surface);
  box-shadow: var(--shadow);
}

.modal.confirm {
  width: min(460px, 100%);
}

.summary-list {
  display: grid;
  gap: 9px;
  margin: 0;
}

.summary-list div {
  display: grid;
  grid-template-columns: 92px 1fr;
  gap: 12px;
}

.summary-list dt {
  color: var(--muted);
}

.summary-list dd {
  margin: 0;
  color: var(--text);
  word-break: break-word;
}

.toast-stack {
  position: fixed;
  right: 18px;
  bottom: 76px;
  z-index: 40;
  display: grid;
  gap: 8px;
  width: min(420px, calc(100vw - 36px));
}

.toast {
  border: 1px solid var(--border);
  border-left: 4px solid var(--accent);
  border-radius: 8px;
  padding: 10px 12px;
  color: var(--text);
  background: var(--surface);
  box-shadow: var(--shadow);
  word-break: break-word;
}

.toast.error {
  border-left-color: var(--danger);
}

.toast.info {
  border-left-color: #61738f;
}

@media (max-width: 860px) {
  body {
    min-width: 360px;
  }

  .workspace {
    grid-template-columns: 1fr;
  }

  .model-list {
    max-height: none;
  }

  .submit-bar {
    align-items: flex-start;
  }
}
</style>
