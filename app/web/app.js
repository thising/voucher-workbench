'use strict';

const icons = {
  layers:'<path d="m12 3 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5M3 16l9 5 9-5"/>',
  history:'<path d="M3 11a9 9 0 1 1 2.7 7M3 4v7h7M12 7v5l3 2"/>',
  shield:'<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z"/><path d="m8 12 3 3 5-6"/>',
  folder:'<path d="M3 7V5a1 1 0 0 1 1-1h6l2 3h8a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z"/>',
  file:'<path d="M14 3H5v18h14V8l-5-5Z"/><path d="M14 3v6h5M8 13h8M8 17h6"/>',
  info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/>',
  search:'<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
  scan:'<path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4M7 9h10M7 12h10M7 15h6"/>',
  download:'<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>',
  alert:'<path d="m12 3 10 18H2L12 3Z"/><path d="M12 9v5M12 17h.01"/>',
  external:'<path d="M14 3h7v7m0-7-10 10M10 3H3v18h18v-7"/>',
  'rotate-left':'<path d="M3 4v6h6M3 10a9 9 0 1 1 2 8"/>',
  'rotate-right':'<path d="M21 4v6h-6M21 10a9 9 0 1 0-2 8"/>',
  flip:'<path d="M3 10a9 9 0 0 1 16-5m2-1v6h-6M21 14a9 9 0 0 1-16 5m-2 1v-6h6"/>',
  check:'<path d="m4 12 5 5L20 6"/>',
  'chevron-left':'<path d="m15 5-7 7 7 7"/>',
  'chevron-right':'<path d="m9 5 7 7-7 7"/>',
  refresh:'<path d="M3 4v6h6M3 10a9 9 0 0 1 16-5M21 20v-6h-6m6 0a9 9 0 0 1-16 5"/>',
  close:'<path d="m6 6 12 12M6 18 18 6"/>',
  up:'<path d="m5 10 7-7 7 7M12 3v18"/>'
};
const $ = id => document.getElementById(id);
const el = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};
function icon(name) {
  const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  node.setAttribute('viewBox', '0 0 24 24');
  node.setAttribute('fill', 'none');
  node.setAttribute('stroke', 'currentColor');
  node.setAttribute('stroke-width', '1.6');
  node.setAttribute('stroke-linecap', 'round');
  node.setAttribute('stroke-linejoin', 'round');
  node.setAttribute('aria-hidden', 'true');
  node.innerHTML = icons[name] || icons.file;
  return node;
}
document.querySelectorAll('[data-icon]').forEach(node => node.replaceWith(icon(node.dataset.icon)));

const fragment = new URLSearchParams(location.hash.slice(1));
let token = fragment.get('token') || sessionStorage.getItem('voucher-session-token') || '';
if (fragment.has('token')) {
  sessionStorage.setItem('voucher-session-token', token);
  history.replaceState(null, '', location.pathname + location.search);
}
const state = {
  plan:null, voucherId:null, fileId:null, page:0, filter:'all', search:'', selected:new Set(),
  status:null, job:null, jobs:[], plans:[], polling:null, pollCount:0, busy:false, mutating:false,
  browser:{mode:'root',path:'',parent:null,selected:null}, generateIds:null, generateRetry:null,
  thumbUrls:[], previewUrl:null, previewRequest:0, thumbsRequest:0, sourceCollapsed:false,
  confirming:null, toastTimer:null, loadingHistory:false, ledgerSheets:[], ledgerSheetPath:'', syncTimer:null, syncing:false,
  settings:{threads:1}, settingsLoaded:false, settingsSaving:false, settingsDraft:null, settingsError:'', settingsRevision:0, startingJob:false
};
const PAGE_SIZE = 8;
const activeStatuses = new Set(['queued','running']);
const statusLabels = {queued:'等待处理',running:'正在处理',completed:'已完成',completed_with_errors:'部分完成',cancelled:'已取消',interrupted:'意外中断',failed:'处理失败',skipped:'已核验，复用结果'};
const fmt = value => Number(value || 0).toLocaleString('zh-CN');
const basename = path => String(path || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();
const time = value => value ? new Date(value).toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false}) : '';
const voucher = () => state.plan?.vouchers.find(item => item.id === state.voucherId);
const activeFile = () => voucher()?.files.find(file => file.id === state.fileId);
const hasTitleResult = file => typeof state.status?.orientation_model === 'string' && state.status.orientation_model.length > 0 && file.orientation?.model === state.status.orientation_model;
const workflow = () => state.plan?.orientation_workflow || {enabled:Boolean(state.plan),analysis_completed:false};
const orientationEnabled = () => workflow().enabled === true;
const hasOrientationConsent = () => typeof state.status?.orientation_disclaimer_version === 'string' && state.status.orientation_disclaimer_version.length > 0 && workflow().consent_version === state.status.orientation_disclaimer_version && Boolean(workflow().consented_at);
const analysisCompleted = () => workflow().analysis_completed === true;
const needsReview = file => orientationEnabled() && file.kind === 'image' && !file.reviewed;
const needsAnalysis = file => file.kind === 'image' && !file.reviewed && !hasTitleResult(file);
const requiresInference = () => Boolean(state.plan?.vouchers.some(item=>item.files.some(needsAnalysis)));
const analysisModelBlocked = () => requiresInference() && !state.status?.model_ready;
const unknown = file => orientationEnabled() && needsAnalysis(file);
const modelUndecided = file => file.kind === 'image' && hasTitleResult(file) && file.orientation.needs_review !== false;
const fileIssue = file => !orientationEnabled() && file.error?.startsWith('方向识别失败：') ? null : file.error;
const effectiveRotation = file => file.kind === 'image' && !orientationEnabled() && !file.reviewed ? 0 : file.rotation_cw;
const filePresentation = file => JSON.stringify(file ? [file.id,file.rotation_cw,file.reviewed,file.error,file.orientation] : null);
const ocrScore = value => Number.isFinite(Number(value)) ? `${(100*Math.max(0,Math.min(1,Number(value)))).toFixed(1)}%` : '—';
const locked = () => state.busy || state.mutating || state.settingsSaving || state.startingJob || (state.job && activeStatuses.has(state.job.status));
const editingLocked = () => locked() || (orientationEnabled() && (!hasOrientationConsent() || !analysisCompleted()));
const jobThreads = job => Number.isInteger(job?.threads) && job.threads >= 1 && job.threads <= 10 ? job.threads : 1;
$('processing-threads').replaceChildren(...Array.from({length:10},(_,index)=>{
  const option=el('option','',`${index+1} 线程`);option.value=index+1;return option;
}));

async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set('X-Session-Token', token);
  if (options.body && typeof options.body !== 'string') {
    options.body = JSON.stringify(options.body);
    headers.set('Content-Type', 'application/json');
  }
  let response;
  try { response = await fetch('/api' + path, {...options, headers, cache:'no-store'}); }
  catch (_) { throw new Error('无法连接本地服务。请确认程序仍在运行，再刷新页面。'); }
  if (!response.ok) {
    let message;
    try { const data = await response.json(); message = data.error; } catch (_) { /* response is not JSON */ }
    if (response.status === 401 || response.status === 403) {
      message = message || '会话已失效，请从程序提供的完整链接重新打开工作台。';
    }
    throw new Error(message || `请求失败（${response.status}），请稍后重试。`);
  }
  if (options.blob) return response.blob();
  return response.status === 204 ? null : response.json();
}
function showError(error) {
  $('global-alert').textContent = error?.message || String(error);
  $('global-alert').hidden = false;
}
function clearError() { $('global-alert').hidden = true; }
function toast(message) {
  clearTimeout(state.toastTimer);
  $('toast').textContent = message;
  $('toast').hidden = false;
  state.toastTimer = setTimeout(() => { $('toast').hidden = true; }, 3500);
}
function loading(container, message = '正在读取…') {
  const wrapper = el('div','loading-inline');
  wrapper.append(el('span','spinner'),el('span','',message));
  container.replaceChildren(wrapper);
}
function action(fn) {
  return async event => { try { await fn(event); } catch (error) { showError(error); } };
}
function acceptSettings(settings) {
  if(!Number.isInteger(settings?.threads) || settings.threads<1 || settings.threads>10)throw new Error('并行设置无效，请刷新页面后重试。');
  state.settings={threads:settings.threads};state.settingsLoaded=true;
}
function renderProcessingSettings() {
  $('processing-threads').value=state.settingsDraft??state.settings.threads;
  $('processing-threads').disabled=locked() || !state.settingsLoaded;
  const status=$('processing-settings-status');status.classList.toggle('error',Boolean(state.settingsError));
  if(state.settingsSaving)status.textContent=`正在保存 ${state.settingsDraft} 线程…`;
  else if(state.settingsError)status.textContent=state.settingsError;
  else if(!state.settingsLoaded)status.textContent='正在读取设置…';
  else if(state.job && activeStatuses.has(state.job.status))status.textContent=`当前任务使用 ${jobThreads(state.job)} 线程；结束后可修改。`;
  else if(state.startingJob)status.textContent='正在启动任务…';
  else status.textContent=`已保存 · 新任务使用 ${state.settings.threads} 线程，重试沿用原任务设置。`;
}
async function saveThreadSelection() {
  if(locked() || !state.settingsLoaded){renderProcessingSettings();return;}
  const threads=Number($('processing-threads').value);
  if(!Number.isInteger(threads) || threads<1 || threads>10){renderProcessingSettings();throw new Error('并行线程数须为 1–10 的整数。');}
  if(threads===state.settings.threads)return;
  state.settingsSaving=true;state.settingsDraft=threads;state.settingsError='';state.settingsRevision++;clearError();controls();
  try {
    acceptSettings(await api('/settings',{method:'PATCH',body:{threads}}));
    toast(`已保存 ${state.settings.threads} 线程，下一次新任务生效`);
  } catch(error) {
    // A lost PATCH response may still have saved the setting. Re-read before
    // enabling tasks instead of assuming the previous browser value is current.
    try {
      acceptSettings(await api('/settings'));
      state.settingsError=`已重新读取设置：当前为 ${state.settings.threads} 线程。`;
    } catch(_) {
      state.settingsLoaded=false;
      state.settingsError='未能确认设置，请刷新页面后重试。';
    }
    throw error;
  } finally {state.settingsSaving=false;state.settingsDraft=null;controls();}
}
async function requestJob(path,options) {
  if(locked())return null;
  if(!state.settingsLoaded)throw new Error('并行设置尚未读取成功，请刷新页面后重试。');
  if(Number($('processing-threads').value)!==state.settings.threads)throw new Error('请先保存并行设置，再启动任务。');
  state.startingJob=true;controls();
  try {const job=await api(path,options);trackJob(job);return job;}
  finally {state.startingJob=false;controls();}
}
function controls() {
  const busy = locked();
  $('scan-button').disabled = busy;
  $('scan-button').textContent = state.busy ? '正在扫描…' : '扫描资料';
  $('sheet-select').disabled = busy;
  $('source-sheet-select').disabled = busy;
  $('orientation-enabled').checked = orientationEnabled() && hasOrientationConsent();
  $('orientation-enabled').disabled = busy || !state.plan || !state.status?.orientation_disclaimer_version;
  $('disable-orientation').hidden = !orientationEnabled() || hasOrientationConsent();
  $('disable-orientation').disabled = busy || !state.plan;
  $('analyze-button').hidden = !orientationEnabled();
  $('analyze-button').disabled = busy || !state.settingsLoaded || !state.plan || !hasOrientationConsent() || analysisModelBlocked();
  $('analyze-button').title = !hasOrientationConsent() ? '请先勾选启用并同意免责声明' : analysisModelBlocked() ? state.status?.model_error || '标题文字识别尚未就绪' : requiresInference() ? '依据标题文字的阅读方向提出旋转建议，完成后须人工确认全部图片' : '复用已有当前版本结果和人工确认，无需重新推理即可完成本轮识别阶段';
  $('generate-button').disabled = busy || !state.settingsLoaded || !state.plan || !state.plan.vouchers.length;
  $('generate-one').disabled = busy || !state.settingsLoaded || !voucher();
  const selectedVouchers=state.plan?.vouchers.filter(item=>!state.generateIds || state.generateIds.includes(item.id)) || [];
  $('start-generate').disabled=busy || !state.settingsLoaded || !state.plan || Boolean(generationBlock(selectedVouchers));
  $('retry-job').disabled=busy || !state.settingsLoaded;
  for (const id of ['rotate-left','rotate-flip','rotate-right','confirm-file','open-original']) {
    $(id).disabled = !activeFile() || (id !== 'open-original' && editingLocked());
  }
  $('confirm-selection').disabled = editingLocked() || !state.selected.size;
  $('confirm-selection').textContent = state.selected.size ? `确认所选 ${state.selected.size} 项` : '确认所选';
  $('select-visible').disabled = editingLocked();
  $('thumbnails').querySelectorAll('input[type="checkbox"]').forEach(input=>{input.disabled=editingLocked();});
  $('browse-root').disabled = busy;
  $('browse-ledger').disabled = busy;
  renderProcessingSettings();
  renderOrientationWorkflow();
}
function renderOrientationWorkflow() {
  if(!state.plan)return;
  const enabled=orientationEnabled();
  const consent=hasOrientationConsent();
  const completed=analysisCompleted();
  const pending=state.plan.vouchers.flatMap(item=>item.files).filter(needsReview).length;
  const analyzing=state.job?.plan_id===state.plan.id && state.job.kind==='analyze' && activeStatuses.has(state.job.status);
  $('orientation-mode').textContent=enabled ? (consent ? '已启用' : '待明确同意') : '未启用';
  $('orientation-mode').className='status-pill'+(enabled&&!consent?' warning':'');
  $('orientation-steps').hidden=!enabled;
  for(const [id,done,current] of [
    ['orientation-step-consent',consent,!consent],
    ['orientation-step-analysis',completed&&consent,consent&&!completed],
    ['orientation-step-review',completed&&consent&&!pending,completed&&consent&&pending>0]
  ]) {
    $(id).classList.toggle('done',Boolean(done));$(id).classList.toggle('current',Boolean(current));
  }
  let stage;
  if(!enabled)stage='未启用方向识别：按原图方向（含照片自身方向信息）生成，保留人工旋转。无需等待识别或逐页确认。';
  else if(!consent)stage='此计划尚未同意当前版本说明。请明确勾选上方选项，或关闭识别后按原方向生成；旧计划不会自动视为已同意。';
  else if(analyzing)stage='正在识别标题方向。请等待本轮识别完成，再人工核对全部图片；处理期间不能更改模式或确认方向。';
  else if(!completed && analysisModelBlocked())stage=`标题识别暂不可用：${state.status?.model_error||'本地模型尚未就绪'}。可关闭识别后按原图方向生成。`;
  else if(!completed && !requiresInference())stage='现有结果可复用，无需重新推理。请点击“识别标题方向”完成本轮识别阶段，再人工核对尚未确认的图片。';
  else if(!completed)stage='下一步：点击“识别标题方向”。本轮识别完成前，暂不能人工确认或生成；中途取消后需继续完成识别。';
  else if(pending)stage=`识别已完成，还有 ${fmt(pending)} 张图片待人工确认。请逐页核对、修正，或明确勾选已核对的页面后确认，再生成 PDF。`;
  else stage='识别与全部图片的人工确认已完成，可以生成 PDF。';
  $('orientation-stage').textContent=stage;
  $('orientation-method').textContent=enabled?'按标题文字方向提出旋转建议；图片宽高决定转正后的横版或竖版。所有图片均须人工核对，包括建议保持原向和识别未决的页面。':'方向识别已关闭：按原图方向生成，可手动旋转；已保存的自动识别建议不影响输出。';
  $('review-guidance').textContent=!enabled?'可直接生成，也可先手动检查和旋转页面。':!consent||!completed?'完成同意和本轮识别后，才能人工确认。':'请检查每张图片，再确认当前页或明确所选页面；系统不会自动勾选。';
}
function setView(view) {
  const work = view === 'work';
  $('work-view').hidden = !work;
  $('history-view').hidden = work;
  $('nav-work').classList.toggle('active',work);
  $('nav-history').classList.toggle('active',!work);
  $('page-title').textContent = work ? '让凭证各就其位' : '处理记录';
  $('page-subtitle').textContent = work ? '选择资料，核对方向，按明细账生成独立 PDF。' : '每一次整理，都有记录可循。';
  if (!work) refreshHistory().catch(showError);
}
function setSourceCollapsed(collapse) {
  state.sourceCollapsed = collapse;
  $('source-form').hidden = collapse;
  $('source-summary').hidden = !collapse;
  $('collapse-source').textContent = collapse ? '修改来源' : '收起设置';
}
function acceptPlan(plan, options = {}) {
  const oldPlan = state.plan;
  if(oldPlan?.id!==plan.id && state.job && state.job.plan_id!==plan.id && !activeStatuses.has(state.job.status)) {
    if(state.polling)clearTimeout(state.polling);
    state.polling=null;state.job=null;$('job-panel').hidden=true;
    $('step-export').classList.remove('current');
  }
  state.plan = plan;
  if (!plan.vouchers.some(item => item.id === state.voucherId)) {
    state.voucherId = plan.vouchers[0]?.id || null;
    state.fileId = null;
    state.page = 0;
    state.selected.clear();
  }
  if (!voucher()?.files.some(file => file.id === state.fileId)) state.fileId = voucher()?.files[0]?.id || null;
  if (!oldPlan || oldPlan.id !== plan.id) {
    state.selected.clear();
    state.page = 0;
  }
  const sourceRoot=options.sourceRoot || plan.requested_root || sessionStorage.getItem(`voucher-scan-root-${plan.id}`) || plan.root;
  sessionStorage.setItem(`voucher-scan-root-${plan.id}`,sourceRoot);
  $('root-path').value = sourceRoot;
  $('ledger-path').value = plan.ledger_path;
  $('welcome').hidden = true;
  $('workspace').hidden = false;
  $('collapse-source').hidden = false;
  const summary = $('source-summary');
  summary.replaceChildren();
  const root = el('span'); root.append(el('strong','', '资料 '), document.createTextNode(sourceRoot));
  const ledger = el('span'); ledger.append(el('strong','', '账表 '), document.createTextNode(basename(plan.ledger_path)));
  summary.append(root,ledger);
  $('sheet-select').replaceChildren(...plan.sheets.map(sheet => {
    const option = el('option','',sheet.name + (sheet.valid ? '' : ' · 缺少必要字段'));
    option.value = sheet.name; option.selected = sheet.name === plan.sheet; option.disabled=!sheet.valid;
    return option;
  }));
  setLedgerSheets(plan.sheets,plan.ledger_path,plan.sheet);
  $('sheet-status').textContent = `共 ${plan.sheets.length} 个工作表`;
  if (options.collapse !== false) setSourceCollapsed(true);
  renderStats(); renderIssues(); renderVouchers(); renderVoucher(); controls();
  $('step-source').classList.remove('current'); $('step-source').classList.add('done');
  $('step-review').classList.add('current');
}
function renderStats() {
  if (!state.plan) return;
  const plan = state.plan;
  const files = plan.vouchers.flatMap(item => item.files);
  const matched = plan.vouchers.filter(item => item.subject !== null && item.subject !== '').length;
  const errors = plan.issues.filter(issue => issue.severity === 'error').length + plan.vouchers.reduce((count,item) => count + item.issues.filter(issue => issue.severity === 'error').length + item.files.filter(fileIssue).length,0);
  $('stat-match').textContent = `${matched} / ${plan.vouchers.length}`;
  $('stat-match-note').textContent = matched === plan.vouchers.length ? '所有凭证均找到科目' : `${plan.vouchers.length - matched} 个凭证未匹配`;
  $('stat-pages').textContent = fmt(files.reduce((count,file) => count + file.pages,0));
  $('stat-files').textContent = `${fmt(files.filter(file => file.kind === 'image').length)} 张图片 · ${fmt(files.filter(file => file.kind === 'pdf').length)} 个 PDF`;
  $('stat-review').textContent = fmt(files.filter(needsReview).length);
  $('stat-unprocessed').textContent = !orientationEnabled() ? '识别未启用 · 无强制确认要求' : analysisCompleted() ? `${fmt(files.filter(modelUndecided).length)} 张模型未决 · 全部图片须人工核对` : `${fmt(files.filter(unknown).length)} 张待识别 · 完成后人工核对`;
  $('stat-errors').textContent = fmt(errors);
  $('stat-errors').parentElement.classList.toggle('error', errors > 0);
  $('stat-ignored').textContent = `已忽略 ${fmt(plan.ignored)} 项辅助资源`;
  $('voucher-count').textContent = plan.vouchers.length;
}
function renderIssues() {
  const issues = [...state.plan.issues.map(issue => ({...issue,label:'批次'}))];
  for (const item of state.plan.vouchers) {
    for (const issue of item.issues) issues.push({...issue,label:item.folder});
    for (const file of item.files) if (fileIssue(file)) issues.push({severity:'error',label:item.folder,message:`${file.name}：${fileIssue(file)}`});
  }
  $('issue-panel').hidden = !issues.length;
  $('issue-summary').textContent = `${issues.length} 项需要注意 · 点击查看明细`;
  $('issue-list').replaceChildren(...issues.map(issue => {
    const row = el('div','issue-row ' + issue.severity);
    row.append(el('strong','',issue.label),el('span','',issue.message));
    return row;
  }));
}
function renderVouchers() {
  const list = $('voucher-list');
  const term = state.search.trim().toLowerCase();
  const rows = state.plan.vouchers.filter(item => `${item.folder} ${item.subject || ''} ${item.item}`.toLowerCase().includes(term));
  if (!rows.length) { list.replaceChildren(el('div','empty-inline','没有找到相应凭证')); return; }
  list.replaceChildren(...rows.map(item => {
    const button = el('button','voucher-button' + (item.id === state.voucherId ? ' active' : ''));
    button.type = 'button'; button.dataset.voucherId = item.id;
    const line = el('span','voucher-code'); line.append(el('span','',item.folder));
    const errors = item.issues.some(issue => issue.severity === 'error') || item.files.some(fileIssue);
    const reviewCount = item.files.filter(needsReview).length;
    if (errors) line.append(el('span','mini-badge error','有问题'));
    else if (reviewCount) line.append(el('span','mini-badge warning',`${reviewCount} 待确认`));
    else line.append(el('span','mini-badge','就绪'));
    const note = el('small'); note.append(el('span','',item.item),el('span','',`${fmt(item.files.reduce((count,file)=>count+file.pages,0))} 页`));
    button.append(line,el('p','',item.subject || '未匹配科目'),note);
    button.addEventListener('click',() => selectVoucher(item.id));
    return button;
  }));
}
function selectVoucher(id) {
  if (id === state.voucherId) return;
  state.voucherId = id; state.fileId = voucher()?.files[0]?.id || null; state.page=0; state.selected.clear();
  renderVouchers(); renderVoucher(); controls();
}
function renderVoucher() {
  const item = voucher();
  if (!item) {
    $('voucher-title').textContent = '暂无凭证';
    $('voucher-subject').textContent = '请检查资料目录与扫描问题';
    $('voucher-item').textContent = '资料';
    $('voucher-note').replaceChildren();
    $('thumbnails').replaceChildren();
    renderPreview(); return;
  }
  $('voucher-title').textContent = item.folder;
  $('voucher-item').textContent = item.item;
  $('voucher-subject').textContent = item.subject || '未找到对应科目';
  const note = $('voucher-note'); note.replaceChildren();
  if (item.ledger_row) note.append(document.createTextNode(`取「${state.plan.sheet}」第 ${item.ledger_row} 行`));
  if (item.duplicate_rows.length) note.append(document.createTextNode(` · 同号 ${item.duplicate_rows.length + 1} 条，采用第一条`));
  if (item.output_relative_path) note.append(el('div','',`输出：${item.output_relative_path}`));
  for (const issue of item.issues) if (issue.severity === 'error') note.append(el('span','inline-error',issue.message));
  renderPages(); renderPreview();
}
function filteredFiles() {
  return (voucher()?.files || []).filter(file => {
    if (state.filter === 'review') return needsReview(file);
    if (state.filter === 'unprocessed') return unknown(file);
    if (state.filter === 'rotated') return effectiveRotation(file) !== 0;
    if (state.filter === 'error') return Boolean(fileIssue(file));
    return true;
  });
}
function clearThumbs() {
  state.thumbsRequest++;
  state.thumbUrls.forEach(url => URL.revokeObjectURL(url));
  state.thumbUrls = [];
}
function fileLabel(file) {
  if (fileIssue(file)) return ['有问题','error'];
  if (file.reviewed) return ['已确认','checked'];
  if (file.kind === 'pdf') return [`${file.pages} 页 PDF`,''];
  if (!orientationEnabled()) return ['原图方向',''];
  if (!hasTitleResult(file)) return [file.orientation ? '待重新识别' : '未识别标题','warning'];
  if (file.orientation.needs_review !== false) return ['未决 · 待确认','warning'];
  return ['待人工确认','warning'];
}

function renderTitleEvidence(file, container) {
  if (file.kind !== 'image') return;
  if (!orientationEnabled()) {
    container.append(el('span','','方向识别未启用；自动识别建议不影响本次输出。'));
    return;
  }
  if (!hasTitleResult(file)) {
    if (!file.reviewed) container.append(el('span','warning',analysisCompleted() ? '本轮未取得可用标题方向，请人工检查、修正后确认。' : file.orientation ? '此识别结果不属于当前算法版本，请完成本轮标题识别后再人工确认。' : '尚未识别标题，请先完成本轮标题方向识别，再人工核对确认。'));
    return;
  }
  const result=file.orientation;
  const evidence=el('section','title-evidence');
  evidence.setAttribute('aria-label','标题方向识别依据');
  const title=el('div','title-found');
  title.append(el('span','evidence-label','识别标题'),el('strong','',result.title || '未找到可用标题'));
  const decision=el('p',result.needs_review !== false ? 'title-decision warning' : 'title-decision',`建议校正 ${result.rotation_cw}°${result.rotation_cw ? '（顺时针）' : ' · 保持原向'}${result.title ? ` · OCR 识别分数 ${ocrScore(result.confidence)}` : ''}`);
  evidence.append(title,decision);
  if(result.reason)evidence.append(el('p','title-reason',result.reason));
  if(file.reviewed)evidence.append(el('p','title-reason','当前输出以人工确认的方向为准。'));
  else evidence.append(el('p','title-reason','此结果仅为识别建议；须人工检查后确认，才能生成。'));
  const candidates=Array.isArray(result.candidates)?result.candidates:[];
  if(candidates.length) {
    const details=el('details','title-candidates');
    details.append(el('summary','',`查看标题候选与方向（${candidates.length} 条）`));
    const list=el('div','candidate-list');
    for(const candidate of candidates) {
      const row=el('article','title-candidate'+(candidate.accepted?' accepted':''));
      const top=el('div','candidate-heading');
      top.append(el('strong','',`${candidate.rotation_cw}°`),el('span','',candidate.accepted?'可用标题':'未采用'),el('span','candidate-score',`OCR ${ocrScore(candidate.confidence)}`));
      row.append(top,el('p','candidate-text',candidate.text || '未读出文字'));
      const notes=[];
      if(candidate.keyword)notes.push(`匹配标题词：${candidate.keyword}`);
      if(Array.isArray(candidate.bbox)&&candidate.bbox.length===4)notes.push(`文字框：${candidate.bbox.map(Number).join(' / ')}（x / y / 宽 / 高，旋转后像素）`);
      if(notes.length)row.append(el('small','candidate-meta',notes.join(' · ')));
      list.append(row);
    }
    details.append(list);evidence.append(details);
  }
  container.append(evidence);
}
function renderPages() {
  clearThumbs();
  const requestId = state.thumbsRequest;
  const files = filteredFiles();
  const pages = Math.ceil(files.length / PAGE_SIZE);
  state.page = Math.max(0,Math.min(state.page,pages-1));
  const visible = files.slice(state.page*PAGE_SIZE,(state.page+1)*PAGE_SIZE);
  $('prev-page').disabled = state.page <= 0;
  $('next-page').disabled = state.page >= pages-1;
  $('page-count').textContent = pages ? `${state.page+1} / ${pages} · ${files.length} 项` : '0 项';
  $('select-visible').checked = visible.length > 0 && visible.every(file => state.selected.has(file.id));
  $('select-visible').indeterminate = visible.some(file => state.selected.has(file.id)) && !$('select-visible').checked;
  const container = $('thumbnails');
  if (!visible.length) { container.replaceChildren(el('div','empty-inline','此筛选下没有文件')); controls(); return; }
  container.replaceChildren(...visible.map(file => {
    const tile = el('div','thumbnail');
    const label = el('label','thumbnail-select');
    const checkbox = el('input'); checkbox.type='checkbox'; checkbox.checked=state.selected.has(file.id); checkbox.setAttribute('aria-label',`选择 ${file.name}`);
    checkbox.addEventListener('change',() => {
      if (checkbox.checked) state.selected.add(file.id); else state.selected.delete(file.id);
      $('select-visible').checked = visible.every(item => state.selected.has(item.id));
      $('select-visible').indeterminate = visible.some(item => state.selected.has(item.id)) && !$('select-visible').checked;
      controls();
    });
    label.append(checkbox);
    const button = el('button','thumbnail-button' + (file.id === state.fileId ? ' active' : ''));
    button.type='button'; button.title=file.name; button.dataset.fileId=file.id;
    const holder = el('div','thumb-image');
    holder.append(el('span','thumb-number',String(file.sequence)));
    if (file.kind === 'pdf') holder.append(el('span','pdf-tile','PDF'));
    else {
      const img = el('img'); img.alt=file.name; img.loading='lazy'; img.style.transform=`rotate(${effectiveRotation(file)}deg)`;
      holder.prepend(img);
      api(`/plans/${state.plan.id}/files/${file.id}?thumbnail=true`,{blob:true}).then(blob => {
        if (requestId !== state.thumbsRequest) return;
        const url = URL.createObjectURL(blob); state.thumbUrls.push(url); img.src=url;
      }).catch(error => {
        if (requestId !== state.thumbsRequest) return;
        img.remove(); holder.append(el('span','mini-badge error','读取失败')); button.title=`${file.name}：${error.message}`;
      });
    }
    const info = el('div','thumb-info');
    const [status,className] = fileLabel(file);
    const rotation=effectiveRotation(file);
    info.append(el('span',className,status),el('span','',rotation ? `${rotation}°` : ''));
    button.append(holder,info);
    button.addEventListener('click',() => {
      state.fileId=file.id;
      container.querySelectorAll('.thumbnail-button').forEach(node => node.classList.toggle('active',node.dataset.fileId === file.id));
      renderPreview(); controls();
    });
    tile.append(label,button); return tile;
  }));
  controls();
}
function fitPreviewImage() {
  const img = $('preview-stage').querySelector('img');
  if (!img) return;
  const angle = Number(img.dataset.rotation || 0);
  const rotated = angle === 90 || angle === 270;
  const width = Math.max(50,$('preview-stage').clientWidth-40);
  const height = Math.max(50,$('preview-stage').clientHeight-40);
  img.style.width = `${rotated ? height : width}px`;
  img.style.height = `${rotated ? width : height}px`;
  img.style.transform = `rotate(${angle}deg)`;
}
async function renderPreview() {
  const file = activeFile();
  const requestId = ++state.previewRequest;
  if (state.previewUrl) { URL.revokeObjectURL(state.previewUrl); state.previewUrl=null; }
  const stage = $('preview-stage');
  $('preview-filename').textContent = file?.name || '请选择文件';
  $('file-detail').replaceChildren();
  if (!file) { stage.replaceChildren(el('div','empty-inline','没有可预览的文件')); return; }
  const detail = $('file-detail');
  const rotation=effectiveRotation(file);
  const effective = el('span'); effective.append(document.createTextNode('当前旋转 '),el('strong','',`${rotation}°`));
  detail.append(effective,el('span','',`${file.pages} 页 · ${(file.bytes/1024).toFixed(0)} KB`));
  if (file.width && file.height) detail.append(el('span','',`${file.width} × ${file.height}`));
  if (file.reviewed) detail.append(el('strong','','方向已人工确认'));
  renderTitleEvidence(file,detail);
  if (file.kind === 'pdf') detail.append(el('span','','PDF 预览为原件；追加旋转将在生成结果中应用。'));
  if (fileIssue(file)) detail.append(el('span','error',fileIssue(file)));
  loading(stage,'正在打开预览…');
  try {
    const blob = await api(`/plans/${state.plan.id}/files/${file.id}${file.kind==='image'?'?thumbnail=true':''}`,{blob:true});
    if (requestId !== state.previewRequest) return;
    const url = URL.createObjectURL(blob); state.previewUrl=url;
    if (file.kind === 'pdf') {
      const iframe = el('iframe'); iframe.title=file.name; iframe.src=url+'#toolbar=0&navpanes=0&view=Fit'; stage.replaceChildren(iframe);
    } else {
      const img = el('img'); img.alt=`${file.name}，顺时针旋转 ${rotation} 度后的预览`; img.src=url; img.dataset.rotation=rotation;
      stage.replaceChildren(img); fitPreviewImage();
    }
  } catch (error) {
    if (requestId === state.previewRequest) stage.replaceChildren(el('div','preview-error',error.message));
  }
}
async function saveFile(rotation) {
  if (!activeFile() || editingLocked()) return;
  clearError();
  const file = activeFile();
  state.mutating=true;controls();
  try {
    const plan = await api(`/plans/${state.plan.id}/files/${file.id}`,{method:'PATCH',body:{rotation_cw:rotation,reviewed:true}});
    acceptPlan(plan,{collapse:state.sourceCollapsed});
    toast(rotation === file.rotation_cw ? '此页方向已确认' : '已保存旋转，并标记为人工确认');
  } finally {state.mutating=false;controls();}
}
async function confirmSelection() {
  const ids = [...state.selected];
  if (!ids.length || editingLocked()) return;
  const accepted = await showConfirm(`将把所选的 ${ids.length} 个文件按当前预览方向标记为已确认。请确保这些页面的内容方向均正确。`);
  if (!accepted || editingLocked()) return;
  state.mutating=true;controls();
  try {
    const plan = await api(`/plans/${state.plan.id}/review`,{method:'POST',body:{file_ids:ids}});
    state.selected.clear(); acceptPlan(plan,{collapse:state.sourceCollapsed}); toast(`已确认 ${ids.length} 个文件的方向`);
  } finally {state.mutating=false;controls();}
}
function showConfirm(message) {
  $('confirm-message').textContent=message;
  $('confirm-dialog').showModal();
  return new Promise(resolve => { state.confirming=resolve; });
}
function endConfirm(result) {
  $('confirm-dialog').close();
  if (state.confirming) { state.confirming(result); state.confirming=null; }
}
async function scan(sheet) {
  if (locked()) return;
  clearError();
  const root = $('root-path').value.trim();
  const ledger = $('ledger-path').value.trim();
  if (!root) { $('root-path').focus(); throw new Error('请先选择资料文件夹。'); }
  state.busy=true; controls();
  try {
    const body = {root}; if (ledger) body.ledger=ledger;
    const chosenSheet=sheet || (state.ledgerSheetPath===ledger ? $('source-sheet-select').value : '');
    if (chosenSheet) body.sheet=chosenSheet;
    const plan = await api('/scan',{method:'POST',body});
    acceptPlan(plan,{sourceRoot:root});
    toast(`扫描完成：${plan.vouchers.length} 个凭证`);
    refreshHistory(false).catch(() => {});
  } finally { state.busy=false; controls(); }
}
async function browse(mode) {
  state.browser.mode=mode; state.browser.selected=null;
  $('browser-title').textContent=mode==='root'?'选择资料文件夹':'选择明细账文件';
  $('browser-choose').textContent=mode==='root'?'选择此文件夹':'选择此明细账';
  $('browser-selected').textContent=mode==='root'?'进入目标文件夹后点击确定':'请选择一个 .xls 或 .xlsx 文件';
  $('browser-dialog').showModal();
  const value = mode==='root' ? $('root-path').value.trim() : $('ledger-path').value.trim();
  let start=value;
  if (mode==='ledger' && /\.xlsx?$/i.test(start)) start=start.replace(/[\\/][^\\/]+$/,'');
  if (!start) start=$('root-path').value.trim();
  await readDirectory(start);
}
async function readDirectory(path) {
  $('browser-error').hidden=true;
  $('browser-choose').disabled=true;
  loading($('browser-entries'),'正在读取本地目录…');
  try {
    const data=await api('/fs'+(path?'?path='+encodeURIComponent(path):''));
    state.browser.path=data.path; state.browser.parent=data.parent; state.browser.selected=null;
    $('browser-path-input').value=data.path;
    $('browser-up').disabled=!data.parent;
    $('browser-choose').disabled=state.browser.mode!=='root';
    const entries=data.entries.filter(entry=>entry.is_dir || state.browser.mode==='ledger');
    const list=$('browser-entries');
    if (!entries.length) { list.replaceChildren(el('div','empty-inline',state.browser.mode==='root'?'此文件夹没有下级目录':'此文件夹没有可选明细账')); return; }
    list.replaceChildren(...entries.map(entry=>{
      const button=el('button','browser-entry');button.type='button';
      button.append(icon(entry.is_dir?'folder':'file'),el('span','entry-name',entry.name),el('span','entry-kind',entry.is_dir?'文件夹':'明细账'));
      button.addEventListener('click',()=>{
        if (entry.is_dir) readDirectory(entry.path);
        else {
          state.browser.selected=entry.path;
          list.querySelectorAll('.browser-entry').forEach(node=>node.classList.remove('selected'));
          button.classList.add('selected');$('browser-selected').textContent=entry.name;$('browser-choose').disabled=false;
        }
      });
      return button;
    }));
    $('browser-selected').textContent=state.browser.mode==='root'?data.path:'请选择一个 .xls 或 .xlsx 文件';
  } catch(error) {
    $('browser-error').textContent=error.message;$('browser-error').hidden=false;
    $('browser-entries').replaceChildren(el('div','empty-inline','可在上方输入其他本地路径'));
  }
}
function setLedgerSheets(sheets,path,selected) {
  state.ledgerSheets=sheets;state.ledgerSheetPath=path;
  $('source-sheet-row').hidden=!sheets.length;
  $('source-sheet-select').replaceChildren(...sheets.map((sheet,index)=>{
    const option=el('option','',sheet.name+(sheet.valid?'':' · 字段不可用'));
    option.value=sheet.name;option.disabled=!sheet.valid;option.selected=selected?sheet.name===selected:index===0;
    return option;
  }));
  const first=selected?sheets.find(sheet=>sheet.name===selected):sheets[0];
  if(first)$('source-sheet-select').value=first.name;
  updateSheetNote();
}
function updateSheetNote() {
  const sheet=state.ledgerSheets.find(item=>item.name===$('source-sheet-select').value);
  $('source-sheet-note').className=sheet&&!sheet.valid?'error':'';
  $('source-sheet-note').textContent=sheet&&!sheet.valid?`${sheet.error||'缺少必要字段'}；请选择其他工作表。`:sheet?`第 ${sheet.header_row||1} 行为表头 · ${sheet.unique_codes} 个不同编号`:'请选择可用的工作表';
}
async function loadLedgerSheets() {
  const path=$('ledger-path').value.trim();
  if(!path){state.ledgerSheets=[];state.ledgerSheetPath='';$('source-sheet-row').hidden=true;return;}
  if(state.ledgerSheetPath===path)return;
  const sheets=await api('/ledger/sheets?path='+encodeURIComponent(path));
  if(path===$('ledger-path').value.trim())setLedgerSheets(sheets,path);
}
async function selectPath() {
  if (state.browser.mode==='root') $('root-path').value=state.browser.path;
  else if (state.browser.selected) $('ledger-path').value=state.browser.selected;
  else return;
  $('browser-dialog').close();
  if(state.browser.mode==='ledger')await loadLedgerSheets();
}
async function setOrientationMode(enabled) {
  if(!state.plan || locked()){controls();return;}
  if(enabled && !state.status?.orientation_disclaimer_version){controls();throw new Error('尚未取得当前免责声明版本，请刷新页面后再启用。');}
  clearError();state.mutating=true;controls();
  try {
    const plan=await api(`/plans/${state.plan.id}/orientation`,{method:'PATCH',body:{enabled,accept_disclaimer:enabled}});
    state.selected.clear();state.filter='all';$('page-filter').value='all';
    acceptPlan(plan,{collapse:state.sourceCollapsed});
    toast(enabled?'已记录同意。请完成标题识别，再人工核对全部图片。':'已关闭识别，将按原图方向和人工旋转生成。');
    refreshHistory(false).catch(()=>{});
  } finally {state.mutating=false;controls();}
}
async function analyze() {
  if (!state.plan || locked()) return;
  if(!orientationEnabled() || !hasOrientationConsent())throw new Error('请先勾选启用标题方向识别，并同意免责声明。');
  if(analysisModelBlocked())throw new Error(state.status?.model_error||'标题文字识别尚未就绪，请检查本地模型。');
  clearError();
  const job=await requestJob(`/plans/${state.plan.id}/analyze`,{method:'POST'});
  if(!job)return;
  state.selected.clear();
  toast(`已开始标题方向识别 · ${jobThreads(job)} 线程`);
}
function generationBlock(vouchers) {
  if(!orientationEnabled())return '';
  if(!hasOrientationConsent())return '尚未明确同意当前版本免责声明。请先勾选启用并同意，或关闭方向识别。';
  if(!analysisCompleted())return '本轮标题方向识别尚未完成。请先完成识别，再人工核对全部图片。';
  const pending=vouchers.flatMap(item=>item.files).filter(needsReview).length;
  return pending?`本次所选凭证还有 ${fmt(pending)} 张图片未经人工确认。请逐页核对、修正并确认，或明确勾选已核对的页面后确认。`:'';
}
function prepareGenerate(ids,retry=null) {
  if (!state.plan || locked() || !state.settingsLoaded) return;
  state.generateIds=ids;
  state.generateRetry=retry;
  const vouchers=ids?state.plan.vouchers.filter(item=>ids.includes(item.id)):state.plan.vouchers;
  const files=vouchers.flatMap(item=>item.files);
  const summary=$('generate-summary');summary.replaceChildren();
  const headline=el('div','summary-line');headline.append(document.createTextNode('本次生成 '),el('strong','',vouchers.length),document.createTextNode(' 份 PDF，共 '),el('strong','',fmt(files.reduce((sum,file)=>sum+file.pages,0))),document.createTextNode(' 页。'));
  summary.append(headline,el('div','summary-line',`使用工作表：${state.plan.sheet}`),el('div','summary-line','输出到各分项同级的「分项-PDF」文件夹。工具生成的结果更新前会保留历史；未知或已被外部修改的文件不会覆盖。'));
  summary.append(el('div','summary-line',retry?`并行上限：${jobThreads(retry)} 线程 · 沿用原任务的设置。`:`并行上限：${state.settings.threads} 线程 · 不同凭证并行，每份 PDF 内的页序保持不变。`));
  const block=generationBlock(vouchers);
  const images=files.filter(file=>file.kind==='image');
  summary.append(el('div','summary-line',orientationEnabled()?`标题识别已启用 · 所选 ${fmt(images.length)} 张图片中 ${fmt(images.filter(file=>file.reviewed).length)} 张已人工确认。`:'标题识别未启用：按原图方向（含照片自身方向信息）与人工旋转生成，无需分析或强制确认。'));
  const invalid=vouchers.filter(item=>item.issues.some(issue=>issue.severity==='error')||item.files.some(fileIssue)).length;
  if(block)summary.append(el('p','error',block));
  if (invalid) summary.append(el('p','error',`${invalid} 个凭证有扫描问题，任务会单独报告失败；其他凭证可继续处理。`));
  $('start-generate').disabled=Boolean(block);
  $('generate-dialog').showModal();
}
async function startGenerate() {
  if (!state.plan || locked()) return;
  const vouchers=state.generateIds?state.plan.vouchers.filter(item=>state.generateIds.includes(item.id)):state.plan.vouchers;
  const block=generationBlock(vouchers);
  if(block){$('start-generate').disabled=true;throw new Error(block);}
  $('start-generate').disabled=true;clearError();
  try {
    const body={};
    if (state.generateIds) body.voucher_ids=state.generateIds;
    const path=state.generateRetry?`/jobs/${state.generateRetry.id}/retry`:`/plans/${state.plan.id}/generate`;
    const job=await requestJob(path,{method:'POST',body});
    if(!job)return;
    $('generate-dialog').close();toast(`生成任务已开始 · ${jobThreads(job)} 线程`);
  } finally { controls(); }
}
function trackJob(job) {
  if (state.polling) clearTimeout(state.polling);
  state.job=job;state.pollCount=0;
  renderJob();controls();
  $('step-review').classList.remove('current');
  $(job.kind==='analyze'?'step-review':'step-export').classList.add('current');
  $('job-panel').scrollIntoView({behavior:'smooth',block:'nearest'});
  if (activeStatuses.has(job.status)) state.polling=setTimeout(pollJob,1000);
}
async function pollJob() {
  if (!state.job) return;
  try {
    const job=await api(`/jobs/${state.job.id}`);
    state.job=job;state.pollCount++;renderJob();controls();
    const done=!activeStatuses.has(job.status);
    if (state.plan?.id===job.plan_id && (done || (job.kind==='analyze' && state.pollCount%5===0))) {
      const plan=await api(`/plans/${job.plan_id}`);
      if (done) acceptPlan(plan,{collapse:state.sourceCollapsed});
      else {
        const beforePreview=filePresentation(activeFile());
        const beforePages=filteredFiles().slice(state.page*PAGE_SIZE,(state.page+1)*PAGE_SIZE).map(filePresentation).join('|');
        state.plan=plan;renderStats();renderVouchers();
        const afterPages=filteredFiles().slice(state.page*PAGE_SIZE,(state.page+1)*PAGE_SIZE).map(filePresentation).join('|');
        if(beforePages!==afterPages)renderPages();
        // A new model correction must be visible before the next manual rotation.
        if(beforePreview!==filePresentation(activeFile()))renderPreview();
      }
    }
    if (done) {
      state.polling=null;
      refreshHistory(false).catch(()=>{});
      if (job.status==='completed') toast(job.kind==='analyze'?'标题识别完成，请人工核对并确认全部图片':'PDF 生成完成');
      else if (job.status==='completed_with_errors') toast(job.kind==='analyze'?'本轮识别已结束，部分图片识别失败；请人工检查、修正并确认全部图片。':'部分任务未完成，请查看下方问题');
      return;
    }
  } catch(error) { showError(error); }
  state.polling=setTimeout(pollJob,1500);
}
function renderJob() {
  const job=state.job;if(!job)return;
  $('job-panel').hidden=false;
  $('job-title').textContent=job.kind==='analyze'?'标题文字方向识别':'PDF 生成进度';
  $('job-status').textContent=statusLabels[job.status]||job.status;
  $('job-threads').textContent=`并行上限 ${jobThreads(job)} 线程`;
  $('job-status').className='status-pill'+(['failed','completed_with_errors'].includes(job.status)?' error':['cancelled','interrupted'].includes(job.status)?' warning':'');
  const active=activeStatuses.has(job.status);
  $('cancel-job').hidden=!active;
  $('retry-job').hidden=active||job.status==='completed';
  $('download-report').hidden=active;
  $('job-current').textContent=job.current|| (active?'正在准备…':statusLabels[job.status]||'处理结束');
  $('job-progress-text').textContent=`${fmt(job.done)} / ${fmt(job.total)}`;
  $('job-progress').style.width=`${job.total?Math.min(100,100*job.done/job.total):0}%`;
  $('job-error').hidden=!job.error;$('job-error').textContent=job.error||'';
  $('job-results').replaceChildren(...(job.results||[]).map(result=>{
    const row=el('div','result-row');
    row.append(el('span','result-name',result.folder),el('span','',`${fmt(result.pages)} 页`),el('span',result.status==='failed'?'result-review':'',statusLabels[result.status]||result.status));
    if (result.output_path && result.status!=='failed') {
      const download=el('button','text-button','下载 PDF');download.type='button';
      download.addEventListener('click',action(()=>downloadBlob(`/jobs/${job.id}/outputs/${result.voucher_id}`,basename(result.output_path))));row.append(download);
    } else row.append(el('span'));
    if (result.error) row.append(el('div','result-error',result.error));
    if (result.needs_review) row.append(el('div','result-error result-review','含待复核方向，请检查后交付。'));
    return row;
  }));
  const outputs=$('output-actions');outputs.replaceChildren();
  if (job.kind==='generate' && state.plan?.id===job.plan_id) {
    const successIds=new Set((job.results||[]).filter(result=>result.output_path&&result.status!=='failed').map(result=>result.voucher_id));
    const items=[...new Set(state.plan.vouchers.filter(item=>successIds.has(item.id)).map(item=>item.item))];
    for (const item of items) {
      const button=el('button','secondary-button compact');button.type='button';button.append(icon('folder'),document.createTextNode(`打开 ${item}-PDF`));
      button.addEventListener('click',action(async()=>{await api(`/plans/${job.plan_id}/open-output`,{method:'POST',body:{item}});toast('已打开输出文件夹');}));outputs.append(button);
    }
  }
}
async function cancelJob() {
  if (!state.job) return;
  $('cancel-job').disabled=true;
  try { state.job=await api(`/jobs/${state.job.id}/cancel`,{method:'POST'});renderJob();toast('已请求取消，正在保存进度'); }
  finally { $('cancel-job').disabled=false; }
}
async function retryJob() {
  if (!state.job || locked()) return;
  if(state.job.kind==='generate'){prepareGenerate(state.job.request?.voucher_ids||null,state.job);return;}
  if(!orientationEnabled() || !hasOrientationConsent())throw new Error('请先启用标题方向识别并同意当前说明，再继续识别任务。');
  if(analysisModelBlocked())throw new Error(state.status?.model_error||'标题文字识别尚未就绪，请检查本地模型。');
  state.selected.clear();
  clearError();await requestJob(`/jobs/${state.job.id}/retry`,{method:'POST'});
}
async function downloadBlob(path,name) {
  const blob=await api(path,{blob:true});
  const url=URL.createObjectURL(blob);const link=el('a');link.href=url;link.download=name;
  document.body.append(link);link.click();link.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);
}
async function openOriginal() {
  const file=activeFile();if(!file)return;
  const popup=window.open('about:blank','_blank');
  if (popup) { popup.opener=null;popup.document.title='正在读取原件';popup.document.body.textContent='正在读取本地原件…'; }
  try {
    const blob=await api(`/plans/${state.plan.id}/files/${file.id}`,{blob:true});
    const url=URL.createObjectURL(blob);
    if (popup) popup.location.href=url;
    else { const link=el('a');link.href=url;link.download=file.name;document.body.append(link);link.click();link.remove(); }
    setTimeout(()=>URL.revokeObjectURL(url),60000);
  } catch(error) { if(popup)popup.close();throw error; }
}
async function refreshHistory(render=true) {
  if(state.loadingHistory)return;state.loadingHistory=true;
  try {
    const [plans,jobs]=await Promise.all([api('/plans'),api('/jobs')]);
    state.plans=plans;state.jobs=jobs;$('history-count').textContent=plans.length;
    if(render)renderHistory();
  } finally {state.loadingHistory=false;}
}
async function syncExternalJobs() {
  if(state.syncing || locked())return;
  state.syncing=true;
  const revision=state.settingsRevision;
  try {
    const [jobs,settings]=await Promise.all([api('/jobs'),api('/settings')]);state.jobs=jobs;
    if(revision===state.settingsRevision && !locked() && state.settingsLoaded){acceptSettings(settings);state.settingsError='';renderProcessingSettings();}
    const running=jobs.find(job=>activeStatuses.has(job.status));
    if(running && !locked() && running.id!==state.job?.id)trackJob(running);
  } catch(_) { /* The foreground operation reports service errors. */ }
  finally {state.syncing=false;}
}
function renderHistory() {
  const list=$('history-list');list.replaceChildren();
  if (!state.plans.length) list.append(el('div','card empty-inline','还没有处理记录。扫描一批资料后，计划会自动保存在这里。'));
  for (const plan of state.plans) {
    const card=el('article','card history-card');const info=el('div');
    info.append(el('h3','',basename(plan.root)),el('p','',plan.root));
    const meta=el('div','history-meta');meta.append(el('span','',`${plan.vouchers} 个凭证`),el('span','',`${fmt(plan.pages)} 页`),el('span','',plan.sheet),el('span','',`${fmt(plan.review_count)} 待人工确认`));
    info.append(meta,el('small','',time(plan.created_at)));
    const button=el('button','secondary-button compact','继续查看');button.type='button';
    button.addEventListener('click',action(async()=>{
      if(locked()&&state.job?.plan_id!==plan.id)throw new Error('当前任务仍在运行，请等待完成或取消后再切换计划。');
      acceptPlan(await api(`/plans/${plan.id}`));setView('work');
      const job=state.jobs.find(item=>item.plan_id===plan.id);
      if(job)trackJob(job);else{$('job-panel').hidden=true;state.job=null;controls();}
    }));
    card.append(info,button);list.append(card);
  }
  if(state.jobs.length) {
    const block=el('section','history-jobs');block.append(el('h2','','最近任务'));const rows=el('div','card');
    for(const job of state.jobs.slice(0,40)) {
      const row=el('div','history-job');const info=el('div');
      info.append(document.createTextNode(`${job.kind==='analyze'?'方向识别':'PDF 生成'} · ${statusLabels[job.status]||job.status}`),el('small','',`${time(job.created_at)} · ${fmt(job.done)} / ${fmt(job.total)} · ${jobThreads(job)} 线程`));
      const button=el('button','text-button','查看任务');button.type='button';
      button.addEventListener('click',action(async()=>{
        if(locked()&&state.job?.id!==job.id)throw new Error('请先等待当前任务完成或取消后，再查看其他任务。');
        acceptPlan(await api(`/plans/${job.plan_id}`));setView('work');trackJob(await api(`/jobs/${job.id}`));
      }));row.append(info,button);rows.append(row);
    }
    block.append(rows);list.append(block);
  }
}
async function initialize() {
  if (!token) {
    $('connection').classList.add('offline');$('connection').lastChild.textContent='缺少会话';
    showError(new Error('请使用本地程序启动时显示的完整链接打开工作台。链接中的会话信息用于保护本机文件。'));
    controls();return;
  }
  try {
    const [status,settings]=await Promise.all([api('/status'),api('/settings')]);
    state.status=status;acceptSettings(settings);state.settingsError='';
    if(state.plan){renderStats();renderVouchers();renderVoucher();}
    $('connection').replaceChildren(el('i'),document.createTextNode('本地服务已连接'));
    $('server-version').textContent=`v${state.status.version} · 离线工作台`;
    await refreshHistory(false);
    const running=state.jobs.find(job=>activeStatuses.has(job.status));
    if(running){acceptPlan(await api(`/plans/${running.plan_id}`));trackJob(running);}
    state.syncTimer=setInterval(syncExternalJobs,5000);
    controls();
  }catch(error){$('connection').classList.add('offline');$('connection').replaceChildren(el('i'),document.createTextNode('连接失败'));state.settingsError='设置尚未就绪，请刷新页面后重试。';showError(error);controls();}
}

$('nav-work').addEventListener('click',()=>setView('work'));
$('nav-history').addEventListener('click',()=>setView('history'));
document.querySelector('.brand').addEventListener('click',event=>{event.preventDefault();setView('work');});
$('source-form').addEventListener('submit',action(async event=>{event.preventDefault();await scan();}));
$('processing-threads').addEventListener('change',action(saveThreadSelection));
$('collapse-source').addEventListener('click',()=>setSourceCollapsed(!state.sourceCollapsed));
$('browse-root').addEventListener('click',action(()=>browse('root')));
$('browse-ledger').addEventListener('click',action(()=>browse('ledger')));
$('close-browser').addEventListener('click',()=>$('browser-dialog').close());
$('browser-up').addEventListener('click',action(()=>readDirectory(state.browser.parent)));
$('browser-path-form').addEventListener('submit',action(async event=>{event.preventDefault();await readDirectory($('browser-path-input').value.trim());}));
$('browser-choose').addEventListener('click',action(selectPath));
$('ledger-path').addEventListener('change',action(loadLedgerSheets));
$('source-sheet-select').addEventListener('change',updateSheetNote);
$('sheet-select').addEventListener('change',action(async()=>{try{await scan($('sheet-select').value);}catch(error){$('sheet-select').value=state.plan.sheet;throw error;}}));
$('voucher-search').addEventListener('input',()=>{state.search=$('voucher-search').value;renderVouchers();});
$('page-filter').addEventListener('change',()=>{
  state.filter=$('page-filter').value;state.page=0;state.selected.clear();
  const files=filteredFiles();
  if(!files.some(file=>file.id===state.fileId)){state.fileId=files[0]?.id||null;renderPreview();}
  renderPages();
});
$('prev-page').addEventListener('click',()=>{state.page--;renderPages();});
$('next-page').addEventListener('click',()=>{state.page++;renderPages();});
$('select-visible').addEventListener('change',()=>{const checked=$('select-visible').checked;filteredFiles().slice(state.page*PAGE_SIZE,(state.page+1)*PAGE_SIZE).forEach(file=>{if(checked)state.selected.add(file.id);else state.selected.delete(file.id);});renderPages();});
$('rotate-left').addEventListener('click',action(()=>saveFile(((activeFile()?effectiveRotation(activeFile()):0)+270)%360)));
$('rotate-flip').addEventListener('click',action(()=>saveFile(((activeFile()?effectiveRotation(activeFile()):0)+180)%360)));
$('rotate-right').addEventListener('click',action(()=>saveFile(((activeFile()?effectiveRotation(activeFile()):0)+90)%360)));
$('confirm-file').addEventListener('click',action(()=>saveFile(activeFile()?effectiveRotation(activeFile()):0)));
$('confirm-selection').addEventListener('click',action(confirmSelection));
$('confirm-cancel').addEventListener('click',()=>endConfirm(false));
$('confirm-accept').addEventListener('click',()=>endConfirm(true));
$('confirm-dialog').addEventListener('cancel',event=>{event.preventDefault();endConfirm(false);});
$('open-original').addEventListener('click',action(openOriginal));
$('orientation-enabled').addEventListener('change',action(event=>setOrientationMode(event.target.checked)));
$('disable-orientation').addEventListener('click',action(()=>setOrientationMode(false)));
$('analyze-button').addEventListener('click',action(analyze));
$('generate-button').addEventListener('click',()=>prepareGenerate(null));
$('generate-one').addEventListener('click',()=>prepareGenerate(state.voucherId?[state.voucherId]:null));
$('close-generate').addEventListener('click',()=>$('generate-dialog').close());
$('start-generate').addEventListener('click',action(startGenerate));
$('cancel-job').addEventListener('click',action(cancelJob));
$('retry-job').addEventListener('click',action(retryJob));
$('download-report').addEventListener('click',action(()=>downloadBlob(`/jobs/${state.job.id}/report`,`处理报告-${state.job.id}.json`)));
$('refresh-history').addEventListener('click',action(()=>refreshHistory()));
window.addEventListener('resize',fitPreviewImage);
window.addEventListener('beforeunload',()=>{clearThumbs();clearInterval(state.syncTimer);if(state.previewUrl)URL.revokeObjectURL(state.previewUrl);});
initialize();
