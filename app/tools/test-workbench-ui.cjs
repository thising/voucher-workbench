'use strict';
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const path=require('node:path');
const web=path.resolve(__dirname,'../web');
const html=fs.readFileSync(path.join(web,'workbench.html'),'utf8');
const pointerCaptures=new Map(),captureCalls=[],releaseCalls=[],frames=new Map();let nextFrame=0,pointerHit=null;
class Node {
 constructor(tag='div'){this.tagName=tag.toUpperCase();this.children=[];this.parentNode=null;this.value='';this.textContent='';this.hidden=false;this.disabled=false;this.checked=false;this.style={};this.dataset={};this.listeners={};this.scrollTop=0;this.attributes={};this.className='';this.clientWidth=500;this.clientHeight=400;this.classList={contains:x=>this.className.split(' ').includes(x),add:(...x)=>{this.className=[...new Set([...this.className.split(' '),...x])].join(' ');},remove:(...x)=>{this.className=this.className.split(' ').filter(c=>!x.includes(c)).join(' ');},toggle:(x,on)=>{const yes=on===undefined?!this.className.split(' ').includes(x):on;if(yes)this.classList.add(x);else this.classList.remove(x);return yes;}};}
 append(...nodes){for(let n of nodes){if(typeof n==='string')n=Object.assign(new Node('#text'),{textContent:n});n.parentNode=this;this.children.push(n);}}
 replaceChildren(...nodes){for(const child of this.children)child.parentNode=null;this.children=[];this.append(...nodes);if(this.tagName==='SELECT')this.value=this.children[0]?.value??'';}
 get options(){return this.children;}
 setAttribute(k,v){this.attributes[k]=String(v);}
 addEventListener(k,v){(this.listeners[k]??=[]).push(v);}
 closest(selector){for(let node=this;node;node=node.parentNode)if(selector.startsWith('.')&&node.className.split(' ').includes(selector.slice(1)))return node;return null;}
 contains(node){for(let current=node;current;current=current.parentNode)if(current===this)return true;return false;}
 get isConnected(){return body.contains(this);}
 getBoundingClientRect(){return{left:0,top:0,right:500,bottom:400};}
 get scrollHeight(){return this.testScrollHeight??400;}
 setPointerCapture(id){if(this.failCapture)throw new Error('capture unavailable');if(this.silentCapture)return;pointerCaptures.set(id,this);captureCalls.push([this,id]);}
 hasPointerCapture(id){return pointerCaptures.get(id)===this;}
 releasePointerCapture(id){assert(this.hasPointerCapture(id));pointerCaptures.delete(id);releaseCalls.push([this,id]);for(const record of document.listeners.lostpointercapture||[])record.fn({type:'lostpointercapture',target:this,pointerId:id});}
 querySelectorAll(selector){
  const matches=(node,part)=>{
   if(part.startsWith('.'))return node.className.split(' ').includes(part.slice(1));
   const tag=part.match(/^[a-z]+/i)?.[0];if(tag&&node.tagName!==tag.toUpperCase())return false;
   const attr=part.match(/\[([^=\]]+)(?:="?([^"\]]*)"?)?\]/);if(!attr)return Boolean(tag);
   const name=attr[1],expected=attr[2];let value;
   if(name.startsWith('data-'))value=node.dataset[name.slice(5).replace(/-([a-z])/g,(_,c)=>c.toUpperCase())];
   else value=node[name]??node.attributes[name];
   return expected===undefined?value!==undefined:String(value)===expected;
  };
  const all=[];const walk=node=>{for(const child of node.children){if(selector.split(',').some(part=>matches(child,part.trim())))all.push(child);walk(child);}};walk(this);return all;
 }
 querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
 focus(){document.activeElement=this;}
 setSelectionRange(start,end){this.selectionStart=start;this.selectionEnd=end;}
 scrollIntoView(){}
 showModal(){this.open=true;this.showCount=(this.showCount||0)+1;}
 close(){this.open=false;for(const fn of this.listeners.close||[])fn({});}
 remove(){if(this.parentNode)this.parentNode.children=this.parentNode.children.filter(n=>n!==this);this.parentNode=null;}
 click(){}
}
const nodes=new Map(),body=new Node('body');let preparation,brand;const stack=[body];
for(const match of html.matchAll(/<(\/?)([a-z][a-z0-9]*)\b([^>]*)>/gi)){
 const closing=match[1],tag=match[2].toLowerCase(),attrs=match[3];
 if(closing){const index=stack.findLastIndex(node=>node.tagName===tag.toUpperCase());if(index>0)stack.length=index;continue;}
 const node=new Node(tag);node.value=attrs.match(/\bvalue="([^"]*)"/)?.[1]||'';node.type=attrs.match(/\btype="([^"]*)"/)?.[1];node.className=attrs.match(/\bclass="([^"]*)"/)?.[1]||'';node.hidden=/\bhidden\b/.test(attrs);node.disabled=/\bdisabled\b/.test(attrs);const id=attrs.match(/\bid="([^"]+)"/)?.[1];if(id){assert(!nodes.has(id));nodes.set(id,node);}stack.at(-1).append(node);if(node.className.split(' ').includes('preparation'))preparation=node;if(node.className.split(' ').includes('brand'))brand=node;if(!['meta','link','input','br','hr','img'].includes(tag))stack.push(node);
}
for(const m of html.matchAll(/<select\b[^>]*\bid="([^"]+)"[^>]*>([\s\S]*?)<\/select>/g)){const node=nodes.get(m[1]);node.value=node.options[0]?.value||'';}
for(const m of html.matchAll(/<textarea\b[^>]*\bid="([^"]+)"[^>]*>([\s\S]*?)<\/textarea>/g))nodes.get(m[1]).value=m[2];
const document={activeElement:body,getElementById:id=>{assert(nodes.has(id),`missing ${id}`);return nodes.get(id);},createElement:tag=>new Node(tag),createTextNode:text=>Object.assign(new Node('#text'),{textContent:text}),querySelector:selector=>selector==='.brand'?brand:preparation,body};
document.listeners={};document.addEventListener=(type,fn,capture=false)=>{(document.listeners[type]??=[]).push({fn,capture});};
document.elementFromPoint=()=>pointerHit;

const clone=value=>structuredClone(value);
const files=[{id:'f1',name:'01.jpg',kind:'image',pages:1},{id:'f2',name:'02.pdf',kind:'pdf',pages:2},{id:'f3',name:'03.jpg',kind:'image',pages:1},{id:'bad',name:'04-broken.pdf',kind:'pdf',pages:0,error:'不是有效的PDF文件'}].map(file=>({relative_path:'A/'+file.name,sha256:file.id,bytes:100,modified_ms:1,rotation_cw:0,page_rotations:[],reviewed:false,reviewed_pages:[],manual_rotation:false,orientation:null,orientation_error:null,error:null,...file}));
files[1].page_rotations=[0,0];files[1].reviewed_pages=[false,false];
files[1].pdf_has_signature=true;
const group=(id,name,items)=>({id,name,relative_path:name,output_name:name+'.pdf',files:items,pages:items.reduce((n,file)=>n+file.pages,0),issues:[],empty:false,shallow:false,sort:{mode:'name',descending:false},rename:{status:'folder',message:'按文件夹名命名'}});
const fixture={id:'p0',parent_plan_id:null,created_at:'2026-11-02T00:00:00Z',root:'/fixture/input',output_dir:'/fixture/input-PDF',level:1,sort:{mode:'name',descending:false},exclude_rules:'',groups:[group('g1','A',files),group('g2','B',[{...files[0],id:'f4',relative_path:'B/01.jpg'}])],issues:[],exclusions:[],ignored_files:0,empty_directories:[],index:null,removed_sources:[],orientation_workflow:{threads:4,analysis_completed:false}};
let plans=[],jobs=[],preferences={orientation_threads:7,merge_threads:3},calls=[],version=0,scans=new Map(),deferredPreview=null,previewResolve=null;
let conflictFailure=false,deferConflict=false,conflictResolve=null,deferJobGet=false,jobGetResolve=null,cancelFailure=false,nextConflictAfterDecision=null;
const summaries=()=>plans.map(plan=>({id:plan.id,parent_plan_id:plan.parent_plan_id,created_at:plan.created_at,root:plan.root,output_dir:plan.output_dir,level:plan.level,index:plan.index,group_count:plan.groups.length,file_count:plan.groups.flatMap(g=>g.files).length,pages:plan.groups.reduce((n,g)=>n+g.pages,0)}));
function metadata(request,rename=false){
 const groups=Number(request.level)===0?[group('root','',[...clone(files),{...clone(files[0]),id:'f4',relative_path:'B/01.jpg'}])]:clone(fixture.groups);
 for(const g of groups){g.name=g.name||'input';g.path=request.root+'/'+g.relative_path;g.candidate_count=g.files.length;g.image_count=g.files.filter(f=>f.kind==='image').length;g.pdf_count=g.files.length-g.image_count;g.bytes=g.files.length*100;g.empty=!g.files.length;g.issues=[];if(rename&&request.index){g.output_name=request.index.naming==='folder_index'?g.name+'-新名称.pdf':'新名称.pdf';g.rename={status:'matched',message:'已匹配'};}else g.output_name=g.name+'.pdf';const manual=request.output_names?.find(item=>item.relative_path===g.relative_path);if(manual)g.output_name=manual.output_name.toLowerCase().endsWith('.pdf')?manual.output_name:manual.output_name+'.pdf';delete g.files;delete g.pages;}
 return {root:request.root,output_dir:request.root+'-PDF',level:Number(request.level||0),max_level:2,levels:[0,1,2].map(level=>({level,folder_count:level?2:1,file_count:5,samples:['A','B']})),groups,summary:{group_count:groups.length,nonempty_groups:groups.filter(g=>!g.empty).length,candidate_count:5,image_count:3,pdf_count:2,bytes:500},exclusions:[],issues:[],ignored_files:0,index:request.index?{config:request.index}:null,index_error:null,elapsed_ms:2};
}
function refreshIssues(plan){for(const g of plan.groups){g.empty=!g.files.length;g.pages=g.files.reduce((n,f)=>n+f.pages,0);g.issues=g.issues.filter(issue=>!['source_error','output_collision','pdf_signature'].includes(issue.code));for(const file of g.files)if(file.error)g.issues.push({severity:'error',code:'source_error',message:file.error,path:file.relative_path});if(g.files.some(file=>file.pdf_has_signature))g.issues.push({severity:'warning',code:'pdf_signature',message:'含签名PDF：静态合并不保留签名验证，原文件不变。'});}for(const g of plan.groups)if(!g.empty&&plan.groups.some(other=>other.id!==g.id&&!other.empty&&other.output_name.toLowerCase()===g.output_name.toLowerCase()))g.issues.push({severity:'error',code:'output_collision',message:'存在同名输出'});}
function createPlan(request){const result={...clone(fixture),id:'p'+(++version),created_at:`2026-11-02T00:${String(version).padStart(2,'0')}:00Z`,root:request.root,level:request.level,index:request.index?{config:request.index,rows:4}:null};const names=metadata(request,true).groups;for(const g of result.groups)g.output_name=names.find(item=>item.relative_path===g.relative_path)?.output_name||g.output_name;refreshIssues(result);plans.push(result);return result;}
async function fetchMock(url,options={}){
 const method=options.method||'GET',b=options.body?JSON.parse(options.body):null;calls.push({url,method,body:b});let result;
 if(url==='/api/status')result={version:'0.6.0',model_ready:true,running_jobs:jobs.filter(j=>['queued','running'].includes(j.status))};
 else if(url==='/api/batch/preferences'){if(method==='PATCH')preferences=b;result=preferences;}
 else if(url==='/api/batch/plans?summary=true')result=summaries();
 else if(url==='/api/jobs')result=jobs;
 else if(/^\/api\/jobs\/[^/]+$/.test(url)){result=clone(jobs.find(j=>j.id===url.split('/').at(-1)));if(deferJobGet){deferJobGet=false;await new Promise(resolve=>{jobGetResolve=resolve;});}}
 else if(url==='/api/batch/initialize')result=metadata(b);
 else if(url==='/api/batch/preview'){result=metadata(b,true);if(deferredPreview){deferredPreview=false;await new Promise(resolve=>{previewResolve=resolve;});}}
 else if(url==='/api/batch/index/inspect')result={path:b.path,sha256:'index',sheet:b.sheet||'Sheet 1',header_row:b.header_row||1,sheets:[{name:'Sheet 1',rows:4,columns:2},{name:'Empty',rows:0,columns:0}],columns:b.sheet==='Empty'?[]:[{index:0,letter:'A',name:'目录编号'},{index:1,letter:'B',name:'输出名称'}],preview_rows:b.sheet==='Empty'?[]:[{row:2,values:['A','新名称']}],suggested_index_column:b.sheet==='Empty'?null:0,suggested_name_column:b.sheet==='Empty'?null:1,sheet_error:b.sheet==='Empty'?'当前工作表为空':null};
 else if(url==='/api/batch/scans'){
  const id='scan'+(scans.size+1);result={id,status:'queued',plan_id:null,phase:'enumerating',done:0,total:0,elapsed_ms:0,timings:{metadata_ms:2},counts:{image_files:0,pdf_files:0},logs:[{level:'info',message:'正在获取目录'}],error:null};scans.set(id,{progress:result,request:b,polls:0});jobs.push({id,plan_id:'',kind:'batch_scan',status:'queued',created_at:'2026-11-02T00:00:00Z',request:{},threads:4,results:[]});
 }
 else if(/^\/api\/batch\/scans\/[^/]+$/.test(url)){
  const entry=scans.get(url.split('/').at(-1)),job=jobs.find(j=>j.id===entry.progress.id);entry.polls++;
  if(job.status==='cancelled')entry.progress.status='cancelled';
  else if(entry.polls<2){entry.progress={...entry.progress,status:'running',phase:'validating',done:2,total:5,current_file:'A/02.pdf',elapsed_ms:25};job.status='running';}
  else{const plan=entry.progress.plan_id?plans.find(p=>p.id===entry.progress.plan_id):createPlan(entry.request);entry.progress={...entry.progress,status:'completed',plan_id:plan.id,phase:'completed',done:5,total:5,elapsed_ms:45,counts:{image_files:3,pdf_files:2,source_errors:1},logs:[{level:'error',message:'坏PDF'},{level:'info',message:'扫描完成'}]};job.status='completed';job.plan_id=plan.id;}result=entry.progress;
 }
 else if(/^\/api\/batch\/plans\/[^/]+$/.test(url)){result=plans.find(p=>p.id===url.split('/').at(-1));assert(result,'plan exists');}
 else if(url.endsWith('/edit')){
  const p=plans.find(p=>p.id===url.split('/').at(-2));result={...clone(p),id:'p'+(++version),parent_plan_id:p.id,created_at:`2026-11-02T00:${String(version).padStart(2,'0')}:00Z`};
  if(b.output_dir){result.output_dir=b.output_dir;for(const g of result.groups)g.issues=g.issues.filter(issue=>issue.code!=='output_existing');}
  for(const id of b.remove_file_ids||[])for(const g of result.groups){const position=g.files.findIndex(f=>f.id===id);if(position>=0){const[file]=g.files.splice(position,1);result.removed_sources.push({group_id:g.id,position,file});}}
  for(const id of b.restore_file_ids||[]){const source=result.removed_sources.find(s=>s.file.id===id);const g=result.groups.find(g=>g.id===source.group_id);g.files.splice(source.position,0,source.file);result.removed_sources=result.removed_sources.filter(s=>s.file.id!==id);}
  for(const change of b.groups||[]){const g=result.groups.find(g=>g.id===change.group_id);if(change.output_name!==undefined)g.output_name=change.output_name.toLowerCase().endsWith('.pdf')?change.output_name:change.output_name+'.pdf';if(change.file_ids){const map=new Map(g.files.map(f=>[f.id,f]));g.files=change.file_ids.map(id=>map.get(id));g.sort={mode:'manual',descending:false};}if(change.sort){g.sort=change.sort;g.files.sort((a,b)=>a.name.localeCompare(b.name));}}
  for(const change of b.files||[]){const f=result.groups.flatMap(g=>g.files).find(f=>f.id===change.file_id);if(change.rotation_cw!==undefined||change.page_rotations){f.reviewed=false;f.reviewed_pages=f.reviewed_pages.map(()=>false);f.manual_rotation=true;f.auto_rotation_change=null;}Object.assign(f,Object.fromEntries(Object.entries(change).filter(([k])=>k!=='file_id')));}
  refreshIssues(result);plans.push(result);
 }
 else if(/^\/api\/batch\/jobs\/[^/]+\/output-conflict$/.test(url)){
  if(conflictFailure){conflictFailure=false;return{ok:false,status:409,json:async()=>({error:'选择暂未提交'})};}
  const job=jobs.find(j=>j.id===url.split('/').at(-2)),conflict=job.output_conflict;
  if(!conflict||conflict.id!==b.conflict_id)return{ok:false,status:409,json:async()=>({error:'同名选择已过期'})};
  assert(['skip','overwrite'].includes(b.action));assert.equal(typeof b.apply_to_batch,'boolean');
  job.results.push({voucher_id:conflict.group_id,folder:conflict.group_name,status:b.action==='skip'?'skipped_existing':'completed',pages:b.action==='skip'?0:2,output_path:b.action==='skip'?null:conflict.output_path,error:null});job.output_conflict=nextConflictAfterDecision;nextConflictAfterDecision=null;result=clone(job);
  if(deferConflict){deferConflict=false;await new Promise(resolve=>{conflictResolve=resolve;});}
 }
 else if(url.endsWith('/cancel')){if(cancelFailure){cancelFailure=false;return{ok:false,status:500,json:async()=>({error:'取消暂未完成'})};}result=jobs.find(j=>j.id===url.split('/').at(-2));result.status='cancelled';result.output_conflict=null;}
 else if(url.endsWith('/analyze')||url.endsWith('/generate')||url.endsWith('/retry')){
  const retry=url.endsWith('/retry'),old=retry?jobs.find(j=>j.id===url.split('/').at(-2)):null,p=plans.find(p=>p.id===(old?.plan_id||url.split('/').at(-2))),analysis=old?old.kind==='batch_analyze':url.endsWith('/analyze');let target=p;
  if(analysis&&!retry){target={...clone(p),id:'p'+(++version),parent_plan_id:p.id};for(const f of target.groups.flatMap(g=>g.files).filter(f=>f.kind==='image')){f.orientation={rotation_cw:0,title:'资料标题',reason:'标题方向',needs_review:false};f.auto_rotation_change=null;if(f.id==='f1'){f.rotation_cw=90;f.auto_rotation_change={from_cw:0,to_cw:90};}else if(f.id==='f3'){f.rotation_cw=0;f.auto_rotation_change={from_cw:90,to_cw:0};}}plans.push(target);}
  result={id:'j'+(jobs.length+1),plan_id:target.id,kind:analysis?'batch_analyze':'batch_generate',status:'running',threads:old?.threads||b.threads,request:{voucher_ids:old?.request.voucher_ids||b.group_ids},results:analysis?target.groups.flatMap(g=>g.files).filter(f=>f.auto_rotation_change).map(f=>({voucher_id:f.id,folder:f.relative_path,status:'completed',pages:1,output_path:null,rotation_change:f.auto_rotation_change})):[],orientation_summary:analysis?{changed:2,unchanged:1,skipped:0,failed:0}:null,done:analysis?3:0,total:3,created_at:'2026-11-02T01:00:00Z'};jobs.push(result);
 }
 else throw new Error('unmocked '+url);
 return {ok:true,json:async()=>clone(result)};
}
const nativeCalls=[];
const context=vm.createContext({console,document,location:{hash:'#token=test',pathname:'/',search:''},sessionStorage:{getItem:()=>null,setItem(){}},history:{replaceState(){}},URLSearchParams,Headers,fetch:fetchMock,URL,Blob,Uint8Array,Map,Set,Date,Math,Number,String,Boolean,Error,Promise,window:{devicePixelRatio:1,__TAURI__:{core:{invoke:async(...args)=>{nativeCalls.push(args);return{saved:true};}}}},ResizeObserver:class{observe(){}},requestAnimationFrame:fn=>{const id=++nextFrame;frames.set(id,fn);return id;},cancelAnimationFrame:id=>frames.delete(id),setTimeout:(fn,ms)=>{if(ms===450||ms===1200)return setImmediate(fn);return 0;},clearTimeout(){},setInterval:()=>0});context.window.listeners={};context.window.addEventListener=(type,fn)=>{(context.window.listeners[type]??=[]).push(fn);};
vm.runInContext(fs.readFileSync(path.join(web,'workbench.js'),'utf8')+'\nrenderPreview=async function(){renderPreviewMeta();};',context);
const run=code=>vm.runInContext(code,context),runAsync=code=>run(`(async()=>{${code}})()`),flush=()=>new Promise(resolve=>setImmediate(resolve));
async function fire(node,type,details={}){
 const event={type,target:node,defaultPrevented:false,preventDefault(){this.defaultPrevented=true;},...details};
 const globals=document.listeners[type]||[];
 if(node===document){for(const record of globals)await record.fn(event);return event;}
 for(const record of globals.filter(record=>record.capture))await record.fn(event);
 for(let current=node;current;current=current.parentNode)for(const fn of current.listeners[type]||[])await fn(event);
 for(const record of globals.filter(record=>!record.capture))await record.fn(event);
 return event;
}
// These dispatch the registered Pointer Events using capture and hit-test stubs.
// They do not exercise native mouse input or an actual WebKit/WebView2 window.
const fileRow=name=>nodes.get('file-list').querySelectorAll('.file-row').find(row=>row.querySelector('.file-name')?.textContent===name);
const handleFor=name=>fileRow(name).querySelector('.file-drag-handle');
const pointer=(more={})=>({pointerId:7,isPrimary:true,pointerType:'mouse',button:0,buttons:1,clientX:15,clientY:300,...more});
const assertDragClean=()=>{assert.equal(nodes.get('file-list').querySelectorAll('.dragging,.drop-target').length,0,'pointer sort leaves no visual state');assert.equal(nodes.get('file-list').classList.contains('pointer-sorting'),false);assert.equal(run('sortingPointer'),null);assert.equal(pointerCaptures.size,0,'capture always released');assert.equal(frames.size,0,'edge scrolling always cancelled');};
const fireId=(id,type)=>fire(nodes.get(id),type);
const guard=setTimeout(()=>{console.error('UI state regression timed out');process.exit(1);},10000);
(async()=>{
 await flush();await flush();assert.equal(run('S.ready'),true);assert.equal(run('S.orientationThreads'),7);assert.equal(run('S.mergeThreads'),3);
 assert(!nodes.has('initialize-folder'));assert(!nodes.has('reviewed-file'));assert(!nodes.has('duplicate-mode'));
 nodes.get('root-path').value='/fixture/input';await fireId('root-path','change');assert.equal(nodes.get('merge-level').value,'0');assert.equal(nodes.get('merge-level').options.length,3);assert.match(nodes.get('level-hint').textContent,/1 个文件夹/);
 nodes.get('merge-level').value='1';await fireId('merge-level','change');assert.match(nodes.get('level-hint').textContent,/2 个文件夹/);assert.equal(nodes.get('directory-list').children.length,2);
 nodes.get('rename-enabled').checked=true;await fireId('rename-enabled','change');nodes.get('index-path').value='/fixture/names.xlsx';await fireId('index-path','change');assert.equal(nodes.get('naming-mode').value,'folder_index');assert.equal(run('preparationRequest().index.duplicates'),'first');assert.equal(nodes.get('rename-collision').hidden,true);
 await run("inspectIndex({sheet:'Empty'})");assert.equal(nodes.get('index-sheet-warning').hidden,false);assert.equal(nodes.get('index-column').disabled,true);assert.equal(nodes.get('index-sheet').disabled,false);await run("inspectIndex({sheet:'Sheet 1'})");assert.equal(nodes.get('index-sheet-warning').hidden,true);
 nodes.get('naming-mode').value='index';await run('refreshNamingPreview()');assert.equal(nodes.get('rename-collision').hidden,false);run("S.renameOverrides.set('A','手改名称.pdf')");await run('refreshNamingPreview()');assert.equal(nodes.get('rename-collision').hidden,true);assert.equal(run("preparationRequest().output_names[0].output_name"),'手改名称.pdf');
 // A late preview is not allowed to bind its names to a different root or level.
 deferredPreview=true;const staleRoot=run('refreshNamingPreview()');await flush();nodes.get('root-path').value='/fixture/other';run('changedSource();S.renameOverrides.clear()');previewResolve();await staleRoot;assert.equal(run('S.renamePreview'),null);
 await run('refreshPreparation({level:1})');deferredPreview=true;const staleLevel=run('refreshNamingPreview()');await flush();nodes.get('merge-level').value='0';run('changedSource();S.renameOverrides.clear();S.renamePreview=null');previewResolve();await staleLevel;assert.equal(run('S.renamePreview'),null);await run('refreshPreparation({level:1})');
 run("S.renameOverrides.set('A','隐藏旧名称.pdf')");nodes.get('rename-enabled').checked=false;await fireId('rename-enabled','change');assert.equal(run('S.renameOverrides.size'),0);assert.equal(run('preparationRequest().index'),null);assert.equal(run('preparationRequest().output_names.length'),0);await run('scan()');
 const scanRequest=calls.filter(c=>c.url==='/api/batch/scans').at(-1).body;assert.equal(scanRequest.index,null);assert.deepEqual(scanRequest.output_names,[]);assert.deepEqual(Array.from(run('S.plan.groups.map(g=>g.output_name)')),['A.pdf','B.pdf']);assert.equal(run('S.selected.size'),2);assert.equal(nodes.get('scan-results').hidden,false);assert.equal(nodes.get('merge-section').hidden,false);assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),1);assert.match(nodes.get('file-list').children.at(-1).children[0].children[1].children[1].textContent,/无法读取页数/);
 await run("removeFile('bad')");assert.equal(run("selectedGroup().files.find(file=>file.id==='f2').pdf_has_signature"),true);assert(run("currentLogs().some(log=>log.level==='warning'&&log.message.includes('签名'))"));assert(nodes.get('file-list').querySelector('.file-warning'));assert.equal(run('S.plan.removed_sources.length'),1);assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),0,'removal clears active source error');assert.equal(nodes.get('removed-files').hidden,false);assert(run("currentLogs().some(log=>log.level==='info'&&log.message.includes('原文件保留'))"));
 await fire(nodes.get('removed-file-list').querySelector('button'),'click');assert.equal(run('S.plan.removed_sources.length'),0);assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),1,'restore recalculates source error');await run("removeFile('bad')");
 // Legacy destination errors are historical; runtime conflicts decide skip/overwrite.
 plans.at(-1).groups[0].issues.push({severity:'error',code:'output_existing',message:'输出已存在且归属未知'});plans.at(-1).issues.push({severity:'error',code:'output_existing',message:'历史输出占用'});await runAsync("acceptPlan(await api('/batch/plans/'+S.plan.id),{preserveSelection:true});");assert.equal(nodes.get('merge-button').disabled,false);run('openMerge()');assert.equal(nodes.get('start-merge').disabled,false,'existing output is decided during generation and signature warning does not block');assert.match(nodes.get('merge-warning').textContent,/不保留签名验证/);assert.equal(run('hasPlanErrors()'),false);assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),0);nodes.get('output-path').value='/fixture/new-output';run('updateOutputDraft()');await run('checkOutput()');assert.equal(nodes.get('start-merge').disabled,false);nodes.get('merge-dialog').close();
 await run("editPlan({groups:[{group_id:'g1',output_name:'同名.pdf'},{group_id:'g2',output_name:'同名.pdf'}]})");assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),2);
 run("S.fileId='f2';resetSourceView();S.sourcePage=2");const nameInput=nodes.get('group-list').querySelector('input[data-group-id="g1"]');nameInput.focus();nameInput.setSelectionRange(2,4);nameInput.value='唯一名称.pdf';await fire(nameInput,'input');assert.equal(run("currentLogs().filter(log=>log.level==='error').length"),0,'name correction clears both current collision messages locally');const historyCalls=calls.filter(c=>c.url.includes('/batch/plans?')).length;await run('saveEdits()');assert.equal(calls.filter(c=>c.url.includes('/batch/plans?')).length,historyCalls,'editing does not refetch full history');assert.equal(run('S.sourcePage'),2);assert.equal(run('S.fileId'),'f2');assert.equal(document.activeElement.dataset.groupId,'g1');assert.equal(document.activeElement.selectionStart,2);
 // A preview click still works, while pointer sorting is exclusive to the handle.
 await fire(fileRow('01.jpg').querySelector('.file-select'),'pointerdown',pointer());assertDragClean();
 await fire(fileRow('01.jpg').querySelector('.file-select'),'click');assert.equal(run('S.fileId'),'f1');
 run("S.fileId='f2';resetSourceView();S.sourcePage=2");
 const source=fileRow('03.jpg'),target=fileRow('01.jpg'),handle=handleFor('03.jpg');
 const beforeMove=calls.filter(call=>call.url.endsWith('/edit')).length;
 const down=await fire(handle,'pointerdown',pointer({buttons:0}));assert.equal(down.defaultPrevented,true);assert.equal(handle.hasPointerCapture(7),true);assert.equal(source.classList.contains('dragging'),false,'threshold required');
 // Actual native evidence: capture succeeds although the down/move buttons mask is 0.
 pointerHit=target.querySelector('.file-select');await fire(handle,'pointermove',pointer({clientX:18,clientY:297,button:-1,buttons:0}));assert.equal(source.classList.contains('dragging'),false,'small motion does not start sort');assert(handle.hasPointerCapture(7));
 await fire(handle,'pointermove',pointer({clientX:25,clientY:100,button:-1,buttons:0}));assert.equal(source.classList.contains('dragging'),true,'owned capture accepts the zero buttons mask');assert.equal(target.classList.contains('drop-target'),true);
 await fire(handle,'pointermove',pointer({clientX:35,clientY:110,button:-1,buttons:0}));assert.equal(calls.filter(call=>call.url.endsWith('/edit')).length,beforeMove,'pointer motion never saves');assert.equal(fileRow('03.jpg'),source,'pointer motion never replaces the source DOM');
 await fire(handle,'pointerup',pointer({clientX:35,clientY:110,buttons:0}));await flush();assert.equal(calls.filter(call=>call.url.endsWith('/edit')).length,beforeMove+1,'release saves exactly once');assert.equal(releaseCalls.filter(([node,id])=>node===handle&&id===7).length,1,'lost capture re-entry does not release recursively');assert.deepEqual(Array.from(run('selectedGroup().files.map(f=>f.id)')),['f3','f1','f2']);assert.equal(run('S.sourcePage'),2);assert.equal(run('S.fileId'),'f2');assertDragClean();
 const beforeRejected=calls.filter(call=>call.url.endsWith('/edit')).length;
 for(const details of [{button:2},{isPrimary:false},{pointerId:undefined}]){await fire(handleFor('03.jpg'),'pointerdown',pointer(details));assertDragClean();}
 const disabled=handleFor('03.jpg');disabled.disabled=true;await fire(disabled,'pointerdown',pointer());disabled.disabled=false;assertDragClean();
 disabled.failCapture=true;await fire(disabled,'pointerdown',pointer());disabled.failCapture=false;assertDragClean();
 disabled.silentCapture=true;await fire(disabled,'pointerdown',pointer());disabled.silentCapture=false;assertDragClean();
 // Capture alone, without threshold motion, does not reorder on release.
 await fire(handleFor('03.jpg'),'pointerdown',pointer());pointerHit=fileRow('01.jpg');await fire(handleFor('03.jpg'),'pointerup',pointer({clientX:30,clientY:100,buttons:0}));assertDragClean();
 for(const finish of ['outside','last-up-outside','unknown-target','cross-group','unknown-file','same-position','pointercancel','lostcapture','escape','blur','visibility','group','render','lock','source-dirty','capture-lost-move','capture-lost-up','non-primary-release','right-release','different-pointer','source-removed']){
  const current=handleFor('03.jpg');pointerHit=fileRow('01.jpg').querySelector('.file-select');await fire(current,'pointerdown',pointer());await fire(current,'pointermove',pointer({clientX:30,clientY:100}));assert.equal(run('sortingPointer.dragging'),true);
  let fake=null;
  if(finish==='outside'){pointerHit=body;await fire(current,'pointermove',pointer({clientX:600,clientY:450}));await fire(current,'pointerup',pointer({clientX:600,clientY:450,buttons:0}));}
  else if(finish==='last-up-outside')await fire(current,'pointerup',pointer({clientX:600,clientY:450,buttons:0}));
  else if(['unknown-target','cross-group','unknown-file','same-position'].includes(finish)){
   if(finish==='unknown-target')pointerHit=nodes.get('group-list');
   else if(finish==='same-position')pointerHit=fileRow('03.jpg');
   else{fake=new Node('div');fake.className='file-row';fake.dataset.groupId=finish==='cross-group'?'g2':'g1';fake.dataset.fileId=finish==='cross-group'?'f4':'missing';nodes.get('file-list').append(fake);pointerHit=fake;}
   await fire(current,'pointerup',pointer({clientX:30,clientY:100,buttons:0}));
  }
  else if(finish==='pointercancel')await fire(current,'pointercancel',pointer());
  else if(finish==='lostcapture')current.releasePointerCapture(7);
  else if(finish==='escape')await fire(document,'keydown',{key:'Escape'});
  else if(finish==='blur')await fire(context.window,'blur');
  else if(finish==='visibility'){document.hidden=true;await fire(document,'visibilitychange');document.hidden=false;}
  else if(finish==='group'){run("selectGroup('g2')");run("selectGroup('g1')");}
  else if(finish==='render')run('renderFiles()');
  else if(finish==='lock'){run('S.operations++;controls()');run('S.operations--;controls()');}
  else if(finish==='source-dirty'){run('S.sourceDirty=true;controls()');run('S.sourceDirty=false;controls()');}
  else if(finish==='capture-lost-move'){pointerCaptures.delete(7);await fire(current,'pointermove',pointer({clientX:30,clientY:100,button:-1,buttons:0}));}
  else if(finish==='capture-lost-up'){pointerCaptures.delete(7);await fire(current,'pointerup',pointer({clientX:30,clientY:100,buttons:0}));}
  else if(finish==='non-primary-release')await fire(current,'pointerup',pointer({clientX:30,clientY:100,buttons:0,isPrimary:false}));
  else if(finish==='right-release')await fire(current,'pointerup',pointer({clientX:30,clientY:100,buttons:0,button:2}));
  else if(finish==='different-pointer'){await fire(current,'pointerup',pointer({pointerId:8,clientX:30,clientY:100,buttons:0}));assert(current.hasPointerCapture(7));await fire(current,'pointercancel',pointer());}
  else if(finish==='source-removed'){current.closest('.file-row').remove();await fire(document,'pointermove',pointer({clientX:30,clientY:100}));run('renderFiles()');}
  fake?.remove();await fire(current,'pointerup',pointer({clientX:30,clientY:100,buttons:0}));await flush();assertDragClean();assert.deepEqual(Array.from(run('selectedGroup().files.map(f=>f.id)')),['f3','f1','f2']);
 }
 // Edge scrolling updates only the scroll position; cancellation removes its RAF.
 const list=nodes.get('file-list');list.testScrollHeight=1000;pointerHit=fileRow('01.jpg');const scrolling=handleFor('03.jpg');await fire(scrolling,'pointerdown',pointer());await fire(scrolling,'pointermove',pointer({clientX:30,clientY:390}));assert.equal(frames.size,1);const[frame,fn]=frames.entries().next().value;frames.delete(frame);fn();assert.equal(list.scrollTop,10);await fire(scrolling,'pointercancel',pointer());assertDragClean();list.testScrollHeight=400;list.scrollTop=0;
 assert.equal(calls.filter(call=>call.url.endsWith('/edit')).length,beforeRejected,'invalid, external or cancelled pointer gestures never write order');
 run("S.fileId='f2';resetSourceView();S.sourcePage=2;renderPreviewMeta()");assert.match(nodes.get('preview-evidence').textContent,/静态页面合并/);assert.equal(nodes.get('rotation-scope').value,'page');await run('rotateFile(90)');assert.deepEqual(Array.from(run('selectedFile().page_rotations')),[0,90]);assert.equal(run('selectedFile().reviewed'),false);assert.deepEqual(Array.from(run('selectedFile().reviewed_pages')),[false,false]);assert.equal(run('fileStatus(selectedFile()).tone'),'manual');
 run('openOrientation()');assert.equal(nodes.get('orientation-consent').checked,false);assert.equal(nodes.get('start-orientation').disabled,true);nodes.get('orientation-consent').checked=true;nodes.get('orientation-threads').value='2';await run('startOrientation()');assert.equal(preferences.orientation_threads,2);assert.equal(run('allFiles(S.plan).reduce((n,f)=>n+pendingReview(f),0)'),2);assert.match(nodes.get('job-result-summary').textContent,/已处理 3 \/ 3 张 · 调整 2 · 未变 1/);assert.equal(nodes.get('job-results').querySelectorAll('.result-row').length,2);for(const job of jobs)job.status='completed';run('S.job=null;S.jobs=[]');run('openMerge()');assert.equal(nodes.get('start-merge').disabled,false);assert.match(nodes.get('merge-warning').textContent,/2 张图片/);nodes.get('merge-threads').value='6';await run('startMerge()');assert.equal(preferences.merge_threads,6);
 const generated=jobs.at(-1);generated.status='failed';generated.threads=5;run('S.job=null;S.jobs=[]');await runAsync(`acceptPlan(await api('/batch/plans/${generated.plan_id}'),{preserveSelection:false});S.job=${JSON.stringify(generated)};`);await run('retryJob()');assert.equal(nodes.get('merge-threads').value,'5');const patches=calls.filter(c=>c.method==='PATCH').length;await run('startMerge()');assert.equal(calls.filter(c=>c.method==='PATCH').length,patches);assert.equal(jobs.at(-1).threads,5);
 await run('saveReport(S.job)');await run("saveOutput(S.job,{voucher_id:'g1',output_path:'/fixture/new-output/唯一名称.pdf'})");assert.deepEqual(nativeCalls.map(c=>c[0]),['save_job_report','save_job_output']);
 // Runtime collision UI: registered actions, per-conflict state and request races.
 for(const job of jobs)job.status='completed';run('S.job=null;S.jobs=[]');
 const collision=(id,group='g2')=>({id,group_id:group,group_name:group==='g2'?'B':'A',output_path:'/fixture/new-output/'+group+'.pdf',reason:'输出位置已有同名普通文件。'});
 function runtimeJob(id,conflictId){const job={id,plan_id:run('S.plan.id'),kind:'batch_generate',status:'running',threads:4,created_at:'2026-11-02T02:00:00Z',updated_at:'2026-11-02T02:00:00Z',request:{voucher_ids:['g1','g2']},done:4,total:6,current:'等待处理同名输出',error:null,output_conflict:collision(conflictId),results:[{voucher_id:'g1',folder:'A',status:'completed',pages:4,output_path:'/fixture/new-output/A.pdf',error:null}]};jobs.push(job);run(`trackJob(${JSON.stringify(job)})`);return job;}
 let runtime=runtimeJob('runtime-skip','c1');assert.equal(nodes.get('output-conflict-dialog').open,true);assert.equal(nodes.get('output-conflict-apply').checked,false);assert.equal(nodes.get('output-conflict-path').textContent,runtime.output_conflict.output_path);
 const escapedConflict=await fireId('output-conflict-dialog','cancel');assert.equal(escapedConflict.defaultPrevented,true);assert.equal(nodes.get('output-conflict-dialog').open,true);assert.match(nodes.get('output-conflict-hint').textContent,/Esc/);nodes.get('output-conflict-dialog').close();assert.equal(nodes.get('output-conflict-dialog').open,true,'unexpected close cannot hide pending output');
 nodes.get('output-conflict-apply').checked=true;await fireId('output-conflict-skip','click');assert.deepEqual(calls.filter(call=>call.url.endsWith('/output-conflict')).at(-1).body,{conflict_id:'c1',action:'skip',apply_to_batch:true});assert.equal(nodes.get('output-conflict-dialog').open,false);assert.equal(nodes.get('output-conflict-apply').checked,false);assert.equal(run("S.job.results.at(-1).status"),'skipped_existing');assert.equal(run("S.job.results.at(-1).output_path"),null);assert.equal(run("S.job.results.at(-1).pages"),0);assert.match(nodes.get('job-result-summary').textContent,/成功 4 页 · 跳过同名 1 份/);assert.equal(nodes.get('job-results').querySelectorAll('button').length,1,'only generated output has download');assert(run("labels.skipped!==labels.skipped_existing"));await run('saveReport(S.job)');assert.equal(nativeCalls.at(-1)[1].jobId,'runtime-skip');
 runtime=runtimeJob('runtime-overwrite','c2');assert.equal(nodes.get('output-conflict-apply').checked,false,'apply never crosses jobs');nodes.get('output-conflict-apply').checked=true;await fireId('output-conflict-overwrite','click');assert.deepEqual(calls.filter(call=>call.url.endsWith('/output-conflict')).at(-1).body,{conflict_id:'c2',action:'overwrite',apply_to_batch:true});assert.match(nodes.get('job-result-summary').textContent,/成功 6 页 · 跳过同名 0 份/);
 // A successful decision may immediately expose a new waiting worker without flashing.
 runtime=runtimeJob('runtime-next','c3');const shown=nodes.get('output-conflict-dialog').showCount;nextConflictAfterDecision=collision('c4','g1');await fireId('output-conflict-skip','click');assert.equal(nodes.get('output-conflict-dialog').open,true);assert.equal(nodes.get('output-conflict-dialog').showCount,shown);assert.equal(run('S.conflictKey'),'runtime-next:c4');assert.equal(nodes.get('output-conflict-apply').checked,false);await fireId('output-conflict-overwrite','click');assert.equal(nodes.get('output-conflict-dialog').open,false);
 // Inline failures preserve the choice and re-enable retry; polling the same pending keeps it.
 runtime=runtimeJob('runtime-failure','c5');nodes.get('output-conflict-apply').checked=true;conflictFailure=true;await fireId('output-conflict-skip','click');assert.equal(nodes.get('output-conflict-error').hidden,false);assert.match(nodes.get('output-conflict-error').textContent,/可重试/);assert.equal(nodes.get('output-conflict-skip').disabled,false);assert.equal(nodes.get('output-conflict-apply').checked,true);run('renderJob()');assert.equal(nodes.get('output-conflict-error').hidden,false);await fireId('output-conflict-skip','click');assert.equal(nodes.get('output-conflict-dialog').open,false);
 // Duplicate submissions are rejected locally, and an old response cannot replace a new pending.
 runtime=runtimeJob('runtime-stale','c6');nodes.get('output-conflict-apply').checked=true;deferConflict=true;const pendingChoice=fireId('output-conflict-skip','click');await flush();const sent=calls.filter(call=>call.url.endsWith('/output-conflict')).length;assert.equal(nodes.get('output-conflict-overwrite').disabled,true);assert.equal(nodes.get('output-conflict-cancel').disabled,false,'pending decision can still cancel');await fireId('output-conflict-overwrite','click');assert.equal(calls.filter(call=>call.url.endsWith('/output-conflict')).length,sent);
 const openCount=nodes.get('output-conflict-dialog').showCount;runtime.output_conflict=collision('c7');run(`trackJob(${JSON.stringify(runtime)})`);assert.equal(nodes.get('output-conflict-dialog').showCount,openCount);assert.equal(nodes.get('output-conflict-apply').checked,false);conflictResolve();await pendingChoice;assert.equal(run('S.job.output_conflict.id'),'c7');assert.equal(run('S.conflictKey'),'runtime-stale:c7');await fireId('output-conflict-overwrite','click');
 // A GET begun before the accepted choice cannot resurrect the old modal.
 runtime=runtimeJob('runtime-poll-race','c8');deferJobGet=true;const oldPoll=run('pollJob()');await flush();await fireId('output-conflict-skip','click');assert.equal(nodes.get('output-conflict-dialog').open,false);jobGetResolve();await oldPoll;assert.equal(run('S.job.output_conflict'),null);assert.equal(nodes.get('output-conflict-dialog').open,false);
 // Cancelling is independently available during a decision request and keeps finished outputs.
 runtime=runtimeJob('runtime-cancel-race','c9');deferConflict=true;const oldDecision=fireId('output-conflict-overwrite','click');await flush();await fireId('output-conflict-cancel','click');assert.equal(nodes.get('output-conflict-dialog').open,false);assert.equal(run('S.job.status'),'cancelled');assert(run('S.job.results.length>=1'));conflictResolve();await oldDecision;assert.equal(run('S.job.status'),'cancelled','late decision cannot revive cancelled task');assert.equal(nodes.get('output-conflict-dialog').open,false);
 runtime=runtimeJob('runtime-cancel-error','c10');cancelFailure=true;await fireId('output-conflict-cancel','click');assert.equal(nodes.get('output-conflict-dialog').open,true);assert.equal(nodes.get('output-conflict-error').hidden,false);assert.equal(nodes.get('output-conflict-cancel').disabled,false);await fireId('output-conflict-cancel','click');assert.equal(nodes.get('output-conflict-dialog').open,false);
 runtime=runtimeJob('runtime-ended','c11');nodes.get('output-conflict-apply').checked=true;runtime.status='completed';run(`trackJob(${JSON.stringify(runtime)})`);assert.equal(nodes.get('output-conflict-dialog').open,false);assert.equal(nodes.get('output-conflict-apply').checked,false);assert.equal(run('S.conflictKey'),null);
 // A legacy snapshot is displayed as-is; only a future scan uses the new first-row rule.
 for(const job of jobs)job.status='completed';run('S.job=null;S.jobs=[]');const legacy={...clone(plans.at(-1)),id:'legacy',created_at:'2026-01-01T00:00:00Z',parent_plan_id:null,index:{config:{path:'/fixture/old.xlsx',sheet:'Old',header_row:1,index_column:0,name_column:1,match_rule:'auto',naming:'index',duplicates:'fallback'}}};plans.push(legacy);await run("loadPlan('legacy')");assert.equal(nodes.get('naming-mode').value,'index');assert.equal(nodes.get('naming-history-note').hidden,false);assert.equal(run('S.plan.index.config.duplicates'),'fallback');assert.equal(run('preparationRequest().index.duplicates'),'first');await run('refreshHistory()');assert(calls.some(c=>c.url==='/api/batch/plans?summary=true'));assert(!calls.some(c=>c.url==='/api/batch/plans'));
 // Old signed-PDF rejection remains a source error until a fresh scan or explicit removal.
 const oldSigned=clone(legacy);oldSigned.id='legacy-signed-error';oldSigned.parent_plan_id=null;oldSigned.groups[0].files.find(file=>file.id==='f2').error='旧版签名PDF来源错误，需重新扫描';oldSigned.groups[0].files.find(file=>file.id==='f2').pdf_has_signature=false;refreshIssues(oldSigned);plans.push(oldSigned);await run("loadPlan('legacy-signed-error')");run('openMerge()');assert.equal(nodes.get('start-merge').disabled,true);assert(run("currentLogs().some(log=>log.level==='error'&&log.message.includes('需重新扫描'))"));nodes.get('merge-dialog').close();
 // Auto color/log details require an explicit, currently applied angle change.
 for(const job of jobs)job.status='completed';run('S.job=null;S.jobs=[];S.scan=null');
 const image=(id,extra={})=>({...clone(files[0]),id,name:id+'.jpg',relative_path:id+'.jpg',rotation_cw:0,orientation:null,orientation_error:null,auto_rotation_change:null,manual_rotation:false,reviewed:false,error:null,...extra});
 const changedRight=image('changed-right',{rotation_cw:90,auto_rotation_change:{from_cw:0,to_cw:90},orientation:{rotation_cw:90,needs_review:false}});
 const changedBack=image('changed-back',{rotation_cw:0,auto_rotation_change:{from_cw:90,to_cw:0},orientation:{rotation_cw:0,needs_review:false}});
 const unchangedZero=image('same-zero',{auto_rotation_change:{from_cw:0,to_cw:0},orientation:{rotation_cw:0,needs_review:false}});
 const unchangedQuarter=image('same-quarter',{rotation_cw:90,auto_rotation_change:{from_cw:90,to_cw:90}});
 const oldNoMarker=image('old-evidence-only',{rotation_cw:90,orientation:{rotation_cw:90,needs_review:true}});delete oldNoMarker.auto_rotation_change;
 const failedDirection=image('failed-not-a-detail',{orientation_error:'识别失败原因保留在计划'});
 const unresolvedDirection=image('unresolved-not-a-detail',{orientation:{rotation_cw:90,needs_review:true}});
 const staleMarker=image('stale-marker',{rotation_cw:180,auto_rotation_change:{from_cw:0,to_cw:90}});
 const manualProtected=image('manual-protected',{rotation_cw:90,auto_rotation_change:{from_cw:0,to_cw:90},manual_rotation:true});
 const brokenSource=image('broken-source',{error:'真实来源错误，仍需处理'});
 const rotationPlan={...clone(fixture),id:'rotation-contract',groups:[group('g1','changed',[changedRight,changedBack]),group('g2','neutral',[unchangedZero,unchangedQuarter,oldNoMarker,failedDirection,unresolvedDirection,staleMarker]),group('g3','manual',[manualProtected]),group('g4','source',[brokenSource])],issues:[],index:null};
 run(`acceptPlan(${JSON.stringify(rotationPlan)},{preserveSelection:false});`);
 assert.equal(run('autoTouched(S.plan.groups[0].files[0])'),true);assert.equal(run('autoTouched(S.plan.groups[0].files[1])'),true);assert.equal(run('pendingReview(S.plan.groups[0].files[1])'),1,'90 to 0 still marks an actual change');assert.match(run('fileStatus(S.plan.groups[0].files[1]).label'),/90° → 0°/);
 for(const file of [unchangedZero,unchangedQuarter,oldNoMarker,failedDirection,unresolvedDirection,staleMarker]){assert.equal(run(`autoTouched(${JSON.stringify(file)})`),false);assert.equal(run(`fileStatus(${JSON.stringify(file)}).tone`),'original','same/no marker/failure/undecided stay neutral');}
 assert.equal(run('pendingReview(S.plan.groups[2].files[0])'),0);assert.equal(run('fileStatus(S.plan.groups[2].files[0]).tone'),'manual');assert.equal(run('groupTone(S.plan.groups[0])'),'review');assert.equal(run('groupTone(S.plan.groups[1])'),'original');assert.equal(run('groupTone(S.plan.groups[2])'),'manual');assert.equal(run('groupTone(S.plan.groups[3])'),'error');assert.equal(nodes.get('summary-review').textContent,'2');assert.equal(nodes.get('file-list').querySelectorAll('.tone-review').length,2);
 run("selectGroup('g2')");assert.equal(nodes.get('file-list').querySelectorAll('.tone-error,.tone-review').length,0);assert.equal(run("S.plan.groups[1].files.find(file=>file.id==='failed-not-a-detail').orientation_error"),'识别失败原因保留在计划');
 const changedLogs=run("currentLogs().filter(log=>log.code==='auto_rotation_change')");assert.equal(changedLogs.length,2);assert(changedLogs.some(log=>log.message.includes('0° → 90°')));assert(changedLogs.some(log=>log.message.includes('90° → 0°')));assert(!run("currentLogs().some(log=>log.message.includes('failed-not-a-detail')||log.message.includes('unresolved-not-a-detail')||log.message.includes('old-evidence-only'))"));assert(run("currentLogs().some(log=>log.level==='error'&&log.message.includes('broken-source'))"));
 const directionJob={id:'direction-contract',plan_id:rotationPlan.id,kind:'batch_analyze',status:'running',threads:4,request:{voucher_ids:['g1','g2']},done:8,total:10,current:'处理方向',error:null,results:[{folder:'changed-right.jpg',status:'completed',pages:1,output_path:null,rotation_change:{from_cw:0,to_cw:90}},{folder:'changed-back.jpg',status:'completed',pages:1,output_path:null,rotation_change:{from_cw:90,to_cw:0}},{folder:'unchanged-not-a-result.jpg',status:'completed',rotation_change:{from_cw:0,to_cw:0},needs_review:true},{folder:'failure-not-a-result.jpg',status:'failed',error:'识别失败',needs_review:true},{folder:'old-no-change-not-a-result.jpg',status:'completed',orientation:{rotation_cw:90},needs_review:true}],orientation_summary:{changed:2,unchanged:3,skipped:1,failed:2}};
 run(`trackJob(${JSON.stringify(directionJob)});renderLogs();`);assert.equal(nodes.get('job-results').querySelectorAll('.result-row').length,2);assert.equal(nodes.get('job-result-summary').textContent,'已处理 8 / 10 张 · 调整 2 · 未变 3 · 跳过 1 · 失败 2');assert.equal(nodes.get('job-progress-text').textContent,'8 / 10 张');assert(run("currentLogs().some(log=>log.code==='orientation_summary'&&log.level==='info'&&log.message.includes('失败 2'))"));
 directionJob.status='cancelled';run(`trackJob(${JSON.stringify(directionJob)})`);assert.equal(nodes.get('job-progress-text').textContent,'8 / 10 张');assert.equal(nodes.get('job-results').querySelectorAll('.result-row').length,2);assert.match(nodes.get('job-result-summary').textContent,/调整 2 · 未变 3 · 跳过 1 · 失败 2/);
 const historyOrientation={...clone(directionJob),id:'historical-direction',kind:'analyze',status:'failed',error:'历史识别未完成',done:17,total:20,orientation_summary:null};context.historyOrientation=historyOrientation;const historicalJson=JSON.stringify(historyOrientation);run('trackJob(historyOrientation)');assert.equal(JSON.stringify(context.historyOrientation),historicalJson,'legacy JSON is never rewritten');assert.match(nodes.get('job-result-summary').textContent,/已处理 17 \/ 20 张 · 历史记录/);assert.equal(nodes.get('job-results').querySelectorAll('.result-row').length,2);assert.equal(nodes.get('job-error').className,'notice neutral');assert.equal(nodes.get('job-status').classList.contains('error'),false);
 context.historyOrientation={...clone(historyOrientation),id:'historical-no-markers',results:historyOrientation.results.filter(result=>!result.rotation_change||result.rotation_change.from_cw===result.rotation_change.to_cw)};run('trackJob(historyOrientation)');assert.equal(nodes.get('job-results').querySelectorAll('.result-row').length,0);assert.equal(nodes.get('job-progress-text').textContent,'17 / 20 张');assert.match(nodes.get('job-result-summary').textContent,/历史记录/);
 assert(html.includes('直接替换已有文件，不保留备份，无法恢复'));assert(!html.includes('覆盖并留备份'));assert(/id="output-conflict-overwrite"[^>]*>覆盖<\/button>/.test(html));assert(html.includes('0度未变、未决或识别失败不标色'));
 const escaped=run("el('span','','<img src=x onerror=alert(1)>')");assert.equal(escaped.textContent,'<img src=x onerror=alert(1)>');assert.equal(escaped.children.length,0);
 const js=fs.readFileSync(path.join(web,'workbench.js'),'utf8'),ids=[...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);assert.equal(new Set(ids).size,ids.length);assert.deepEqual([...new Set([...js.matchAll(/\$\('([^']+)'\)/g)].map(m=>m[1]))].filter(id=>!ids.includes(id)),[]);assert(!/innerHTML|outerHTML|insertAdjacentHTML|eval\(/.test(js));assert(!/dataTransfer|DataTransfer|addEventListener\('drag/.test(js),'internal sorting no longer uses native HTML drag');for(const match of html.matchAll(/<label\b[^>]*>([\s\S]*?)<\/label>/g))assert([...match[1].matchAll(/<(input|button|select|textarea)\b/g)].length<=1);
 assert(fs.readFileSync(path.join(web,'workbench.css'),'utf8').includes('--control-height: 40px;'));
 // Exercise the actual native-session boundary without a Windows substitute:
 // EdgeDriver args are switches, not a raw argv array, and errors must be useful
 // while excluding every authentication URL/fragment before truncation.
 const driverSource=fs.readFileSync(path.join(__dirname,'smoke-windows-desktop.mjs'),'utf8'),requests=[];
 const driverContext=vm.createContext({assert,path:path.win32,fs:{rm:async()=>{}},exe:'C:\\程序 含空格\\PDF工作台.exe',expectedVersion:'0.6.0',session:null,owned:new Set(),probe:async()=>({webview_loaded:true,window_visible:true,pid:777}),until:async fn=>{const value=await fn();assert(value);return value;},js:async()=>true,wd:async(method,endpoint,body)=>{if(endpoint==='/session'){requests.push(body);return{sessionId:'synthetic-session'};}return{};}});
 vm.runInContext(driverSource.slice(driverSource.indexOf('function safeDriverDiagnostic('),driverSource.indexOf('async function wd('))+driverSource.slice(driverSource.indexOf('async function open('),driverSource.indexOf('async function endSession(')),driverContext);
 assert.equal(await driverContext.open('C:\\合成 资料\\处理记录','C:\\合成 探针',['--startup-delay-ms=20']),777);
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0].capabilities.alwaysMatch['tauri:options'].args)),['--state-dir=C:\\合成 资料\\处理记录','--test-probe-dir=C:\\合成 探针','--startup-delay-ms=20']);
 const diagnostic=driverContext.safeDriverDiagnostic('version mismatch 152; http://127.0.0.1:1234/#token=synthetic-secret #token=separate-secret '+ 'x'.repeat(5000));
 assert(diagnostic.includes('version mismatch 152'));assert(!diagnostic.includes('synthetic-secret')&&!diagnostic.includes('separate-secret')&&!diagnostic.includes('127.0.0.1'));assert(diagnostic.length<=1600);
 assert.equal(driverContext.normalizedWindowsPath("\\\\?\\C:\\合成 资料\\save.PDF"),driverContext.normalizedWindowsPath("c:\\合成 资料\\save.pdf"));
 assert.equal(driverContext.normalizedWindowsPath("\\\\?\\UNC\\SERVER\\share\\file.pdf"),driverContext.normalizedWindowsPath("\\\\server\\share\\file.pdf"));

 clearTimeout(guard);console.log(`PDF workbench UI state regression passed: ${calls.length} API calls; ${ids.length} DOM IDs. Covers rename defaults/toggle/stale responses, async scan, current errors, remove/undo, output recovery, autosave focus/page, registered Pointer Events/capture/hit-test stubs, rotation, consent, threads/retry/native saves, runtime output decisions/races/cancel, actual rotation-change-only colors/logs/results, summary/legacy/cancel, signed-PDF warnings, native-session switch/diagnostic contracts and safe text rendering. Native pointer dragging remains a separate acceptance check.`);
})().catch(error=>{clearTimeout(guard);console.error(error);process.exitCode=1;});
