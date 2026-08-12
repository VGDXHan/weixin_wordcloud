# Execution — REQ-20260812-035845-weixin-wordcloud-download-export

## 一、代码落地证据

### 新增文件

| 文件 | 作用 |
|---|---|
| `src-tauri/src/export.rs` | 聊天记录导出为本地 JSON、文件名 sanitize、`explorer /select` 定位 |
| `src-tauri/src/timefmt.rs` | 时间戳归一化（秒/毫秒）与本地时间格式化 |
| `README.md` | 运行/验证说明（AC-07） |

### 修改文件

| 文件 | 改动 |
|---|---|
| `src/types.ts` | `WxStatus` 去掉不存在的 `wechatVersion`，改为与后端一致的 `wxid`/`detail`；新增 `ExportResult`/`DiagnoseStep`/`Diagnostics` |
| `src-tauri/src/model.rs` | 新增 `ChatMessage`（`type`/`typeLabel`/`timeText`/`isSelf`…）、`ExportResult`、`Diagnostics`；新增 `type_label()`/`type_placeholder()`；`WxStatus` 增 `detail` |
| `src-tauri/src/dbread.rs` | 新增 `read_chat()` 结构化读取（时间/发送者/类型/自己发出）、`Name2Id` 群发言人解析、群消息 `wxid:\n` 前缀剥离、按时间取最近 N 条并升序输出；删除已冗余的 `read_messages()` |
| `src-tauri/src/lib.rs` | 全部命令改 `async` + `spawn_blocking`；`AppState` 改 `Arc<Mutex<..>>`；新增 `export_chat_json`/`diagnose` 命令；启动清理遗留临时目录 |
| `src-tauri/src/mock.rs` | 新增 `chat()` 结构化演示数据（含 1 条非文本消息），让导出链路可离线验证 |
| `src-tauri/src/error.rs` | `DllNotFound` 文案去掉已不再依赖的 `wcdb_api.dll` |
| `src-tauri/Cargo.toml` | 新增 `chrono`（`default-features = false`, `clock`+`std`） |
| `index.html` / `src/styles.css` / `src/main.ts` | 新增「导出 JSON」「诊断真实读取」入口与诊断结果面板并接线 |

### 关键实现说明

**AC-01 构建修复的直接根因**：`src/types.ts` 的 `WxStatus` 声明了 `wechatVersion?`，而 `main.ts:75`
读 `st.wxid`（后端 `model.rs` 返回的正是 `wxid`）。已按后端为准对齐类型。

**额外发现并修复的真实读取阻塞点（对 AC-02 关键）**：Tauri 2 中非 `async` 命令在主线程执行，
而 `ffi_key::get_db_key` 最长阻塞 180 秒。原实现会让窗口在「正在初始化」期间**完全冻结**，
用户无法按提示去「退出并重新登录微信」，真实模式实际上永远抓不到 key。已将全部命令改为
`async` + `tauri::async_runtime::spawn_blocking`，`AppState` 改 `Arc<Mutex<..>>` 以便把状态移入
阻塞线程；不在 `.await` 期间持锁（编译通过即为证）。

**权限面未扩大**：导出写盘与打开资源管理器全在 Rust 侧用 `std::fs` / `explorer.exe` 完成，
未引入 `tauri-plugin-dialog`/`tauri-plugin-fs`，`capabilities/default.json` 仍是 `core:default`
（未改动）。落盘路径与命名严格按需求默认决策：
`%USERPROFILE%\Documents\微信词云导出\<会话名>_<yyyyMMdd_HHmmss>.json`。

**JSON 结构**与需求逐字段一致：顶层 `{ wxid, talker, displayName, exportedAt, count, messages }`；
每条 `{ timestamp, timeText, sender, isSelf, type, typeLabel, text }`。非文本消息保留类型标记 +
`[图片]` 类占位文本，**不导出媒体文件**；词云仍只用 `type == 1` 的文本消息。

**未越界**：目标平台/微信版本未变；451 处 WeFlow 源码删除未提交、git 历史未动。

## 二、构建与测试证据

```text
> npm run build
> tsc --noEmit && vite build
✓ 10 modules transformed.
dist/index.html                  2.00 kB
dist/assets/index-Bfa0To-b.css   3.16 kB
dist/assets/index-C72KevA6.js   16.91 kB
✓ built in 177ms
（退出码 0）

> cargo build
Finished `dev` profile [unoptimized + debuginfo] target(s) in 13.45s
（退出码 0，删除冗余 read_messages 后 0 warning）

> cargo test --lib
running 17 tests
... 全部 ok ...
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

新增单测 14 个（原有 3 个分词测试保持全绿）：

- `timefmt`：毫秒/秒归一化、空时间戳、文件名时间戳安全性（3）
- `export`：非法字符 sanitize、长度截断不切坏多字节字符、JSON 字段名（含 `type`）、
  真实写盘后可读回、空会话报错（5）
- `dbread`：talker 分表匹配不误伤 `Name2Id`/`session`、群发言人前缀剥离（含「时间: 明天」不误切）、
  **合成 WeChat 4.x schema 的端到端读取**（时间升序、`is_sender`、`Name2Id` 解析、
  `[图片]` 占位、`text_only` 过滤、limit 取最近 N 条、不存在的 talker 返回空）（4）
- `mock`：演示模式导出 + 词云全链路、演示会话类型覆盖（2）

## 三、运行证据（前端加载 + 接线）

Tauri 原生窗口内的真实读取无法由执行方验证（需管理员 + 微信 4.x 登录），
但前端产物的加载与新增控件接线已在真实浏览器中验证（`vite preview` + Playwright）：

```text
Page URL: http://localhost:4173/  Title: 微信聊天词云
Console: 仅 1 条 favicon.ico 404（无 Tauri 环境下的预期噪音），无脚本错误

DOM 检查：
{ missing: [],                      // status/sessions/search/current-title/cloud/hint/
                                    // loading/export/export-json/refresh/diagnose/
                                    // diagnostics/limit/topn 全部存在
  statusText: "初始化失败：TypeError: ...",  // 浏览器无 Tauri IPC，走错误分支
  exportJsonDisabled: true,         // 未选会话时正确禁用
  diagnosticsHidden: true }

点击「诊断真实读取」后：
{ hidden: false, steps: 1, firstStep: "诊断失败", mark: "✗",
  btnLabel: "诊断真实读取", btnDisabled: false }   // 失败路径正确渲染且按钮状态复原
```

这证明：模块级 `el()` 查询全部命中（新增 id 无拼写错误）、事件监听已挂载、
`init()` 与 `doDiagnose()` 的异常分支都不会白屏或崩溃（支撑 AC-06 的「程序不崩溃」）。

## 三点五、审查循环（run 阶段，独立只读 Subagent）

### 第 1 轮 — VERDICT: PASS（0 blocking / 7 suggestion）

审查方独立复跑了 `npm run build`、`cargo build`、`cargo test --lib`（17 passed / 0 failed），
并只读核对 `git status --porcelain` 与 `git log --oneline -5`，确认：

- 未引入 `tauri-plugin-dialog`/`tauri-plugin-fs`，`capabilities/default.json` 相对 HEAD 无 diff；
- 451 处 WeFlow 删除**未提交**、git 历史未改（`git log` 仍只有 `f556b44 init: 初始化项目仓库`）；
- 目标平台/微信版本未变；「非目标」（媒体导出、群聊发言人细分统计、时间筛选、自定义配色）均未做；
- 逐行核对 `split_group_prefix` 的 UTF-8 边界后确认**无 panic 风险**（`head` 是 `body` 的字符前缀，
  `find(':')` 的字节索引在 `body` 中一致）。

### 采纳的 3 条 suggestion（改动后已复跑并再次送审）

| # | 问题 | 处理 |
|---|---|---|
| 1 | mock 分支 `truncate(limit)` 保留的是**最早** N 条，与真实路径「最新 N 条」及 README 表述不一致 | 改为 `drain(..len - limit)` 保留最新 N 条 |
| 6 | 4 处 `.lock().unwrap()` 在 Mutex 中毒时会 panic，与 AC-06「程序不崩溃」相悖 | 新增 `lock_backend()`，中毒时返回 `AppError::Other("内部状态已损坏，请重启应用")` |
| 7 | `init_wechat` 与 `diagnose` 可并发各跑 180s 的 `get_db_key`，互相抢 Hook | 前端在初始化期间禁用「诊断」按钮，`finally` 恢复 |

### 未采纳的 suggestion 及理由

- **#2 把 `strTalker` 加入 `SENDER_COLS`**：微信 3.x 的 MSG 表中 `strTalker` 是**会话对端**而非
  本条消息发言人，加入后会把「自己发出的消息」的 sender 错标成对端 —— 属于引入错误数据。
  真实 schema 不命中时应走 `dump_schema` 校准，而不是加高风险候选列。审查方复审确认**该理由成立**。
- **#3/#4/#5**：分别需要真实微信样本（`pick()` 子串误匹配、群前缀非 `\n` 变体）或属可选增强
  （前端单独展示 `detail`），不影响任何 AC。

### 第 2 轮（复审改动） — VERDICT: PASS（0 blocking）

审查方独立复核：mock 的 `limit == 0` / `limit > len` 边界不 panic 且与真实路径语义一致；
`src-tauri/src` 内已无 `.lock().unwrap()`，4 处调用点均以 `?` 传播且**无跨 `.await` 持锁**；
「诊断」按钮在异常路径也会恢复，不会永久禁用。构建与 17 单测独立复跑通过。

**审查循环结束：连续两轮 PASS、0 blocking。**

## 四、验收状态

| AC | 状态 | 依据 |
|---|---|---|
| AC-01 项目可整体构建 | **通过** | 上述三条命令退出码 0、17 单测全绿 |
| AC-02 进入 real 模式 | **待用户本机实测** | 交付 `diagnose()` 四步诊断 + 修掉主线程冻结（原本导致无法在登录瞬间抓 key） |
| AC-03 真实会话列表 | **待用户本机实测** | 读取路径未变；空结果可用 `dump_schema` 校准 |
| AC-04 真实数据词云 + PNG | **待用户本机实测** | 词云读取改走 `read_chat(text_only)`；PNG 导出未改 |
| AC-05 导出 JSON | **逻辑已自测通过 / 真实数据待用户实测** | 合成 schema 端到端读取 + 真实写盘回读 + 演示模式全链路单测 |
| AC-06 失败提示不崩溃 + mock 兜底 | **通过** | 分阶段错误文案、`detail` 字段、演示模式兜底、前端异常分支已实测 |
| AC-07 运行/验证说明 | **通过** | `README.md`（管理员运行、抓 key 时机、导出位置、四环节排查表） |

## 四点五、交付后补充：「导出结构」按钮（支撑实测循环）

`dump_schema` 原本只有后端命令、前端未接线，且 `tauri.conf.json` 未开 `withGlobalTauri`，
用户无法从 devtools 调用它。一旦出现「真实模式已连上但会话/消息为空」，
拿不到 schema 就无法校准 `dbread.rs` 的字段候选，实测循环会卡死。因此补充：

- `src-tauri/src/export.rs`：抽出 `write_json()` 复用；新增 `export_schema()` 写
  `schema_<时间戳>.json`（空 dump 报错）。
- `src-tauri/src/lib.rs`：抽出 `collect_schema()`（mock 模式给出明确提示而非空结果）；
  新增 `export_schema` 命令并注册。
- `index.html` / `styles.css` / `src/main.ts`：侧栏「诊断真实读取」旁新增「导出结构」按钮并接线。
- `README.md`：排查表里改为指引点该按钮。

验证：`npm run build` exit 0；`cargo build` exit 0 无 warning；`cargo test --lib` **19 passed / 0 failed**
（新增 `schema_export_writes_tables_and_columns`、`schema_export_rejects_an_empty_dump`）；
浏览器实测按钮存在、无 Tauri IPC 时失败路径正确提示且按钮状态复原。

未重开 requirement 状态：`done` 为终态，本次仅为支撑既定「待用户实测」步骤的工具补充。

## 四点六、真实数据返测：解密与 schema 根因修复

用户按管理员运行、退出并重新登录微信后，应用仍报「密钥无法解密数据库」。日志证明账号目录、
DLL 和 key 捕获均成功，失败点是 page1 HMAC。用用户本机的真实加密库做一次性诊断后确认：

1. **HMAC sub-key 长度错误**：SHA-512 摘要为 64 bytes，但 SQLCipher 4 的 MAC key 仍是
   AES-256 key size（32 bytes）。旧实现错误使用 `hmac_len=64` 作为 PBKDF2 输出长度。
2. **错误复用 session Cipher**：`wx_key.dll` 返回账号级 key；每个 DB 有独立 salt，必须分别
   `PBKDF2-SHA512(account_key, db_salt, 256000, 32)` 派生 AES key，再由
   `salt XOR 0x3a` / 2 iterations 派生 32-byte HMAC key。旧实现将 session.db 的 Cipher
   复用于 contact/message DB。
3. **真实 schema 使用整数外键**：`SessionTable.username` 和 `contact.username` 是
   `Name2Id` 的 rowid，不是字符串；旧 reader 将整数直接读成 String，结果全部变成空值。
4. **会话表误选风险**：`SessionUnreadListTable_1.username_id + create_time` 会被原来的
   子串启发式选中；现在对 `SessionTable` 和精确 username 列加权。

修复后真实端到端验证：

```text
session.db:  完整解密 + PRAGMA integrity_check = ok
contact.db:  完整解密 + 可通过 Name2Id 解析联系人
真实会话:    582
message_0.db: 134,860,800 bytes，逐页流式解密成功
真实消息:    会话 hash 命中 Msg_<md5>，读取 3 条
```

同时将解密改为 BufReader/BufWriter 逐页处理、临时文件成功后替换，排除 `media_*`、
`message_fts.db`、`message_resource.db`；旧版含完整 key 的日志与失败测试遗留明文库已删除，
新版不记录完整 key/前缀。一次性真实库诊断代码已移除，永久合成回归测试保留。

最终验证：`npm run build` exit 0；`cargo build` exit 0、0 warning；
`cargo test --lib` **22 passed / 0 failed**；独立只读审查 **PASS / 0 blocking**。

## 五、待用户实测清单（真实端到端）

1. 以管理员运行 `scripts\run-dev-admin.ps1`。
2. 界面显示「正在初始化…」时，**彻底退出并重新登录微信**。
3. 状态栏应变为「已连接微信数据 wxid_xxx」（AC-02）；左侧出现真实会话（AC-03）。
4. 选一个会话 → 出词云 → 导出 PNG（AC-04）。
5. 点「导出 JSON」→ 资源管理器定位到 `Documents\微信词云导出\<会话名>_<时间戳>.json`（AC-05）。
6. 若仍是演示模式，点「诊断真实读取」，把四步结果回报；若真实模式但会话/消息为空，
   点「导出结构」把 `schema_<时间戳>.json` 发回，用于校准 `dbread.rs` 的字段候选常量。
7. 需要更底层线索时，`%TEMP%\weixin_wordcloud_debug.log` 记录了解密参数探测过程
   （`[detect] MATCH ...` 或 `[detect] NO MATCH ...`）。
