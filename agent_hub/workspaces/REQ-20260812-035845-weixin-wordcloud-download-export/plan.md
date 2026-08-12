# Plan — REQ-20260812-035845-weixin-wordcloud-download-export

目标：修复构建 → 让真实读取可诊断可跑通 → 新增聊天记录导出为本地 JSON → 真实数据上词云可用 → 交付运行/验证说明。

## 约束回顾（不可越界）

- 平台仅 Windows + 微信 4.x；技术栈仍为 Tauri 2.x + Rust + 原生 TS/Vite。
- **不引入** `tauri-plugin-dialog`/`tauri-plugin-fs`，`capabilities` 保持 `core:default`：写盘与「打开文件夹」全在 Rust 后端做。
- 导出落盘按默认决策：`Documents\微信词云导出\<会话名或talker>_<时间戳>.json`，完成后打开所在文件夹。
- 不导出媒体；非文本消息在 JSON 中保留类型标记 + 占位文本；词云仍只用文本消息。
- 不提交、不改动那 451 处 WeFlow 源码删除，不动 git 历史。

## 任务分解

### T1 修复整体构建（AC-01）
- `src/types.ts`：`WxStatus` 的 `wechatVersion?` 替换为 `wxid?: string | null`，与 `model.rs` 的 `WxStatus` 对齐（camelCase 序列化）。
- 验证 `npm run build`（`tsc --noEmit && vite build`）通过。

### T2 结构化消息读取（支撑 AC-05，复用给 AC-04）
- `model.rs` 新增 `ChatMessage`（serde camelCase，`msg_type` 用 `#[serde(rename = "type")]`）：
  `timestamp / timeText / sender / isSelf / type / typeLabel / text`。
- `model.rs` 新增 `ExportResult { path, count, dir }`、`Diagnostics`（见 T4）。
- `dbread.rs` 新增 `read_chat(dbs, talker, limit, text_only) -> AppResult<Vec<ChatMessage>>`：
  - 沿用既有 `Msg_<md5(talker)>` 表名匹配；
  - 新增候选列常量：时间（`create_time`/`createTime`/`timestamp`/`sequence`）、
    发送者（`real_sender_id`/`sender_username`/`talker_id`/`sender`/`strTalker`）、
    自己发出（`is_sender`/`is_self`/`isSend`/`des`）；
  - 按时间升序输出；`local_type` → 可读标签映射；非文本消息给 `[图片]` 类占位文本；
  - `text_only = true` 时只取 `local_type == 1`（词云路径）。
- 群聊发言人尽力解析：若库中存在 `Name2Id` 形态表（单 username 列），用 rowid → username 映射把
  `real_sender_id` 数字翻译成 wxid；失败则保留原始 id 字符串（不阻塞导出）。
- `read_messages`（词云）改为复用 `read_chat(..., text_only = true)`，只保留一条读取代码路径。

### T3 导出为本地 JSON（AC-05）
- 新增 `src-tauri/src/export.rs`：
  - `export_json(wxid, talker, display_name, messages) -> AppResult<ExportResult>`；
  - 目录 `%USERPROFILE%\Documents\微信词云导出`（不存在则创建）；
  - 文件名 `<sanitize(displayName|talker)>_<yyyyMMdd_HHmmss>.json`，过滤 Windows 非法字符并限长；
  - 顶层结构 `{ wxid, talker, displayName, exportedAt, count, messages }`（严格按需求）；
  - `reveal_in_explorer(path)`：`explorer.exe /select,<path>`，失败不影响导出成功。
- `lib.rs` 新增命令 `export_chat_json(talker, displayName, limit)`；mock 模式也可导出（便于离线自测 AC-05 逻辑）。
- 时间可读化需要本地时区：新增 Rust 依赖 `chrono`（非 Tauri 插件，不涉及 capabilities）。

### T4 真实读取诊断与错误提示（AC-02 诊断、AC-06）
- 新增命令 `diagnose()`：逐步返回「账号目录 / wx_key.dll / 提取密钥 / 参数探测」四步的
  成功与否 + 具体原因，供用户定位为何进不了 `real` 模式。
- `WxStatus` 增加 `detail`（失败阶段与原始错误），前端可展示。
- `error.rs`：`DllNotFound` 文案去掉已不再需要的 `wcdb_api.dll`。
- **并发/卡死修复（真实跑通的关键）**：`ffi_key::get_db_key` 最长阻塞 180s，而 Tauri 2 中
  非 `async` 命令在主线程执行 → 会冻结整个窗口，用户无法「在初始化中重新登录微信」。
  将 `init_wechat`/`get_sessions`/`build_wordcloud`/`export_chat_json`/`dump_schema`/`diagnose`
  改为 `async` + `tauri::async_runtime::spawn_blocking`；`AppState` 改为 `Arc<Mutex<Option<Backend>>>`
  以便把状态移入阻塞线程。
- 顺带收尾：init 时清理上次遗留的 `%TEMP%\weixin_wordcloud_*` 目录。

### T5 前端接线（AC-04/05/06）
- `index.html`：工具栏新增「导出 JSON」按钮；侧栏新增「诊断」按钮与诊断结果展示区。
- `main.ts`：接线导出（进行中禁用、成功后 status 显示落盘路径）、诊断（列出四步结果）。
- `types.ts`：补 `ExportResult`、`Diagnostics` 类型。

### T6 文档（AC-07）
- 新建 `README.md`：管理员运行方式、抓 key 时机（需退出并重新登录微信）、导出文件位置、
  常见失败排查（权限/未登录/版本不兼容/未抓到 key/schema 不命中用 `dump_schema`）、mock 兜底说明。

### T7 测试与验证（AC-01）
- 新增单测：文件名 sanitize、`local_type` → 标签映射、导出 JSON 结构（serde 字段名含 `type`）、
  本地时间格式化。
- 依次执行并留证：`npm run build`、`cargo build`、`cargo test`。

## 验收对应关系

| AC | 由谁保证 | 备注 |
|---|---|---|
| AC-01 | 执行方 | 三条命令全绿，留终端证据 |
| AC-02 | 用户实测 | 执行方交付 `diagnose()` + 非阻塞初始化 + 明确原因 |
| AC-03 | 用户实测 | 代码路径不变，仅确保可诊断 |
| AC-04 | 用户实测 | 词云读取改为复用 `read_chat` |
| AC-05 | 执行方（mock 逻辑自测）+ 用户实测（真实数据） | 单测 + mock 模式导出可跑 |
| AC-06 | 执行方 | mock 兜底保留；错误分阶段可读 |
| AC-07 | 执行方 | `README.md` |

## 风险

- `read_chat` 的列候选仍是启发式猜测；真实 schema 不命中会导致空结果 —— 用 `dump_schema` 校准，
  文档写明该排查路径。
- `async` 化涉及 `Mutex` 跨线程移动，需保证不在 `.await` 期间持锁（编译期即可暴露）。
- 真实端到端（AC-02~05 的真实数据部分）无法由执行方验证，必须标注为待用户实测。
