---
id: REQ-20260812-035845-weixin-wordcloud-download-export
status: done
resolution: completed
created_at: 2026-08-12T03:58:45Z
updated_at: 2026-08-12T04:33:00Z
---

> **执行结论（2026-08-12）**：执行方交付范围已全部完成，run 阶段审查连续两轮 PASS、0 blocking。
> AC-01/06/07 达成；AC-05 的导出逻辑已由单测与合成 schema 端到端验证。
> AC-02/03/04 与 AC-05 的**真实数据部分**需管理员权限 + 微信 4.x 登录，按本需求「约束」与「已确认决策」
> 预先约定由用户本机实测，不在执行方可验证范围内。
> 详见 `agent_hub/workspaces/REQ-20260812-035845-weixin-wordcloud-download-export/execution.md`
> 的「待用户实测清单」。若本机实测发现真实读取仍不可用，请新建需求（`done` 为终态）。

# 微信聊天词云生成器 — 跑通真实读取 + 聊天记录导出为本地 JSON + 词云

## 背景
- 本工作区根目录 `D:\Mainpage\weixin_wordcloude` 已有一个 Tauri 2.x 桌面项目，完整实现了链路：
  选会话 → 定位并读取本地微信 4.x 数据库 → **自实现 SQLCipher-4 页级解密** → jieba 中文分词 →
  wordcloud2.js 词云渲染 → 导出 PNG。任一步失败会**自动回退到演示(mock)数据**，保证 UI 可用。
- 读取密钥复用 WeFlow 的原生库 `wx_key.dll`（已内置于 `src-tauri/resources/`，通过 `libloading` FFI
  调用 `InitializeHook/PollKeyData/...`）；**解密与数据库读取均为本项目自实现**（不依赖 WeFlow 的
  `wcdb_api.dll`/`WCDB.dll`）。
- 用户反馈「这个项目没做完」，诉求是：能**直接把微信聊天记录下载/保存到本地**，再**转化为词云**；
  读取/破解方法基于 WeFlow 这个开源项目的原理。
- 冷启动执行者须知：真实读取从未在用户机器上端到端验证过；且目前**前端构建失败**（见「已知事实」），
  项目当前无法整体构建运行。

## 目标
1. **修复构建并跑通真实读取**：让应用在用户 Windows + 微信 4.x 环境下，`init_wechat` 真正进入
   `real` 模式（不再只停留在 mock），能列出真实会话、读到真实消息。
2. **新增「聊天记录导出为本地 JSON」功能**：对所选会话把聊天记录保存为本地 JSON 文件。
3. **真实数据上词云可用**：选会话即出词云并可导出 PNG。

## 交付物
- 可整体构建运行的桌面应用（前端 + Rust 后端）。
- 真实模式下：会话列表、消息读取、词云、JSON 导出均可用。
- 导出的 JSON 文件（含时间、发送者、消息类型、文本内容）。
- 简短运行/验证说明（如何以管理员运行、抓 key 的时机、导出文件位置、真实读取失败时的排查）。

## 范围与边界
- 目标平台：**Windows + 微信 4.x**（沿用既有决策）。
- 数据合规：仅读取**用户本机、用户本人**的微信数据，全程本地、绝不外传。
- 技术栈：沿用 **Tauri 2.x + Rust 后端 + 原生 TypeScript/Vite 前端**。
- 读取方式：沿用 `wx_key.dll` 提 key + 项目**自实现** SQLCipher-4 解密与 DB 读取。
- 词云范围：仅对**文本消息**做分词与词云（沿用首版范围）。

## 约束
- 提 key 需**管理员权限**；`wx_key.dll` 通常在**微信登录瞬间**捕获密钥，因此可能需要「退出并重新登录微信」才能抓到 key（现有 `ffi_key.rs` 已实现重挂 Hook 逻辑）。
- 微信不同版本的**消息表/字段随版本变化**；现为多候选字段猜测匹配（`dbread.rs` 的 `*_COLS` 常量、`Msg_<md5(talker)>` 表名匹配），需按真实解密后的 schema 校准（可用现有 `dump_schema` 命令）。
- 真实端到端只能在用户本机验证；执行方在无真实微信环境时，只能保证：能编译、`cargo test` 通过、代码逻辑正确、mock 模式可用——真实读取/导出/词云的实测由用户在场完成，并明确标注哪些 AC 属「待用户实测」。
- `src-tauri/capabilities/default.json` 当前仅授予 `core:default` 权限；若导出需要系统「保存对话框」或前端直接写盘，需评估是否引入 `tauri-plugin-dialog`/`tauri-plugin-fs`——见「不得擅自决定」。

## 不得擅自决定的事项
- 改变目标平台或微信版本范围（Windows + 微信 4.x 之外）。
- 是否把「删除 WeFlow 原始源码」的 **451 处未提交删除**提交进 git / 改动 git 历史（默认**不动**）。
- 导出 JSON 的**最终落盘目录与命名规则**、以及是否弹「选择目录/保存对话框」（默认见下方，需与用户确认后才更改为对话框方案）。
- 是否为导出/写盘**引入额外 Tauri 插件**（`dialog`/`fs`）或调整 `capabilities` 权限。
- 是否导出**媒体文件**（图片/语音/视频）——默认不导出。

## 非目标
- 微信 3.x、macOS、Linux。
- 群聊按发言人细分、时间范围筛选、自定义词云形状/配色/字体（留待后续迭代）。
- 媒体文件导出、语音转文字等 WeFlow 高级功能。
- 图片/语音/视频等非文本消息纳入词云统计。

## 默认决策（可改，未获用户改动前按此执行）
- **导出范围**：默认「当前所选会话」（与词云一致）；后续可扩展多选/全部。
- **导出落盘**：默认写入 `Documents\微信词云导出\<会话名或talker>_<时间戳>.json`，完成后在资源管理器打开所在文件夹；暂不加系统选目录对话框（受限于当前 `core:default` 权限）。
- **媒体**：不导出媒体文件；非文本消息在 JSON 中保留类型标记与占位文本（如 `[图片]`/`[语音]`），词云仍只用文本消息。
- **JSON 结构**：
  - 顶层：`{ wxid, talker, displayName, exportedAt, count, messages: [...] }`
  - 每条消息：`{ timestamp, sender, isSelf, type, typeLabel, text }`
    （`timestamp` 尽量给出可读本地时间；`sender` 为发送者标识/群内发言人；`type` 为原始 `local_type`，`typeLabel` 为其可读标签）。

## 验收标准
- [x] AC-01: 项目可整体构建 —— `npm run build` 通过、`cargo build` 通过、`cargo test` 全绿。
      → 三条命令均 exit 0（`cargo build` 0 warning），`cargo test --lib` **17 passed / 0 failed**；审查方独立复跑一致。
- [ ] AC-02: 以**管理员**运行且微信 4.x 已登录时，`init_wechat` 进入 `real` 模式（非 mock），状态栏显示已连接真实数据。【真实数据部分待用户本机实测】
      → 执行方已交付：`diagnose()` 四步诊断，**并修掉一个结构性阻塞** —— Tauri 2 非 async 命令跑在主线程，
        而 `get_db_key` 最长阻塞 180s，原实现会让窗口在「正在初始化」期间完全冻结，用户无法按提示重新登录微信，
        真实模式实际上抓不到 key。现已全部 `async` + `spawn_blocking`。
- [ ] AC-03: `get_sessions` 返回**真实**会话列表（非演示的 4 条），可搜索过滤。【待用户本机实测】
- [ ] AC-04: 选择某会话可基于**真实消息**生成词云，且可导出 PNG。【待用户本机实测】
- [x] AC-05: 新增「导出聊天记录为 JSON」入口；对所选会话导出 JSON 文件，字段至少含 时间戳、发送者标识、消息类型、文本内容；导出完成后能在磁盘找到该文件。【逻辑已自测通过；真实数据部分待用户本机实测】
      → 顶层与每条消息字段与「默认决策」逐字段一致（另加 `timeText` 可读本地时间）；
        落盘 `Documents\微信词云导出\<会话名>_<yyyyMMdd_HHmmss>.json` 并 `explorer /select` 定位；
        由「合成微信 4.x schema 端到端读取」「真实写盘后回读」「mock 全链路」三组单测覆盖。
- [x] AC-06: 真实读取不可用时给出清晰原因提示（权限不足/微信未登录/版本不兼容/未抓到 key），程序不崩溃，并保留 mock 兜底。
      → 分阶段错误文案 + `WxStatus.detail` + mock 兜底；`Mutex` 中毒改为返回可读错误而非 panic；
        前端异常分支已在真实浏览器实测不白屏。
- [x] AC-07: 提供简短运行/验证说明（如何以管理员运行、抓 key 时机、导出文件位置、常见失败排查）。
      → `README.md`（含四环节排查表与 `dump_schema` 校准指引）。

## 已确认决策
- 交付范围：**both** —— 既要把聊天记录下载/导出到本地，也要生成词云，两者都要真正跑通。
- 导出格式：**JSON**（结构化，含时间/发送者）。
- 运行与验收：用户以**管理员 + 微信 4.x** 实测，**真实端到端**为准；执行方仅保证编译/单测/逻辑正确 + mock 可用。
- 首要任务：**先构建运行并诊断**真实读取为何进不了 `real` 模式（用户尚未真正跑过）。

## 已知事实与证据
- 代码链路完整，关键文件：
  - 后端：`src-tauri/src/{lib,locate,config,ffi_key,dbcrypt,dbread,wordcloud,mock,model,error}.rs`
  - 前端：`src/{main,wordcloud,types}.ts`、`index.html`、`src/styles.css`
  - 配置：`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/capabilities/default.json`、`vite.config.ts`
- 2026-08-12 实测：`cargo test`（首次编译 ~2m56s）**编译通过**，3 个分词单测**全绿**。
- 2026-08-12 实测：`npm run build` **失败** —— `src/main.ts(75,31): error TS2339: Property 'wxid' does not exist on type 'WxStatus'`。
  根因：`src/types.ts` 的 `WxStatus` 声明了 `wechatVersion?`，而 Rust 后端（`model.rs`）返回的是 `wxid`，`main.ts` 读取 `st.wxid`。**这是当前项目无法整体构建的直接原因之一**，修复很小（对齐类型）。
- `wx_key.dll` 已内置于 `src-tauri/resources/`；`dist/`、`node_modules/` 均存在（曾构建/安装过）。
- `tauri.conf.json` 仅 `resources/wx_key.dll` 参与打包；解密自实现，不再需要 `wcdb_api.dll`/`WCDB.dll`/`SDL2.dll`。
- 真实读取从未在用户机器上端到端验证（见 `agent_hub/projects/weixin-wordcloud/experiment/exp_weixin-wordcloud-tauri.md`）。
- 工作区已从磁盘删除 WeFlow 原始源码（仅剩词云项目），但该删除（git 中约 451 处 `D`）**尚未提交**，WeFlow 源码仍在 git 历史中。
- 临时明文库写入 `%TEMP%\weixin_wordcloud_<pid>`（`dbcrypt.rs`/`lib.rs`），用完未清理（可作收尾项，非阻塞）。

## 待验证假设
- 自实现 `dbcrypt::detect()` 的参数搜索空间（HMAC-SHA512/256、reserve、PBKDF2 轮数、page-no 端序、raw-key 与 KDF 派生 key）能命中该用户微信 4.x 库 page1 的 HMAC 校验。
- 消息表命名 `Msg_<md5(talker)>` 及字段候选（`message_content`/`compress_content`、`local_type == 1` 为文本）在该微信版本命中；`compress_content` 的 zstd 解码假设成立。
- 会话表选择启发式（含 username-like 列 + 时间列）能在 `session.db` 命中真实会话表；`contact.db` 显示名可 join。
- 随附 `wx_key.dll` 的导出函数签名与 `ffi_key.rs` 声明一致（`InitializeHook(u32)->bool`、`PollKeyData(*mut c_char,c_int)->bool` 等）。

## 风险与成本
- 提 key 依赖微信版本与登录时机，可能需退出并重新登录微信；读取其他进程内存易被杀毒软件误报/拦截。
- schema 不命中会导致会话/消息为空——需用 `dump_schema` 导出真实表结构后校准 `dbread.rs` 的字段/表匹配常量。
- 真实读取/导出/词云的验收依赖用户在本机、在场实测；执行方无法独立完成 AC-02~05 的真实数据部分。
- 若最终决定改为「保存对话框/自选目录」，需引入 `tauri-plugin-dialog`/`tauri-plugin-fs` 并调整 capabilities（成本+权限面扩大）。

## 相关材料
- 代码与配置：见「已知事实与证据」中的文件清单。
- 依赖原生库：`src-tauri/resources/wx_key.dll`（源自 WeFlow）。
- 项目纪要：`agent_hub/projects/weixin-wordcloud/goal.md`、`.../plan/plan_weixin-wordcloud-tauri.html`、`.../experiment/exp_weixin-wordcloud-tauri.md`、`agent_hub/memory.md`、`agent_hub/tr.md`。
- WeFlow 开源项目（读取原理参考；其公开源码已因 DMCA 移除解密逻辑）。

## Grilling
- 用户选择：未使用
- 结论：无（用户选择直接落盘为 ready）
