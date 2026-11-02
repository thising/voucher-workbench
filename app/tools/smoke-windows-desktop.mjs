// W3C WebDriver against the actual shipped Tauri EXE and WebView2. No browser
// stand-in, mock IPC, embedded test server, or test-only application build.
import assert from 'node:assert/strict';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import crypto from 'node:crypto';

const exec = promisify(execFile);
const [exe, fixtureTool, tools, work, evidence] = process.argv.slice(2);
assert(exe && fixtureTool && tools && work && evidence, 'Desktop smoke arguments are required.');
const expectedVersion=JSON.parse(await fs.readFile(path.join(path.dirname(exe),'package-manifest.json'),'utf8')).version;
const nativeScript = path.join(import.meta.dirname, 'windows-desktop-native.ps1');
const report = {schema_version:1, synthetic_only:true, native_window:true, external_browser:false,
  version:expectedVersion, checks:[], artifacts:['desktop-window.png'], customer_windows10:'pending'};
async function writeReport() {
  const target=path.join(evidence,'desktop-smoke-report.json'),temporary=target+'.tmp';
  await fs.writeFile(temporary,JSON.stringify(report,null,2));
  await fs.rename(temporary,target);
}
async function checked(name) { report.checks.push(name);report.phase=name;await writeReport(); }
function settle(promise) { return promise.then(value=>({ok:true,value}),error=>({ok:false,error})); }
async function settledValue(promise) { const result=await promise;if(!result.ok)throw result.error;return result.value; }
const owned = new Set();
const childProcesses = new Set();
let driver, session, driverPort;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, label, timeout=40000) {
  const end = Date.now()+timeout;
  while (Date.now()<end) {
    try { const value=await fn(); if(value) return value; } catch (_) { /* bounded startup/poll race */ }
    await wait(150);
  }
  throw new Error(`Timed out: ${label}`);
}
async function freePort() {
  const listener=net.createServer(); await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));
  const port=listener.address().port; await new Promise(resolve=>listener.close(resolve)); return port;
}
function safeDriverDiagnostic(value) {
  return String(value || '').replace(/#token=[^\s"<>]+/gi,'#token=[redacted]')
    .replace(/https?:\/\/[^\s"<>]+/gi,'[URL hidden]').slice(0,1600);
}
function normalizedWindowsPath(value) {
  let text=String(value || '');
  if(text.toUpperCase().startsWith('\\\\?\\UNC\\'))text='\\\\'+text.slice(8);
  else if(text.startsWith('\\\\?\\'))text=text.slice(4);
  return path.win32.normalize(text).toLowerCase();
}
async function wd(method, endpoint, body) {
  const response=await fetch(`http://127.0.0.1:${driverPort}${endpoint}`,{
    method,headers:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),
    signal:AbortSignal.timeout(180000)});
  const result=await response.json();
  if(!response.ok || result.value?.error) {
    // Preserve a bounded cause while removing URLs/session fragments before
    // it can reach the report or public CI log.
    throw new Error(`Native WebDriver ${method} failed (${response.status}, ${String(result.value?.error||'unknown').replace(/[^a-z -]/gi,'')}). ${safeDriverDiagnostic(result.value?.message)}`);
  }
  return result.value;
}
async function js(script,...args) { return wd('POST',`/session/${session}/execute/sync`,{script,args}); }
async function asyncJs(script,...args) { return wd('POST',`/session/${session}/execute/async`,{script,args}); }
async function api(endpoint,body,method=body===undefined?'GET':'POST') {
  const result=await asyncJs(`const done=arguments[arguments.length-1];api(arguments[0],{method:arguments[2],body:arguments[1]??undefined}).then(value=>done({ok:true,value}),()=>done({ok:false}));`,endpoint,body??null,method);
  assert(result.ok,`Authenticated workbench API rejected ${endpoint.replace(/[a-z0-9]{20,}/gi,'<id>')}.`);
  return result.value;
}
async function scanPlan(body) {
  const started=await api('/batch/scans',body);
  const completed=await until(async()=>{
    const progress=await api(`/batch/scans/${started.id}`);
    return progress.status==='completed' && progress.plan_id && progress;
  },'asynchronous scan completed',90000);
  return api(`/batch/plans/${completed.plan_id}`);
}
async function click(selector) { await js(`const n=document.querySelector(arguments[0]);if(!n||n.disabled)throw Error('control unavailable');n.click();`,selector); }
async function input(selector,value,event='input') {
  await js(`const n=document.querySelector(arguments[0]);n.value=arguments[1];n.dispatchEvent(new Event(arguments[2],{bubbles:true}));`,selector,String(value),event);
}
async function dragFileHandle(from,to) {
  const points=await js(`
    const handles=Array.from(document.querySelectorAll('#file-list .file-drag-handle'));
    const source=handles[arguments[0]],target=handles[arguments[1]];
    if(!source||!target||source.disabled||target.disabled)throw Error('sort handle unavailable');
    source.scrollIntoView({block:'center'});
    const point=node=>{const r=node.getBoundingClientRect();return {x:Math.round(r.left+r.width/2),y:Math.round(r.top+r.height/2)};};
    return {from:point(source),to:point(target)};`,from,to);
  // Native WebDriver input: do not replace this with dispatched PointerEvents.
  try {
    await wd('POST',`/session/${session}/actions`,{actions:[{
      type:'pointer',id:'file-sort-mouse',parameters:{pointerType:'mouse'},actions:[
        {type:'pointerMove',duration:0,origin:'viewport',...points.from},
        {type:'pointerDown',button:0},{type:'pause',duration:150},
        {type:'pointerMove',duration:600,origin:'viewport',...points.to},
        {type:'pointerUp',button:0},
      ],
    }]});
  } finally {
    await wd('DELETE',`/session/${session}/actions`);
  }
}
async function probe(directory) { return JSON.parse(await fs.readFile(path.join(directory,'desktop-probe.json'),'utf8')); }
async function native(pid,action,extra=[]) {
  report.last_native_action={action,state:'requested'};await writeReport();
  const result=await exec('pwsh',['-NoProfile','-File',nativeScript,'-OwnedProcessId',String(pid),'-Action',action,...extra],{windowsHide:true,timeout:35000,maxBuffer:100000});
  const control=JSON.parse(result.stdout);
  assert(control.controlled,'Owned native control did not confirm its action.');
  report.last_native_action={action,state:'controlled',save_target_verified:control.save_target_verified,filename_controls:control.filename_controls,expected_text_source:control.expected_text_source,button_source:control.button_source};await writeReport();
}
function start(args) {
  const child=spawn(exe,args,{stdio:['ignore','pipe','pipe'],windowsHide:false});
  childProcesses.add(child);
  // Drain without persisting/printing. A headless legacy instance intentionally
  // prints a session fragment; it remains only in this anonymous pipe.
  child.stdout.on('data',chunk=>{if(/http:\/\/127\.0\.0\.1:/.test(chunk.toString('utf8')))child.serviceReady=true;});
  child.stdout.resume();child.stderr.resume();
  return child;
}
async function ended(child,expected=0,timeout=30000) {
  await until(()=>child.exitCode!==null || child.signalCode!==null,'owned child exit',timeout);
  assert.equal(child.exitCode,expected,'Owned child returned an unexpected exit code.');
  childProcesses.delete(child);
}
async function exited(pid) {
  await until(()=>{try{process.kill(pid,0);return false;}catch{return true;}},'safe desktop exit',60000);
  owned.delete(pid);
}
async function open(state,probeDirectory,extra=[]) {
  // A crash can leave the last visible/loaded record behind. Never accept a
  // prior PID as evidence for a newly launched application.
  await fs.rm(path.join(probeDirectory,'desktop-probe.json'),{force:true});
  const result=await wd('POST','/session',{capabilities:{alwaysMatch:{'tauri:options':{
    // EdgeDriver treats args as switches; values must use '=' so a Chinese or
    // space-containing path is not parsed as another standalone switch.
    application:exe,args:[`--state-dir=${state}`,`--test-probe-dir=${probeDirectory}`,...extra]}}}});
  session=result.sessionId;
  assert(session,'Native WebDriver did not return a session.');
  await wd('POST',`/session/${session}/timeouts`,{script:90000,pageLoad:90000,implicit:0});
  const status=await until(async()=>{const p=await probe(probeDirectory);return p.webview_loaded && p.window_visible && p;},'real workbench window loaded',90000);
  owned.add(status.pid);
  await until(()=>js(`return document.querySelector('#app-version')?.textContent.includes(arguments[0]) && typeof window.__TAURI__?.core?.invoke==='function';`,expectedVersion),'authenticated native UI connected',90000);
  return status.pid;
}
async function endSession() { if(session){try{await wd('DELETE',`/session/${session}`);}catch{}session=null;} }
function hash(bytes) { return crypto.createHash('sha256').update(bytes).digest('hex'); }
async function screenshot(name) {
  const png=await wd('GET',`/session/${session}/screenshot`);
  await fs.writeFile(path.join(evidence,name),Buffer.from(png,'base64'));
}
async function nativeSave(pid,command,args,destination) {
  const pending=settle(asyncJs(`const done=arguments[arguments.length-1];window.__TAURI__.core.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value}),()=>done({ok:false}));`,command,args));
  await native(pid,'save',['-SavePath',destination,'-ExpectedText','另存处理结果']);
  return settledValue(pending);
}

await fs.mkdir(evidence,{recursive:true});
await fs.mkdir(work,{recursive:true});
report.passed=false;report.phase='starting';await writeReport();
try {
  driverPort=await freePort(); let nativePort=await freePort();while(nativePort===driverPort)nativePort=await freePort();
  driver=spawn(path.join(tools,'bin/tauri-driver.exe'),['--port',String(driverPort),'--native-port',String(nativePort),'--native-driver',path.join(tools,'edge/msedgedriver.exe')],{stdio:'ignore',windowsHide:true});
  await until(async()=>{const r=await fetch(`http://127.0.0.1:${driverPort}/status`);return r.ok;},'native driver listening',30000);
  const fixtures=path.join(work,'合成资料 含空格');
  const generated=JSON.parse((await exec(fixtureTool,['create',fixtures],{timeout:30000})).stdout);
  assert(generated.synthetic,'Only a synthetic fixture may be used.');
  const state=path.join(work,'处理记录 含空格'); const probes=path.join(work,'原生窗口探针');
  const pid=await open(state,probes);
  await checked('shipped-gui-exe-chinese-space-path-native-webview2');

  // Exact remote origin capability and native pickers, with cancellation through
  // the real Windows file dialogs owned by this process.
  for(const command of ['select_folder','select_index_file']) {
    const pending=settle(asyncJs(`const done=arguments[arguments.length-1];window.__TAURI__.core.invoke(arguments[0],{}).then(value=>done({ok:true,value}),()=>done({ok:false}));`,command));
    await native(pid,'cancel'); const result=await settledValue(pending);
    assert(result.ok && result.value===null,'A real native picker did not cancel cleanly.');
  }
  await checked('real-native-folder-and-index-picker-cancellation');
  const origin=await js(`return location.origin;`);
  await js(`location.href='https://example.invalid/blocked-navigation';`);
  await wait(300); assert.equal(await js(`return location.origin;`),origin,'Remote navigation was not blocked.');
  assert.equal(await js(`window.open('https://example.invalid/blocked-popup');return true;`),true);
  assert.equal((await wd('GET',`/session/${session}/window/handles`)).length,1,'Unexpected additional desktop window.');
  await checked('remote-navigation-and-new-window-blocked');

  await input('#root-path',path.join(fixtures,'合成分项'),'change');
  await until(()=>js(`return !document.querySelector('#merge-level').disabled && document.querySelector('#merge-level').options.length>=2;`),'actual folder levels');
  await input('#merge-level','1','change');
  await until(()=>js(`return !document.querySelector('#scan-button').disabled;`),'automatic preparation ready');
  await click('#scan-button');
  await until(()=>js(`return document.querySelector('#summary-groups').textContent==='2' && document.querySelectorAll('#file-list .file-row').length===3;`),'three-pane scan');
  assert.equal(await js(`return document.querySelector('#summary-pages').textContent;`),'8');
  assert.equal(await js(`return document.querySelector('#initialize-folder')===null && !document.querySelector('#scan-results').hidden;`),true);
  const scanJob=(await api('/jobs')).find(job=>job.kind==='batch_scan' && job.status==='completed');
  assert(scanJob,'The UI did not finish a registered asynchronous scan.');
  const scanProgress=await api(`/batch/scans/${scanJob.id}`);
  assert.equal(scanProgress.counts.image_files,4);assert.equal(scanProgress.counts.pdf_files,2);
  assert.equal(scanProgress.counts.source_errors,0);
  await checked('real-ui-automatic-folder-initialization-asynchronous-scan-and-three-pane');
  await js(`Array.from(document.querySelectorAll('#file-list .file-select')).find(n=>n.textContent.includes('.pdf')).click();`);
  await until(()=>js(`const c=document.querySelector('#preview-surface canvas');return !!c && c.width>0 && c.height>0;`),'offline PDF.js renders native PDF',60000);
  await input('#source-page-number',2,'change');
  await click('#rotate-right');
  await until(async()=>{const plans=await api('/batch/plans');const p=plans.find(p=>p.groups.some(g=>g.files.some(f=>f.kind==='pdf'&&f.page_rotations?.[1]===90)));return p;},'PDF manual page-two rotation');
  assert.equal(await js(`return document.querySelector('#rotation-scope').value;`),'page');
  assert.equal(await js(`return document.querySelector('#reviewed-file')===null;`),true);
  await screenshot('desktop-window.png');
  await checked('offline-pdfjs-real-canvas-and-per-page-manual-rotation');
  const viewport=await js(`const area=document.querySelector('.work-area'),pane=document.querySelector('.preview-pane').getBoundingClientRect(),zoom=document.querySelector('#zoom-in').getBoundingClientRect();return {width:innerWidth,height:innerHeight,work_area_width:area.clientWidth,work_area_scroll_width:area.scrollWidth,preview_right:pane.right,zoom_right:zoom.right};`);
  assert(viewport.width>=1000,'The native host did not provide the supported minimum viewport.');
  assert(viewport.work_area_scroll_width<=viewport.work_area_width+1,'The three panes overflow the native viewport.');
  assert(viewport.preview_right<=viewport.width && viewport.zoom_right<=viewport.preview_right,'Native preview controls are clipped.');
  report.viewport=viewport;
  await checked('real-ui-three-pane-and-preview-controls-visible-at-native-viewport');

  // Exercise the native pointer gesture, then the existing move/top buttons.
  await dragFileHandle(2,0);
  await until(()=>js(`return document.querySelector('#file-list .file-select').textContent.includes('.pdf') && document.querySelector('#edit-state').textContent==='调整已保存';`),'native pointer sorting saved');
  await checked('native-webdriver-pointer-handle-reorder-and-autosave');
  await js(`document.querySelectorAll('#file-list .file-moves')[0].querySelectorAll('button')[2].click();`);
  await until(()=>js(`return !document.querySelector('#file-list .file-select').textContent.includes('.pdf') && document.querySelector('#edit-state').textContent==='调整已保存';`),'move-down button saved');
  await js(`document.querySelectorAll('#file-list .to-top')[1].click();`);
  await until(()=>js(`return document.querySelector('#file-list .file-select').textContent.includes('.pdf') && document.querySelector('#edit-state').textContent==='调整已保存';`),'top button saved');
  await input('#group-list .group-rename input','桌面合成成品.pdf');
  await until(()=>js(`return document.querySelector('#edit-state').textContent==='调整已保存' && !document.querySelector('#merge-button').disabled;`),'order and rename autosaved');
  const renamedPlan=await until(async()=>{
    const plans=await api('/batch/plans');
    return plans.find(plan=>plan.groups.some(group=>group.output_name==='桌面合成成品.pdf' && group.files[0]?.kind==='pdf'));
  },'persisted automatic order and filename save');
  assert(renamedPlan,'The automatic edit save did not persist file order and the output name.');
  await click('#merge-button');
  await until(()=>js(`return document.querySelector('#merge-dialog').open;`),'merge dialog');
  await input('#merge-threads',3,'change');
  await click('#start-merge');
  const completed=await until(async()=>{const jobs=await api('/jobs');const j=jobs.find(j=>j.kind==='batch_generate');return j && j.status==='completed' && j;},'desktop batch generation',90000);
  assert.equal(completed.threads,3);assert.equal(completed.results.length,2);
  assert.equal(completed.results.reduce((n,r)=>n+r.pages,0),8);
  const delivered=completed.results.find(r=>path.basename(r.output_path||'')==='桌面合成成品.pdf');assert(delivered,'UI rename did not reach generated output.');
  const outputBytes=await fs.readFile(delivered.output_path);assert(outputBytes.subarray(0,4).equals(Buffer.from('%PDF')));
  await checked('real-ui-manual-order-rename-threads-merge-without-review-gate');

  const savedPDF=path.join(work,'原生另存 副本.pdf');
  const copy=await nativeSave(pid,'save_job_output',{jobId:completed.id,groupId:delivered.voucher_id},savedPDF);
  assert(copy.ok && copy.value.saved);
  assert.equal(normalizedWindowsPath(copy.value.path),normalizedWindowsPath(savedPDF),'Native PDF save selected an unexpected destination.');
  assert.equal(hash(await fs.readFile(savedPDF)),hash(outputBytes));
  const savedReport=path.join(work,'原生另存 报告.json');
  const saved=await nativeSave(pid,'save_job_report',{jobId:completed.id},savedReport);
  assert(saved.ok && saved.value.saved);
  assert.equal(normalizedWindowsPath(saved.value.path),normalizedWindowsPath(savedReport),'Native report save selected an unexpected destination.');
  assert.equal(JSON.parse(await fs.readFile(savedReport,'utf8')).job.id,completed.id);
  await checked('real-native-save-pdf-and-report-verified-bytes');

  // Existing destinations are runtime decisions, including outputs produced
  // by this tool. Exercise the real controls rather than auto-answering APIs.
  const previousHashes=new Map(await Promise.all(completed.results.map(async r=>[r.output_path,hash(await fs.readFile(r.output_path))])));
  async function repeatMergeToConflict() {
    await until(()=>js(`return !document.querySelector('#merge-button').disabled;`),'merge available after prior task');
    const oldIds=new Set((await api('/jobs')).map(j=>j.id));
    await click('#merge-button');
    await until(()=>js(`return document.querySelector('#merge-dialog').open;`),'repeat merge dialog');
    await click('#start-merge');
    const task=await until(async()=>{const jobs=await api('/jobs');return jobs.find(j=>j.kind==='batch_generate'&&!oldIds.has(j.id));},'new conflict merge task');
    await until(()=>js(`return document.querySelector('#output-conflict-dialog').open;`),'runtime same-name dialog');
    const pending=await api(`/jobs/${task.id}`);
    assert.equal(pending.status,'running');assert(pending.output_conflict?.id);
    assert(await js(`return document.querySelector('#output-conflict-path').textContent.length>0 && !document.querySelector('#output-conflict-apply').checked;`),'Conflict details or fresh checkbox missing.');
    return task;
  }
  const skipTask=await repeatMergeToConflict();
  await screenshot('desktop-output-conflict.png');
  await click('#output-conflict-apply');await click('#output-conflict-skip');
  const skipped=await until(async()=>{const j=await api(`/jobs/${skipTask.id}`);return j.status==='completed'&&j;},'skip applies to remaining batch');
  assert.equal(skipped.output_conflict,null);
  assert.equal(skipped.results.length,2);
  assert(skipped.results.every(r=>r.status==='skipped_existing'&&r.pages===0&&r.output_path===null));
  assert.equal((await api(`/jobs/${skipTask.id}/report`)).successful_pages,0);
  for(const [target,expected] of previousHashes) assert.equal(hash(await fs.readFile(target)),expected);
  await until(()=>js(`return !document.querySelector('#output-conflict-dialog').open;`),'skip dialog closes');
  const cancelTask=await repeatMergeToConflict();
  await click('#output-conflict-cancel');
  const cancelled=await until(async()=>{const j=await api(`/jobs/${cancelTask.id}`);return j.status==='cancelled'&&j;},'cancel while conflict pending');
  assert.equal(cancelled.output_conflict,null);
  for(const [target,expected] of previousHashes) assert.equal(hash(await fs.readFile(target)),expected);
  await until(()=>js(`return !document.querySelector('#output-conflict-dialog').open;`),'cancel dialog closes');
  const overwriteTask=await repeatMergeToConflict();
  await click('#output-conflict-apply');await click('#output-conflict-overwrite');
  const overwritten=await until(async()=>{const j=await api(`/jobs/${overwriteTask.id}`);return j.status==='completed'&&j;},'overwrite applies to remaining batch',90000);
  assert.equal(overwritten.results.length,2);
  assert(overwritten.results.every(r=>r.status==='completed'&&r.output_path));
  assert.equal(overwritten.results.reduce((n,r)=>n+r.pages,0),8);
  assert.equal(overwritten.output_conflict,null);
  async function noArchives(directory) {
    for(const entry of await fs.readdir(directory,{withFileTypes:true})) {
      assert.notEqual(entry.name,'.history','Batch overwrite retained an archive.');
      assert(!/\.(?:partial\.pdf|previous|bak|backup)$/i.test(entry.name),'A completed overwrite retained a temporary/backup file.');
      if(entry.isDirectory()) await noArchives(path.join(directory,entry.name));
    }
  }
  await noArchives(path.dirname(completed.results[0].output_path));
  await noArchives(path.join(state,'outputs'));
  await checked('real-ui-runtime-output-conflict-skip-direct-overwrite-batch-scope-cancel-without-backups');

  await native(pid,'minimize');
  const before=(await probe(probes)).activations;
  await ended(start(['--state-dir',state]));
  await until(async()=>{const p=await probe(probes);return p.activations>before && p.window_visible && p.window_minimized===false;},'duplicate activation restores existing window');
  assert.equal((await probe(probes)).pid,pid);
  await checked('duplicate-launch-restores-original-window-and-exits-zero');

  // A bounded long OCR task allows proving activation/close semantics against
  // real work; each image is a copy of our generated bitmap, never customer data.
  const busyRoot=path.join(work,'合成后台任务');await fs.mkdir(busyRoot);
  const syntheticImage=path.join(fixtures,'合成分项','260101-00001','2601_00001_1.jpg');
  for(let i=0;i<256;i++) await fs.copyFile(syntheticImage,path.join(busyRoot,`image_${String(i).padStart(4,'0')}.jpg`));
  const busyPlan=await scanPlan({root:busyRoot,level:0});
  const busyJob=await api(`/batch/plans/${busyPlan.id}/analyze`,{accept_disclaimer:true,threads:1});
  await until(async()=>['queued','running'].includes((await api(`/jobs/${busyJob.id}`)).status),'actual OCR task active');
  const busyBefore=await api(`/jobs/${busyJob.id}`);const count=(await probe(probes)).activations;
  await ended(start(['--state-dir',state]));
  await until(async()=>(await probe(probes)).activations>count,'task-running activation');
  const busyAfter=await api(`/jobs/${busyJob.id}`);
  assert(['queued','running'].includes(busyAfter.status),'Synthetic OCR finished before the task-running activation could be verified.');
  assert(busyAfter.done>=busyBefore.done,'Repeated startup reset task progress.');
  await native(pid,'close');
  await native(pid,'cancel',['-ExpectedText','任务或文件保存仍在进行','-ButtonText','继续处理']);
  await until(async()=>(await probe(probes)).event==='close_cancelled_task_continues','native continue choice');
  assert(['queued','running'].includes((await api(`/jobs/${busyJob.id}`)).status),'Continue choice stopped the task.');
  await native(pid,'close');await native(pid,'accept',['-ExpectedText','安全停止并退出','-ButtonText','安全停止并退出']);
  await exited(pid);await endSession();
  const stopped=JSON.parse(await fs.readFile(path.join(state,'jobs',`${busyJob.id}.json`),'utf8'));
  assert.equal(stopped.status,'cancelled','Safe close did not persist a cancelled task.');
  await checked('real-ocr-task-activation-progress-continue-and-safe-close');

  let restarted=await open(state,probes);
  assert((await api('/jobs')).some(j=>j.id===completed.id),'Prior output history was lost.');
  assert.equal((await api('/batch/preferences')).merge_threads,3,'Batch thread preference was lost.');
  await native(restarted,'close');await exited(restarted);await endSession();
  await checked('normal-restart-history-results-and-thread-preferences');
  restarted=await open(state,probes);
  process.kill(restarted,'SIGKILL');await exited(restarted);await endSession();
  restarted=await open(state,probes);await native(restarted,'close');await exited(restarted);await endSession();
  await checked('abnormal-exit-restart-without-lock-file-deletion');

  // Startup races require no WebDriver session; the actual WebView still emits
  // only bounded readiness/visibility evidence, with no discovery URL or token.
  for(const [name,delay] of [['starting-instance',1800],['simultaneous-launch',0]]) {
    const d=path.join(work,name); const p=path.join(d,'probe');
    const one=start(['--state-dir',path.join(d,'state'),'--test-probe-dir',p,'--startup-delay-ms',String(delay)]);
    if(delay) await until(async()=>{try{return (await fs.stat(path.join(d,'state','instance.lock'))).isFile();}catch{return false;}},'delayed primary published its record lock');
    const two=start(['--state-dir',path.join(d,'state'),'--test-probe-dir',p]);
    if(delay) {await wait(250);assert.equal(two.exitCode,null,'Secondary exited before the delayed primary was ready.');}
    const ready=await until(async()=>{const s=await probe(p);return s.webview_loaded&&s.activations>=2&&s;},name,90000);
    owned.add(ready.pid);const secondary=one.pid===ready.pid?two:one;
    await ended(secondary);await native(ready.pid,'close');await exited(ready.pid);
    const primary=one.pid===ready.pid?one:two;await ended(primary);
    await checked(name+'-one-real-window');
  }

  // A legacy HTTP instance uses protocol 1. Desktop protocol 2 must explain the
  // mismatch and must never ask that legacy process to open an external browser.
  const legacyState=path.join(work,'legacy-state');
  const legacy=start(['--headless','--no-browser','--state-dir',legacyState]);
  await until(()=>legacy.serviceReady,'legacy service fully ready');
  const blocked=start(['--state-dir',legacyState]);
  await native(blocked.pid,'error',['-ExpectedText','不同界面版本']);await ended(blocked,1);
  assert(legacy.exitCode===null,'The desktop ended the legacy process.');legacy.kill();await until(()=>legacy.exitCode!==null||legacy.signalCode!==null,'owned legacy cleanup');childProcesses.delete(legacy);
  await checked('legacy-protocol-instance-explicit-error-without-activation');
  const missing=start(['--state-dir',path.join(work,'missing-webview-state'),'--test-force-missing-webview2']);
  await native(missing.pid,'error',['-ExpectedText','MicrosoftEdgeWebView2RuntimeInstallerX64.exe']);await ended(missing,1);
  await checked('missing-webview2-same-native-error-path-and-instructions');
  const invalid=path.join(work,'state-is-a-file');await fs.writeFile(invalid,'synthetic');
  const failed=start(['--state-dir',invalid]);
  await native(failed.pid,'error',['-ExpectedText','无法启动']);await ended(failed,1);
  await checked('state-directory-failure-visible-native-dialog');
  report.passed=true;
} catch(error) {
  report.passed=false;
  report.failure=String(error.message).replace(/#token=[^\s"<>]+/gi,'#token=[redacted]');
  await writeReport();
  throw new Error(report.failure);
} finally {
  await endSession();
  for(const child of childProcesses) if(child.exitCode===null && child.signalCode===null) child.kill();
  for(const pid of owned) {try{process.kill(pid,'SIGKILL');}catch{}}
  driver?.kill();
  await writeReport();
}
console.log(`Native desktop verification passed (${report.checks.length} checks, synthetic inputs only).`);
