# PDF工作台

PDF工作台是 **Tauri 独立窗口＋Rust 核心**的本地工具。当前主功能是“批量合并”：按目录层级把图片和 PDF 整理成多份 PDF，可选择索引命名、排除、排序、逐页旋转及图片方向辅助识别。资料、模型推理和处理记录保留在本机；运行不使用 CDN，也不需要安装 Office、Python、Node 或 Docker。

当前有效依据是[批量合并规格](openspec/specs/batch-merge/spec.md)及[桌面与 Windows 发行规格](openspec/specs/windows-distribution/spec.md)。[凭证规格](openspec/specs/voucher-pdf/spec.md)、[旧通用 PDF 规格](openspec/specs/generic-pdf/spec.md)仅约束 0.5.x 历史兼容入口。发行目标是 **0.6.0 Windows 10/11 x64 客户试用包**；实际构建、验证和下载状态统一记录在[发行验收记录](docs/windows-release.md)。

## 使用流程

1. 选择“待处理文件夹”，读取真实文件夹层级。默认当前文件夹，即递归合并为一份；选择更深层级时按该层文件夹分组，旁边显示目录示例。未达到所选层级的直属文件另成组，避免遗漏。
2. 可选 XLS/XLSX 重命名索引。指定工作表、表头行、文件夹索引列和文件名列；只使用所选表。默认把匹配列值作为完整 PDF 名称。无匹配、空名称或同一索引对应不同名称时，提醒并回退文件夹名。可显式采用“文件夹名-索引名”或“取首条”兼容规则。
3. 编辑排除规则，一行一条，以 `#` 开头的整行为注释。支持相对路径和受限 glob；无效行忽略并显示原因。默认排除 `indexImg`、`index.html` 等辅助内容，输出和处理记录子树自动排除。
4. 扫描后在三栏中核对：左栏选择合并单元及输出名称，中栏查看文件并按文件名、修改时间、前缀或后缀排序，也可拖动、上下移动或置顶；右栏预览并旋转。PDF 支持当前页或整份文件的额外旋转，原文件不改动。
5. 按需“自动调整图片方向”。阅读使用须知和免责声明，明确同意后开始，线程默认 4、可选 1–10。此功能仅处理图片，可能误转、漏转，耗时随设备和资料变化。自动处理结果以颜色及文字提醒人工核对；识别失败或尚未核对不阻止合并，来源损坏等实际错误仍需处理。
6. 点击核心操作“执行合并”，选择输出位置，默认待处理目录同级的“目录名-PDF”。确认范围、名称和未核对提示后生成。每组一份 PDF，组内按当前文件顺序，源 PDF 内部页序保持；不加页眉、页脚或分隔页。
7. 在“处理记录”查看进度、取消、重试、保存成品和报告。重试沿用原计划与线程快照。未知或被外部修改的同名文件拒绝覆盖；更新工具拥有的旧结果前保留历史。

支持 JPG、PNG、BMP、静态 WebP、单页 TIFF 及普通静态 PDF。图片先应用 EXIF，再按所选角度生成 A4 横/纵页面，等比居中、不裁切；源 PDF 保留页尺寸与原有旋转，再叠加人工旋转。不跟随符号链接、目录联接、云占位文件或网络盘；加密、表单、签名、脚本、附件等不支持的 PDF 结构会明确提示。完整边界见有效规格。

## Windows 启动与记录

新版直接打开独立窗口，双击 `PDF工作台.exe` 或安装后的“PDF工作台”快捷方式；重复启动唤回原窗口，任务继续。WebView2 已安装时直接使用，缺少时按提示安装；联网按需安装器和独立离线附件的具体版本、体积及校验值见发行记录。WebView2 是桌面窗口运行组件，不会打开外部浏览器。

默认处理记录仍是 `%LOCALAPPDATA%\VoucherWorkbench`，保持旧计划、复核结果和旧线程设置兼容。升级前先正常退出旧服务；新版不会强杀占锁进程、删除锁文件或另建目录绕过冲突。关闭桌面窗口时，若有任务或保存操作，程序会让用户选择继续处理，或取消并等待进度安全保存后退出。

新批量合并的识别/合并线程偏好分别保存在 `batch-preferences.json`，缺省均为 4；旧 `settings.json` 仍按旧规则缺省 1，不改写。一个记录目录同时只运行一个任务。线程越多，CPU/内存占用通常越高，不保证按线程数等比例提速。

PDF 旁不新建 `manifest.json`。来源指纹、页码、方向及输出校验记录保存在处理记录目录的 `outputs/<输出路径摘要>/`；任务报告、计划另存于该目录。交付和阅读 PDF 不需要这些记录，但升级、备份或换电脑请保留完整记录目录，否则无法自动证明旧成品归属、复用或覆盖。

## 开发与验证

Rust 版本及依赖固定在[Windows依赖清单](app/distribution/windows-dependencies.json)、`app/Cargo.lock` 和 `app/desktop/Cargo.lock`。桌面开发另需对应平台的 Tauri 原生构建依赖；Node 仅用于固定版本的构建 CLI，不随客户包发布。

```sh
# 核心检查
sh app/tools/cargo-local.sh test --locked
sh app/tools/cargo-local.sh clippy --locked --all-targets -- -D warnings

# 本机独立窗口（另需标题模型/ONNX Runtime，缺少时仍可手工合并）
sh app/tools/cargo-local.sh run --locked --manifest-path desktop/Cargo.toml -- --state-dir /absolute/path/to/test-state --model /absolute/path/to/title-ocr --runtime /absolute/path/to/libonnxruntime.dylib

# 浏览器仅作为开发诊断入口
sh app/tools/dev.sh
```

工作区脚本优先使用 `.local` 隔离工具链，没有时使用本机 Cargo。模型下载和固定校验见[模型说明](app/models/README.md)。本地 HTTP 仅监听 `127.0.0.1`，API 使用会话认证与来源校验；桌面原生文件选择/保存仅授予本窗口的实际来源。认证链接不写入普通发现文件。

Windows 构建与验收：

```powershell
pwsh -File app/tools/build-windows-desktop.ps1
```

GitHub CI 验证 Rust 核心、桌面壳、便携包和安装包，并实际启动 Tauri 窗口检查交互；测试使用合成资料，不发布业务数据。CI 不能替代客户 Windows 10 的真实双击、WebView2 安装与单位安全策略验收，具体证据和缺口见发行记录。

代码采用 [MIT](LICENSE)，模型、运行库及离线 PDF.js 分别遵循上游许可，见[第三方说明](app/THIRD_PARTY_NOTICES.md)。公开范围由[白名单](tools/public-source.json)和[核验工具](tools/prepare-source.py)约束：仅工具、合成测试和有效规格；业务资料、历史实现/规格、模型权重、本地工具链、状态、预览和日志不入库。[本地 API 契约](app/API.md)供开发与兼容测试使用。
