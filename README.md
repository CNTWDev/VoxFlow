# Vox Flow

跨平台语音输入工具（macOS / Windows）。按下快捷键，说话，松开——转录文字自动出现在光标位置。

灵感来自 [Wispr Flow](https://wisprflow.ai)，用 Rust + Tauri 从零构建。

---

## 功能

- 全局快捷键触发录音（macOS：`Option+Space` 或 `Fn` 长按；Windows：`Ctrl+Alt+Space`）
- 两种触发模式：**按住录音**（HoldKey）/ **按一下开，再按关**（ToggleKey）
- ASR 三路：**OpenAI** `gpt-4o-transcribe` / **VoxNexus** REST + WebSocket / **本地** Whisper.cpp（离线）
- Language Profile 系统：每个 Profile 独立绑定语言、模型、prompt，热键切换
- 转录完成后通过剪贴板模拟粘贴，文字注入当前焦点窗口
- 透明状态 Overlay：始终置顶、点击穿透，展示录音 / 处理 / 注入 / 完成四态动画
- 权限检测（Input Monitoring / Accessibility），应用启动时自动检查并可一键跳转系统设置
- 配置持久化到系统标准目录（TOML）

---

## 快速开始

### 环境依赖

| 工具 | 说明 |
|---|---|
| [Rust stable](https://rustup.rs) | `rustup update stable` |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |
| Xcode CLT（macOS） | `xcode-select --install` |
| MSVC Build Tools（Windows） | 本地 ASR 编译必须 |

> macOS 无需 Node.js，前端为纯 HTML + WebKit。

### 开发模式

```bash
git clone <repo>
cd vox-flow

cargo tauri dev --config src-tauri/tauri.dev.conf.json
```

窗口自动弹出。Rust 日志在终端，前端 console 在 WebKit Inspector（右键 → 检查元素）。

> 开发模式使用独立的 bundle id（`com.voxflow.dev`），避免 macOS 麦克风 / 辅助功能 / 输入监控权限绑定到 dev 路径后影响安装包。

### 生产打包

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./scripts/build-macos-release.sh
```

产物在 `src-tauri/target/release/bundle/`：
- macOS：`Vox Flow.app` + `.dmg`
- Windows：`.msi` + NSIS 安装包

若你还没有签名证书，先检查：

```bash
security find-identity -v -p codesigning
```

看到 `0 valid identities found` 表示还不能做稳定签名发布，需要先在钥匙串安装可用的 `Developer ID Application` 证书。

### 固定签名（避免每次升级重授权）

为避免 macOS 在每次升级后重复要求麦克风 / 辅助功能 / 输入监控授权，请保持以下三点稳定：

- 固定 `bundle identifier`（当前为 `com.voxflow.desktop`）
- 固定签名证书（每次发布都使用同一个 `APPLE_SIGNING_IDENTITY`）
- 固定安装路径（始终覆盖 `/Applications/Vox Flow.app`）

> `tauri dev` 使用独立 id（`com.voxflow.dev`），权限与生产包天然隔离，属于预期行为。

---

## 配置

配置文件路径（自动创建）：
- macOS：`~/Library/Application Support/VoxFlow/config.toml`
- Windows：`%APPDATA%\VoxFlow\config\config.toml`

示例配置：

```toml
active_profile_id = "auto"

[profiles.auto]
display_name = "Auto Detect"
[profiles.auto.backend]
type = "OpenAi"
model = "gpt-4o-transcribe"

[profiles.zh]
display_name = "中文"
language_hint = "zh"
prompt = "输出简体中文，保留中英文混排。"
[profiles.zh.backend]
type = "OpenAi"
model = "gpt-4o-mini-transcribe"

[profiles.vn]
display_name = "VoxNexus 超清"
prompt = "将我的口语整理成简洁、自然、专业的简体中文，保留必要的中英文混排。"
[profiles.vn.backend]
type = "VoxNexus"
model_id = "vn-stt-ultra"
transport = "web_socket_streaming" # 或 "rest_batch"
enable_timestamps = false
enable_speaker_diarization = false
enable_llm_transform = true
llm_model_id = ""                  # 留空使用服务端默认 LLM
llm_max_tokens = 1024

[profiles.en-local]
display_name = "English (Offline)"
language_hint = "en"
[profiles.en-local.backend]
type = "Local"
model_path = "/path/to/ggml-base.en.bin"

[app]
activation_mode = "ToggleKey"   # 或 "HoldKey"
hotkey = "Option+Space"         # macOS 也可设为 "Fn"（需 Input Monitoring 权限）
global_api_key = "sk-..."       # OpenAI API key（Phase 9 前明文存储）
```

> **注意**：`ProfileBackendConfig` 的 `type` 字段已从旧版 `"Cloud"` 改为 `"OpenAi"`，升级时需手动更新配置文件。

### VoxNexus WebSocket

VoxNexus profile 可选择两种请求方式：

| transport | 说明 |
|---|---|
| `rest_batch` | 停止录音后上传完整 WAV，返回最终文本 |
| `web_socket_streaming` | 连接 `wss://api.voxnexus.ai/v1/stt/realtime?token=...`，发送 PCM16 audio chunks，接收 `ready` / `transcript` / `llm` / `flush_done` / `error` |

REST 请求使用 `X-Api-Key` header 鉴权；WebSocket 继续使用 `?token=...`，服务端也支持 `X-Api-Key` header。VoxNexus LLM 润色由 `enable_llm_transform` 控制，开启后会把 profile 的 `prompt` 作为 `llm_prompt` 发送。

当前版本已经在 `vf-asr` 层实现 VoxNexus `StreamingTranscriber`，包括 transcript/LLM 事件解析。应用主流程仍是“松开热键后转写并注入”；真正边录边显示 partial 的 UI 流式体验会在录音管线改为边采集边推送后接上。

---

## 项目结构

```
vox-flow/
├── crates/
│   ├── vf-agent-os/   # 轻量 Agent OS：prompt 编译 + memory/skill/context/session 文件协议
│   ├── vf-audio/      # 麦克风采集 + 重采样到 16kHz mono f32
│   ├── vf-asr/        # AsrBackend trait，OpenAI + VoxNexus + Local（Whisper.cpp）
│   ├── vf-inject/     # 剪贴板注入 + enigo 模拟粘贴
│   ├── vf-config/     # AppConfig + LanguageProfile + TOML 持久化
│   └── vf-core/       # VoxEngine 状态机，编排录音→ASR→注入全流程
└── src-tauri/         # Tauri shell，前端 UI，overlay，全局快捷键，权限检测
```

所有 `vf-*` crate 不依赖 Tauri，可单独 `cargo test`。

### Agent OS

仓库内包含一个轻量 `.agent-os/` 规范和 `vf-agent-os` crate，用来沉淀 Hermes Agent 风格的上下文组织方式：

- `SOUL.md` / `AGENTS.md` / `MEMORY.md` / `USER.md` 组成稳定上下文层。
- `skills/<name>/SKILL.md` 只在 system prompt 中暴露索引，完整内容按需加载。
- `PromptCompiler` 输出 `cached_system` 与 `ephemeral_context`，便于后续接入 CLI、Web 或其他 agent runtime。
- 首版只使用 Markdown 文件和 JSONL session 记录，不引入数据库或向量检索。

---

## 开发

```bash
# 编译检查
cargo build --workspace

# 运行测试（CI 可跑，不需要麦克风）
cargo test --workspace

# 需要麦克风的集成测试（本地手动）
cargo test --workspace -- --ignored
```

日志级别通过环境变量控制：

```bash
RUST_LOG=debug cargo tauri dev
```

---

## 实现进度

| Phase | 状态 | 内容 |
|---|---|---|
| 0 | ✅ | Workspace + Tauri + config 读写 |
| 1 | ✅ | 手动录音 → 16kHz mono f32 → debug wav |
| 2 | ✅ | Cloud ASR（OpenAI Transcription API） |
| 3 | ✅ | 剪贴板注入，文字进光标位置 |
| 4 | ✅ | 状态机完善 + 错误恢复 + mock 测试 |
| 5 | ✅ | 全局快捷键（HoldKey + ToggleKey）+ macOS Fn 专项监听 + 状态 Overlay + 权限检测 + VoxNexus REST/WebSocket ASR |
| 6 | ⬜ | VAD 自动停止 |
| 7 | ⬜ | Local Whisper.cpp + Profile 切换 UI |
| 8 | ⬜ | 设置 UI + 系统托盘 |
| 9 | ⬜ | API Key → Keychain + 签名 + 公证 + CI |

---

## 平台注意事项

### macOS
- 首次使用需授权**麦克风**（系统设置 → 隐私 → 麦克风）
- 生产包 bundle id 为 `com.voxflow.desktop`；开发调试使用 `com.voxflow.dev`（见 dev 配置）
- 文字注入需授权 **Accessibility**（系统设置 → 隐私 → 辅助功能）
- 使用 `Fn` 长按热键还需授权 **Input Monitoring**（系统设置 → 隐私 → 输入监控）
- 标准快捷键使用 `Option+Space`；若设为 `"Fn"`，则通过 `CGEventTap` 监听，与其他快捷键框架互不干扰

### Windows
- 麦克风权限需在系统设置手动开启（无法通过代码请求）
- 本地 ASR 编译需要 MSVC toolchain（`x86_64-pc-windows-msvc`）
- 快捷键默认 `Ctrl+Alt+Space`（避开 CJK 输入法占用的 `Alt+Space`）
- 管理员权限窗口和部分反作弊游戏可能阻止 enigo 模拟按键（已知限制）

---

## License

MIT
