# Windows构建、打包与验收

当前发布目标：0.5.0，Windows10/11 x64，CPU OCR，ZIP测试包。增加可手动选择的1–10线程并行处理；默认1，方向识别按图片并行，PDF生成按凭证并行。

当前状态（2026-11-02）：0.5.0并行变更已实现，本地79项测试、格式、Clippy及JS语法检查通过，另1项私有OCR测试按设计忽略；界面与真实OCR检查、Windows CI、新版ZIP及随包验收待完成。0.4.1的本机及客户包历史结果不能代表0.5.0已通过，客户Windows10实机验收仍待反馈。此页是唯一发布状态入口，CI与本机macOS试用不能替代客户Windows10实测。

并行设置在工作台手动选择1–10，保存在程序数据目录，活动任务期间不可更改；任务保存当时线程数，重试沿用原值。识别每个工作线程独立模型，单模型内部推理线程为1，生成保持单份PDF内部页序和独立事务。更多线程占用更多CPU和内存，不承诺按线程数成倍加速。

## 0.4.1测试包（历史基线）

- 公开源码：[thising/voucher-workbench](https://github.com/thising/voucher-workbench)，仅包含当前工具与有效规格。
- 完整构建与验证：[Windows CI #6](https://github.com/thising/voucher-workbench/actions/runs/41160637785)。构建源码为 [`1188e17`](https://github.com/thising/voucher-workbench/commit/1188e17604fbf6d82125fbb3a4c6ba90cd389591)；后续文档更新不改变该包内容。
- 下载：[Windows x64测试构件](https://github.com/thising/voucher-workbench/actions/runs/41160637785/artifacts/13026908712)，需登录GitHub，构件保留14天。下载得到的Actions归档内包含程序ZIP及SHA256文件；尚未创建公开Release。
- 程序包：`voucher-workbench-0.4.1-windows-x64.zip`，28,767,405字节（约27.4MiB）。SHA256如下，校验对象是程序包而非外层Actions归档：

```text
f37d3789c93b50b518cf296707a15e5e280f8f06d05ffacb1b07bda9d0290ff0
```

Windows CI通过70项测试，另1项依赖本地私有资料的OCR测试按设计忽略。格式、Clippy及JS语法检查通过。实际解压包从中文含空格路径及不同工作目录启动，成功加载随包OCR，验证原向合并、未同意/未人工确认时禁止生成、人工旋转修正、生成4页PDF、退出重启及结果复用。新增验证覆盖旧旁置记录及.previous残留迁移、冲突/篡改保护、同名输出隔离；随包冒烟确认输出及新历史目录只有PDF，报告快照在重生成和重启后保持正确。

下载后已独立核对ZIP校验值、429个有效载荷文件的大小与hash、模型固定hash、三个PE文件的x64架构及两份DLL与官方原包的一致性；没有清单外文件。主程序静态链接Rust CRT；OCR DLL仍依赖微软VC++运行库。macOS通过69项测试和PowerShell语法检查，并在两个不同文件系统上实测生成、OCR、归档、旧记录迁移、报告及重启复用。验包正负例及actionlint沿用未改动构建流程的0.4.0基线结果。

## 交付方式

第一阶段采用ZIP，包含主程序、中文启动脚本、OCR模型/字典、Windows原生运行库、使用说明、完整第三方许可和逐文件hash清单。用户解压整个目录后双击启动，不单独移动EXE。

此前整理的0.4.1客户资料包 `凭证工作台-0.4.1-Windows客户试用包.zip`（28,458,472字节）仅作历史，不包含本次多线程功能。其最外层附 `先读我-简明使用说明.txt`，解压一次后进入程序子文件夹启动；同名 `.zip.sha256` 用于核对资料包。程序目录的430个文件（429个有效载荷文件及package-manifest.json）与上述CI程序包逐字节一致，仅增加客户说明，不包含业务资料、处理记录或日志。该历史客户资料包SHA256为：

```text
3dbbbfbe38cfb3aae41f7519a052926d3e7f109550b388d4ba2f65becadae7a3
```

这是附说明后的外层资料包校验值，区别于上方原始CI程序ZIP校验值；未重新编译程序，未创建公开Release。

程序数据默认在 `%LOCALAPPDATA%\VoucherWorkbench`；旧开发状态可通过 `--state-dir` 显式选取，不自动搬迁。0.4.1的输出manifest、事务及新历史manifest存入数据目录的 `outputs` 子目录，PDF旁不再新建记录JSON；旧PDF仍归档至输出目录 `.history`。旧旁置manifest仅在对应PDF再次生成/复用时，经验证复制到数据目录后移除；既有历史归档不批量迁移。备份或换电脑时应保留整个数据目录，缺少可验证记录时不得自动复用或覆盖已有PDF。程序控制台保持可见，重复启动同一状态目录会提示已有实例；关闭网页不会退出服务，`Ctrl+C`等待当前页/提交收尾。脚本会保留启动错误，避免窗口闪退。

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
pwsh -File app/tools/smoke-windows.ps1 -ZipPath dist/voucher-workbench-0.5.0-windows-x64.zip -FixtureExecutable app/target/windows-dist/x86_64-pc-windows-msvc/release/examples/package_fixture.exe
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

需要一台客户Windows10 x64电脑和少量可供本人核对的资料。在普通用户权限、中文含空格路径中解压，检查启动/浏览器、明细账匹配、关闭识别生成、开启识别同意、人工旋转确认、输出打开和退出重启；分别试用1与10线程，确认设置保存、任务运行时禁改、页序和结果正确、取消/重试能继续且保留原任务线程数。确认新PDF旁没有manifest JSON、报告仍能下载，重试能复用已有结果。使用旧版测试资料时另核对旁置记录迁移成功，且PDF内容不变。缺少VC++运行库时按提示处理，再验证识别可用。客户数据无需上传GitHub，也不需要远程管理员权限。

通过后再决定是否制作安装器、签名和公开Release；这些状态与CI构件是否生成分开记录。
