# Windows 构建、打包与验收

当前唯一发行基线：**0.6.0 PDF工作台，Tauri独立窗口＋Rust核心，Windows 10/11 x64**。业务依据为[批量合并规格](../openspec/specs/batch-merge/spec.md)，桌面与发行依据为[桌面发行规格](../openspec/specs/windows-distribution/spec.md)。本页是构建、源码、附件和验收状态的唯一入口；0.5.x规格只约束历史兼容入口。

## 当前状态（2026-11-03）

用户已授权固定基线、Windows CI和客户包整理，此前暂停要求已解除。**Windows2022 CI与独立验包已通过，客户Windows10实机待验。**

- 软件源码及版本标签：`v0.6.0` → `0fbf0f27d8fd5b2037ad30a5854619ad180e0f54`。
- 完整构建与验收：[Windows CI #30](https://github.com/thising/voucher-workbench/actions/runs/41519710271)；不把旧失败轮次作为发行依据。
- 随包`release-source.json`记录准确源码、组件和两份Cargo.lock摘要，`package-manifest.json`记录逐文件大小/SHA256。附件清单在`pdf-workbench-0.6.0-windows-x64-release.json`。
- 公开范围仍由`tools/public-source.json`和`tools/prepare-source.py --verify-staged`约束；本轮321份工具源码，资料、处理记录、预览、日志、模型权重和本地工具链不入Git。不创建公开GitHub Release。

## 客户交付文件

本地交付目录：`dist/customer-trial/0.6.0/`。客户便携包只重组目录并附加说明，软件文件与CI原始ZIP逐字相同；安装包和离线组件保持CI附件原始字节。

| 文件 | 字节数 | SHA256 |
| --- | ---: | --- |
| PDF工作台-0.6.0-Windows客户试用包.zip | 35,789,375 | `395bfcc970951a7617fe1a2f4816d71c616848cd01197eff01ba255459525e16` |
| pdf-workbench-0.6.0-windows-x64-setup.exe | 29,265,921 | `0a080cb512b6b831fdbcb6d33094807074309a1b9d7eb772ef52d476afdee76c` |
| 离线组件/MicrosoftEdgeWebView2RuntimeInstallerX64.exe | 215,017,168 | `62863f0f0611a16ccf124b4f15650c3b66624504eba74d7cde1e2ae5d951249b` |

同时提供`简明使用说明.txt`、`试用反馈模板.txt`、`版本与校验说明.txt`、单附件`.sha256`和`SHA256SUMS.txt`。中文TXT为UTF-8 BOM及CRLF。推荐完整解压客户ZIP后双击内层“PDF工作台.exe”；安装包与便携版任选其一。离线WebView2为独立可选附件，已有组件无需重复安装。

## 当前功能与兼容口径

- 独立窗口默认进入“批量合并”；日常双击 `PDF工作台.exe` 或安装快捷方式，不需要外部浏览器、终端、Rust、Office、Python、Node或Docker。
- 准备资料：选择目标即初始化，按实际层级分组；排除规则默认折叠。重命名默认关闭，开启后选择XLS/XLSX、工作表、表头行及两列；默认“文件夹名-指定列内容”，可选独立名称。同索引固定取原始第一条，未命中或首条为空回退文件夹名；支持逐行手改和实时重名提示。
- 预处理：三栏展示合并单元、文件顺序和预览；支持手柄拖动、置顶、上下移及从本次任务移除/恢复来源，原文件不删除。图片与PDF可混合，PDF保持内部页序并支持逐页手动旋转。
- 自动方向仅处理图片，用户同意须知后启动；默认4线程、允许1–10。仅本次实际变角的图片和所属单元标色，方向明细只列变角文件，其余结果汇总；同角缓存重跑不重复标记。旧记录缺少变化证据时保持中性，不改原JSON或保存角度。人工检查建议保留，未核对不阻止合并。
- 执行合并：每单元一份PDF，默认目标同级加“-PDF”，可更改输出位置；按当前顺序和角度生成，不加页眉、页脚或分隔页。识别和合并线程分别记忆，默认4、范围1–10。
- 已有普通同名文件只在执行合并时询问跳过、覆盖或取消，可应用本批次。**覆盖直接替换，不保留旧PDF或登记备份，覆盖后无法恢复**；提交前生成/校验失败或取消保留旧目标，已完成项目在任务取消后保留。事务只保存新结果供提交中断恢复，目录、链接、来源同目标等保护不能绕过。已有历史归档不自动清理。
- 带签名PDF按可靠静态外观纳入；不验证、不保留原数字签名验证，输入原件不变。缺乏可用静态外观或其他不支持内容仍明确报错。旧计划中因此被拒绝的来源须重新扫描。
- 同一处理记录目录仅一个实例；重复启动唤回原窗口，任务继续。活动任务关闭时选择继续或安全停止后退出，不自动强杀旧进程、删锁、放宽权限或另换目录。
- 默认记录目录继续使用 `%LOCALAPPDATA%\VoucherWorkbench`，保留旧计划、复核及线程设置；新批量偏好与旧设置分开。输出PDF旁不生成manifest，来源/角度/页码/校验记录在该目录 `outputs/<输出路径摘要>/`，历史计划和报告另存。升级/换机保留整个记录目录；历史报告不能恢复已直接覆盖的旧PDF。

## 软件包与运行组件

本轮交付便携ZIP、当前用户NSIS安装包、中文使用说明、附件校验值，以及独立WebView2离线安装器。便携与安装版主EXE须逐字一致；随包提供CPU模型、ONNX Runtime、PDF.js离线资源、许可和来源清单，不含客户资料及CI helper。

WebView2已安装直接使用；安装包缺少时按需下载微软官方引导程序，便携版给出步骤。离线组件单独附带，不塞入每份主程序更新。本轮锁定微软x64离线安装器 **215,017,168字节（约205MiB）**，SHA256：

```text
62863f0f0611a16ccf124b4f15650c3b66624504eba74d7cde1e2ae5d951249b
```

来源URL、版本和摘要见 `app/distribution/windows-dependencies.json`；Windows构建验证Microsoft Authenticode签名。主程序Rust CRT静态链接，OCR库另需微软VC++2019或兼容更新版x64运行库；缺少OCR依赖仍可手动合并。PDF.js固定6.5.438，主模块/worker、字体、CMap、WASM均离线提供。本工具和安装器未签名，不要求关闭系统防护。

## 本机验证的范围

- 此前本机功能基线：核心156项通过、0失败、1项私有资料OCR按设计跳过；桌面2项通过。其中43项批量集成覆盖无备份事务提交边界/恢复、失败保旧、现存历史保持、角度变化/未变/未决/手动跳过/取消/并发汇总及旧记录兼容；6项签名集成通过。核心与桌面格式、Clippy均通过，OpenSpec严格校验10项通过。
- 前端59次API、193个DOM控件及Pointer捕获/21类取消回归通过。真实原生手柄排序、外部释放取消、PDF页码保留、同记录重启、命名清错、移除/恢复、原生保存和任务退出保护已验。
- 5张合成图真实OCR首次调整1/未变4，颜色与方向详情只列该图；缓存重跑调整0/未变5，角度保持。原生窗口及最终release业务API均验证。
- 合成既存PDF实际直接覆盖2份5页，无旧PDF/登记副本及临时残留，5个来源摘要不变；全部5页经Poppler渲染并查看，顺序与角度正确。最终release API成品与原生输出摘要相同。另有混合PDF/图片6份10页及4例签名外观逐像素一致证据，未覆盖客户成品。
- 2026-11-02原本地入口 `.local/pdf-workbench-trial/启动PDF工作台.command` 已更新为release，更新前23个记录文件字节保持，原3条历史和最新计划可继续，HTML/JS/CSS与当时本机验证源码一致。后续用户试用可新增记录，不将23视为永久文件数。当前Windows基线另含窄屏CSS和共享路径观察修正，现有Mac窗口未因此重启换版。
- 共享路径修正后，本机5项受控回归、完整release库57项单测及Clippy通过；Unix socket测试在所需本地IPC权限下完成。Windows真实junction和其余整套集成以本轮MSVC结果为准。此前GNU编译检查是旧基线证据，不代替本轮实测。

扫描性能证据仅限本机暖缓存：3274个来源一次只读核心扫描约2.486秒，保留完整SHA与解析检查；不包含窗口、索引和记录写入，不代表客户PC耗时或整批加速倍数。此前2份签名拒绝是修订前规则；现已按上述静态外观导入。原始证据、截图和指纹保留在本地验收目录，不公开业务数据。


## 本轮Windows验证与独立验包

- Windows MSVC：171项Rust测试通过，另1项私有OCR按设计跳过；核心与桌面格式、Clippy均通过。前端59次API/193个DOM及Pointer回归通过。
- 原完整检查之外，默认并发generic兼容套件完整重复5轮通过；不跳过重启重试断言。共享路径回归覆盖首次NotFound后目录出现、仍缺失、非目录、权限拒绝和真实Windows junction拒绝。
- 同GUI EXE headless业务、兼容CMD真实代码页/中文空格路径/重复与同时启动/任务中激活/正常异常重启/权限端口旧锁矩阵通过。
- 保存控制器先经独立现代Windows对话框预检；完整随包程序另实际执行原生PDF及报告保存，精确比对返回路径和内容，预检不能代替产品测试。
- 原生便携/安装验收共24项通过：三栏和窄屏布局、离线PDF.js逐页预览/旋转、Pointer排序及自动保存、命名/线程/混合合并、执行时同名跳过/直接覆盖/批次应用/取消且无备份、重复激活、OCR中继续/安全关闭、正常及异常重启、启动竞态、旧协议/组件/记录目录故障、当前用户安装、模型运行库解析、可见安装窗口及卸载完成。便携与安装版主EXE SHA256严格一致：`a9c6832f3b46dd0874615aa5b881d2f032fa7aefdf37b19677d8c510a59a7610`。
- 下载后独立核对ZIP全部704个清单文件及manifest、路径白名单/链接/大小写冲突、Windows x64 GUI子系统PE、准确源码、双锁、CPU模型/运行库/PDF.js/许可、全部附件大小与SHA256；重新打开客户ZIP逐文件校验。真实窗口及同名决策截图已查看。

工作流为`.github/workflows/windows.yml`，固定Windows2022、Rust/MSVC及核验组件；手动dispatch才构建客户附件，权限contents:read。只发布合成截图和有界报告，服务认证链接不写入发现文件或上传日志。开发重建入口：`pwsh -File app/tools/build-windows-desktop.ps1`；`-CheckOnly`只检查，目录可隔离，同名发行文件拒绝覆盖。

验证边界：Windows原生OCR场景验证任务中激活和安全停止，不是整批完成识别率验收；完整变角提示以Windows核心/前端回归和本机真实5图证据为限。签名静态外观有Windows核心和本机4例渲染证据，原生窗口fixture并未覆盖全部签名类型。旧字段兼容由核心测试覆盖，CI窗口历史重启使用的是新生成记录，不能宣称已验收客户全部旧历史。客户实际联网/离线安装WebView2、Windows10和单位安全策略尚待实机反馈。

## 本轮发行修订与证据范围

固定功能流程后，仅增加发布所需打包/验收修订、窄屏CSS和共享路径检查加固。Tauri CLI打包期间写NSIS标记会造成便携/安装EXE差异；本项目不用updater，通过[官方选项](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-cli/src/build.rs)`--no-binary-patching`固定两份相同EXE，来源清单记录该策略。WebDriver启动参数遵循[微软EdgeOptions约定](https://learn.microsoft.com/en-us/microsoft-edge/webdriver/capabilities-edge-options)的`--name=value`，失败报告有界脱敏并保留检查点。

现代Windows文件名面板未提供ValuePattern时，测试助手只在精确UIA主机边界内匹配同PID的唯一原生Edit，定向发送有超时的控件消息并精确读回，随后核对真实保存文件，不使用全桌面输入。任务对话框由[专用TDM_CLICK_BUTTON消息](https://learn.microsoft.com/en-us/windows/win32/controls/tdm-click-button)及锁定rfd的按钮ID触发，事先验证自有根和正文，随后验证真实选择及任务收尾。NSIS按[官方命令行](https://nsis.sourceforge.io/Docs/Chapter3.html#3.2.2)原地卸载，等待完成并确认主EXE消失。

[CI #21](https://github.com/thising/voucher-workbench/actions/runs/41502187192)曾单次在generic重试出现分组错误，旧断言没有打印分组原因，不能认定根因。后续诊断、5轮复跑与本轮全部通过。独立受控回归证明了共享路径的过期NotFound竞态，已修复并保留完整目录/链接/权限保护；不将它冒充该次失败的已证实原因。其余失败轮次的测试定位证据保留在本地验收目录，均不是客户附件来源。

## 客户Windows10待验

普通用户在中文/空格路径实际解压、安装及双击；已有/缺少WebView2的真实联网或离线安装；单位安全策略；连续双击、任务中激活与安全退出；升级后旧历史、角度和线程偏好；真实资料的层级、索引、顺序、移除、PDF逐页旋转、仅变角提示、签名静态外观和同名直接覆盖。分别试用1与10线程，核对份数/页数/方向/顺序及原件不变。CI仅证明所覆盖环境，不等同客户实机已验。

## 历史交付与磁盘清理

0.5.1浏览器版仅作历史回滚：[CI #14](https://github.com/thising/voucher-workbench/actions/runs/41389303328)，源码653112c，客户包SHA256`4388d312547fa3deac249dd9e29ad823f5d0215241c2efc001b053773eaf685f`。0.5.0线程版为[CI #8](https://github.com/thising/voucher-workbench/actions/runs/41375857170)，源码196f3ca。旧CMD客户解析异常触发条件未复现，不断言编码损坏；旧备份式覆盖证据不能当作当前直接覆盖验收。

本轮清理8处可再生成的debug/GNU构建缓存、旧验包解压目录及重复工具压缩包，占用合计16.44GiB，清理时实测可用空间增加16.07GiB。删除前确认无进程占用；当时8个保护文件SHA256保持。源码、业务资料、输出PDF、处理记录、运行中程序、有效工具链、验收证据和3个历史客户包保留。测量清单在本地`.local/windows-060-qualification/cleanup-result.json`；新交付文件另计占用。
