# Windows构建、打包与验收

当前发布目标：0.4.0，Windows10/11 x64，CPU OCR，ZIP测试包。功能基线采用默认关闭方向识别、启用时明确同意且全部待输出图片人工确认的流程。

当前状态：本地实现和59项自动测试通过，PowerShell脚本解析、验包正例及4项拒绝用例、actionlint通过；Windows CI、ZIP产物及客户Windows10验收分别待验证。此页是唯一发布状态入口，更新结果时不把源码检查、CI配置文件或本机macOS运行当作Windows成品。

## 交付方式

第一阶段采用ZIP，包含主程序、中文启动脚本、OCR模型/字典、Windows原生运行库、使用说明、完整第三方许可和逐文件hash清单。用户解压整个目录后双击启动，不单独移动EXE。

程序数据默认在 `%LOCALAPPDATA%\VoucherWorkbench`；旧开发状态可通过 `--state-dir` 显式选取，不自动搬迁。程序控制台保持可见，重复启动同一状态目录会提示已有实例；关闭网页不会退出服务，`Ctrl+C`等待当前页/提交收尾。脚本会保留启动错误，避免窗口闪退。

OCR功能仍默认关闭；缺少或损坏的模型/原生库不阻止原向合并。随包OCR使用CPU，不要求CUDA或独立显卡。ONNX Runtime官方Windows包要求VC++运行库；运行库缺失时按包内说明从微软官方安装，不能因CI机器已经装有开发环境就宣称任何客户电脑都零依赖。第一版不静默安装系统组件。[ONNX Runtime官方要求](https://onnxruntime.ai/docs/install/#requirements)

测试包尚无代码签名，Windows可能显示未知发布者提示。签名不作为本次内部测试前提，正式分发如需签名另行安排证书；不要求关闭系统安全功能。

## 自动化与权限

GitHub Actions是本项目推荐的Windows构建环境；也可在已装相应开发工具的Windows机上执行同一脚本。CI使用固定Windows runner、Rust版本、Cargo.lock和校验过的固定依赖，避免用macOS交叉检查冒充Windows实测。

- push/PR执行检查与测试；手动 `workflow_dispatch` 构建ZIP并实跑随包程序及合成资料流程。
- 工作流token仅 `contents: read`，无需用户额外提供PAT或密码。上传Actions构件不要求仓库内容写权限；不会自动创建公开Release。
- 只使用代码生成的合成资料。私有输入、输出、历史、日志、预览不提交；失败时也不上传带会话凭据的服务原始日志。
- 第三方action按完整提交固定。发布材料记录官方来源、版本、SHA256以及完整许可。

[GitHub工作流权限说明](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#permissions)；[GITHUB_TOKEN机制](https://docs.github.com/en/actions/concepts/security/github_token)。GitHub托管Windows环境不能替代客户Windows10的实际使用验证。

## Windows本机开发构建

开发机需Rust、MSVC构建工具、PowerShell7和用于JS语法检查的Node；这些不是客户运行包的安装要求。版本锁定于 `app/distribution/windows-dependencies.json`。在项目根目录执行：

```powershell
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
pwsh -File app/tools/build-windows.ps1
pwsh -File app/tools/smoke-windows.ps1 -ZipPath dist/voucher-workbench-0.4.0-windows-x64.zip -FixtureExecutable app/target/windows-dist/x86_64-pc-windows-msvc/release/examples/package_fixture.exe
```

完整构建包括格式、Clippy、全部测试、release、固定依赖下载/hash检查、完整许可与白名单组包；不覆盖已有ZIP。可用 `-OutputDirectory`、`-CacheDirectory` 指定路径，`-CheckOnly`仅执行检查。冒烟从中文含空格路径及不同工作目录运行真正随包程序，完成实载OCR、合成账表匹配、原向生成、同意和人工门禁、修正及重启复用。

ONNX Runtime固定1.23.2、两份OCR权重和字典均按SHA256核验；程序通过环境API显式关闭原生库telemetry。算法未调整，版本仍为title-ocr-v1.0.2。

## 源码管理边界

Git采用默认排除规则，只纳入 `tools/public-source.json` 列明的当前Rust工具、前端、合成测试、发行脚本及有效规格。旧版规格和旧Python文件保留在本地，不进入Git；真实数据与 `.local`、`.voucher-state` 同样排除。

```sh
python3 tools/prepare-source.py --list
python3 tools/prepare-source.py --export /path/to/new/source-snapshot
python3 tools/prepare-source.py --verify-staged
```

导出目标必须为不存在的新目录，不覆盖原文件；导出附SHA256清单。暂存核验要求文件集合及内容与白名单完全一致，拒绝额外文件、符号链接、数据文件和未经审阅的暂存差异。

## 客户验收

需要一台客户Windows10 x64电脑和少量可供本人核对的资料。在普通用户权限、中文含空格路径中解压，检查启动/浏览器、明细账匹配、关闭识别生成、开启识别同意、人工旋转确认、输出打开和退出重启。缺少VC++运行库时按提示处理，再验证识别可用。客户数据无需上传GitHub，也不需要远程管理员权限。

通过后再决定是否制作安装器、签名和公开Release；这些状态与CI构件是否生成分开记录。
