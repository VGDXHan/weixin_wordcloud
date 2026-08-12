import { invoke } from "@tauri-apps/api/core";
import type {
  Diagnostics,
  ExportResult,
  Session,
  WordFreq,
  WxStatus,
} from "./types";
import { renderCloud, exportPng } from "./wordcloud";

const el = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const statusEl = el<HTMLDivElement>("status");
const sessionsEl = el<HTMLUListElement>("sessions");
const searchEl = el<HTMLInputElement>("search");
const titleEl = el<HTMLSpanElement>("current-title");
const canvas = el<HTMLCanvasElement>("cloud");
const hintEl = el<HTMLDivElement>("hint");
const loadingEl = el<HTMLDivElement>("loading");
const exportBtn = el<HTMLButtonElement>("export");
const exportJsonBtn = el<HTMLButtonElement>("export-json");
const refreshBtn = el<HTMLButtonElement>("refresh");
const diagnoseBtn = el<HTMLButtonElement>("diagnose");
const schemaBtn = el<HTMLButtonElement>("export-schema");
const diagnosticsEl = el<HTMLDivElement>("diagnostics");
const limitEl = el<HTMLInputElement>("limit");
const topnEl = el<HTMLInputElement>("topn");

const RENDER_BATCH = 80;

let allSessions: Session[] = [];
let filtered: Session[] = [];
let renderCursor = 0;
let activeTalker: string | null = null;
let hasCloud = false;
let firstCloudBuild = true;

function setStatus(text: string, kind: "" | "ok" | "err" = "") {
  statusEl.textContent = text;
  statusEl.className = "status" + (kind ? " " + kind : "");
}

function kindLabel(kind: Session["kind"]): string {
  return { friend: "好友", group: "群聊", official: "公众号", other: "其他" }[kind];
}

function makeSessionItem(s: Session): HTMLLIElement {
  const li = document.createElement("li");
  li.dataset.talker = s.talker;
  if (s.talker === activeTalker) li.classList.add("active");
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = s.displayName || s.talker;
  const sub = document.createElement("span");
  sub.className = "sub";
  sub.textContent = kindLabel(s.kind);
  li.append(name, sub);
  return li;
}

// Progressive rendering: only append a batch at a time; more loads on scroll.
function renderMore() {
  const slice = filtered.slice(renderCursor, renderCursor + RENDER_BATCH);
  if (slice.length === 0) return;
  const frag = document.createDocumentFragment();
  for (const s of slice) frag.appendChild(makeSessionItem(s));
  sessionsEl.appendChild(frag);
  renderCursor += slice.length;
}

function renderSessionList(items: Session[]) {
  filtered = items;
  renderCursor = 0;
  sessionsEl.innerHTML = "";
  renderMore();
}

function messageLimit(): number {
  return Math.max(100, Number(limitEl.value) || 30000);
}

async function init() {
  setStatus("正在初始化…（若长时间停在此处，请彻底退出并重新登录微信，密钥在登录瞬间捕获）");
  // Initialization already holds the wx_key hook; a parallel diagnose would
  // fight over it. The schema dump needs an initialized backend anyway.
  diagnoseBtn.disabled = true;
  schemaBtn.disabled = true;
  try {
    const st = await invoke<WxStatus>("init_wechat");
    if (st.mode === "mock") {
      setStatus(`演示模式：${st.message}`, "err");
    } else {
      setStatus(`已连接微信数据 ${st.wxid ?? ""}`.trim(), "ok");
    }
    await loadSessions();
  } catch (e) {
    setStatus(`初始化失败：${e}`, "err");
  } finally {
    diagnoseBtn.disabled = false;
    schemaBtn.disabled = false;
  }
}

async function loadSessions() {
  try {
    allSessions = await invoke<Session[]>("get_sessions");
    searchEl.placeholder = `搜索会话…（共 ${allSessions.length} 个）`;
    applyFilter();
    if (allSessions.length === 0) setStatus("未找到任何会话", "err");
  } catch (e) {
    setStatus(`加载会话失败：${e}`, "err");
  }
}

function applyFilter() {
  const q = searchEl.value.trim().toLowerCase();
  const items = q
    ? allSessions.filter(
        (s) =>
          s.displayName.toLowerCase().includes(q) ||
          s.talker.toLowerCase().includes(q)
      )
    : allSessions;
  renderSessionList(items);
}

async function selectSession(talker: string) {
  const s = allSessions.find((x) => x.talker === talker);
  if (!s) return;
  activeTalker = s.talker;
  titleEl.textContent = s.displayName || s.talker;
  exportJsonBtn.disabled = false;
  document
    .querySelectorAll(".sessions li")
    .forEach((li) =>
      li.classList.toggle(
        "active",
        (li as HTMLLIElement).dataset.talker === s.talker
      )
    );
  await buildCloud();
}

async function buildCloud() {
  if (!activeTalker) return;
  hintEl.classList.add("hidden");
  loadingEl.textContent = firstCloudBuild
    ? "首次读取正在解密消息库，可能需要 1–3 分钟…"
    : "生成中…";
  loadingEl.classList.remove("hidden");
  exportBtn.disabled = true;
  try {
    const topN = Math.max(20, Number(topnEl.value) || 150);
    const freqs = await invoke<WordFreq[]>("build_wordcloud", {
      talker: activeTalker,
      limit: messageLimit(),
      topN,
    });
    if (freqs.length === 0) {
      setStatus("该会话没有可用于分词的文本消息", "err");
      hintEl.textContent = "没有可用文本";
      hintEl.classList.remove("hidden");
      hasCloud = false;
    } else {
      renderCloud(canvas, freqs);
      hasCloud = true;
    }
  } catch (e) {
    setStatus(`生成失败：${e}`, "err");
  } finally {
    firstCloudBuild = false;
    loadingEl.classList.add("hidden");
    exportBtn.disabled = !hasCloud;
  }
}

function doExport() {
  if (!hasCloud) return;
  const dataUrl = exportPng(canvas);
  const a = document.createElement("a");
  a.href = dataUrl;
  a.download = `wordcloud_${activeTalker ?? "chat"}.png`;
  a.click();
  setStatus("已导出 PNG", "ok");
}

async function doExportJson() {
  if (!activeTalker) return;
  const s = allSessions.find((x) => x.talker === activeTalker);
  exportJsonBtn.disabled = true;
  setStatus("正在导出聊天记录…");
  try {
    const res = await invoke<ExportResult>("export_chat_json", {
      talker: activeTalker,
      displayName: s?.displayName ?? activeTalker,
      limit: messageLimit(),
    });
    setStatus(`已导出 ${res.count} 条消息：${res.path}`, "ok");
  } catch (e) {
    setStatus(`导出失败：${e}`, "err");
  } finally {
    exportJsonBtn.disabled = false;
  }
}

function renderDiagnostics(d: Diagnostics) {
  diagnosticsEl.innerHTML = "";
  for (const step of d.steps) {
    const row = document.createElement("div");
    row.className = "step " + (step.ok ? "ok" : "bad");
    const mark = document.createElement("span");
    mark.className = "mark";
    mark.textContent = step.ok ? "✓" : "✗";
    const body = document.createElement("span");
    const name = document.createElement("div");
    name.className = "step-name";
    name.textContent = step.name;
    const info = document.createElement("div");
    info.className = "step-info";
    info.textContent = step.info;
    body.append(name, info);
    row.append(mark, body);
    diagnosticsEl.appendChild(row);
  }
  diagnosticsEl.classList.remove("hidden");
}

async function doExportSchema() {
  schemaBtn.disabled = true;
  setStatus("正在导出数据库结构…");
  try {
    const res = await invoke<ExportResult>("export_schema");
    setStatus(`已导出 ${res.count} 张表的结构：${res.path}`, "ok");
  } catch (e) {
    setStatus(`导出结构失败：${e}`, "err");
  } finally {
    schemaBtn.disabled = false;
  }
}

async function doDiagnose() {
  diagnoseBtn.disabled = true;
  const label = diagnoseBtn.textContent;
  diagnoseBtn.textContent = "诊断中…（可能需要重新登录微信）";
  try {
    renderDiagnostics(await invoke<Diagnostics>("diagnose"));
  } catch (e) {
    renderDiagnostics({ steps: [{ name: "诊断失败", ok: false, info: String(e) }] });
  } finally {
    diagnoseBtn.disabled = false;
    diagnoseBtn.textContent = label;
  }
}

// Event delegation: one click listener for the whole list.
sessionsEl.addEventListener("click", (e) => {
  const li = (e.target as HTMLElement).closest("li");
  const talker = (li as HTMLLIElement | null)?.dataset.talker;
  if (talker) selectSession(talker);
});

// Lazy-load more items when scrolling near the bottom.
sessionsEl.addEventListener("scroll", () => {
  const nearBottom =
    sessionsEl.scrollTop + sessionsEl.clientHeight >=
    sessionsEl.scrollHeight - 200;
  if (nearBottom && renderCursor < filtered.length) renderMore();
});

searchEl.addEventListener("input", applyFilter);
refreshBtn.addEventListener("click", loadSessions);
exportBtn.addEventListener("click", doExport);
exportJsonBtn.addEventListener("click", doExportJson);
diagnoseBtn.addEventListener("click", doDiagnose);
schemaBtn.addEventListener("click", doExportSchema);
[limitEl, topnEl].forEach((e) =>
  e.addEventListener("change", () => {
    if (activeTalker) buildCloud();
  })
);

init();
