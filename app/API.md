# 本地 API 契约

当前产品规则见[批量合并规格](../openspec/specs/batch-merge/spec.md)。Tauri 独立窗口加载随机端口的本地服务，HTTP仅监听127.0.0.1。首次URL fragment含token，页面存入sessionStorage后移除fragment；所有 `/api` 请求带 `X-Session-Token`，校验回环Host和同源Origin，拒绝跨站请求。JSON业务错误为 `{error:"中文原因"}`；认证失败403，退出期间写请求503，格式错误由框架拒绝。缩略图/原件通过认证fetch取得，不把token放查询参数。

`/` 是新批量合并界面，`/legacy`、`/generic` 仅作0.5.x兼容。静态文件、PDF.js主模块/worker、字体、CMap与WASM资源内嵌；未知静态路径404，无运行时CDN。关闭桌面时禁止新写操作、取消并等待活动任务和原生保存完成，再释放IPC/目录锁。

## 批量合并（0.6.0）

以下路径均以 `/api/batch` 为前缀，文件系统路径使用真实绝对路径：

- POST `/initialize` `{root,level?:0,exclude_rules?:string}` → Initialization。目标选择后自动调用，排除后只读metadata，不解码/哈希；提供真实层级及完整分组预览。level0递归一份，更深层级保留浅层直属文件组，安全上限256。
- POST `/index/inspect` `{path,sheet?,header_row?}` → `{path,sha256,sheets,sheet,sheet_error,header_row,columns,preview_rows,suggested_index_column,suggested_name_column}`。默认第一个工作表，不跨表补匹配；表头行为Excel的1起始行，列索引是0起始绝对列。首表为空/不可读仍返回其他工作表信息供选择，`sheet_error`解释当前问题。
- POST `/scan` `{root,level:0,exclude_rules:string,index?:Config,sort?:Sort,output_names?:[{relative_path,output_name}]}` → 完整Plan（旧同步接口，兼容保留）。Config为 `{path,sheet,index_column,name_column,header_row,match_rule:"auto"|"exact"|"voucher_code",naming:"index"|"folder_index",duplicates:"fallback"|"first"}`；新请求默认auto/folder_index/first。新UI固定重复取首条，无冲突策略下拉；显式旧fallback字段仍可解析，但新preview/scan/scans统一使用first；历史已存Evidence/输出名称不追改。Sort为 `{mode:"name"|"modified"|"prefix"|"suffix",descending:false,separator:null|string}`，默认文件名自然升序。无效排除行忽略并列入issues，来源损坏不忽略；索引不可用时回退目录名并提示。
- POST `/preview` 与扫描相同请求 → Initialization。只读取metadata、索引与命名，不哈希/解码来源；支持output_names单行覆盖，返回当前名称冲突。索引关闭时不传index，目标名称作为默认。
- POST `/scans` 与扫描相同请求 → ScanProgress。正式扫描进入共同Job(kind=batch_scan)/cancel体系，默认4路有界校验并可见进度；未完成/取消不发布半份计划。GET `/scans/{id}` → ScanProgress；完成后按plan_id读取完整Plan。取消/重试/报告沿用公共jobs接口，扫描请求冻结为scan-request.json；重试不取页面当前配置。
- GET `/plans` → Plan[]兼容；GET `/plans?summary=true` → 轻量摘要[]（id、parent_plan_id、created_at、root、output_dir、level、group_count、file_count、pages、index、orientation_workflow、removed_file_count），不带全部源文件。新UI历史使用摘要，继续整理时GET `/plans/{id}`读取完整Plan，避免每次编辑传回所有大计划修订。
- POST `/plans/{id}/edit` `{output_dir?,groups:[{group_id,file_ids?,output_name?,sort?}],files:[{file_id,rotation_cw?,page_rotations?,reviewed?,reviewed_pages?}],remove_file_ids?:string[],restore_file_ids?:string[]}` → 派生Plan。原计划不改；移除来源只更改计划并存入removed_sources，恢复回原组，不删除原件，更新页数/来源错误/重名与成品冲突；拒绝未知ID、重复/跨集合ID。文件顺序必须完整排列当前纳入的本组ID，禁止跨组/重复/遗漏；PDF page_rotations和reviewed_pages与页数严格一致。角度仅0/90/180/270；旋转以EXIF规范后的图片或源PDF原旋转为基准，PDF整份额外角度与逐页角度相加。输出迁移后重新计算自动排除，并可恢复原先仅因旧输出子树而省略的来源。
- POST `/plans/{id}/analyze` `{accept_disclaimer:true,threads:1..10,group_ids?}` → kind=batch_analyze的Job。默认4，创建单独分析修订计划，job.plan_id指向它；只处理图片，人工设置角度优先保留。仅本次实际变角设置auto_rotation_change并进入方向详情，未变/跳过/失败只计汇总；done/total仍全量。记录当前免责声明版本与时间，人工确认不是生成门槛。
- POST `/plans/{id}/generate` `{group_ids?,threads:1..10,output_dir?}` → kind=batch_generate的Job。默认4，指定输出先派生并校验命名/路径；不传group_ids为全部。所选任一空组或真实来源/同批重名error会拒绝整个启动请求，调用方可显式仅选有效组；既存普通目标不计扫描错误，在实际合并中等待用户选择。运行中才出现的来源变化按组失败，其他组继续。不以未核对或OCR失败拒绝生成。
- POST `/jobs/{id}/output-conflict` `{conflict_id,action:"skip"|"overwrite",apply_to_batch:false|true}` → Job。仅当前batch_generate的待决ID可回答，过期/错任务/已结束拒绝；取消使用公共`/api/jobs/{id}/cancel`。Job.status仍为running、output_conflict给出唯一待决目标；多个工作线程不并列弹窗。apply仅当前job后续冲突有效，重试不继承。覆盖绑定目标SHA256后以同盘新PDF原子替换，不保留旧PDF/manifest备份，提交前发生外部变化不得沿用旧授权。
- GET `/plans/{id}/files/{file_id}?thumbnail=true` → EXIF规范、尚未应用额外旋转的图片缩略图；不传thumbnail返回已核验来源（PDF供PDF.js逐页绘制）。实际读取字节核对SHA256，禁止任意来源路径或越权ID。
- POST `/plans/{id}/open-output` → `{ok:true}`，打开注册且核验过的输出目录。
- GET `/jobs` → 新批量任务；取消、重试、报告及成品使用公共 `/api/jobs` 接口。
- GET/PATCH `/preferences` → `{orientation_threads,merge_threads}`。缺省均4，分别为整数1–10；PATCH要求完整结构，任务运行中/退出中拒绝修改。持久化到 `state-dir/batch-preferences.json`，不写旧settings.json。调用方应在新任务前保存偏好，再显式传递线程数；旧任务重试不读取新偏好。

Plan：`{schema_version:1,mode:"batch_merge",id,parent_plan_id,created_at,root,output_dir,level,sort,exclude_rules,index,index_error,groups,issues,exclusions,ignored_files,empty_directories,orientation_workflow,omitted_sources,removed_sources,scan_metrics}`。Group包含稳定id、relative_path/name、shallow、output_name/output_path、files/pages、issues/empty、rename匹配证据、sort、review_count/image_count/pdf_count。File包含稳定id、name/relative_path、kind:image|pdf、bytes/modified_ms/sha256、pages/width/height、rotation_cw/page_rotations、reviewed/reviewed_pages/manual_rotation、orientation/orientation_error/error，以及兼容默认false的pdf_has_signature、默认null的auto_rotation_change。RotationChange为`{from_cw,to_cw}`，仅实际变角有效；包含90→0，排除同角、未决/失败和手动跳过。Mapping同样记录auto_rotation_change。旧记录缺变化证据不推断为发生调整；手动改角清除自动marker。Workflow为 `{consent_version,consented_at,analysis_completed,threads}`。未核对计数与模型未决计数不同，不称为错误率或准确率。

新batch的review_count、自动待检查及unreviewed统计只计有效自动变角标记；人工/PDF的manual_rotation、page_rotations和历史reviewed字段保留，独立表示当前角度与历史核对，不混入自动调整数。

签名PDF在batch-only导入中去掉原签名验证结构，可靠的可见外观作为静态注释保留；扫描给pdf_signature警告，不阻止合并。PDF原件预览仍返回未修改来源，不代表验证过签名；报告Mapping保存pdf_has_signature。无可靠可见外观、混合普通表单等仍明确拒绝。旧签名错误快照需重新扫描，不能因升级自动变为有效计划。

Initialization：`{root,max_level,levels:[{level,folder_count,file_count,samples}],folder_count,file_count,issues,output_dir,level,groups,summary,exclusions,ignored_files,elapsed_ms,index,index_error}`。预览组为 `{relative_path,path,name,shallow,candidate_count,image_count,pdf_count,bytes,output_name,rename,issues,empty}`；summary为group_count/nonempty_groups/candidate_count/image_count/pdf_count/bytes。候选文件计数不是已解析页数。

ScanProgress：`{id,status,plan_id,phase,done,total,elapsed_ms,current_file,timings,counts,logs,error}`。timings为metadata_ms/read_ms/hash_ms/pdf_ms/image_ms/index_ms/output_ms；counts为image_files/pdf_files/source_errors/bytes_read/hash_calls。并行阶段是累计工时，不能直接相加当成elapsed_ms。logs逐项为 `{level,code,message}`。完成才返回plan_id；活动进度保存在内存，不逐文件fsync，结束/失败/取消将scan-progress.json持久化。旧人工reviewed字段仅供兼容记录，新UI不提供确认控件，也不代用户设为true。

新计划放在 `state-dir/batch-plans`。合并任务启动冻结 `reports/{job_id}/batch-plan.json`；识别任务先保存初始快照，结束或取消时再冻结最终识别状态。报告逐组冻结来源顺序、指纹、实际页码、索引证据、旋转和复核状态；successful_pages只计算成功合并页，analyzed_files使用真实已处理数，不能用仅调整项results条数代替。Job.request.voucher_ids兼容字段保存所选group ID（分析结果按file ID）。编辑生成新修订，不改旧报告。输出记录mode=batch_merge，不能冒用其他模式归属；所有既存普通同名目标均在合并时决策，覆盖经授权后原子替换且不保留旧内容副本，缺manifest不再生成“找不到PDF”的扫描错误。来源同目标、不安全路径或不可安全恢复的事务仍失败。既有历史归档和旧模式/旧未完成事务按旧协议兼容，不自动清理；新批量覆盖不创建归档。

新批量manifest可选`overwrite_authorization`记录`job_id/conflict_id/output_path/previous_sha256/approved_at/apply_to_batch/policy_applied`，绑定当次明确选择及被替换内容。缺字段旧记录默认null。大小写折叠只对应一个真实普通目标时，可对该真实文件决策并保留实际名称；若目录中存在多个大小写变体则拒绝猜测目标。授权记录不替代提交前的来源/目标指纹复核。

## Tauri 原生调用

仅实际main窗口和实际本机服务origin具备能力，其他来源与外部导航拒绝；不授予通用文件系统/任意命令能力。通过 `window.__TAURI__.core.invoke` 调用：

- `select_folder({initialPath?})`、`select_index_file({initialPath?})` → 绝对路径或null，使用原生目录/工作簿选择。
- `save_job_report({jobId})`、`save_job_output({jobId,groupId})` → `{saved,path?}`。服务根据注册任务重新核验快照/成品哈希，原生另存为取消正常返回；不允许覆盖目标。保存进行中计入退出保护。

## 0.5.x 兼容接口

以下旧规则只适用于原mode/接口，不适用于上述批量合并。旧方向复核门禁和默认1线程保持兼容，新批量合并不继承门禁。

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

Job 字段：`{id,plan_id,kind:"analyze"|"generate"|"generic_generate"|"batch_analyze"|"batch_generate"|"batch_scan",status:"queued"|"running"|"completed"|"completed_with_errors"|"cancelled"|"interrupted"|"failed",threads:整数1..10,created_at,updated_at,total,done,current,error:null|string,results:[],output_conflict:null|{id,group_id,group_name,output_path,reason},orientation_summary:null|{changed,unchanged,skipped,failed}}`。两个新增可选字段旧Job默认null；output_conflict仅batch_generate在等待同名处理时非空，任务仍运行且可取消。orientation_summary用于新batch_analyze，四项为u32，合计为实际已处理done，已处理进度独立于详情条数；取消保留已完成汇总。
结果字段：`{voucher_id,folder,status:"completed"|"skipped"|"skipped_existing"|"failed",output_path:null|string,pages,error:null|string,needs_review:boolean,rotation_change:null|{from_cw,to_cw}}`。新batch_analyze只为实际变角保存results及rotation_change，未变/跳过/失败保留汇总，原始证据仍在计划；旧无字段结果保持兼容但不臆测变化。旧skipped表示核验后的复用；新batch的skipped_existing表示用户跳过既存目标，pages=0、output_path=null，不计successful_pages也不提供该目标下载。done仍反映已处理进度，不能代指生成成功页。
服务重启将残留运行任务标为 interrupted并清除待决冲突，可以重试；方向选择、计划、结果持久保存在启动参数指定 state-dir。本批次覆盖/跳过偏好不传给重试或其他新任务。

新任务启动时快照全局线程设置，旧任务缺少 `threads` 时默认1。同一时刻仍只允许一个活动任务。方向识别按图片并行，每个工作线程使用独立模型实例；单模型内部推理线程为1，避免嵌套扩张。PDF生成按凭证并行，单份PDF页序不变，`results`按原凭证顺序排列。实际工作线程不超过设置上限与可处理项目数；线程数不是进程全部线程的承诺，也不代表等比例加速。

并行任务的 `done` 汇总已处理图片或文档页数，不因完成顺序而回退或超过 `total`。取消后停止领取后续项目，已经提交的PDF仍计入结果和报告；各凭证事务独立，重试仍须核验来源和输出指纹。并行设置不免除同意、识别完成和人工确认门禁。

UI：中文。资料选择→可选识别（说明并勾选同意）→识别→人工核对修正→生成；未启用可直接生成。review_count为启用时所有未人工确认的图片数，禁用时为0；这不是模型未决率或正确率。OCR字符分数只作证据，PDF默认不做方向识别。支持单凭证生成、历史、进度/取消/重试/固定报告。
