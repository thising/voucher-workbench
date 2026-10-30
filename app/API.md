# 本地 API 契约（实现接口）

唯一业务规则见 `../openspec/specs/voucher-pdf/spec.md`。服务只监听 127.0.0.1；首次打开的 URL fragment 含 token，前端存 sessionStorage 后移除 fragment。所有 /api 请求带 `X-Session-Token`，拒绝异源请求。请求/响应 JSON；错误 `{error: "中文原因"}`。文件缩略图/原件也需用 fetch + blob（不得 token 放 URL）。静态文件内嵌，离线无 CDN。

- GET `/api/status` → `{version, orientation_model, orientation_disclaimer_version, model_ready, model_error, running_jobs}`；方向结果仅在 model 与 orientation_model 精确相等时为当前版本。
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
- POST `/api/jobs/{id}/retry` → 新 Job；方向识别跳过已识别页，生成跳过经 hash 验证完成项，继续失败/取消项。
- GET `/api/jobs/{id}/report` → JSON处理报告（浏览器下载），含来源、排序、旋转、页码映射、sha256、异常。
- POST `/api/plans/{id}/open-output` body `{item:string}` → `{ok:true}`，用系统文件管理器打开该分项输出文件夹。
- GET `/api/jobs/{id}/outputs/{voucher_id}` → 已完成 PDF（fetch blob下载/预览）。

Plan.requested_root 保留原先单分项/批次选择，换表重扫使用此字段；Plan.root 是规范后的批次目录。SourceFile.sha256 在扫描时确定，方向和复核绑定此内容指纹。任务报告使用当次固定快照；旧任务的输出文件若已更新，下载提示使用最新任务，历史归档仍保留。工具拥有且未被外部修改的旧输出，在方向改变后归档 `.history` 再生成；其他已存在文件仍拒绝覆盖。

Plan.orientation_workflow为`{enabled,consent_version,consented_at,analysis_completed}`，manifest同步保存当次值。缺字段旧计划默认enabled、无同意记录且分析阶段未确认完成，保留旧图片证据和人工确认；不会代用户接受新免责声明。启用后所有图片均需要人工确认，算法自身Orientation.needs_review仅代表模型未决。取消/重启中断不能冒充已完成分析。

Job 字段：`{id,plan_id,kind:"analyze"|"generate",status:"queued"|"running"|"completed"|"completed_with_errors"|"cancelled"|"interrupted"|"failed",created_at,updated_at,total,done,current,error:null|string,results:[]}`。
结果字段：`{voucher_id,folder,status:"completed"|"skipped"|"failed",output_path:null|string,pages,error:null|string,needs_review:boolean}`。
服务重启将残留运行任务标为 interrupted，可以重试；方向选择、计划、结果持久保存在启动参数指定 state-dir。

UI：中文。资料选择→可选识别（说明并勾选同意）→识别→人工核对修正→生成；未启用可直接生成。review_count为启用时所有未人工确认的图片数，禁用时为0；这不是模型未决率或正确率。OCR字符分数只作证据，PDF默认不做方向识别。支持单凭证生成、历史、进度/取消/重试/固定报告。
