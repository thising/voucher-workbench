# 凭证 PDF 工作台

当前版本是 **Rust 本地服务＋浏览器界面**，包含凭证整理和通用PDF合并两种模式。凭证模式按明细账整理；通用模式按自选目录层级合并PDF。所有文件、模型推理和处理记录均保留在本机。

当前有效规则：[凭证 PDF 规格](openspec/specs/voucher-pdf/spec.md)、[通用PDF规格](openspec/specs/generic-pdf/spec.md)及[Windows启动与发行规格](openspec/specs/windows-distribution/spec.md)。凭证方向识别默认关闭，启用需同意免责声明并在识别后完成全部待输出图片的人工核对。通用模式不使用明细账和OCR。[通用模式说明](docs/generic-pdf-mode.md)、[发行验收记录](docs/windows-release.md)、[API契约](app/API.md)。旧Python流程、旧版规格及私有验收材料仅在本地保留，不纳入本仓库。

## Windows使用

0.5.1 Windows10/11 x64 ZIP测试包已完成，含通用PDF模式和重复启动复用。[Windows CI #14](https://github.com/thising/voucher-workbench/actions/runs/41389303328)的106项测试、随包业务及真实CMD入口验收通过，构件已独立核验。下载及校验值见[发行说明](docs/windows-release.md)；客户Windows10实机试用单独验收。解压整个程序目录后双击 `启动凭证工作台.cmd`，自动打开本机浏览器。再次双击会由原服务重新开网页，任务继续；原服务启动中短暂等待。保留控制台窗口，退出时按 `Ctrl+C`；关闭网页不会停止任务。

随包提供CPU OCR模型和ONNX Runtime；方向识别仍默认关闭。Windows VC++运行库要求及缺失处理见[发行说明](docs/windows-release.md)。处理记录默认保存在 `%LOCALAPPDATA%\VoucherWorkbench`，移动或替换程序目录不会删除记录；升级前先在旧版服务窗口退出，再启动新版。旧版占锁或无法联系时，新版显示中文处理步骤；不会自动结束旧进程、删除锁文件或改用新记录目录。

## 通用PDF合并

1. 在左侧选择“通用 PDF 合并”，指定包含PDF的本地资料文件夹和输出目录。输出留空时默认在资料目录同级建立“资料名-合并PDF”。
2. 选择分组层级：0将目录递归合成一份；1每个直接子文件夹一份；2按第二层分组，最多32层。未达到所选层级的文件另列浅层组，不静默遗漏。
3. 设置明确排除的相对路径、受限glob与自然/修改时间顺序。可编辑的默认排除规则为`**/indexImg/**`；嵌套输出与本程序记录目录自动排除。
4. 扫描后核对全部分组、源PDF、页数、排除结果与输出名称；可逐文件排除、拖动或上下移动，并更改输出文件名。保存后形成新计划；范围修改后须重新扫描。
5. 勾选要生成的有效组，核对确认窗口后生成。1–10线程按组并行，单组内原PDF页序、页尺寸和旋转保持；不增加页眉页脚、分隔页等内容。
6. 在通用处理记录查看任务、下载PDF/报告、取消或重试。重试保持原计划与线程数；未知/修改过的输出不覆盖，已拥有的旧结果更新前保留历史。

本版用于普通静态PDF；加密、表单、签名、脚本、嵌入文件和不支持的文档结构会明确提示。仅支持真实本地磁盘，不跟随符号链接、目录联接或云占位文件，不支持UNC/映射网络盘。更多边界见[通用规格](openspec/specs/generic-pdf/spec.md)。

## 开发运行

项目脚本优先使用本工作区 `.local` 下的隔离工具链和运行库，没有时使用本机Cargo；不自动安装系统工具。启动：

```sh
sh app/tools/dev.sh
```

浏览器自动打开本机工作台，端口自动分配。终端保留运行，按 Ctrl+C 停止；关闭网页不会停止后台任务。服务仅监听 `127.0.0.1`，启动链接带一次会话凭据，重启后使用新链接。

1. 选择批次目录或单个分项；指定明细账，或留空自动查找 XLS/XLSX。
2. 默认使用工作簿第一个工作表，也可手动选择。核对匹配数量与错误提示。
3. 设置“并行线程数”：默认1，可手动选择1–10。设置自动保存在本机；任务运行时不能修改，重试沿用原任务线程数。
4. 选择是否启用方向识别。默认不启用，按EXIF规范后的原图方向合并，也可自行旋转；无需方向模型或先运行识别。
5. 如需启用，阅读耗时、识别局限和免责声明，勾选表示同意。完成识别后，检查**每张图片**，必要时左转、右转或180°修正，然后明确确认当前页或所选页面。全部待输出图片确认后才能生成；没有跳过复核选项。
6. 下载 PDF/报告，或打开分项输出目录。任务可取消、重试，历史可以继续。

线程数同时控制图片方向识别和凭证PDF生成的并行上限。识别按图片并行，生成按凭证并行；同一份PDF内部仍按文件数字序号合并，任务结果按凭证原顺序展示。同一时刻只运行一个任务。线程越多，CPU和内存占用通常越高；可处理项目少于所选线程数时，会相应减少工作线程，不承诺按线程数成倍提速。

并行设置保存在 `state-dir/settings.json`，重启后保留；旧状态没有设置时默认1。任务记录保存当时的 `threads`，旧任务缺少此字段时按1处理；修改全局设置不会改变已有任务，重试继续使用原任务线程数。

输出规则：`分项/230501-04067/` → 同级 `分项-PDF/230501-04067-科目名称.pdf`。PDF旁不再新建 `.manifest.json`；处理记录保存在程序数据目录，可从界面下载报告。原件不改动。已验证相同的结果重用；调整方向重生成时旧PDF保存在输出目录 `.history/`，对应的新历史记录保存在程序数据目录；来源不明或被外部改动的同名文件不会覆盖。

`manifest.json`记录账表匹配、来源文件、页码、旋转/复核状态及文件校验值，用于结果复用、中断恢复和防止误覆盖；阅读、复制或交付PDF不需要它。记录位置为 `state-dir/outputs/<输出路径摘要>/manifest.json`，Windows默认在 `%LOCALAPPDATA%\VoucherWorkbench` 下，开发环境默认在 `.voucher-state` 下，也可通过 `--state-dir` 指定整体数据目录。不同路径的同名PDF使用不同记录。

升级后，再次生成或复用某份PDF时，会先核验其旧旁置记录与PDF；复制并核验到程序数据目录后才移除旧旁置记录。冲突、未知或被修改的记录保留并报错，已有历史归档不批量搬迁。升级、备份或换电脑时请保留数据目录；清空记录或只转移PDF仍能阅读PDF，但工具无法在缺少可验证记录时自动复用或覆盖它。

## 处理约定

- 编号按凭证目录第二段匹配，仅读取“凭证编号”“科目名称”。重复编号取所选表原始行序第一条，不按日期换表，不跨表补匹配。
- JPG/PNG/BMP/静态 WebP/单页 TIFF 和 PDF 按文件名末尾数字排序；多页 PDF 保持内部顺序。多帧图片、多页 TIFF、坏文件、断号、重号和未知内容明确报错，避免静默漏页。
- 所有层级忽略 `index.html` 和 `indexImg` 子树；操作系统隐藏辅助文件也跳过。
- 图片先应用EXIF。未启用识别时，不应用未确认的自动旋转建议；启用后以标题方向提供建议，并以人工核对修正为准。宽高决定A4横纵版，等比居中、不裁切。源PDF保留页尺寸、内容和旋转。
- 不生成封面、目录、分隔页、页眉、页脚或附加页码。
- 每份 PDF 的记录包含账表指纹、工作表、匹配行、来源页码、源文件和输出 SHA256、旋转与复核状态。提交中断后重试会恢复事务。

## 在其他开发机运行

开发需要Rust stable；可选方向识别另需对应系统架构的ONNX Runtime原生库和约21.4MB的文字检测/识别模型。未启用识别时缺少模型不阻止合并，不需要Python或Office。开发验证使用Rust 1.99.1、ONNX Runtime 1.23.2。依赖版本锁定于 `app/Cargo.lock`。

```sh
sh app/tools/fetch-title-models.sh
ORT_DYLIB_PATH=/absolute/path/to/libonnxruntime.so sh app/tools/dev.sh
```

Windows 开发环境可在项目根目录使用 PowerShell：

```powershell
cargo build --release --locked --manifest-path app/Cargo.toml
.\app\target\release\voucher-workbench.exe --model app/models/title-ocr --runtime C:\path\to\onnxruntime.dll
```

模型来源、固定版本、hash及预处理见 [模型说明](app/models/README.md)。`--port` 可指定端口，`--no-browser` 不自动开网页（已有实例时仅确认存活），`--state-dir` 指定状态目录；非Windows开发默认状态 `.voucher-state/`，Windows默认 `%LOCALAPPDATA%\VoucherWorkbench`。一个状态目录只允许一个服务实例，新版重复启动复用原服务。目录权限、端口占用及浏览器失败有不同中文提示。`--check-runtime` 实际加载模型及原生库并输出JSON诊断，不创建状态或启动服务。

## 开发验证

```sh
sh app/tools/cargo-local.sh test --locked
sh app/tools/cargo-local.sh clippy --locked --all-targets -- -D warnings
sh app/tools/cargo-local.sh fmt --all -- --check
```

测试使用合成图片、账表和 PDF；真实样本属于本地验收，不进入测试夹具。核心模块在 `app/src/`，离线前端在 `app/web/`，运行时不访问外网/CDN。

代码采用[MIT](LICENSE)，模型及原生运行库分别遵循上游许可，见[第三方说明](app/THIRD_PARTY_NOTICES.md)。开源范围仅当前工具、合成测试和有效规格，由[白名单](tools/public-source.json)与[核验工具](tools/prepare-source.py)共同约束；真实资料、历史规格、模型权重、工具链、任务状态、预览和日志均排除。

Windows CI使用MSVC runner，测试、验包、随包运行库实载和合成流程全部通过后才上传ZIP；不自动公开Release。CI产物、客户Windows10验收、签名及分发分别在[发行说明](docs/windows-release.md)记录，不能相互替代。
