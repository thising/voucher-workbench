# Windows构建、打包与验收

当前唯一发布目标是 **0.6.0：PDF工作台，Tauri 独立窗口＋Rust 核心，Windows10/11 x64**。有效依据为[批量合并规格](../openspec/specs/batch-merge/spec.md)及[桌面发行规格](../openspec/specs/windows-distribution/spec.md)。以下0.5.1记录为已交付历史，不作为新版行为约束。

## 0.6.0 当前状态

实现已进入实际桌面与发布验收：统一三栏批量合并、可选索引、图片自动方向、PDF逐页手动旋转，以及不阻塞生成的人工核对；新模式默认4线程，旧状态与线程设置单独保留。本机已完成核心与Tauri联编、新增合成回归；Windows构建、真实窗口和安装包尚待本轮CI验证，不能视为已交付。

分发采用便携ZIP与当前用户NSIS安装包。WebView2按需检测/安装，微软x64离线安装器独立附带，本次实际附件215,017,168字节（约205MiB），不嵌入每份主程序更新。PDF.js离线预览资源内嵌并锁定版本/哈希，运行不从CDN加载。

当前构建入口为 `app/tools/build-windows-desktop.ps1`；CI需实际运行随包GUI、原生文件选择/保存与安全退出，仅headless API通过不代表窗口通过。最终CI链接、成品校验值及客户Win10待验项将在验收后更新。

## 0.5.1 历史交付记录

历史发布目标：**0.5.1，Windows10/11 x64，CPU OCR，ZIP客户试用包**。有效规则见[Windows发行规格](../openspec/specs/windows-distribution/spec.md)、[凭证规则](../openspec/specs/voucher-pdf/spec.md)及[通用PDF规则](../openspec/specs/generic-pdf/spec.md)。本文记录构建与验收证据，不替代这些规则。

当前状态（2026-11-02）：通用PDF模式与启动优化已实现，本地104项合成测试通过（另1项私有OCR测试按设计忽略），格式、Clippy、JS语法及Windows GNU全目标Clippy通过。Windows106项测试及完整打包、两种随包业务流程和真实CMD入口矩阵通过；构件已下载并独立验包，客户资料包与校验值已生成。客户当前Windows10实际双击与浏览器复测仍待反馈。截图证明旧入口存在解析异常，具体触发条件尚未在客户环境复现，不能断言文件编码损坏。

## 0.5.1变更

通用PDF模式不依赖Excel或OCR。可选择合并层级0–32、输出目录、明确排除的文件/目录和受限glob规则；预览每组的文件、页数、输出名称。支持自然顺序、修改时间顺序、手动拖动或上下移动。保存顺序/名称会创建新计划，旧任务仍引用原计划。默认输出为资料目录同级的“资料名-合并PDF”；嵌套输出自动排除，未知或被修改的同名文件拒绝覆盖。

凭证模式的明细账首表/指定表、原向合并、可选标题识别及人工复核规则保持兼容。两种模式共用1–10线程设置、全局单活动任务、取消/重试和历史；旧任务缺少线程字段时仍按1，重试沿用原值。通用PDF的记录明确标记模式，不能冒用凭证输出记录。

随包`启动凭证工作台.cmd`只含ASCII、使用CRLF，定位同目录EXE、转发参数，非零退出时pause；不改代码页，中文提示由Rust输出。首次启动服务并开网页；重复启动通知原服务重新打开页面，第二进程正常退出。原服务启动中最多等待15秒，任务执行中再次启动不重置任务。

Windows激活使用当前用户SID专属DACL的本地命名管道，拒绝远程客户端；认证URL只由原服务从内存交给浏览器，IPC不返回URL/token，也不保存普通会话发现文件。macOS开发使用私有用户目录与socket，校验对端用户。IPC关闭完成后才释放目录锁，锁文件不会自动删除。

## 启动故障处理

| 情况 | 行为与处理 |
| --- | --- |
| 新版服务已运行 | 由原服务再开认证页面；原任务继续，新进程返回成功 |
| 原服务仍在启动 | 中文提示并短暂等待，15秒仍不可联系则报错 |
| 旧版或无法联系的占锁实例 | 原窗口等待任务完成后Ctrl+C退出，再启动新版；保留记录和锁文件 |
| 记录目录无写入权限 | 明示目录与权限/磁盘原因；检查当前账号读写权限及安全软件后重试 |
| 指定端口被占用 | 检查占用应用，或显式使用`--port 0`；不自动结束其他进程 |
| 首次浏览器打开失败 | 服务继续运行，检查HTTP/HTTPS默认应用，或复制原控制台完整地址手动打开 |
| 重复启动时浏览器失败 | 第二进程返回非零并保留错误窗口，原任务不受影响；修复后再次双击 |

程序不为启动自动结束占锁进程、删除锁文件、放宽访问权限或改用新记录目录。正常重复启动不视为故障。关闭网页不退出服务，结束使用在原服务窗口按Ctrl+C，等待当前工作收尾。

## 自动化与验证范围

公开仓库：[thising/voucher-workbench](https://github.com/thising/voucher-workbench)。GitHub Actions固定Windows2022 runner、Rust1.99.1、Cargo.lock及带SHA256的模型/原生库。push/PR执行检查，手动workflow_dispatch才构建并验证ZIP；工作流仅`contents: read`，不自动创建公开Release。

完整CI先通过格式、Clippy、全部合成测试与release构建，再解压真实ZIP验证原EXE、两种业务模式，以及通过cmd.exe执行随包CMD。入口测试覆盖：

- 中文、空格解压位置及不同工作目录，代码页936和65001。
- 首次、连续、同时启动，原服务启动中和任务执行中再次启动。
- Ctrl+C正常退出后重启，异常终止后任务标为interrupted并重试。
- 记录目录无权限、端口占用、旧版持锁、浏览器启动失败与恢复。
- 历史、方向复核、输出PDF指纹及线程设置兼容；浏览器替身核验由原进程打开认证本机页面。

浏览器替身与独立控制台helper只用于CI，不入客户包。原始URL、token、服务stdout/stderr仅在测试内存与匿名管道内，CI不上传带会话凭据的服务原始日志。只使用代码生成的合成资料，不上传业务资料或本地状态。

本机浏览器已验证第二层分组保留浅层文件、预设与明确排除、自然顺序、手动移动与改名、新计划、10线程生成4组共10页、同源模式切换及历史计划。实际浏览器下载的报告已核对4组/10页、手动顺序和成品哈希；自动化下载事件观测超时，不影响实际文件校验。临时隐藏一份合成来源后，仅该组失败，其他输出保留；恢复来源、全局改为1后，旧任务仍按10线程重试并核验复用全部4份结果。独立Poppler渲染的4页成品与对应来源逐页像素一致，实际查看了竖页及保留270度旋转的页面；本轮不改变来源已有旋转。通用合成用例还核对真实页树/内容、普通注释、严格损坏页树拒绝、历史和同哈希外来输出保护。随包通用冒烟脚本已在本机隔离服务和原生Windows真实ZIP运行通过。两种模式全局单任务、旧复核和线程设置保持兼容；既有本机两份记录目录共205文件与变更前指纹完全一致。

CI通过仅证明覆盖的自动环境；不能替代客户Windows10双击、浏览器、单位安全策略及真实资料验收。

## 交付与依赖

ZIP包含主程序、ASCII启动脚本、离线CPU OCR模型及ONNX Runtime、使用说明、完整许可和逐文件package-manifest.json。客户外层资料包另附简明说明和压缩包SHA256，其中程序文件与CI包逐字节相同。

- 完整构建与验收：[Windows CI #14](https://github.com/thising/voucher-workbench/actions/runs/41389303328)，源码 [`653112c`](https://github.com/thising/voucher-workbench/commit/653112cbfb36bd66b725406575951bab634349e1)。后续说明更新不改变包内容。
- 下载：[Windows x64测试构件](https://github.com/thising/voucher-workbench/actions/runs/41389303328/artifacts/13117161264)，需登录GitHub，保留至2026-11-16。外层Actions归档包含程序ZIP和SHA256；没有创建公开Release。
- 程序ZIP：`voucher-workbench-0.5.1-windows-x64.zip`，29,207,643字节（约27.9MiB）；429个载荷文件及1个package-manifest.json。逐文件SHA256、来源提交/Cargo.lock、模型、三个x64 PE及两份DLL与官方原包均已独立核对，无清单外文件。启动脚本ASCII/CRLF已复核，测试helper未入包。

程序ZIP的SHA256（区别于外层Actions归档）：

```text
8e4f870bd3b369d8977e35a5126b7a4d39566b82ce032c181bc66adfccbb7896
```

客户资料包：`凭证工作台-0.5.1-Windows客户试用包.zip`，28,879,156字节。430个程序文件与CI包逐字节相同，仅在最外层增加UTF-8 BOM/CRLF简明说明；不含业务数据、历史、预览或日志。其ZIP旁另有同名`.sha256`校验文件。客户资料包SHA256：

```text
4388d312547fa3deac249dd9e29ad823f5d0215241c2efc001b053773eaf685f
```

默认记录目录为`%LOCALAPPDATA%\VoucherWorkbench`，移动或替换程序目录不删除记录。升级前退出旧版服务，解压新版；保留整个记录目录即可继续使用原计划、复核和线程设置。`--state-dir`可指定整体记录目录，不自动搬迁原记录。

PDF旁不再新建manifest JSON。处理记录位于数据目录的`outputs/<输出路径摘要>/manifest.json`，保存来源、页码、方向/顺序及SHA256用于复用、恢复和防覆盖；报告仍可在界面下载，交付PDF不必附带记录。旧旁置凭证记录仅在再次生成/复用时，验证并迁移后才移除；已有历史归档不批量搬迁。更新工具拥有的输出前，旧PDF保留在输出目录`.history`，历史记录保存在程序数据目录。

应用无需客户安装Rust、Python、Office、Node或Docker；运行时不下载模型。主程序静态链接Rust CRT，OCR原生DLL仍要求微软VC++2019或兼容更新版x64运行库。缺少OCR依赖不阻止关闭识别时的凭证合并或通用PDF合并。需要识别时按随包说明从微软官方安装运行库；本工具不静默安装系统组件。[ONNX Runtime官方要求](https://onnxruntime.ai/docs/install/#requirements)

客户包未签名；若单位策略阻止运行，请联系管理员，不能要求关闭安全防护。签名、安装器、公开Release与本次试用包分别验收。

## Windows开发构建

开发机需Rust/MSVC、PowerShell7、Node；这些不是客户安装要求。依赖固定于`app/distribution/windows-dependencies.json`：

```powershell
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
pwsh -File app/tools/build-windows.ps1
pwsh -File app/tools/smoke-windows.ps1 -ZipPath dist/voucher-workbench-0.5.1-windows-x64.zip -FixtureExecutable app/target/windows-dist/x86_64-pc-windows-msvc/release/examples/package_fixture.exe
pwsh -File app/tools/smoke-windows-launcher.ps1 -ZipPath dist/voucher-workbench-0.5.1-windows-x64.zip -FixtureExecutable app/target/windows-dist/x86_64-pc-windows-msvc/release/examples/package_fixture.exe -ProbeExecutable app/target/windows-dist/x86_64-pc-windows-msvc/release/examples/startup_probe.exe
```

组包脚本不覆盖已有ZIP，可用`-OutputDirectory`及`-CacheDirectory`指定位置，`-CheckOnly`仅检查。ONNX Runtime固定1.23.2；模型算法不变，仍为title-ocr-v1.0.2。

开源边界由`tools/public-source.json`与`tools/prepare-source.py`共同约束：只发布当前Rust工具、合成测试、发行脚本和有效规格；历史Python/规格、业务数据、模型权重、本地工具链、状态、预览和日志均排除。源码导出和暂存核验不放宽此边界。

## 客户Windows10待验

普通用户在中文含空格路径解压：实际双击、连续双击、任务中再次双击，确认默认浏览器重新进入原工作台且任务继续。正常退出/异常退出后重启，确认历史、复核和线程设置保留；若旧版占锁，按提示手动退出旧版。

分别以1和10线程试用凭证与通用模式。通用模式核对0/1/2层分组、浅层文件、排除、手动排序/改名、输出目录、成品页序和报告；凭证模式检查首表/指定表、默认关闭识别、同意/识别/全部图片人工确认门禁。检查取消/重试仍用原线程，输出无额外页眉页脚或JSON，未知同名PDF被保护。

截图原异常触发条件与修复后结果单独记录。客户实际复测尚未完成，不把CI结论扩展为客户PC已验收。

## 历史构建

0.5.0线程功能测试包：[CI #8](https://github.com/thising/voucher-workbench/actions/runs/41375857170)，源码196f3ca，本地79/Windows80项通过、1项私有OCR测试忽略。此包未包含0.5.1通用模式与启动优化，仅作历史。
