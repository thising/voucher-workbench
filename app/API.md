# 本地 API 契约（实现接口）

凭证规则见 `../openspec/specs/voucher-pdf/spec.md`，通用规则见 `../openspec/specs/generic-pdf/spec.md`。服务只监听 127.0.0.1；首次打开的 URL fragment 含 token，前端存 sessionStorage 后移除 fragment。所有 /api 请求带 `X-Session-Token`，拒绝异源请求。请求/响应 JSON；错误 `{error: "中文原因"}`。文件缩略图/原件也需用 fetch + blob（不得 token 放 URL）。静态文件内嵌，离线无 CDN。`/`为凭证界面，`/generic`为通用界面；同源页面共享当前标签页sessionStorage。

## 通用PDF模式（0.5.1）

- POST `/api/generic/scan` body `{root,output_dir?:string|null,level?:0..32,exclude_paths?:string[],exclude_globs?:string[],sort?:"natural_asc"|"natural_desc"|"modified_asc"|"modified_desc"}` → GenericPlan。缺省level=0、自然升序；缺省glob为`["**/indexImg/**"]`，显式`[]`取消预设；只扫描PDF。
- GET `/api/generic/plans` → GenericPlan[]（新到旧）；GET `/api/generic/plans/{id}` → 注册的完整计划。
- POST `/api/generic/plans/{id}/order` body `{groups:[{group_id,file_ids:string[],output_name?:string}]}` → 新GenericPlan。给出的每组必须完整排列该组全部文件且恰好一次；拒绝跨组、重复、遗漏和未知ID。新plan.id，parent_plan_id指向原计划；组/文件ID保持，sort标记manual；保留旧计划与任务快照。
- POST `/api/generic/plans/{id}/generate` body `{group_ids?:string[]}` → Job，kind=generic_generate。未传为所有组；包含空组/错误组会拒绝启动，可显式只选有效组。与凭证模式共用单活动任务和线程设置。
- GET `/api/generic/jobs` → 通用Job[]。GET/POST `/api/jobs/{id}`、cancel、retry、report及outputs沿用公共接口，按job.kind分派。Job.request.voucher_ids在通用模式保存组ID；旧字段保留以兼容已有任务。
- GET `/api/generic/plans/{id}/files/{file_id}` → 重新核验指纹的PDF原件；未知或非本计划ID拒绝。
- POST `/api/generic/plans/{id}/open-output` → `{ok:true}`，只打开注册计划中经过校验的输出目录。

GenericPlan字段：`{schema_version:1,mode:"generic_pdf",id,parent_plan_id,created_at,root,output_dir,level,sort,exclude_paths,exclude_globs,groups,issues,exclusions,ignored_files:数量,empty_directories:string[]}`。groups：`{id,relative_path,name,shallow,output_name,output_path,files,pages,issues,empty}`；files：`{id,name,relative_path,bytes,modified_ms,sha256,pages,error}`。issues：`{severity,code,message,path:null|string}`；exclusions：`{rule,kind:"path"|"glob"|"output"|"state",matches:string[]}`。命中目录列一个目录项，不虚报未遍历后代数量。

通用计划持久保存在`state-dir/generic-plans`，任务启动时在reports保存固定计划快照，报告包含分组、范围、排除规则、逐源首末页和SHA256。输出记录带mode=generic_pdf，不采用凭证或外来记录。新顺序/名称不改变旧报告；若成品已更新，旧任务不得下载新成品冒充旧版本。路径名保持原Unicode规范形，比较大小写折叠，不做NFC转换。

## 凭证与公共接口

- GET `/api/status` → `{version, orientation_model, orientation_disclaimer_version, model_ready, model_error, running_jobs}`；方向结果仅在 model 与 orientation_model 精确相等时为当前版本。
- GET `/api/settings` → `{threads:整数}`；默认1，返回当前全局并行设置。
- PATCH `/api/settings` body `{threads:整数1..10}` → 同结构；拒绝0、超过10、非整数或错误类型。活动任务期间拒绝修改；成功后持久化到 `state-dir/settings.json`，重启保留。
- GET `/api/fs?path=绝对路径` → `{path, parent, entries:[{name,path,is_dir}]}`；未填 path 返回启动工作目录。仅列目录与 XLS/XLSX，供本地选取。
- GET `/api/ledger/sheets?path=绝对明细账路径` → `SheetInfo[]`；首表无效也可列出其他表供用户选择。
- POST `/api/scan` body `{root,ledger?:string,sheet?:string}` → `Plan`（字段见 src/model.rs）；新计划默认关闭方向识别；同步请求，前端显示扫描中。
- GET `/api/plans` → Plan 概要数组 `{id,root,sheet,created_at,vouchers,pages,orientation_workflow,review_count,algorithm_unresolved_count}`。review_count是本流程待人工确认数，algorithm_unresolved_count仅为模型未决数量。
- GET `/api/plans/{id}` → Plan。所有后续操作均以此 ID 定位，不接收任意文件路径。
- PATCH `/api/plans/{id}/orientation` body `{enabled,accept_disclaimer:false}` → Plan。启用必须显式accept_disclaimer=true，服务记录当前免责声明版本和时间。关闭取消未确认的自动角度，保留人工确认和OCR证据。活动任务期间禁止改变模式。
- POST `/api/plans/{id}/analyze` → Job。仅已启用且同意当前免责声明可运行；可以取消、重试，完成后进入人工核对阶段。只有存在未人工确认且无当前版本结果的图片时才要求模型；全部结果可复用时允许零推理完成阶段。
- PATCH `/api/plans/{id}/files/{file_id}` body `{rotation_cw:0|90|180|270,reviewed:true}` → Plan。对 PDF 为额外顺时针旋转，默认0；图片以 EXIF 校正后的方向为基准。活动任务期间不可修改；启用识别时须先完成识别阶段。
- POST `/api/plans/{id}/review` body `{file_ids:[...]}` → Plan，将指定文件当前方向标为人工确认。UI 操作前明确列示数量，不默认全选。
- GET `/api/plans/{id}/files/{file_id}?thumbnail=true` → 已按 EXIF 校正的图片缩略图（尚未应用 rotation_cw，前端 CSS 旋转）；无 query 返回原件（PDF 可用 blob URL 内嵌原生预览）。
- POST `/api/plans/{id}/generate` body `{voucher_ids?:[]}` → Job。未传voucher_ids为全部。禁用识别时无OCR/强制确认前提，只使用EXIF规范后原图方向或已人工确认的旋转。启用时必须有效同意、分析完成、且本次选定凭证内所有图片已人工确认；旧allow_unreviewed=true被拒绝。其他扫描错误仍按凭证报告；已有正确且hash匹配的输出可复用，模式/同意/核对变化需更新报告；未知输出不覆盖。
- GET `/api/jobs` → Job[]（新到旧）
- GET `/api/jobs/{id}` → Job，前端每1秒轮询，含进度和逐凭证结果。
- POST `/api/jobs/{id}/cancel` → Job；逐页检查取消，临时输出不冒充成品。
- POST `/api/jobs/{id}/retry` → 新 Job；方向识别跳过已识别页，生成跳过经 hash 验证完成项，继续失败/取消项。线程数沿用被重试任务的 `threads`，不读取当前全局设置。
- GET `/api/jobs/{id}/report` → JSON处理报告（浏览器下载），含来源、排序、旋转、页码映射、sha256、异常。
- POST `/api/plans/{id}/open-output` body `{item:string}` → `{ok:true}`，用系统文件管理器打开该分项输出文件夹。
- GET `/api/jobs/{id}/outputs/{voucher_id}` → 已完成 PDF（fetch blob下载/预览）。

Plan.requested_root 保留原先单分项/批次选择，换表重扫使用此字段；Plan.root 是规范后的批次目录。SourceFile.sha256 在扫描时确定，方向和复核绑定此内容指纹。任务报告使用当次固定快照；旧任务的输出文件若已更新，下载提示使用最新任务，历史归档仍保留。工具拥有且未被外部修改的旧输出，在方向改变后归档 `.history` 再生成；其他已存在文件仍拒绝覆盖。

输出记录统一保存在 `state-dir/outputs/<规范化输出路径的SHA256>/manifest.json`，事务记录及新归档的历史manifest保存在同一记录目录下；不同输出路径的同名PDF独立管理。旧PDF仍归档到输出目录 `.history`。新的生成不在PDF旁写manifest或事务JSON；任务报告快照继续保存在 `state-dir/reports/{job_id}/{voucher_id}.json`，报告与PDF下载API保持不变。

旧版旁置manifest在对应输出再次生成/复用时迁移：核验producer、root、folder和PDF SHA256，复制并核验到state-dir后才删除旁置文件。遇到未知、冲突或篡改记录时保留文件并拒绝覆盖；旧事务先按兼容流程恢复再迁移，既有历史归档不批量移动。记录目录缺失时不得仅因同名PDF存在就复用或覆盖。`--state-dir`修改整个处理记录目录，不自动移动原状态；使用已有记录应迁移或恢复整个状态目录。

Plan.orientation_workflow为`{enabled,consent_version,consented_at,analysis_completed}`，manifest同步保存当次值。缺字段旧计划默认enabled、无同意记录且分析阶段未确认完成，保留旧图片证据和人工确认；不会代用户接受新免责声明。启用后所有图片均需要人工确认，算法自身Orientation.needs_review仅代表模型未决。取消/重启中断不能冒充已完成分析。

Job 字段：`{id,plan_id,kind:"analyze"|"generate"|"generic_generate",status:"queued"|"running"|"completed"|"completed_with_errors"|"cancelled"|"interrupted"|"failed",threads:整数1..10,created_at,updated_at,total,done,current,error:null|string,results:[]}`。
结果字段：`{voucher_id,folder,status:"completed"|"skipped"|"failed",output_path:null|string,pages,error:null|string,needs_review:boolean}`。
服务重启将残留运行任务标为 interrupted，可以重试；方向选择、计划、结果持久保存在启动参数指定 state-dir。

新任务启动时快照全局线程设置，旧任务缺少 `threads` 时默认1。同一时刻仍只允许一个活动任务。方向识别按图片并行，每个工作线程使用独立模型实例；单模型内部推理线程为1，避免嵌套扩张。PDF生成按凭证并行，单份PDF页序不变，`results`按原凭证顺序排列。实际工作线程不超过设置上限与可处理项目数；线程数不是进程全部线程的承诺，也不代表等比例加速。

并行任务的 `done` 汇总已处理图片或文档页数，不因完成顺序而回退或超过 `total`。取消后停止领取后续项目，已经提交的PDF仍计入结果和报告；各凭证事务独立，重试仍须核验来源和输出指纹。并行设置不免除同意、识别完成和人工确认门禁。

UI：中文。资料选择→可选识别（说明并勾选同意）→识别→人工核对修正→生成；未启用可直接生成。review_count为启用时所有未人工确认的图片数，禁用时为0；这不是模型未决率或正确率。OCR字符分数只作证据，PDF默认不做方向识别。支持单凭证生成、历史、进度/取消/重试/固定报告。
