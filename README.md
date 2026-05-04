# Vox Flow

跨平台语音输入工具（macOS / Windows）。按下快捷键，说话，松开——转录文字自动出现在光标位置。

灵感来自 [Wispr Flow](https://wisprflow.ai)，用 Rust + Tauri 从零构建。

---

## 功能

- 全局快捷键触发录音（`Option+Space` on macOS，`Ctrl+Alt+Space` on Windows）
- 两种触发模式：**按住录音**（HoldKey）/ **按一下开，再按关**（ToggleKey）
- ASR 双路：**云端** OpenAI `gpt-4o-transcribe` / **本地** Whisper.cpp（离线）
- Language Profile 系统：每个 Profile 独立绑定语言、模型、prompt，热键切换
- 转录完成后通过剪贴板模拟粘贴，文字注入当前焦点窗口
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

cargo tauri dev
```

窗口自动弹出。Rust 日志在终端，前端 console 在 WebKit Inspector（右键 → 检查元素）。

### 生产打包

```bash
cargo tauri build
```

产物在 `src-tauri/target/release/bundle/`：
- macOS：`Vox Flow.app` + `.dmg`
- Windows：`.msi` + NSIS 安装包

> 当前版本（Phase 4）未签名。macOS 首次打开若被 Gatekeeper 拦截，在**系统设置 → 隐私与安全性**手动允许，或执行：
> ```bash
> xattr -cr "Vox Flow.app"
> ```

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
type = "Cloud"
model = "gpt-4o-transcribe"

[profiles.zh]
display_name = "中文"
language_hint = "zh"
prompt = "输出简体中文，保留中英文混排。"
[profiles.zh.backend]
type = "Cloud"
model = "gpt-4o-mini-transcribe"

[profiles.en-local]
display_name = "English (Offline)"
language_hint = "en"
[profiles.en-local.backend]
type = "Local"
model_path = "/path/to/ggml-base.en.bin"

[app]
activation_mode = "ToggleKey"   # 或 "HoldKey"
hotkey = "Option+Space"
global_api_key = "sk-..."       # OpenAI API key（Phase 9 前明文存储）
```

---

## 项目结构

```
vox-flow/
├── crates/
│   ├── vf-audio/      # 麦克风采集 + 重采样到 16kHz mono f32
│   ├── vf-asr/        # AsrBackend trait，Cloud（OpenAI）+ Local（Whisper.cpp）
│   ├── vf-inject/     # 剪贴板注入 + enigo 模拟粘贴
│   ├── vf-config/     # AppConfig + LanguageProfile + TOML 持久化
│   └── vf-core/       # VoxEngine 状态机，编排录音→ASR→注入全流程
└── src-tauri/         # Tauri shell，前端 UI，Tauri commands
```

所有 `vf-*` crate 不依赖 Tauri，可单独 `cargo test`。

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
| 5 | ⬜ | 全局快捷键（HoldKey + ToggleKey） |
| 6 | ⬜ | VAD 自动停止 |
| 7 | ⬜ | Local Whisper.cpp + Profile 切换 |
| 8 | ⬜ | 设置 UI + 系统托盘 + 权限提示 |
| 9 | ⬜ | API Key → Keychain + 签名 + 公证 + CI |

---

## 平台注意事项

### macOS
- 首次使用需授权**麦克风**（系统设置 → 隐私 → 麦克风）
- 文字注入需授权 **Accessibility**（系统设置 → 隐私 → 辅助功能）
- 快捷键统一使用 `Option+Space`（而非 `Alt+Space`）

### Windows
- 麦克风权限需在系统设置手动开启（无法通过代码请求）
- 本地 ASR 编译需要 MSVC toolchain（`x86_64-pc-windows-msvc`）
- 快捷键默认 `Ctrl+Alt+Space`（避开 CJK 输入法占用的 `Alt+Space`）
- 管理员权限窗口和部分反作弊游戏可能阻止 enigo 模拟按键（已知限制）

---

## License

MIT
