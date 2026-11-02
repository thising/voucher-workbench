'use strict';

const $ = id => document.getElementById(id);
function el(tag, className = '', text = '') {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = String(text);
  return node;
}
const fragment = new URLSearchParams(location.hash.slice(1));
const token = fragment.get('token') || sessionStorage.getItem('voucher-session-token') || '';
if (fragment.has('token')) {
  sessionStorage.setItem('voucher-session-token', token);
  history.replaceState(null, '', location.pathname + location.search);
}
const DEFAULT_RULES = '# 一行一条；以 # 开头的行为注释\n**/indexImg/**\n**/index.html\n**/.DS_Store\n**/._*\n**/Thumbs.db';
const active = job => job && ['queued', 'running'].includes(job.status);
const batchJob = job => job && ['batch_analyze', 'batch_generate'].includes(job.kind);
const labels = {queued:'等待处理',running:'正在处理',completed:'已完成',completed_with_errors:'部分完成',cancelled:'已取消',interrupted:'意外中断',failed:'处理失败',skipped:'已核验，复用结果'};
const fmt = value => Number(value || 0).toLocaleString('zh-CN');
const basename = path => String(path || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();
const shortTime = value => value ? new Date(value).toLocaleString('zh-CN', {hour12:false,month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}) : '';
const prettyPath = value => String(value || '').replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\/, '');
const samePath = (a, b) => prettyPath(a).replace(/[\\/]+$/, '') === prettyPath(b).replace(/[\\/]+$/, '');
const S = {
  ready:false, status:null, operations:0, operationLabel:'', plan:null, plans:[], jobs:[], job:null,
  initialization:null, indexInspection:null, groupId:null, fileId:null, sourcePage:1, selected:new Set(),
  drafts:new Map(), sourceDirty:false, preparationCollapsed:false, orientationThreads:4, mergeThreads:4,
  previewEpoch:0, renderEpoch:0, previewKey:null, previewUrl:null, image:null, pdf:null, pdfTask:null,
  pdfRender:null, pdfModule:null, zoom:1, fit:true, polling:null, syncing:false, pollCount:0,
  toastTimer:null, mergeRetry:null, orientationRetry:null, historyLoading:false,
  browser:{kind:'folder',target:'root-path',path:'',parent:null,selected:null}, closeAfterSave:false
};
const selectedGroup = () => S.plan?.groups.find(g => g.id === S.groupId);
const selectedFile = () => selectedGroup()?.files.find(f => f.id === S.fileId);
const busy = () => S.operations > 0 || active(S.job) || S.jobs.some(active);
const hasDrafts = () => S.drafts.size > 0;
const selectedGroups = () => S.plan?.groups.filter(g => S.selected.has(g.id)) || [];
const allFiles = plan => (plan?.groups || []).flatMap(g => g.files);
const hasErrors = group => group.empty || !group.files.length || (group.issues || []).some(i => i.severity === 'error') || group.files.some(f => f.error);
const hasPlanErrors = () => (S.plan?.issues || []).some(i => i.severity === 'error');
const imageFile = file => file?.kind === 'image';
const pdfFile = file => file?.kind === 'pdf';
function pdfReviewedPages(file) { return Array.from({length:file.pages}, (_, i) => Boolean(file.reviewed_pages?.[i])); }
function autoTouched(file) { return imageFile(file) && Boolean(file.orientation || file.orientation_error); }
function pendingReview(file) {
  if (imageFile(file)) return (autoTouched(file) || file.manual_rotation) && !file.reviewed ? 1 : 0;
  if (!pdfFile(file)) return 0;
  return pendingPdfPages(file) > 0 ? 1 : 0;
}
function pendingPdfPages(file) { return pdfReviewedPages(file).filter((reviewed, i) => !reviewed && (Number(file.rotation_cw || 0) !== 0 || Number(file.page_rotations?.[i] || 0) !== 0)).length; }
function fileStatus(file) {
  if (file.error) return {tone:'error',label:'来源有问题',symbol:'!'};
  if (file.orientation_error && !file.reviewed) return {tone:'error',label:'识别失败 · 待人工检查',symbol:'!'};
  if (imageFile(file)) {
    if (file.reviewed) return {tone:'reviewed',label:file.orientation_error ? '已人工核对 · 原识别失败' : '已人工核对',symbol:'✓'};
    if (autoTouched(file)) return {tone:'review',label:'自动处理后待核对',symbol:'◷'};
    if (file.manual_rotation) return {tone:'review',label:'手动调整 · 尚未确认',symbol:'◷'};
    return {tone:'original',label:'原图方向',symbol:'—'};
  }
  const reviews = pdfReviewedPages(file).filter(Boolean).length;
  if (reviews === file.pages && file.pages) return {tone:'reviewed',label:`已核对 ${fmt(reviews)} / ${fmt(file.pages)} 页`,symbol:'✓'};
  if (pendingReview(file)) return {tone:'review',label:`方向已调整 · ${fmt(pendingPdfPages(file))} 页待核对`,symbol:'◷'};
  if (reviews) return {tone:'original',label:`已核对 ${fmt(reviews)} / ${fmt(file.pages)} 页`,symbol:'✓'};
  return {tone:'original',label:'保留原 PDF 页向',symbol:'—'};
}
function groupTone(group) {
  if (hasErrors(group)) return 'error';
  if (group.files.some(f => fileStatus(f).tone === 'error')) return 'error';
  if (group.files.some(f => pendingReview(f))) return 'review';
  if (group.files.some(f => imageFile(f) ? f.reviewed : pdfReviewedPages(f).some(Boolean))) return 'reviewed';
  return 'original';
}
function groupDraft(group) {
  return {group_id:group.id,file_ids:group.files.map(f => f.id),output_name:group.output_name,...S.drafts.get(group.id)};
}
function setDraft(group, changes) {
  S.drafts.set(group.id, {group_id:group.id,...S.drafts.get(group.id),...changes}); controls();
}
function orderedFiles(group) {
  const map = new Map(group.files.map(f => [f.id, f]));
  return groupDraft(group).file_ids.map(id => map.get(id)).filter(Boolean);
}
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {}); headers.set('X-Session-Token', token);
  if (options.body && typeof options.body !== 'string') {
    options.body = JSON.stringify(options.body); headers.set('Content-Type', 'application/json');
  }
  let response;
  try { response = await fetch('/api' + path, {...options,headers,cache:'no-store'}); }
  catch (_) { throw new Error('无法连接 PDF工作台，请确认应用仍在运行，必要时从启动入口重新打开。'); }
  if (!response.ok) {
    let message;
    try { message = (await response.json()).error; } catch (_) { /* not a JSON error */ }
    throw new Error(message || `操作未完成（${response.status}），请检查输入或重新打开应用。`);
  }
  return options.blob ? response.blob() : response.json();
}
function nativeInvoke() { return window.__TAURI__?.core?.invoke; }
function showError(error) {
  $('global-alert').textContent = error.message || String(error); $('global-alert').hidden = false;
}
function action(fn) {
  return async event => {
    $('global-alert').hidden = true;
    try { await fn(event); } catch (error) { showError(error); } finally { controls(); }
  };
}
async function operation(label, fn) {
  S.operations++; S.operationLabel = label; controls();
  try { return await fn(); } finally { S.operations--; controls(); }
}
function toast(message) {
  clearTimeout(S.toastTimer); $('toast').textContent = message; $('toast').hidden = false;
  S.toastTimer = setTimeout(() => { $('toast').hidden = true; }, 4300);
}
function setView(view) {
  $('batch-view').hidden = view !== 'batch'; $('history-view').hidden = view !== 'history';
  $('tab-batch').classList.toggle('active', view === 'batch'); $('tab-history').classList.toggle('active', view === 'history');
  $('tab-batch').setAttribute('aria-selected', String(view === 'batch')); $('tab-history').setAttribute('aria-selected', String(view === 'history'));
  if (view === 'history') refreshHistory().catch(showError);
}
function setPreparationCollapsed(collapsed) {
  S.preparationCollapsed = collapsed; $('source-form').hidden = collapsed; $('source-summary').hidden = !collapsed;
  $('toggle-preparation').textContent = collapsed ? '更改设置' : '收起设置';
  $('source-form').closest('.preparation').classList.toggle('collapsed', collapsed);
}

function controls() {
  const locked = busy() || !S.ready;
  $('source-form').querySelectorAll('input,select,textarea,button').forEach(node => { node.disabled = locked; });
  $('merge-level').disabled = locked || !S.initialization;
  $('initialize-folder').disabled = locked || !$('root-path').value.trim();
  $('inspect-index').disabled = locked || !$('index-path').value.trim();
  for (const id of ['index-column','name-column']) $(id).disabled = locked || !S.indexInspection?.columns?.length;
  $('scan-button').disabled = locked || !$('root-path').value.trim();
  $('scan-button').textContent = S.operations ? S.operationLabel || '正在处理…' : '扫描资料 →';
  $('source-dirty-note').hidden = !S.sourceDirty || !S.plan;
  $('save-edits').hidden = !hasDrafts(); $('save-edits').disabled = locked || S.sourceDirty;
  $('draft-notice').hidden = !hasDrafts();
  const selected = selectedGroups();
  $('merge-button').disabled = locked || !selected.length || S.sourceDirty || hasDrafts();
  $('merge-button').textContent = selected.length ? `执行合并 · ${fmt(selected.length)} 份` : '执行合并';
  const imageCount = selected.reduce((n, g) => n + g.files.filter(f => imageFile(f) && !f.error).length, 0);
  $('auto-orient').disabled = locked || !imageCount || S.sourceDirty || hasDrafts();
  $('auto-orient').title = !imageCount ? '所选单元没有可识别的图片；PDF可在预览中逐页手动旋转。' : '可选：识别后提示人工核对，不阻止后续合并。';
  $('start-orientation').disabled = locked || !$('orientation-consent').checked || !imageCount || !S.status?.model_ready;
  $('orientation-threads').disabled = locked || Boolean(S.orientationRetry);
  $('merge-threads').disabled = locked || Boolean(S.mergeRetry);
  $('select-all-groups').disabled = locked || S.sourceDirty;
  const groups = S.plan?.groups.filter(g => !g.empty && g.files.length) || [];
  $('select-all-groups').checked = groups.length > 0 && groups.every(g => S.selected.has(g.id));
  $('select-all-groups').indeterminate = selected.length > 0 && selected.length < groups.length;
  $('group-list').querySelectorAll('input[type=checkbox]').forEach(node => {
    const g = S.plan.groups.find(g => g.id === node.value); node.disabled = locked || S.sourceDirty || !g || g.empty || !g.files.length;
  });
  $('group-list').querySelectorAll('[data-edit]').forEach(node => { node.disabled = locked || S.sourceDirty; });
  for (const id of ['file-sort','sort-direction','sort-separator','apply-sort']) $(id).disabled = locked || S.sourceDirty || !selectedGroup();
  $('file-list').querySelectorAll('[data-edit]').forEach(node => { node.disabled = locked || S.sourceDirty || node.dataset.limit === 'true'; });
  $('file-list').querySelectorAll('[draggable]').forEach(node => { node.draggable = !locked && !S.sourceDirty; });
  const file = selectedFile();
  const canEditFile = Boolean(file && !file.error && !locked && !S.sourceDirty && !hasDrafts());
  for (const id of ['rotate-left','rotate-right','rotate-half','reset-rotation','rotation-scope','reviewed-file']) $(id).disabled = !canEditFile;
  $('rotation-scope').disabled = !canEditFile || !pdfFile(file);
  $('previous-source-page').disabled = !file || !pdfFile(file) || S.sourcePage <= 1;
  $('next-source-page').disabled = !file || !pdfFile(file) || S.sourcePage >= file.pages;
  $('source-page-number').disabled = !file || !pdfFile(file);
  for (const id of ['zoom-in','zoom-out','fit-preview']) $(id).disabled = !file || Boolean(file.error);
  $('retry-job').disabled = locked || S.sourceDirty || hasDrafts();
  $('start-merge').disabled = locked || !S.plan || !selected.length || hasPlanErrors() || selected.some(hasErrors) || !samePath($('output-path').value.trim(), S.plan.output_dir);
  $('choose-output').disabled = locked || Boolean(S.mergeRetry);
  $('output-path').disabled = locked || Boolean(S.mergeRetry);
  $('check-output').disabled = locked;
  $('deselect-errors').hidden = !selected.some(hasErrors);
  $('deselect-errors').disabled = locked;
  $('summary-selected').textContent = `已选 ${fmt(selected.length)}`;
  $('group-selection-count').textContent = `已选择 ${fmt(selected.length)} / ${fmt(groups.length)}`;
  const external = S.jobs.find(job => active(job) && !batchJob(job));
  $('external-task').hidden = !external;
  $('external-task').textContent = external ? '旧版任务仍在处理中，完成或取消后才能开始新任务。当前任务进度与已有记录将保留。' : '';
}
function changedSource() {
  if (S.plan) S.sourceDirty = true;
  if (S.initialization && !samePath($('root-path').value.trim(), S.initialization.root)) {
    S.initialization = null; $('level-hint').textContent = '文件夹已更改，请重新读取实际目录层级。';
  }
  controls();
}
function issueList(issues) {
  const list = el('ul', 'issue-list');
  for (const issue of issues) {
    const row = el('li', issue.severity === 'error' ? 'error' : '', issue.message);
    if (issue.path) row.append(el('small', '', ` · ${prettyPath(issue.path)}`));
    list.append(row);
  }
  return list;
}
async function initializeFolder() {
  const root = $('root-path').value.trim(); if (!root) throw new Error('请先选择待处理文件夹。');
  const previous = S.initialization && samePath(S.initialization.root, root) ? Number($('merge-level').value || 0) : 0;
  return operation('正在读取文件夹层级…', async () => {
    const result = await api('/batch/initialize', {method:'POST',body:{root}});
    S.initialization = result; $('root-path').value = prettyPath(result.root);
    renderLevels(previous); $('initialization-note').hidden = false;
    const count = result.file_count ?? result.supported_files ?? result.total_files;
    const summary = count === undefined ? `已读取 ${Number(result.max_level || 0) + 1} 个可选层级。` : `已读取目录结构：${fmt(count)} 个候选文件，${Number(result.max_level || 0) + 1} 个可选层级。`;
    const warnings = result.issues?.length ? ` ${fmt(result.issues.length)} 条目录提示，扫描后可查看详情。` : '';
    $('initialization-note').textContent = summary + ' 此处尚未应用排除规则。' + warnings;
    changedSource(); return result;
  });
}
function renderLevels(preferred = 0) {
  const select = $('merge-level'); select.replaceChildren();
  const levels = S.initialization?.levels || Array.from({length:Number(S.initialization?.max_level || 0) + 1}, (_, level) => ({level}));
  for (const info of levels) {
    const option = el('option', '', info.level === 0 ? '当前文件夹 · 递归合并为一份' : `第 ${info.level} 级文件夹 · 每个目录一份`);
    option.value = info.level; select.append(option);
  }
  if (!select.options.length) { const option = el('option', '', '当前文件夹 · 递归合并为一份'); option.value = 0; select.append(option); }
  select.value = [...select.options].some(option => Number(option.value) === preferred) ? String(preferred) : '0';
  renderLevelHint(); controls();
}
function renderLevelHint() {
  if (!S.initialization) return;
  const level = Number($('merge-level').value);
  const info = S.initialization.levels?.find(item => Number(item.level) === level);
  const samples = (info?.samples || []).slice(0, 2).map(value => typeof value === 'string' ? value : value.path || value.name).filter(Boolean);
  if (!level) {
    $('level-hint').textContent = `将“${basename(S.initialization.root)}”内及其全部子文件夹的文件合并为一个 PDF。`;
  } else {
    const count = info?.folder_count === undefined ? '' : `共 ${fmt(info.folder_count)} 个目标目录；`;
    const example = samples.length ? `例如“${samples.join('”“')}”。` : '';
    $('level-hint').textContent = `将每个第 ${level} 级目录内（含子目录）的文件分别合并为一份 PDF。${count}${example}更浅层的直属文件另列单元，不会遗漏。`;
  }
}
async function inspectIndex({sheet, headerRow} = {}) {
  const path = $('index-path').value.trim();
  if (!path) { S.indexInspection = null; $('index-settings').hidden = true; changedSource(); return null; }
  return operation('正在读取重命名索引…', async () => {
    const body = {path}; if (sheet) body.sheet = sheet;
    if (headerRow !== undefined) {
      if (!Number.isInteger(headerRow) || headerRow < 1) throw new Error('表头行应为从 1 开始的整数。');
      body.header_row = headerRow;
    }
    const data = await api('/batch/index/inspect', {method:'POST',body});
    S.indexInspection = data; $('index-path').value = prettyPath(data.path);
    $('index-settings').hidden = false;
    $('index-sheet-warning').hidden = !data.sheet_error;
    $('index-sheet-warning').textContent = data.sheet_error ? `${data.sheet_error}。可改选其他工作表；也可清空索引文件，按文件夹名命名。` : '';
    $('index-sheet').replaceChildren(...data.sheets.map(info => { const option = el('option', '', info.name); option.value = info.name; return option; }));
    $('index-sheet').value = data.sheet; $('header-row').value = data.header_row;
    for (const id of ['index-column','name-column']) {
      $(id).replaceChildren(...data.columns.map(column => {
        const option = el('option', '', `${column.letter} · ${column.name || '未命名列'}`); option.value = column.index; return option;
      }));
    }
    $('index-column').value = String(data.suggested_index_column ?? data.columns[0]?.index ?? '');
    $('name-column').value = String(data.suggested_name_column ?? data.columns[1]?.index ?? data.columns[0]?.index ?? '');
    renderIndexPreview(data); changedSource(); return data;
  });
}
function renderIndexPreview(data) {
  if (!data.columns.length) { $('index-preview-table').replaceChildren(el('p','help','当前工作表没有可预览的列，请改选工作表或调整表头行。')); return; }
  const table = el('table'); const head = el('thead'); const headRow = el('tr'); headRow.append(el('th', '', '行号'));
  for (const column of data.columns) headRow.append(el('th', '', `${column.letter} · ${column.name || '未命名列'}`));
  head.append(headRow); table.append(head); const body = el('tbody');
  for (const record of (data.preview_rows || []).slice(0, 8)) {
    const row = el('tr'); row.append(el('td', '', record.row));
    for (const column of data.columns) {
      const value = record.values[column.index] ?? ''; const cell = el('td', '', value); cell.title = value; row.append(cell);
    }
    body.append(row);
  }
  table.append(body); $('index-preview-table').replaceChildren(table);
}
function indexRequest() {
  const path = $('index-path').value.trim(); if (!path) return null;
  if (!S.indexInspection || !samePath(S.indexInspection.path, path)) throw new Error('重命名索引已更改，请先读取工作表与列。');
  if (S.indexInspection.sheet_error) throw new Error(`${S.indexInspection.sheet_error}。请改选其他工作表，或清空索引文件后扫描。`);
  const index = Number($('index-column').value); const name = Number($('name-column').value);
  if ($('index-column').value === '' || $('name-column').value === '') throw new Error('请选择索引列与文件名列。');
  if (Number($('header-row').value) !== Number(S.indexInspection.header_row)) throw new Error('表头行已更改，请先点击“读取列”再扫描。');
  return {path,sheet:$('index-sheet').value,index_column:index,name_column:name,header_row:Number($('header-row').value),match_rule:$('match-rule').value,naming:$('naming-mode').value,duplicates:$('duplicate-mode').value};
}
async function scan() {
  if (busy()) return;
  return operation('正在扫描资料…', async () => {
    if (!S.initialization || !samePath(S.initialization.root, $('root-path').value.trim())) await initializeFolder();
    if ($('index-path').value.trim() && (!S.indexInspection || !samePath(S.indexInspection.path, $('index-path').value.trim()))) await inspectIndex();
    const request = {root:$('root-path').value.trim(),level:Number($('merge-level').value),exclude_rules:$('exclude-rules').value,index:indexRequest(),sort:{mode:'name',descending:false}};
    const plan = await api('/batch/scan', {method:'POST',body:request});
    acceptPlan(plan, {syncForm:true,collapse:true,preserveSelection:false}); S.job = null; $('job-panel').hidden = true;
    await refreshHistory(false); toast('扫描完成，请在三栏中核对合并单元、文件顺序和方向。');
  });
}
function acceptPlan(plan, {syncForm = false, collapse = S.preparationCollapsed, preserveSelection = true} = {}) {
  const previous = S.plan; const oldGroup = selectedGroup(); const oldFile = selectedFile();
  const oldIds = new Set(S.selected); const oldPaths = new Set((previous?.groups || []).filter(g => oldIds.has(g.id)).map(g => g.relative_path));
  S.plan = plan; S.drafts.clear(); S.sourceDirty = false;
  S.selected = new Set(plan.groups.filter(g => !g.empty && g.files.length && (!previous || !preserveSelection || oldIds.has(g.id) || oldPaths.has(g.relative_path))).map(g => g.id));
  S.groupId = plan.groups.find(g => g.id === oldGroup?.id)?.id || plan.groups.find(g => g.relative_path === oldGroup?.relative_path)?.id || plan.groups.find(g => g.files.length)?.id || plan.groups[0]?.id || null;
  const currentGroup = selectedGroup();
  S.fileId = currentGroup?.files.find(f => f.id === oldFile?.id)?.id || currentGroup?.files[0]?.id || null;
  if (S.fileId !== oldFile?.id) resetSourceView();
  S.sourcePage = Math.min(Math.max(1, S.sourcePage), selectedFile()?.pages || 1);
  if (syncForm) {
    $('root-path').value = prettyPath(plan.root); $('merge-level').value = plan.level;
    if (!S.initialization || !samePath(S.initialization.root, plan.root)) {
      S.initialization = null; const option = el('option','',plan.level ? `第 ${plan.level} 级（历史配置）` : '当前文件夹（历史配置）'); option.value = plan.level;
      $('merge-level').replaceChildren(option); $('level-hint').textContent = '已载入历史分组设置；如需更改级别，请重新读取实际文件夹层级。'; $('initialization-note').hidden = true;
    } else { $('merge-level').value = String(plan.level); renderLevelHint(); }
    $('exclude-rules').value = plan.exclude_rules ?? DEFAULT_RULES;
    const index = plan.index?.request || plan.index?.config || plan.index;
    $('index-path').value = prettyPath(index?.path || '');
    if (index?.path) {
      const inspected = S.indexInspection && samePath(S.indexInspection.path, index.path) && S.indexInspection.sheet === index.sheet && Number(S.indexInspection.header_row) === Number(index.header_row) && S.indexInspection.columns.length > 0;
      if (!inspected) S.indexInspection = {path:index.path,sheet:index.sheet,header_row:index.header_row,columns:[],sheets:[]};
      $('index-settings').hidden = false;
      $('index-sheet-warning').hidden = !S.indexInspection.sheet_error;
      if (!inspected) $('index-sheet').replaceChildren(el('option', '', index.sheet));
      $('index-sheet').value = index.sheet;
      $('header-row').value = index.header_row || 1;
      for (const [id, value] of [['index-column', index.index_column], ['name-column', index.name_column]]) {
        if (!inspected) { const option = el('option', '', `第 ${Number(value) + 1} 列`); option.value = value; $(id).replaceChildren(option); }
        $(id).value = String(value);
      }
      $('match-rule').value = index.match_rule || 'auto'; $('naming-mode').value = index.naming || 'index'; $('duplicate-mode').value = index.duplicates || 'fallback';
      if (!inspected) $('index-preview-table').replaceChildren(el('p','help','历史配置已载入；更换工作表或列前可重新读取索引。'));
    } else { S.indexInspection = null; $('index-settings').hidden = true; $('index-sheet-warning').hidden = true; }
  }
  $('source-summary').textContent = `${prettyPath(plan.root)} · ${plan.level ? `第 ${plan.level} 级分组` : '当前文件夹'} · ${fmt(plan.groups.length)} 个单元`;
  $('welcome').hidden = true; $('workspace').hidden = false; $('toggle-preparation').hidden = false;
  setPreparationCollapsed(collapse); renderPlan(); controls();
}
function renderPlan() {
  const p = S.plan; if (!p) return;
  const files = allFiles(p); $('summary-groups').textContent = fmt(p.groups.length); $('summary-files').textContent = fmt(files.length);
  $('summary-pages').textContent = fmt(files.reduce((n, f) => n + Number(f.pages || 0), 0));
  $('summary-review').textContent = fmt(files.reduce((n, f) => n + pendingReview(f), 0));
  $('groups-count').textContent = fmt(p.groups.length);
  const issues = [...(p.issues || []), ...p.groups.flatMap(g => (g.issues || []).map(i => ({...i,path:i.path || g.relative_path || g.name})))];
  if (p.index_error) issues.unshift({severity:'warning',message:`索引未启用：${p.index_error}`});
  $('summary-issues').textContent = fmt(issues.length);
  $('scan-issues').replaceChildren(issues.length ? issueList(issues) : el('p', 'help', '未发现扫描问题。生成前仍会核验来源和输出归属。'));
  const box = $('scan-exclusions'); box.replaceChildren();
  for (const rule of p.exclusions || []) {
    const detail = el('details', 'exclusion-detail');
    const title = rule.kind === 'output' ? '输出目录自动排除' : rule.kind === 'state' ? '处理记录自动排除' : '排除规则';
    detail.append(el('summary', '', `${title}：${rule.rule} · ${fmt(rule.matches?.length)} 个命中项`));
    const list = el('ul'); list.append(...(rule.matches || []).map(path => el('li', '', path))); detail.append(list); box.append(detail);
  }
  if (p.ignored_files) box.append(el('p', 'help', `跳过 ${fmt(p.ignored_files)} 个非支持格式或辅助文件。被排除目录只列目录项，不估算内部文件数量。`));
  if (p.empty_directories?.length) {
    const detail = el('details', 'exclusion-detail'); detail.append(el('summary', '', `空目录 · ${fmt(p.empty_directories.length)}`));
    const list = el('ul'); list.append(...p.empty_directories.map(path => el('li', '', path || '资料根目录'))); detail.append(list); box.append(detail);
  }
  renderGroups(); renderFiles(); renderPreview().catch(showError);
}
function renameDescription(group) {
  const rename = group.rename; if (!rename) return '';
  if (rename.message) return rename.message;
  return {folder:'按文件夹名命名',matched:'索引已匹配',unmatched:'未匹配索引，使用文件夹名',conflict:'索引名称冲突，使用文件夹名',empty:'索引名称为空，使用文件夹名'}[rename.status] || '';
}
function renderGroups() {
  if (!S.plan) return;
  const box = $('group-list'); const scroll = box.scrollTop; box.replaceChildren();
  const search = $('group-search').value.trim().toLocaleLowerCase();
  for (const group of S.plan.groups.filter(g => `${g.relative_path} ${g.output_name} ${groupDraft(g).output_name}`.toLocaleLowerCase().includes(search))) {
    const tone = groupTone(group); const row = el('div', `group-row tone-${tone}${group.id === S.groupId ? ' active' : ''}`); row.setAttribute('role','listitem');
    const main = el('div', 'group-main'); const check = document.createElement('input'); check.type = 'checkbox'; check.value = group.id; check.checked = S.selected.has(group.id); check.setAttribute('aria-label', `选择合并单元 ${group.relative_path || group.name}`);
    check.addEventListener('change', () => { if (check.checked) S.selected.add(group.id); else S.selected.delete(group.id); controls(); });
    const open = el('button', 'group-open'); open.type = 'button'; open.title = group.relative_path || group.name;
    open.append(el('span', 'group-path', group.relative_path || basename(S.plan.root)), el('span','group-output', groupDraft(group).output_name));
    open.addEventListener('click', () => selectGroup(group.id)); main.append(check, open); row.append(main);
    const meta = el('div', 'group-meta'); meta.append(el('span','',`${fmt(group.files.length)} 文件 · ${fmt(group.pages)} 页`));
    if (group.shallow) meta.append(el('span', 'pill', '浅层直属'));
    const pending = group.files.reduce((n, f) => n + pendingReview(f), 0);
    if (pending) meta.append(el('span','pill warning',`${fmt(pending)} 待核对`));
    if (group.empty) meta.append(el('span','pill','空单元'));
    if (hasErrors(group)) meta.append(el('span','pill error','需处理问题'));
    row.append(meta);
    const naming = renameDescription(group);
    if (naming) {
      const warning = group.rename && ['unmatched','conflict','empty'].includes(group.rename.status);
      row.append(el('span', `group-status${warning ? ' warning' : ''}`, naming));
    }
    if (group.id === S.groupId) {
      const edit = el('div', 'group-rename'); const name = document.createElement('input'); name.value = groupDraft(group).output_name; name.dataset.edit = 'name'; name.setAttribute('aria-label', `输出文件名 ${group.relative_path || group.name}`); name.title = '修改名称后点击“保存顺序与名称”';
      name.addEventListener('input', () => { setDraft(group, {output_name:name.value}); open.querySelector('.group-output').textContent = name.value; });
      const prefix = el('button', 'text-button', '加原名'); prefix.type = 'button'; prefix.dataset.edit = 'name'; prefix.title = '在当前名称前加入文件夹路径，区分同名输出';
      prefix.addEventListener('click', () => {
        const base = group.relative_path ? group.relative_path.replace(/[\\/]/g, '__') : basename(S.plan.root);
        const value = `${base}-${groupDraft(group).output_name}`; name.value = value; setDraft(group,{output_name:value}); open.querySelector('.group-output').textContent = value;
      });
      edit.append(name, prefix); row.append(edit);
    }
    box.append(row);
  }
  if (!box.children.length) box.append(el('div','empty-pane',S.plan.groups.length ? '没有匹配的合并单元' : '没有可合并文件，请检查文件格式、层级和排除规则。'));
  box.scrollTop = scroll; controls();
}
function selectGroup(id) {
  if (id === S.groupId) return;
  S.groupId = id; S.fileId = selectedGroup()?.files[0]?.id || null; resetSourceView();
  renderGroups(); renderFiles(); renderPreview().catch(showError); controls();
}
function resetSourceView() {
  S.sourcePage = 1; S.fit = true; S.zoom = 1;
  $('rotation-scope').value = pdfFile(selectedFile()) ? 'page' : 'file';
}
function renderFiles() {
  const box = $('file-list'); const scroll = box.scrollTop; box.replaceChildren(); const group = selectedGroup();
  $('files-count').textContent = fmt(group?.files.length); $('selected-group-name').textContent = group?.relative_path || (group ? basename(S.plan.root) : '');
  $('selected-group-name').title = group?.relative_path || '';
  if (!group) { box.append(el('div','empty-pane','选择左栏合并单元，查看文件顺序。')); $('file-list-summary').textContent = '选择合并单元'; controls(); return; }
  const sort = groupDraft(group).sort || group.sort || S.plan.sort || {mode:'name',descending:false};
  $('file-sort').value = S.drafts.get(group.id)?.file_ids ? 'manual' : sort.mode || 'name';
  if (group.sort?.mode === 'manual') $('file-sort').value = 'manual';
  $('sort-direction').value = sort.descending ? 'desc' : 'asc'; $('sort-separator').value = sort.separator || '';
  renderSortHint(); const files = orderedFiles(group); let first = 1;
  files.forEach((file,index) => {
    const status = fileStatus(file); const row = el('div',`file-row tone-${status.tone}${file.id === S.fileId ? ' active' : ''}`); row.setAttribute('role','listitem'); row.draggable = !busy() && !S.sourceDirty;
    const line = el('div','file-line'); const position = el('span','file-position',index + 1); position.title = '拖动调整顺序';
    const select = el('button','file-select'); select.type = 'button'; select.title = file.relative_path;
    select.append(el('span','file-name',file.name), el('span','file-meta',`${imageFile(file) ? '图片' : 'PDF'} · ${fmt(file.pages)} 页 · 合并后 ${fmt(first)}–${fmt(first + Number(file.pages || 0) - 1)} 页`));
    select.addEventListener('click', () => { S.fileId = file.id; resetSourceView(); renderFiles(); renderPreview().catch(showError); controls(); });
    line.append(position,select); row.append(line); first += Number(file.pages || 0);
    const bottom = el('div','file-bottom'); bottom.append(el('span','file-review-label',`${status.symbol} ${status.label}`));
    const moves = el('div','file-moves');
    for (const [label,text,to,limit] of [['置顶','置顶',0,index === 0],['上移','↑',index - 1,index === 0],['下移','↓',index + 1,index === files.length - 1]]) {
      const button = el('button',label === '置顶' ? 'to-top' : '',text); button.type = 'button'; button.dataset.edit = 'order'; button.dataset.limit = String(limit); button.setAttribute('aria-label',`${label} ${file.name}`);
      button.addEventListener('click', () => moveFile(index,to)); moves.append(button);
    }
    bottom.append(moves); row.append(bottom);
    if (file.error) row.append(el('p','file-error',file.error));
    row.addEventListener('dragstart',event => {
      if (busy() || S.sourceDirty) { event.preventDefault(); return; }
      event.dataTransfer.setData('text/plain',`${group.id}:${file.id}`); event.dataTransfer.effectAllowed = 'move'; row.classList.add('dragging');
    });
    row.addEventListener('dragend',() => { row.classList.remove('dragging'); box.querySelectorAll('.drop-target').forEach(node => node.classList.remove('drop-target')); });
    row.addEventListener('dragover',event => { if (!busy() && !S.sourceDirty) { event.preventDefault(); row.classList.add('drop-target'); event.dataTransfer.dropEffect = 'move'; } });
    row.addEventListener('dragleave',() => row.classList.remove('drop-target'));
    row.addEventListener('drop',event => {
      event.preventDefault(); row.classList.remove('drop-target');
      const value = event.dataTransfer.getData('text/plain'); if (!value.startsWith(`${group.id}:`)) return;
      moveFile(groupDraft(group).file_ids.indexOf(value.slice(group.id.length + 1)),index);
    });
    box.append(row);
  });
  if (!files.length) box.append(el('div','empty-pane','该单元为空，不生成空 PDF。'));
  $('file-list-summary').textContent = `${fmt(files.length)} 个文件 · ${fmt(group.pages)} 页`;
  box.scrollTop = scroll; controls();
}
function renderSortHint() {
  const mode = $('file-sort').value; const segmented = ['prefix','suffix'].includes(mode); $('sort-separator').hidden = !segmented;
  $('sort-hint').textContent = mode === 'modified' ? '按修改时间排序，时间相同则按文件名自然顺序。' : segmented ? `去除扩展名后，按${mode === 'prefix' ? '第一段' : '最后一段'}自然排序；默认用 _、- 或空格分段。应用会替换本组手动顺序。` : mode === 'manual' ? '当前为手动顺序；可继续拖动、上下移动或置顶，修改后保存。' : '2 在 10 前。可拖动文件，或用上下移动、置顶按钮调整。';
}
function moveFile(from,to) {
  const group = selectedGroup(); if (!group || busy() || S.sourceDirty) return;
  const ids = [...groupDraft(group).file_ids]; if (from < 0 || to < 0 || from >= ids.length || to >= ids.length || from === to) return;
  const [id] = ids.splice(from,1); ids.splice(to,0,id); setDraft(group,{file_ids:ids,sort:{mode:'manual',descending:false}}); renderFiles(); controls();
}
async function editPlan(body) {
  if (!S.plan || busy() || S.sourceDirty) return;
  return operation('正在保存调整…', async () => {
    const plan = await api(`/batch/plans/${S.plan.id}/edit`, {method:'POST',body});
    acceptPlan(plan,{preserveSelection:true}); await refreshHistory(false); return plan;
  });
}
async function saveEdits() {
  if (!hasDrafts()) return;
  const groups = [...S.drafts.values()].map(value => {
    const {sort,...rest} = value; return rest;
  });
  await editPlan({groups}); toast('顺序与名称已保存。');
}
async function applySort() {
  const group = selectedGroup(); if (!group || $('file-sort').value === 'manual') return;
  const sort = {mode:$('file-sort').value,descending:$('sort-direction').value === 'desc',separator:$('sort-separator').value || null};
  const changed = {group_id:group.id,sort};
  if (S.drafts.has(group.id)) changed.output_name = groupDraft(group).output_name;
  const others = [...S.drafts.values()].filter(value => value.group_id !== group.id).map(({sort,...value}) => value);
  await editPlan({groups:[...others,changed]}); toast('已应用本单元的排序。');
}

function clearPreview() {
  S.previewEpoch++; S.renderEpoch++;
  if (S.previewUrl) URL.revokeObjectURL(S.previewUrl); S.previewUrl = null;
  if (S.pdfRender) { S.pdfRender.cancel(); S.pdfRender = null; }
  if (S.pdfTask) { const task = S.pdfTask; S.pdfTask = null; Promise.resolve(task.destroy()).catch(() => {}); }
  S.pdf = null; S.image = null; S.previewKey = null;
}
function previewMessage(message, error = false) {
  const box = el('div', `preview-placeholder${error ? ' error' : ''}`); box.append(el('span','','▤'),el('p','',message)); $('preview-surface').replaceChildren(box);
}
async function pdfLibrary() {
  if (!S.pdfModule) {
    S.pdfModule = import('/pdfjs/pdf.mjs').then(pdfjs => { pdfjs.GlobalWorkerOptions.workerSrc = '/pdfjs/pdf.worker.mjs'; return pdfjs; }).catch(error => { S.pdfModule = null; throw new Error(`PDF预览组件无法加载，请确认应用文件完整：${error.message}`); });
  }
  return S.pdfModule;
}
function previewFileKey(file) { return file ? `${file.id}:${file.sha256}:${file.kind}:${file.pages}` : null; }
function renderPreviewMeta() {
  const file = selectedFile(); $('preview-filename').textContent = file?.name || '选择文件以预览'; $('preview-filename').title = file?.relative_path || '';
  $('preview-kind').hidden = !file; $('preview-kind').textContent = imageFile(file) ? '图片' : 'PDF';
  $('source-page-number').value = S.sourcePage; $('source-page-number').max = file?.pages || 1; $('source-page-total').textContent = `/ ${fmt(file?.pages)}`;
  const status = file ? fileStatus(file) : null;
  $('preview-file-status').textContent = status ? `${status.symbol} ${status.label}` : '';
  $('preview-file-status').className = status ? ({review:'warning',error:'error',reviewed:'success'}[status.tone] || '') : '';
  $('rotation-scope').value = pdfFile(file) ? $('rotation-scope').value : 'file';
  $('reviewed-label').textContent = pdfFile(file) && $('rotation-scope').value === 'page' ? '当前页已人工核对' : '此文件已人工核对';
  $('reviewed-file').checked = Boolean(file && (pdfFile(file) && $('rotation-scope').value === 'page' ? file.reviewed_pages?.[S.sourcePage - 1] : pdfFile(file) ? pdfReviewedPages(file).every(Boolean) : file.reviewed));
  const angle = file ? (Number(file.rotation_cw || 0) + Number(pdfFile(file) ? file.page_rotations?.[S.sourcePage - 1] || 0 : 0)) % 360 : 0;
  $('rotation-note').textContent = angle ? `额外旋转 ${angle}°` : '旋转不改动原文件';
  $('preview-summary').textContent = !file ? '原件保留' : `${pdfFile(file) ? `第 ${fmt(S.sourcePage)} / ${fmt(file.pages)} 页` : '已应用照片自身方向信息'} · ${S.fit ? '适合窗口' : `${Math.round(S.zoom * 100)}%`}`;
  let evidence = '';
  if (file?.orientation_error) evidence = `识别未完成：${file.orientation_error}。可手动调整，也可按当前角度合并。`;
  else if (file?.orientation) evidence = `${file.manual_rotation ? '人工角度优先。' : '自动结果仅供参考。'}${file.orientation.title ? `文字证据：${file.orientation.title}。` : ''}${file.orientation.reason || ''}`;
  else if (pdfFile(file)) evidence = 'PDF保持原有页向；当前操作是在原有旋转上追加角度。自动方向识别仅处理图片。';
  else if (file) evidence = '可直接按当前方向合并，也可手动调整；勾选“已人工核对”才记录核对状态。';
  $('preview-evidence').textContent = evidence; controls();
}
async function renderPreview() {
  const file = selectedFile(); renderPreviewMeta();
  if (!file) { clearPreview(); previewMessage('选择中间栏的文件，查看页面与方向。'); return; }
  if (file.error) { clearPreview(); previewMessage(file.error, true); return; }
  const key = previewFileKey(file);
  if (S.previewKey === key && (S.image || S.pdf)) { await drawPreview(); return; }
  clearPreview(); S.previewKey = key; const epoch = S.previewEpoch;
  previewMessage('正在读取本地原件…');
  try {
    const blob = await api(`/batch/plans/${S.plan.id}/files/${file.id}${imageFile(file) ? '?thumbnail=true' : ''}`, {blob:true});
    if (epoch !== S.previewEpoch || previewFileKey(selectedFile()) !== key) return;
    if (imageFile(file)) {
      S.previewUrl = URL.createObjectURL(blob); const image = new Image(); image.alt = file.name;
      await new Promise((resolve,reject) => { image.onload = resolve; image.onerror = () => reject(new Error('图片预览无法显示，请检查源文件。')); image.src = S.previewUrl; });
      if (epoch !== S.previewEpoch) return; S.image = image;
    } else {
      const pdfjs = await pdfLibrary(); if (epoch !== S.previewEpoch) return;
      const data = new Uint8Array(await blob.arrayBuffer()); if (epoch !== S.previewEpoch) return;
      const task = pdfjs.getDocument({data,cMapUrl:'/pdfjs/cmaps/',cMapPacked:true,standardFontDataUrl:'/pdfjs/standard_fonts/',wasmUrl:'/pdfjs/wasm/',enableXfa:false});
      S.pdfTask = task;
      const document = await task.promise; if (epoch !== S.previewEpoch) return; S.pdf = document;
      if (document.numPages !== file.pages) throw new Error('PDF预览页数与扫描不符，请重新扫描。');
    }
    await drawPreview();
  } catch (error) {
    if (epoch !== S.previewEpoch) return;
    previewMessage(error.message || '无法预览此文件，请检查应用与来源文件。', true);
  }
}
async function drawPreview() {
  const file = selectedFile(); if (!file || S.previewKey !== previewFileKey(file)) return;
  const stage = $('preview-surface'); const width = Math.max(80, stage.clientWidth - 36); const height = Math.max(80, stage.clientHeight - 36);
  const angle = (Number(file.rotation_cw || 0) + Number(pdfFile(file) ? file.page_rotations?.[S.sourcePage - 1] || 0 : 0)) % 360;
  const epoch = ++S.renderEpoch;
  if (S.pdfRender) { S.pdfRender.cancel(); S.pdfRender = null; }
  if (imageFile(file) && S.image) {
    const image = S.image; const quarter = angle % 180 !== 0;
    const originalWidth = quarter ? image.naturalHeight : image.naturalWidth;
    const originalHeight = quarter ? image.naturalWidth : image.naturalHeight;
    const scale = S.fit ? Math.min(width / originalWidth, height / originalHeight, 2) : S.zoom;
    if (S.fit) S.zoom = scale;
    const page = el('div','preview-page'); page.style.width = `${Math.ceil(originalWidth * scale)}px`; page.style.height = `${Math.ceil(originalHeight * scale)}px`;
    image.style.width = `${image.naturalWidth * scale}px`; image.style.height = `${image.naturalHeight * scale}px`; image.style.transform = `translate(-50%, -50%) rotate(${angle}deg)`;
    page.append(image); stage.replaceChildren(page);
  } else if (pdfFile(file) && S.pdf) {
    try {
      const page = await S.pdf.getPage(S.sourcePage); if (epoch !== S.renderEpoch) return;
      const rotation = (page.rotate + angle) % 360; const base = page.getViewport({scale:1,rotation});
      const scale = S.fit ? Math.min(width / base.width, height / base.height, 2) : S.zoom;
      if (S.fit) S.zoom = scale;
      const view = page.getViewport({scale,rotation}); const density = Math.min(window.devicePixelRatio || 1, 2);
      const canvas = document.createElement('canvas'); canvas.width = Math.ceil(view.width * density); canvas.height = Math.ceil(view.height * density);
      canvas.style.width = `${view.width}px`; canvas.style.height = `${view.height}px`; canvas.setAttribute('aria-label', `${file.name} 第 ${S.sourcePage} 页`);
      const container = el('div','preview-page'); container.style.width = `${view.width}px`; container.style.height = `${view.height}px`; container.append(canvas);
      stage.replaceChildren(container);
      const task = page.render({canvasContext:canvas.getContext('2d',{alpha:false}),viewport:view,transform:density === 1 ? undefined : [density,0,0,density,0,0]});
      S.pdfRender = task; await task.promise;
      if (S.pdfRender === task) S.pdfRender = null;
    } catch (error) {
      if (epoch !== S.renderEpoch || error.name === 'RenderingCancelledException') return;
      throw error;
    }
  }
  renderPreviewMeta();
}
async function navigateSourcePage(page) {
  const file = selectedFile(); if (!file) return;
  const value = Math.min(Math.max(1, Math.trunc(Number(page) || 1)), file.pages || 1);
  if (value === S.sourcePage) return; S.sourcePage = value; await drawPreview(); renderPreviewMeta();
}
async function rotateFile(delta, reset = false) {
  const file = selectedFile(); if (!file || busy() || S.sourceDirty || hasDrafts()) return;
  const change = {file_id:file.id};
  if (pdfFile(file) && $('rotation-scope').value === 'page') {
    const angles = Array.from({length:file.pages}, (_, i) => Number(file.page_rotations?.[i] || 0));
    angles[S.sourcePage - 1] = reset ? (360 - Number(file.rotation_cw || 0)) % 360 : (angles[S.sourcePage - 1] + delta + 360) % 360;
    change.page_rotations = angles;
  } else {
    change.rotation_cw = reset ? 0 : (Number(file.rotation_cw || 0) + delta + 360) % 360;
    if (reset && pdfFile(file)) change.page_rotations = Array(file.pages).fill(0);
  }
  await editPlan({files:[change]}); toast('方向已保存，核对状态待您确认。');
}
async function markReviewed() {
  const file = selectedFile(); if (!file || busy() || hasDrafts()) return;
  const checked = $('reviewed-file').checked; const change = {file_id:file.id};
  if (pdfFile(file) && $('rotation-scope').value === 'page') {
    const pages = pdfReviewedPages(file); pages[S.sourcePage - 1] = checked; change.reviewed_pages = pages;
  } else change.reviewed = checked;
  try { await editPlan({files:[change]}); toast(checked ? '已记录人工核对。' : '已取消核对标记。'); }
  finally { renderPreviewMeta(); }
}
function selectedImageCount() { return selectedGroups().reduce((n, group) => n + group.files.filter(file => imageFile(file) && !file.error).length, 0); }
function openOrientation(retry = null) {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty) return;
  const count = selectedImageCount(); if (!count) throw new Error('所选单元没有可识别图片；PDF请在预览中手动调整。');
  S.orientationRetry = retry;
  $('orientation-scope-summary').textContent = `将处理所选 ${fmt(selectedGroups().length)} 个合并单元中的 ${fmt(count)} 张图片。PDF页向不自动调整。`;
  $('orientation-consent').checked = false; $('orientation-threads').value = String(retry?.threads || S.orientationThreads);
  if (!S.status?.model_ready) {
    $('orientation-scope-summary').textContent += ` 方向模型当前不可用：${S.status?.model_error || '请检查随包组件'}。可取消此步骤，按当前方向合并。`;
  }
  $('orientation-dialog').showModal(); controls();
}
async function startOrientation() {
  if (busy() || !$('orientation-consent').checked || !S.status?.model_ready) return;
  const threads = Number($('orientation-threads').value);
  await operation('正在启动方向调整…', async () => {
    if (!S.orientationRetry) await saveThreadPreferences({orientation_threads:threads});
    const path = S.orientationRetry ? `/jobs/${S.orientationRetry.id}/retry` : `/batch/plans/${S.plan.id}/analyze`;
    const job = await api(path,{method:'POST',body:{accept_disclaimer:true,threads,group_ids:selectedGroups().map(g => g.id)}});
    const plan = await api(`/batch/plans/${job.plan_id}`); acceptPlan(plan,{preserveSelection:true});
    $('orientation-dialog').close(); S.orientationRetry = null; trackJob(job); toast(`方向调整已开始 · ${job.threads || threads} 线程`);
  });
}
function openMerge(retry = null) {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty) return;
  if (!selectedGroups().length) throw new Error('请先选择要合并的单元。');
  S.mergeRetry = retry; $('merge-dialog-title').textContent = retry ? '按原任务配置继续合并' : '准备生成 PDF';
  $('output-path').value = prettyPath(S.plan.output_dir); $('merge-threads').value = String(retry?.threads || S.mergeThreads);
  renderMergeSummary(); $('merge-dialog').showModal(); controls();
}
function renderMergeSummary() {
  if (!S.plan) return;
  const groups = selectedGroups(); const files = groups.flatMap(g => g.files);
  const pages = groups.reduce((n, g) => n + Number(g.pages || 0),0); const pending = files.reduce((n,f) => n + pendingReview(f),0);
  const summary = $('merge-summary'); summary.replaceChildren();
  const line = el('p'); line.append(document.createTextNode('本次生成 '),el('strong','',fmt(groups.length)),document.createTextNode(' 份 PDF，共 '),el('strong','',fmt(pages)),document.createTextNode(' 页。'));
  summary.append(line,el('p','help',`图片 ${fmt(files.filter(imageFile).length)} 张 · PDF ${fmt(files.filter(pdfFile).length)} 份 · 每个合并单元独立输出。`));
  if (S.mergeRetry) summary.append(el('p','help',`沿用原任务的计划、输出位置和 ${S.mergeRetry.threads || 4} 线程设置，已核验的成品可以复用。`));
  const warning = [];
  if (pending) warning.push(`本次有 ${fmt(pending)} 个文件的方向调整结果尚未人工核对。可以按当前角度继续合并；请在交付前检查成品内容与方向，最终由您自行确认。`);
  const autoErrors = files.filter(f => f.orientation_error).length;
  if (autoErrors) warning.push(`${fmt(autoErrors)} 张图片自动识别失败，将使用当前原向或人工角度。`);
  const renamedFallbacks = groups.filter(g => ['unmatched','conflict','empty'].includes(g.rename?.status)).length;
  if (renamedFallbacks) warning.push(`${fmt(renamedFallbacks)} 个单元使用文件夹名回退命名，详情见下方交付名称与左栏提示。`);
  $('merge-warning').hidden = !warning.length; $('merge-warning').textContent = warning.join(' ');
  const invalid = groups.filter(hasErrors);
  const errors = [];
  if (hasPlanErrors()) errors.push('资料范围存在扫描错误，请先处理后重新扫描。');
  if (invalid.length) errors.push(`${fmt(invalid.length)} 个所选单元存在错误或为空。请更改名称/输出位置，或返回取消选择有问题的单元；不会静默漏过来源。`);
  $('merge-errors').hidden = !errors.length; $('merge-errors').textContent = errors.join(' ');
  $('merge-name-list').replaceChildren(...groups.map(g => el('li','',`${g.output_name} · ${fmt(g.pages)} 页${hasErrors(g) ? ' · 需处理问题' : ''}`)));
  updateOutputDraft();
}
function updateOutputDraft() {
  const dirty = Boolean(S.plan && !samePath($('output-path').value.trim(), S.plan.output_dir));
  $('output-dirty-note').hidden = !dirty; $('check-output').hidden = !dirty; controls();
}
async function checkOutput() {
  const path = $('output-path').value.trim(); if (!path) throw new Error('请选择输出文件夹。');
  await editPlan({output_dir:path}); $('output-path').value = prettyPath(S.plan.output_dir); renderMergeSummary();
  toast('输出位置已预检，请核对当前单元、页数与名称后执行合并。');
}
async function startMerge() {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty || selectedGroups().some(hasErrors) || hasPlanErrors()) return;
  if (!samePath($('output-path').value.trim(), S.plan.output_dir)) throw new Error('输出位置已修改，请先更新预检。');
  const threads = Number($('merge-threads').value);
  await operation('正在启动合并…', async () => {
    if (!S.mergeRetry) await saveThreadPreferences({merge_threads:threads});
    const path = S.mergeRetry ? `/jobs/${S.mergeRetry.id}/retry` : `/batch/plans/${S.plan.id}/generate`;
    const job = await api(path,{method:'POST',body:{group_ids:selectedGroups().map(g => g.id),threads}});
    if (job.plan_id !== S.plan.id) acceptPlan(await api(`/batch/plans/${job.plan_id}`),{preserveSelection:true});
    $('merge-dialog').close(); S.mergeRetry = null; trackJob(job); toast(`合并已开始 · ${job.threads || threads} 线程`);
  });
}
function trackJob(job) {
  clearTimeout(S.polling); S.job = job; S.pollCount = 0;
  S.jobs = [job,...S.jobs.filter(j => j.id !== job.id)]; renderJob(); controls();
  if (active(job)) S.polling = setTimeout(pollJob,700);
}
async function pollJob() {
  if (!S.job) return;
  const id = S.job.id;
  try {
    const job = await api(`/jobs/${id}`); if (S.job?.id !== id) return;
    S.job = job; S.pollCount++; S.jobs = [job,...S.jobs.filter(j => j.id !== id)];
    if (batchJob(job) && S.plan?.id === job.plan_id && (job.kind === 'batch_analyze' || !active(job))) {
      const plan = await api(`/batch/plans/${job.plan_id}`); acceptPlan(plan,{preserveSelection:true});
    }
    renderJob(); controls();
    if (!active(job)) {
      S.polling = null; await refreshHistory(false);
      toast(job.kind === 'batch_analyze' ? '方向调整已结束，可核对修正，也可按当前结果合并。' : labels[job.status] || '处理已结束');
      return;
    }
  } catch (error) { showError(error); }
  S.polling = setTimeout(pollJob,1200);
}
function renderJob() {
  const job = S.job; if (!job) return;
  $('job-panel').hidden = false; const analysis = job.kind === 'batch_analyze';
  $('job-title').textContent = analysis ? '图片方向调整' : '批量合并进度';
  $('job-status').textContent = labels[job.status] || job.status;
  $('job-status').className = `pill${job.status === 'failed' ? ' error' : ['completed_with_errors','cancelled','interrupted'].includes(job.status) ? ' warning' : ''}`;
  $('job-threads').textContent = `${job.threads || 4} 线程`;
  $('job-current').textContent = job.current || labels[job.status] || '准备中';
  $('job-progress-text').textContent = `${fmt(job.done)} / ${fmt(job.total)}${analysis ? ' 张' : ' 页'}`;
  $('job-progress').style.width = `${job.total ? Math.min(100,Number(job.done) / Number(job.total) * 100) : 0}%`;
  $('cancel-job').hidden = !active(job); $('retry-job').hidden = active(job) || job.status === 'completed' || !batchJob(job);
  $('save-report').hidden = active(job); $('job-error').hidden = !job.error; $('job-error').textContent = job.error || '';
  const box = $('job-results'); box.replaceChildren();
  for (const result of job.results || []) {
    const resultLabel = analysis && result.status === 'skipped' ? '保留人工角度或核对结果' : labels[result.status] || result.status;
    const row = el('div','result-row'); row.append(el('span','',result.folder),el('span','',analysis ? '' : `${fmt(result.pages)} 页`),el('span',result.status === 'failed' ? 'error-text' : '',resultLabel));
    if (result.output_path && result.status !== 'failed') {
      const save = el('button','text-button','保存 PDF'); save.type = 'button'; save.addEventListener('click',action(() => saveOutput(job,result))); row.append(save);
    } else row.append(el('span'));
    if (result.error) row.append(el('div','result-error',result.error));
    else if (result.needs_review) row.append(el('div','result-error warning-text','包含尚未人工核对的方向结果，请检查成品。'));
    box.append(row);
  }
  $('open-output').hidden = analysis || !(job.results || []).some(r => r.output_path && r.status !== 'failed');
}
async function retryJob() {
  const job = S.job; if (!job || busy() || !batchJob(job) || hasDrafts() || S.sourceDirty) return;
  if (S.plan?.id !== job.plan_id) acceptPlan(await api(`/batch/plans/${job.plan_id}`),{syncForm:true,preserveSelection:false,collapse:true});
  const ids = job.request?.voucher_ids;
  S.selected = new Set(S.plan.groups.filter(g => !ids || ids.includes(g.id)).map(g => g.id)); renderGroups(); controls();
  if (job.kind === 'batch_analyze') openOrientation(job); else openMerge(job);
}
async function cancelJob() {
  if (!S.job || !active(S.job)) return;
  $('cancel-job').disabled = true;
  try { const job = await api(`/jobs/${S.job.id}/cancel`,{method:'POST'}); trackJob(job); toast('已请求取消，等待当前工作安全收尾。'); }
  finally { $('cancel-job').disabled = false; }
}
async function downloadBlob(path,name) {
  const blob = await api(path,{blob:true}); const url = URL.createObjectURL(blob);
  const link = el('a'); link.href = url; link.download = name; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url),30000);
}
async function saveReport(job = S.job) {
  if (!job) return;
  const invoke = nativeInvoke();
  if (invoke) { const result = await invoke('save_job_report',{jobId:job.id}); if (result?.saved) toast('处理报告已保存。'); }
  else await downloadBlob(`/jobs/${job.id}/report`,`合并报告-${job.id}.json`);
}
async function saveOutput(job,result) {
  const invoke = nativeInvoke();
  if (invoke) { const saved = await invoke('save_job_output',{jobId:job.id,groupId:result.voucher_id}); if (saved?.saved) toast('PDF已保存。'); }
  else await downloadBlob(`/jobs/${job.id}/outputs/${result.voucher_id}`,basename(result.output_path));
}

async function refreshHistory(render = true) {
  if (S.historyLoading) return; S.historyLoading = true;
  try {
    const [plans,jobs] = await Promise.all([api('/batch/plans'),api('/jobs')]);
    S.plans = plans; S.jobs = jobs; $('history-count').textContent = fmt(historyFamilies(plans).length);
    if (render) renderHistory(); controls();
  } finally { S.historyLoading = false; }
}
function historyFamilies(plans) {
  const map = new Map(plans.map(plan => [plan.id,plan])); const families = new Map();
  for (const plan of plans) {
    let root = plan; const seen = new Set([root.id]);
    while (root.parent_plan_id && map.has(root.parent_plan_id) && !seen.has(root.parent_plan_id)) {
      root = map.get(root.parent_plan_id); seen.add(root.id);
    }
    if (!families.has(root.id)) families.set(root.id,{latest:plan,ids:new Set()});
    const family = families.get(root.id); family.ids.add(plan.id);
    if (plan.created_at > family.latest.created_at) family.latest = plan;
  }
  return [...families.values()].sort((a,b) => b.latest.created_at.localeCompare(a.latest.created_at));
}
async function loadPlan(planId, job = null) {
  if (busy() && (!job || S.job?.id !== job.id)) throw new Error('当前任务仍在运行，请等待完成或取消后再切换资料。');
  if (hasDrafts() || S.sourceDirty) throw new Error('当前有未保存的调整或资料设置。请保存调整，或重新扫描后再切换记录。');
  const plan = await api(`/batch/plans/${planId}`);
  if (!S.initialization || !samePath(S.initialization.root,plan.root)) {
    try { S.initialization = await api('/batch/initialize',{method:'POST',body:{root:plan.root}}); renderLevels(plan.level); }
    catch (_) { S.initialization = null; }
  }
  acceptPlan(plan,{syncForm:true,collapse:true,preserveSelection:false}); setView('batch');
  if (job) {
    const loaded = await api(`/jobs/${job.id}`); const ids = loaded.request?.voucher_ids;
    if (ids) { S.selected = new Set(ids); renderGroups(); }
    trackJob(loaded);
  } else { clearTimeout(S.polling); S.job = null; $('job-panel').hidden = true; controls(); }
}
function renderHistory() {
  const box = $('history-list'); box.replaceChildren(); const families = historyFamilies(S.plans);
  if (!families.length) box.append(el('div','card empty-pane','还没有批量合并记录。扫描后，您的资料范围和调整会保存在这里。'));
  for (const family of families) {
    const plan = family.latest; const card = el('article','card history-card'); const info = el('div');
    info.append(el('h2','',basename(plan.root)),el('p','',prettyPath(plan.root)));
    const meta = el('div','history-meta');
    meta.append(el('span','',`${fmt(plan.groups.length)} 个合并单元`),el('span','',`${fmt(plan.groups.reduce((n,g) => n + Number(g.pages || 0),0))} 页`),el('span','',plan.level ? `第 ${plan.level} 级分组` : '当前文件夹'),el('span','',plan.index ? '使用重命名索引' : '按文件夹名命名'));
    info.append(meta,el('small','',shortTime(plan.created_at)));
    const jobs = S.jobs.filter(job => batchJob(job) && family.ids.has(job.plan_id));
    if (jobs.length) {
      const taskBox = el('div','history-tasks');
      for (const job of jobs.slice(0,12)) {
        const row = el('div','history-task'); const content = el('div');
        content.append(el('span','',job.kind === 'batch_analyze' ? '图片方向调整' : '批量合并'),el('span',`pill${['failed','completed_with_errors'].includes(job.status) ? ' warning' : ''}`,labels[job.status] || job.status),el('small','',`${shortTime(job.created_at)} · ${job.threads || 4} 线程`));
        const open = el('button','text-button','查看任务'); open.type = 'button'; open.addEventListener('click',action(() => loadPlan(job.plan_id,job)));
        row.append(content,open); taskBox.append(row);
      }
      info.append(taskBox);
    }
    const resume = el('button','secondary-button compact','继续整理'); resume.type = 'button'; resume.addEventListener('click',action(() => loadPlan(plan.id)));
    card.append(info,resume); box.append(card);
  }
  const legacy = S.jobs.filter(job => !batchJob(job)); $('legacy-history').hidden = !legacy.length;
  const legacyBox = $('legacy-history-list'); legacyBox.replaceChildren();
  for (const job of legacy.slice(0,50)) {
    const item = el('article','legacy-job'); item.append(el('p','',`${job.kind === 'analyze' ? '旧版方向识别' : '旧版合并'} · ${labels[job.status] || job.status}`),el('small','',`${shortTime(job.created_at)} · ${job.threads || 1} 线程 · ${fmt(job.done)} / ${fmt(job.total)}`));
    const actions = el('div','legacy-result-actions');
    if (!active(job)) { const report = el('button','text-button','保存原报告'); report.type = 'button'; report.addEventListener('click',action(() => saveReport(job))); actions.append(report); }
    for (const result of job.results || []) {
      if (!result.output_path || result.status === 'failed') continue;
      const save = el('button','text-button',`保存 ${basename(result.output_path)}`); save.type = 'button'; save.addEventListener('click',action(() => saveOutput(job,result))); actions.append(save);
    }
    if (active(job)) item.append(el('p','help','该旧版任务正在运行；新批量任务暂不能同时启动。'));
    item.append(actions); legacyBox.append(item);
  }
}
async function syncGlobal() {
  if (S.syncing || S.operations || active(S.job)) return; S.syncing = true;
  try {
    const [status,jobs] = await Promise.all([api('/status'),api('/jobs')]); S.status = status; S.jobs = jobs;
    const running = jobs.find(job => active(job) && batchJob(job));
    if (running && !active(S.job)) {
      trackJob(running);
      if (!S.plan) acceptPlan(await api(`/batch/plans/${running.plan_id}`),{syncForm:true,collapse:true,preserveSelection:false});
    }
    controls();
  } catch (_) { /* Foreground operations display actionable connection errors. */ }
  finally { S.syncing = false; }
}
async function browseDirectory(path) {
  $('folder-error').hidden = true; $('confirm-folder').disabled = true;
  try {
    const result = await api('/fs' + (path ? '?path=' + encodeURIComponent(path) : ''));
    S.browser.path = result.path; S.browser.parent = result.parent; S.browser.selected = null;
    $('folder-path').value = prettyPath(result.path); $('folder-selection').textContent = prettyPath(result.path); $('folder-up').disabled = !result.parent;
    const box = $('folder-entries'); box.replaceChildren();
    const entries = result.entries.filter(entry => entry.is_dir || S.browser.kind === 'index');
    for (const entry of entries) {
      const button = el('button','folder-entry'); button.type = 'button';
      button.append(el('span','',entry.is_dir ? '▣' : '▤'),el('span','',entry.name));
      button.addEventListener('click',() => {
        if (entry.is_dir) browseDirectory(entry.path);
        else {
          S.browser.selected = entry.path; $('folder-selection').textContent = prettyPath(entry.path); $('confirm-folder').disabled = false;
          box.querySelectorAll('.selected').forEach(node => node.classList.remove('selected')); button.classList.add('selected');
        }
      });
      box.append(button);
    }
    if (!entries.length) box.append(el('div','empty-pane',S.browser.kind === 'index' ? '本目录没有表格文件，可进入其他目录或手动填写路径。' : '没有下级文件夹，可以选择当前目录。'));
    $('confirm-folder').disabled = S.browser.kind === 'index';
  } catch (error) { $('folder-error').textContent = error.message; $('folder-error').hidden = false; }
}
async function choosePath(kind, target) {
  if (busy()) return; const invoke = nativeInvoke(); const initial = $(target).value.trim() || $('root-path').value.trim() || null;
  if (invoke) {
    const value = await invoke(kind === 'index' ? 'select_index_file' : 'select_folder',{initialPath:initial});
    if (!value) return; $(target).value = prettyPath(value);
    if (target === 'root-path') { changedSource(); await initializeFolder(); }
    else if (target === 'index-path') { changedSource(); await inspectIndex(); }
    else updateOutputDraft();
    return;
  }
  S.browser = {kind,target,path:'',parent:null,selected:null}; $('folder-browser-title').textContent = kind === 'index' ? '选择重命名索引文件' : target === 'output-path' ? '选择输出文件夹' : '选择待处理文件夹';
  $('confirm-folder').textContent = kind === 'index' ? '选择此索引文件' : '选择此文件夹'; $('folder-browser').showModal();
  let directory = initial;
  if (kind === 'index' && directory && /\.xlsx?$/i.test(directory)) directory = directory.replace(/[\\/][^\\/]+$/, '');
  await browseDirectory(directory);
}
async function confirmBrowserSelection() {
  const value = S.browser.kind === 'index' ? S.browser.selected : S.browser.path; if (!value) return;
  const target = S.browser.target; $(target).value = prettyPath(value); $('folder-browser').close();
  if (target === 'root-path') { changedSource(); await initializeFolder(); }
  else if (target === 'index-path') { changedSource(); await inspectIndex(); }
  else updateOutputDraft();
}
function threadOptions(select) {
  select.replaceChildren(...Array.from({length:10},(_, index) => { const option = el('option','',`${index + 1} 线程`); option.value = index + 1; return option; })); select.value = '4';
}
function applyThreadPreferences(preferences) {
  const orientation = Number(preferences.orientation_threads); const merge = Number(preferences.merge_threads);
  if (![orientation,merge].every(value => Number.isInteger(value) && value >= 1 && value <= 10)) throw new Error('线程偏好无效，应为 1–10 的整数。');
  S.orientationThreads = orientation; S.mergeThreads = merge;
}
async function saveThreadPreferences(changes) {
  const preferences = {orientation_threads:S.orientationThreads,merge_threads:S.mergeThreads,...changes};
  const saved = await api('/batch/preferences',{method:'PATCH',body:preferences}); applyThreadPreferences(saved);
}
async function initialize() {
  threadOptions($('orientation-threads')); threadOptions($('merge-threads')); controls();
  if (!token) { $('connection').classList.add('offline'); $('connection').replaceChildren(el('i'),el('span','','缺少会话')); showError(new Error('请从 PDF工作台应用或开发启动入口打开此页面，以取得本机访问会话。')); return; }
  try {
    const status = await api('/status'); S.status = status; S.ready = true;
    $('app-version').textContent = `v${status.version}`; $('connection').replaceChildren(el('i'),el('span','','本机已连接'));
    try { applyThreadPreferences(await api('/batch/preferences')); }
    catch (error) { showError(new Error(`无法读取线程偏好，当前使用默认 4 线程。${error.message}`)); }
    await refreshHistory(false);
    const running = S.jobs.find(job => active(job) && batchJob(job));
    if (running) { acceptPlan(await api(`/batch/plans/${running.plan_id}`),{syncForm:true,collapse:true,preserveSelection:false}); trackJob(running); }
    setInterval(syncGlobal,4000); controls();
  } catch (error) { $('connection').classList.add('offline'); $('connection').replaceChildren(el('i'),el('span','','连接失败')); showError(error); }
}

$('tab-batch').addEventListener('click',() => setView('batch'));
$('tab-history').addEventListener('click',() => setView('history'));
document.querySelector('.brand').addEventListener('click',event => { event.preventDefault(); setView('batch'); });
$('toggle-preparation').addEventListener('click',() => setPreparationCollapsed(!S.preparationCollapsed));
$('source-form').addEventListener('submit',action(async event => { event.preventDefault(); await scan(); }));
for (const id of ['root-path','index-path','merge-level','index-column','name-column','header-row','match-rule','naming-mode','duplicate-mode','exclude-rules']) $(id).addEventListener('input',changedSource);
$('initialize-folder').addEventListener('click',action(initializeFolder));
$('merge-level').addEventListener('change',() => { renderLevelHint(); changedSource(); });
$('choose-root').addEventListener('click',action(() => choosePath('folder','root-path')));
$('choose-index').addEventListener('click',action(() => choosePath('index','index-path')));
$('inspect-index').addEventListener('click',action(() => inspectIndex()));
$('index-sheet').addEventListener('change',action(() => inspectIndex({sheet:$('index-sheet').value})));
$('reload-columns').addEventListener('click',action(() => inspectIndex({sheet:$('index-sheet').value,headerRow:Number($('header-row').value)})));
$('reset-rules').addEventListener('click',() => { $('exclude-rules').value = DEFAULT_RULES; changedSource(); });
$('show-issues').addEventListener('click',() => { $('scan-details').open = !$('scan-details').open; });
$('group-search').addEventListener('input',() => renderGroups());
$('select-all-groups').addEventListener('change',() => {
  S.selected = new Set($('select-all-groups').checked ? S.plan.groups.filter(group => !group.empty && group.files.length).map(group => group.id) : []); renderGroups(); controls();
});
$('deselect-errors').addEventListener('click',() => { S.selected = new Set(selectedGroups().filter(group => !hasErrors(group)).map(group => group.id)); renderGroups(); controls(); });
$('save-edits').addEventListener('click',action(saveEdits));
$('file-sort').addEventListener('change',renderSortHint);
$('apply-sort').addEventListener('click',action(applySort));
$('rotation-scope').addEventListener('change',renderPreviewMeta);
$('previous-source-page').addEventListener('click',action(() => navigateSourcePage(S.sourcePage - 1)));
$('next-source-page').addEventListener('click',action(() => navigateSourcePage(S.sourcePage + 1)));
$('source-page-number').addEventListener('change',action(() => navigateSourcePage($('source-page-number').value)));
$('zoom-out').addEventListener('click',action(async () => { S.fit = false; S.zoom = Math.max(.15,S.zoom / 1.25); await drawPreview(); }));
$('zoom-in').addEventListener('click',action(async () => { S.fit = false; S.zoom = Math.min(5,S.zoom * 1.25); await drawPreview(); }));
$('fit-preview').addEventListener('click',action(async () => { S.fit = true; await drawPreview(); }));
$('rotate-left').addEventListener('click',action(() => rotateFile(270)));
$('rotate-right').addEventListener('click',action(() => rotateFile(90)));
$('rotate-half').addEventListener('click',action(() => rotateFile(180)));
$('reset-rotation').addEventListener('click',action(() => rotateFile(0,true)));
$('reviewed-file').addEventListener('change',action(markReviewed));
$('auto-orient').addEventListener('click',action(() => openOrientation()));
$('orientation-consent').addEventListener('change',controls);
$('start-orientation').addEventListener('click',action(startOrientation));
$('close-orientation').addEventListener('click',() => $('orientation-dialog').close());
$('orientation-dialog').addEventListener('close',() => { S.orientationRetry = null; controls(); });
$('merge-button').addEventListener('click',action(() => openMerge()));
$('start-merge').addEventListener('click',action(startMerge));
$('close-merge').addEventListener('click',() => $('merge-dialog').close());
$('merge-dialog').addEventListener('close',() => { S.mergeRetry = null; controls(); });
$('output-path').addEventListener('input',updateOutputDraft);
$('choose-output').addEventListener('click',action(() => choosePath('folder','output-path')));
$('check-output').addEventListener('click',action(checkOutput));
$('cancel-job').addEventListener('click',action(cancelJob));
$('retry-job').addEventListener('click',action(retryJob));
$('save-report').addEventListener('click',action(() => saveReport()));
$('open-output').addEventListener('click',action(async () => { if (S.job) { await api(`/batch/plans/${S.job.plan_id}/open-output`,{method:'POST',body:{}}); toast('已打开输出文件夹。'); } }));
$('refresh-history').addEventListener('click',action(() => refreshHistory()));
$('close-folder-browser').addEventListener('click',() => $('folder-browser').close());
$('folder-up').addEventListener('click',() => browseDirectory(S.browser.parent));
$('folder-path-form').addEventListener('submit',event => { event.preventDefault(); browseDirectory($('folder-path').value.trim()); });
$('confirm-folder').addEventListener('click',action(confirmBrowserSelection));
let resizeTimer;
new ResizeObserver(() => { if (S.fit && (S.image || S.pdf)) { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => drawPreview().catch(showError),100); } }).observe($('preview-surface'));
window.addEventListener('beforeunload',() => { clearTimeout(S.polling); clearPreview(); });
initialize();
