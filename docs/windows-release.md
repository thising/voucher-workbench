# Windows构建、打包与验收

当前发布目标：0.5.0，Windows10/11 x64，CPU OCR，ZIP测试包。增加可手动选择的1–10线程并行处理；默认1，方向识别按图片并行，PDF生成按凭证并行。

当前状态（2026-11-02）：0.5.0实现、本地79项测试、浏览器与真实OCR检查、Windows80项测试、完整打包及随包验证通过；另1项私有OCR测试按设计忽略。新版ZIP已下载并独立核验，本机测试服务已备份原记录后升级并打开。客户Windows10实际试用待反馈；CI与本机macOS试用不能替代客户PC验收。

并行设置在工作台手动选择1–10，保存在程序数据目录，活动任务期间不可更改；任务保存当时线程数，重试沿用原值。识别每个工作线程独立模型，单模型内部推理线程为1，生成保持单份PDF内部页序和独立事务。更多线程占用更多CPU和内存，不承诺按线程数成倍加速。

## 已验证的0.5.0测试包

- 公开源码：[thising/voucher-workbench](https://github.com/thising/voucher-workbench)，仅包含当前工具与有效规格。
- 完整构建与验证：[Windows CI #8](https://github.com/thising/voucher-workbench/actions/runs/41375857170)。构建源码为 [`196f3ca`](https://github.com/thising/voucher-workbench/commit/196f3cacf7d84938f61218657c6a0447727c3b5c)；后续文档更新不改变包内容。
- 下载：[Windows x64测试构件](https://github.com/thising/voucher-workbench/actions/runs/41375857170/artifacts/13112005182)，需登录GitHub，构件保留14天。Actions归档内含程序ZIP及SHA256文件；尚未创建公开Release。
- 程序包：`voucher-workbench-0.5.0-windows-x64.zip`，28,822,715字节（约27.5MiB）。下列SHA256校验程序ZIP，区别于外层Actions归档：

```text
922a9627a314975f05ab89c64b5605b39045c915820568af4d9ce2f1171c6767
```

本地79项、Windows80项测试通过，另1项私有OCR测试忽略；格式、Clippy及JS语法检查通过。合成测试覆盖1–10真实工作项重叠与上限、设置及非法值、任务/重试快照、旧任务默认值、多凭证页序和结果顺序、进度、单项失败继续、取消/重启、输出记录及报告。并发协调器在工作线程异常或保存失败时继续收集在途结果，保留已提交PDF。

浏览器实测设置10后刷新保留、来源折叠后设置仍可见、活动任务禁止修改、两份PDF生成、真实OCR处理4张图片及人工门禁；模拟一份来源异常后恢复，界面将全局设为1，旧10线程任务的确认与重试仍按10执行。取消由受控工作线程与任务集成测试验证；短小浏览器样例在点击取消前已经完成，不计作UI取消通过。

Windows随包验证从中文含空格路径及不同工作目录启动实际EXE，加载随包OCR，处理2个凭证（4张图片＋4页原PDF，共8页）；验证10线程识别/生成、同意与人工确认门禁、修正、结果/页序、报告快照、设置恢复、新任务1线程及旧任务重试10线程。下载后独立核对ZIP校验值、429个载荷文件、模型、三个x64 PE文件和两份DLL与官方原包的一致性；没有清单外文件。主程序静态链接Rust CRT，OCR DLL仍依赖微软VC++运行库。

## 交付方式

采用ZIP，包含主程序、中文启动脚本、CPU OCR组件、使用说明、完整第三方许可和逐文件校验清单。客户资料包 `凭证工作台-0.5.0-Windows客户试用包.zip`（28,512,678字节）在最外层另附 `先读我-简明使用说明.txt`，含1–10线程操作说明；解压一次后进入程序子目录双击启动。其430个程序文件（429个载荷及package-manifest.json）与上述CI包逐字节一致，不含业务资料、处理记录、截图或日志。客户资料包SHA256为：

```text
79c98cdef75107fdd67a6ac217b5b61c2a41ef947bf72ef0acea7a2e667a9966
```

这是附说明的外层资料包校验值。旧0.4.1客户资料在本地保留为历史，不包含并行功能。

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
