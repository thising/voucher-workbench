# Windows 构建、打包与验收

当前唯一发布目标：**0.6.0 PDF工作台，Tauri 独立窗口＋Rust 核心，Windows 10/11 x64**。业务依据为[批量合并规格](../openspec/specs/batch-merge/spec.md)，桌面与发行依据为[桌面发行规格](../openspec/specs/windows-distribution/spec.md)。本文是构建、基线、构件和验收状态的唯一入口；0.5.x规则仅约束其历史兼容入口。

## 当前状态（2026-11-03）

用户已明确授权固定当前功能基线、统一文档、运行Windows CI并准备客户软件包，之前的CI暂停要求已解除。本机功能和原生窗口验证已完成。首次固定源码 `e232dac028befd6a79cadfee36e4e789dcd06865` 的[Windows CI #16](https://github.com/thising/voucher-workbench/actions/runs/41480619765)通过167项Rust测试（核心165、桌面2，另1项私有OCR跳过）、启动矩阵、构建及headless兼容；真实桌面阶段在便携/安装版EXE一致性处失败，未交付附件。[第二轮CI #17](https://github.com/thising/voucher-workbench/actions/runs/41484740038)，源码 `d280a292f4fa90b759c2a8186eedc682ecdc41eb`，已通过便携/安装EXE一致性，但首次WebDriver会话创建失败。[第三轮CI #18](https://github.com/thising/voucher-workbench/actions/runs/41487723974)，源码 `6de1db38eebe8b10f93fa7691ecbc6aa55d470e6`，已通过7项真实窗口/三栏/PDF预览旋转/Pointer排序/重命名与合并，失败停在原生另存副本未抵达指定位置。该轮仍不作客户交付。客户Windows10实际双击、组件安装、单位安全策略和真实资料验收始终单列，不以CI代替。

版本固定为0.6.0；构件准确源码以随包 `release-source.json` 的 `source_commit` 及同轮CI为准，ZIP逐文件清单在 `package-manifest.json`，附件大小、SHA256及双Cargo.lock摘要在 `pdf-workbench-0.6.0-windows-x64-release.json`。完成CI和独立验包后再补充本页的构建来源、附件及校验结果。

[公开仓库](https://github.com/thising/voucher-workbench)仅发布工具源码及现行规格。来源白名单为 `tools/public-source.json`；提交前运行 `tools/prepare-source.py --verify-staged`，禁止带入资料、处理记录、预览、日志、模型权重及临时验收目录。

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

## 已完成的本机验证

- 核心156项通过、0失败、1项私有资料OCR按设计跳过；桌面2项通过。其中43项批量集成覆盖无备份事务提交边界/恢复、失败保旧、现存历史保持、角度变化/未变/未决/手动跳过/取消/并发汇总及旧记录兼容；6项签名集成通过。核心与桌面格式、Clippy均通过，OpenSpec严格校验10项通过。
- 前端59次API、193个DOM控件及Pointer捕获/21类取消回归通过。真实原生手柄排序、外部释放取消、PDF页码保留、同记录重启、命名清错、移除/恢复、原生保存和任务退出保护已验。
- 5张合成图真实OCR首次调整1/未变4，颜色与方向详情只列该图；缓存重跑调整0/未变5，角度保持。原生窗口及最终release业务API均验证。
- 合成既存PDF实际直接覆盖2份5页，无旧PDF/登记副本及临时残留，5个来源摘要不变；全部5页经Poppler渲染并查看，顺序与角度正确。最终release API成品与原生输出摘要相同。另有混合PDF/图片6份10页及4例签名外观逐像素一致证据，未覆盖客户成品。
- 2026-11-02原本地入口 `.local/pdf-workbench-trial/启动PDF工作台.command` 已更新为release，更新前23个记录文件字节保持，原3条历史和最新计划可继续，HTML/JS/CSS与当时本机验证源码一致。后续用户试用可新增记录，不将23视为永久文件数。当前Windows基线另含下述窄屏CSS修正，现有Mac窗口未因此重启换版。
- 现有Windows GNU目标全目标编译检查通过；这只证明条件编译，不替代Windows MSVC、WebView2或安装器实测。

扫描性能证据仅限本机暖缓存：3274个来源一次只读核心扫描约2.486秒，保留完整SHA与解析检查；不包含窗口、索引和记录写入，不代表客户PC耗时或整批加速倍数。此前2份签名拒绝是修订前规则；现已按上述静态外观导入。原始证据、截图和指纹保留在本地验收目录，不公开业务数据。

## Windows CI 与独立验包

工作流 `.github/workflows/windows.yml` 使用Windows2022、固定Rust/MSVC及依赖摘要；push/PR执行检查，手动workflow_dispatch构建客户附件，权限仅contents:read，不创建公开Release。新一轮以本次固定源码启动；不能将[首轮失败CI](https://github.com/thising/voucher-workbench/actions/runs/41412847079)视为发行通过。首轮在合法依赖版本号“+”路径白名单处中止，修复及后续启动竞态测试已纳入本轮。

便携/安装EXE差异已追溯到固定Tauri CLI 2.12.4在NSIS打包期间写入类型标记、随后恢复原文件的[上游实现](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle.rs)。本项目不使用updater，按[官方CLI选项](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-cli/src/build.rs)传入 `--no-binary-patching`，保留严格EXE SHA256一致性门禁，来源清单记录 `binary_patching=disabled-no-updater`。真实日志也记录了类型标记写入；最终结论仍以重跑的相同EXE检查为准。

本轮要求：核心/桌面格式、Clippy、单测与UI契约；兼容CMD真实代码页/中文空格/重复与同时启动/任务中激活/正常异常重启/权限端口旧锁；同GUI EXE的headless兼容业务；真实Tauri/WebView2窗口的三栏、PDF预览/逐页旋转、排序命名、合并、运行时同名决策及无备份检查、原生保存、重复唤回、OCR期间关闭继续/安全停止、启动竞态、旧协议/缺运行时/目录故障；当前用户NSIS安装及安装版真实窗口。

CI仅上传受限合成截图与摘要，不上传含token的原始服务日志。成功后下载附件，独立核对ZIP全文件SHA/大小/白名单、Windows x64 GUI子系统PE、便携与安装版一致证据、来源提交、双锁、模型/运行库/许可及离线附件签名检查结果，再整理客户目录及总校验表。真实界面截图须查看，不以编译或替身代替。

第二轮还发现测试启动参数按普通argv拆分传递，违反[微软EdgeOptions约定](https://learn.microsoft.com/en-us/microsoft-edge/webdriver/capabilities-edge-options)：带值开关应为 `--name=value`。测试已修正中文/空格路径参数，补充实际会话边界回归及有界脱敏驱动错误，外层报告保留内层原因。第三轮实际会话及后续7项原生检查已通过，验证了该驱动参数修订。业务和桌面运行时代码未改。

第三轮另存自动化使用第一个可见Edit ID1001，存在误选搜索框的风险；保存返回成功却未落在请求路径。第四轮测试曾限制为经典文件名主机并精确读回，后来第五轮证实现代对话框的父控件ID为0，不能沿用经典1148定位。测试仍保留native返回路径与请求路径的独立比对，以及最终文件SHA检查，产品功能未改。

第三轮真实Windows截图还发现1000px视口三栏最小宽度约1003px，而内容区约945px，导致右栏裁切。仅在现有窄屏规则内收窄列最小宽度并允许预览控件换行，加入工作区横向溢出及放大按钮右边界断言；Rust/前端业务逻辑保持冻结。该CSS修正已在第四轮实际截图及第五轮边界断言中通过。

[第四轮CI #19](https://github.com/thising/voucher-workbench/actions/runs/41492661250)，源码 `d9b8a8ecaeee5f1cecbd90466c38e87556e87578`，检查/构建/兼容通过，真实截图已查看：1000px窗口右侧预览类型、适合窗口及放大按钮均可见。原生阶段后续仍因异步脚本超时失败，未产出完整通过报告；原生操作失败时未被立即捕获的待决请求可在收尾阶段遮蔽原错误。测试已为待决请求立即登记成功/失败、逐检查点持久化报告，并记录另存候选数/写入/读回/按钮阶段；按钮分发回到前轮验证过的实现。业务及CSS未再变化。

[第五轮CI #20](https://github.com/thising/voucher-workbench/actions/runs/41497787382)，源码 `1f4b344e7f6e2402a6223780d22c9e7029bb52c9`，真实窗口8项已通过，窄屏无横向溢出；checkpoint保留的明确原因是另存文件名候选数0，物理控件链为Edit1001→ComboBox0。现改用Windows内置.NET Framework UI Automation，在确切测试PID/HWND内按FileNameControlHost查找可写编辑控件，SetValue后精确读回，再调用Save；不依赖搜索框顺序或物理ComboBox编号。独立无窗口客户端采用[微软建议的MTA线程](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading)，只输出结构化有界证据。9份PowerShell语法检查、C#控制器编译、外层报告保留与59API/193DOM回归通过；真实UIA行为待下一轮Windows验证。

[第六轮CI #21](https://github.com/thising/voucher-workbench/actions/runs/41502187192)，源码 `08099da3471c4e9d034e58f41ef97854bb1a96d9`，50项lib、43项batch及15项generic已通过；generic重启重试用例出现completed_with_errors，未到原生UIA验证。原断言未输出分组错误，原因待查；已加强失败诊断，不跳过或放宽用例。下一轮在原全套检查之外，另完整重复5轮默认并发generic suite，验证重启重试与同进程其他并发测试；没有证据前不改业务代码。

NSIS卸载验收另按[官方命令行](https://nsis.sourceforge.io/Docs/Chapter3.html#3.2.2)使用末尾不带引号的 `_?=` 原地卸载，等待其完成并断言安装EXE消失，避免只等待转交给临时副本的启动进程。该修订仅作用于CI测试助手。

[第七轮CI #22](https://github.com/thising/voucher-workbench/actions/runs/41503680980)，源码 `3b5467a1ad64021abe6296078562d7f33e418d2e`，完整检查/构建、兼容smoke及5轮默认并发generic均通过；原生业务8项、NSIS原地卸载及EXE消失检查通过。UIA报告仅找到SearchEditBox，证明Edit-only枚举未包含虚拟文件名主机；正在补充FileNameControlHost自身可写ValuePattern，继续坚持精确定位、读回及最终落盘校验。未产出客户附件。

独立路径复核发现了可受控复现的缺失→目录出现竞态：首次NotFound后若另一worker已创建目录，旧exists守卫会返回过期错误。共享路径检查改为重新观察并验证当前元数据；中间非目录、链接/重解析点/云占位/权限保护保留，末尾canonicalize前再次受检。本机5项受控回归、57项release库单测及Clippy通过；Windows真实junction及完整业务待下一轮。第六轮缺分组错误，尚不能确认它必定命中此竞态。

[第八轮CI #23](https://github.com/thising/voucher-workbench/actions/runs/41507355753)，源码 `955b4c5c8e0f1c7cbce026020341cc404062b13c`，快速现代另存预检失败，尚未进入Rust/产品窗口检查。明确证据：FileNameControlHost是AppControlHost类型Pane，自身无ValuePattern；UIA树只有搜索Edit，物理Win32 filename Edit仍存在。正在以精确主机范围关联其内部原生输入控件，继续要求同PID、唯一匹配、写入读回和实际目标落盘验证。

文件名测试助手已增加精确UIA主机矩形→同PID原生Edit/ComboBox树→FromHandle可写ValuePattern桥接，必要时仅对自有ID1保存按钮投递消息；所有权、唯一性、物理像素边界和读回均须通过。C#5真实定义编译及10份PowerShell解析通过，Windows行为仍待快速预检和完整随包测试。

[第九轮CI #24](https://github.com/thising/voucher-workbench/actions/runs/41509095101)，源码 `d4c4235e587197a5e4e55627197ea3e4cdd97908`，快速预检已精确匹配主机范围内的原生Edit；FromHandle却仍将其标为Pane且没有ValuePattern，显式加载Framework proxies后亦相同。原生PID/HWND/结构匹配通过，尚无产品运行结果。测试现保留ValuePattern优先，缺失时仅向该唯一受检Edit发送聚焦/选择/UTF16字符消息、精确WM_GETTEXT读回，单消息及总耗时有界，每字符重验边界、只读和归属；没有全局输入。

[第十轮CI #25](https://github.com/thising/voucher-workbench/actions/runs/41509972932)，源码 `77f35047dec7567b7f4c2bc0e78f510e78330d53`，现代保存预检已通过并实际落盘至指定位置。Windows库53项通过，新增junction用例因仅接受“重解析点”文案失败：函数已经返回拒绝错误，但未接受同样有效的“符号链接”拒绝分支。测试现接受这两种安全拒绝并打印未知错误，不改变程序保护或放行junction。

[第十一轮CI #26](https://github.com/thising/voucher-workbench/actions/runs/41511261964)，源码 `e054c6af1bfb7fd17f3621c424096ff0ac3d4e2a`，完整检查/构建、兼容和5轮并发复跑通过。11项真实业务通过，含Tauri PDF/报告原生保存、真实路径与bytes、执行时同名所有选择且无备份、重复启动唤回；原地卸载亦过。OCR中关闭确认的期望正文与源码一致，但Win32读取未命中现代TaskDialog的虚拟文字。测试助手现在仅在原读取不命中时用exact owned UIA Name进行有界精确Contains比较，仅返回布尔与计数、不输出正文；快速预检增加真实common-controls TaskDialog取消/确认及中文正文校验。已通过本地C#5、11份PowerShell语法检查。

[第十二轮CI #27](https://github.com/thising/voucher-workbench/actions/runs/41514251194)，源码 `71fa933e543e6a04d5d2a2dca24644f948a5b727`，快速预检的保存、中文正文及取消通过，确认按钮未由直接child GetDlgItem找到。测试现在传入明确按钮名称，保留原生句柄优先，并对虚拟按钮使用exact owned root内唯一可见可用、Name完全相同的InvokePattern；调用后仍验证实际确认/取消结果及产品任务状态。

**本轮构件状态：准备第十三轮快速预检与完整CI；完整Windows产品验收、附件/SHA256尚未完成。**

CI在昂贵构建之前增加独立、仅控测试自有窗口的现代另存对话框预检：验证文件名控制器的真实写入、读回和落盘位置，快速暴露UIA适配问题。该预检不能代替后面的随包Tauri真实保存；完整Rust、兼容、5轮并发稳定性、便携与安装窗口门禁全部保留。10份PowerShell语法及actionlint通过。

开发重建入口：`pwsh -File app/tools/build-windows-desktop.ps1`；`-CheckOnly`只检查，输出与缓存支持隔离目录，同名发行文件拒绝覆盖。准确依赖许可与源码归档地址随包提供。

## 客户Windows10待验

普通用户在中文/空格路径实际解压、安装及双击；已有/缺少WebView2的真实联网或离线安装；单位安全策略；连续双击、任务中激活与安全退出；重启后历史、角度和线程偏好；真实资料的层级、索引、排序、移除、PDF逐页旋转、仅变角提示、签名静态外观和同名直接覆盖结果。分别试用1与10线程，核对成品份数/页数/方向/顺序与原件不变。CI通过只说明覆盖环境，客户实测尚待反馈。

## 历史交付与回滚范围

- 0.5.1浏览器版：[CI #14](https://github.com/thising/voucher-workbench/actions/runs/41389303328)，源码653112c；客户包SHA256为 `4388d312547fa3deac249dd9e29ad823f5d0215241c2efc001b053773eaf685f`，仅作历史回滚。旧CMD客户解析异常的触发条件未复现，不断言编码损坏。
- 0.5.0线程版：[CI #8](https://github.com/thising/voucher-workbench/actions/runs/41375857170)，源码196f3ca，旧规格历史。
- 前轮150/2回归及覆盖归档证据属于旧修订，当前覆盖已不保留副本；已有历史归档保留，不将旧验证冒充当前无备份行为。

本轮已清理8处可再生成的项目debug/GNU构建缓存、旧验包解压目录及重复工具压缩包，目录占用合计16.44GiB，实测磁盘可用空间增加16.07GiB。删除前确认缓存无进程占用；当前试用EXE、运行中release、模型/运行库及3个历史客户包共8个保护文件SHA256保持。源码、输入、输出PDF、处理记录、有效工具链及验收证据均保留。清理独立于待完成的远端GUI驱动验收，详细清单和空间测量在本地 `.local/windows-060-qualification/cleanup-result.json`。
