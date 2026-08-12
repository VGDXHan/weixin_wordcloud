# Log — REQ-20260812-035845-weixin-wordcloud-download-export

## 2026-08-12T04:05:00Z | loop-start | INFO
- agent-loop 启动；requirement 由 `ready` 迁移到 `active`（已写目标文件并回读验证后删除源文件）。
- 已冷启动读取全部关键代码：`src-tauri/src/{lib,locate,config,ffi_key,dbcrypt,dbread,wordcloud,mock,model,error}.rs`、`src/{main,types,wordcloud}.ts`、`index.html`、`Cargo.toml`、`tauri.conf.json`、`capabilities/default.json`。
- 确认需求文件中的构建失败根因存在：`src/types.ts` 的 `WxStatus` 无 `wxid` 字段，而 `src/main.ts:75` 读取 `st.wxid`。

## 2026-08-12T04:06:00Z | loop-start | DECISION
- 导出写盘与「打开资源管理器」全部在 Rust 后端用 `std::fs` / `explorer.exe` 完成，**不引入** `tauri-plugin-dialog`/`tauri-plugin-fs`，`capabilities` 保持 `core:default`。符合需求「不得擅自决定」与「默认决策」。
- 理由：后端进程无需 Tauri 前端权限即可写盘，权限面最小、改动最小。

## 2026-08-12T04:08:00Z | gen-plan | INFO
- 已写 `plan.md`：T1 修构建 → T2 结构化读取 → T3 导出 JSON → T4 诊断与 async 化 → T5 前端 → T6 文档 → T7 验证。

## 2026-08-12T04:09:00Z | run | DECISION
- 新增 Rust 依赖 `chrono`（`default-features = false`, features `clock`+`std`）用于导出的本地可读时间与文件名时间戳。
- 理由：需求要求 `timestamp` 尽量给出可读本地时间；本地时区无法用 std 正确获得。这是 Rust crate 而非 Tauri 插件，不触及 capabilities/权限面。

## 2026-08-12T04:09:30Z | run | DECISION
- 额外修复：将全部 Tauri 命令改为 `async` + `spawn_blocking`，`AppState` 改 `Arc<Mutex<..>>`。
- 理由：Tauri 2 非 async 命令跑在主线程，而 `get_db_key` 最长阻塞 180s → 窗口在「正在初始化」期间完全冻结，用户无法按提示去重新登录微信，**真实模式实际上不可能抓到 key**。这是「真实读取从未跑通」的一个结构性原因，属目标 1 范围内，不算扩范围。

## 2026-08-12T04:11:00Z | run | **WARN**
- 首次 `cargo build --lib` 失败。
- detail: `error[E0283]: type annotations needed` @ `src/lib.rs:295` —— `Diagnostics::push(impl Into<String>)` 收到 `&str::into()` 造成推导歧义。
- action: continue（去掉多余的 `.into()`/`.to_string()` 后编译通过）

## 2026-08-12T04:14:00Z | run | INFO
- `cargo test --lib` 15 passed / 0 failed；`npm run build` 退出码 0（tsc 与 vite 均通过，AC-01 前端部分达成）。

## 2026-08-12T04:15:00Z | run | **WARN**
- `cargo build` 出现 `dead_code` 警告：`read_messages` 在词云路径改走 `read_chat` 后已冗余。
- action: continue（删除该函数并把单测改为直接断言 `read_chat(text_only)`，重建后 0 warning）

## 2026-08-12T04:17:00Z | run | INFO
- 补齐 mock 模式导出 + 词云全链路单测；`cargo test --lib` **17 passed / 0 failed**。
- `cargo build` / `npm run build` 均退出码 0 —— AC-01 三条命令全部达成。

## 2026-08-12T04:19:00Z | run | INFO
- 真实浏览器（`vite preview` + Playwright）验证前端产物：14 个 DOM id 全部命中、无脚本错误（仅 favicon 404）、
  未选会话时「导出 JSON」正确禁用、点击「诊断」在失败路径正确渲染且按钮状态复原。
- 已关闭浏览器并停掉 preview 服务（pid 91248）。

## 2026-08-12T04:21:00Z | run | INFO
- 已写 `execution.md`（代码落地 / 构建测试 / 运行接线 三类证据 + AC 状态 + 待用户实测清单）。
- 确认未越界：`capabilities/default.json` 未改动；451 处 WeFlow 删除未提交、git 历史未动。
- 下一步：进入 run 阶段审查（独立只读 Subagent）。

## 2026-08-12T04:26:00Z | run-review | INFO
- 第 1 轮审查（独立只读 Subagent）**VERDICT: PASS**，0 blocking / 7 suggestion。
- 审查方独立复跑三条命令并只读核对 git：capabilities 未改、451 处删除未提交、历史未动、非目标未做。

## 2026-08-12T04:27:00Z | run-review | DECISION
- 采纳 3 条 suggestion 并返工：#1 mock 的 limit 改为保留最新 N 条（与真实路径及 README 一致）；
  #6 4 处 `.lock().unwrap()` 换成 `lock_backend()` 避免 Mutex 中毒 panic（服务 AC-06「程序不崩溃」）；
  #7 初始化期间禁用「诊断」按钮，避免与 `init_wechat` 并发抢 wx_key Hook。
- 不采纳 #2（把 `strTalker` 加入 `SENDER_COLS`）：3.x 语境下它是会话对端而非发言人，会把自己发出的消息 sender 错标成对端，属引入错误数据；schema 不命中应走 `dump_schema` 校准。#3/#4/#5 需真实样本或属可选增强，不影响 AC。

## 2026-08-12T04:29:00Z | run-review | **WARN**
- 复跑三条验证命令时，串联的单条命令被用户中断（35s 处）。
- detail: 用户手动打断后要求「继续」；改为逐条执行以缩短单次阻塞时间。
- action: retry（逐条重跑，全部通过）

## 2026-08-12T04:32:00Z | run-review | INFO
- 返工后复跑：`npm run build` exit 0；`cargo build` exit 0 无 warning；`cargo test --lib` **17 passed / 0 failed**。
- 第 2 轮（复审改动）**VERDICT: PASS**，0 blocking：边界不 panic、`src-tauri/src` 已无 `.lock().unwrap()`、无跨 await 持锁、按钮不会永久禁用、不采纳 #2 的理由成立。
- **审查循环结束：连续两轮 PASS。** 本需求无长时运行实验，不进入 monitor 阶段。

## 2026-08-12T04:33:00Z | state-migrate | INFO
- 执行方交付范围（AC-01/06/07 + AC-05 逻辑）已全部完成并通过审查；AC-02~05 的真实数据部分按需求预先约定属「待用户本机实测」。
- requirement 迁移 `active → done`，`resolution: completed`。workspace 暂不归档，便于用户对照 `execution.md` 做本机实测。

## 2026-08-12T04:48:00Z | post-delivery | DECISION
- 用户询问如何配合实测后，补充「导出结构」按钮（`export_schema` 命令 + 侧栏入口）。
- 理由：`dump_schema` 原本只有后端命令、前端未接线，且 `tauri.conf.json` 未开 `withGlobalTauri`，用户无法从 devtools 调用它。一旦出现「真实模式已连上但会话/消息为空」，我方拿不到 schema 就无法校准 `dbread.rs` 的字段候选，实测循环会卡死。这是**打通既有「待用户实测」环节的必要工具**，不是新增功能范围。
- 未重开 requirement 状态：`done` 为终态，本次仅为支撑既定实测步骤的工具补充；若实测后需要真正改 schema 匹配逻辑，将按规则新建需求。
- 验证：`npm run build` exit 0；`cargo build` exit 0 无 warning；`cargo test --lib` **19 passed / 0 failed**（新增 2 个 schema 导出单测）；浏览器实测按钮存在、失败路径正确提示且按钮状态复原。

## 2026-08-12T05:30:00Z | post-delivery-real-test | **WARN**
- 用户按管理员 + 退出/重新登录步骤实测后仍进入演示模式，错误为「密钥无法解密数据库」。
- evidence: `%LOCALAPPDATA%\Temp\weixin_wordcloud_debug.log` 显示账号目录和 32-byte key 均已获取，但 `session.db` page1 HMAC 为 NO MATCH；因此不是权限/登录时机问题。
- action: continue（用真实库定位解密参数实现错误）

## 2026-08-12T05:38:00Z | post-delivery-real-test | DECISION
- 根因 1：SHA-512 的 digest 长度是 64，但 SQLCipher 4 的 HMAC sub-key 仍是 AES key size（32 bytes）；旧实现错误地用 `hmac_len=64` 作为 PBKDF2 输出长度。
- 根因 2：`wx_key.dll` 返回账号级数据库 key；每个 DB 有独立 salt，需分别执行 PBKDF2-SHA512(256000) 派生 AES key，再以 salt^0x3a / 2 iterations 派生 32-byte HMAC key。旧实现把 session.db 的 Cipher 复用于 contact/message DB，必然失败。
- 处理：修正 HMAC key length；RealCtx 改存账号 key，每个 DB 独立 detect/decrypt；detect/matches 只读 page1；解密改逐页流式写盘并清零 reserve，避免 134MB+ 消息库双倍内存。

## 2026-08-12T05:48:00Z | post-delivery-real-test | INFO
- 真实库端到端证据（一次性诊断测试，随后已从代码移除）：
  - `session.db`、`contact.db` 完整解密并通过 SQLite `PRAGMA integrity_check`；
  - 真实 schema 显示 `SessionTable.username` / `contact.username` 为 Name2Id 整数 rowid，修正读取与表选择评分后读到 **582 个真实会话**；
  - 流式解密真实 `message_0.db`（**134,860,800 bytes**），成功将会话映射到 `Msg_<md5>` 并读取 **3 条真实消息**。
- 安全收尾：删除一次性本机诊断代码、删除含旧版完整 key 的 debug log、删除 2 个失败测试遗留的明文库目录；新版日志不再写完整 key 或 key 前缀。

## 2026-08-12T05:55:00Z | post-delivery-real-test | INFO
- 永久回归覆盖增至 **22 tests**：32-byte HMAC key + 流式页输出、WeChat 4.x numeric Name2Id session/contact、仅选择编号 message/biz_message shard。
- 验证：`npm run build` exit 0；`cargo test --lib` **22 passed / 0 failed**；`cargo build` exit 0、0 warning。
- 独立只读审查 VERDICT: **PASS**，0 blocking；采纳安全/稳健性建议：debug 文件仅 debug 构建写入且不含 key 指纹、诊断 UI 不展示 key 前缀、Windows replace 错误清理 `.tmp`、24h 阈值 + RealCtx Drop 防多实例互删。
