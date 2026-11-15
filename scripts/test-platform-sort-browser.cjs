const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), { spawn } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const base = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-sortable-'));
const net = require('node:net');
const browserPaths = [process.env.PLATFORM_SORT_BROWSER,
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome',
].filter(Boolean);
const browserPath = browserPaths.find(candidate => fs.existsSync(candidate));
let browserProcess;
require('esbuild').buildSync({ entryPoints: [path.join(root, 'tests/platform-sort/fixture.tsx')], bundle: true, outfile: path.join(base, 'platform-sort-fixture.js'), format: 'iife', loader: { '.css': 'css', '.png': 'dataurl', '.svg': 'dataurl' } });
fs.copyFileSync(path.join(root, 'tests/platform-sort/fixture.html'), path.join(base, 'platform-sort-fixture.html'));

let launchError;

const wait = ms => new Promise(r => setTimeout(r, ms));
let socket;
(async () => {
    if (!browserPath) throw Error('Set PLATFORM_SORT_BROWSER to an installed Chromium browser executable.');
    const port = await new Promise((resolve, reject) => {
        const server = net.createServer();
        server.on('error', reject);
        server.listen(0, '127.0.0.1', () => {
            const port = server.address().port;
            server.close(error => error ? reject(error) : resolve(port));
        });
    });
    browserProcess = spawn(browserPath, ['--headless=new', '--window-size=1280,800', '--disable-gpu', '--no-first-run', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=' + port, '--user-data-dir=' + base + '/edge-platform-sort-fixture', 'file:///' + base.replaceAll('\\', '/') + '/platform-sort-fixture.html'], { windowsHide: true, stdio: 'ignore' });
    browserProcess.on('error', error => { launchError = error; });
    let pages;
    for (let i = 0; i < 50; i++) {
        try {
            pages = await (await fetch('http://127.0.0.1:' + port + '/json')).json();
            if (pages.length)
                break;
        }
        catch { }
        await wait(200);
    }
    if (launchError)
        throw launchError;
    if (!pages?.length)
        throw Error('Browser did not expose a debug page. Set PLATFORM_SORT_BROWSER to a Chromium executable.');
    const page = pages.find(t => t.type === 'page'), ws = new WebSocket(page.webSocketDebuggerUrl), pending = new Map();
    let id = 0;
    socket = ws;
    await new Promise(r => ws.addEventListener('open', r, { once: true }));
    ws.addEventListener('message', e => { const r = JSON.parse(e.data), p = pending.get(r.id); if (p) {
        pending.delete(r.id);
        r.error ? p.reject(Error(r.error.message)) : p.resolve(r.result);
    } });
    const send = (method, params = {}) => new Promise((resolve, reject) => { pending.set(++id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); });
    const ev = async (expression) => { const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails)
        throw Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text); return r.result.value; };
    const check = (value, message) => { if (!value)
        throw Error(message); };
    await wait(1500);

 const result={};
 const state=()=>ev(`({order:window.fixture.order,writes:window.fixture.writes,closed:window.fixture.closed})`);
 const rect=selector=>ev(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
 const roots='.platform-layout-list > [data-sortable-id]';
 const mouse=(type,p,extra={})=>send('Input.dispatchMouseEvent',{type,...p,...extra});
 const key=(key,code,virtual)=>send('Input.dispatchKeyEvent',{type:'keyDown',key,code,windowsVirtualKeyCode:virtual});
 const start=async selector=>{const p=await rect(selector);await mouse('mouseMoved',p);await mouse('mousePressed',p,{button:'left',clickCount:1});await mouse('mouseMoved',{x:p.x,y:p.y+8},{button:'left',buttons:1});await wait(100);return p;};
 const end=async p=>{await mouse('mouseReleased',p,{button:'left',clickCount:1});await wait(250)};
 check(await ev(`!!document.querySelector('.platform-layout-list')`),'Layout modal did not mount');
 const before=await state();
 const p=await start(roots+':nth-child(1) .platform-layout-drag-trigger');
 const target=await rect(roots+':nth-child(2) .platform-layout-row');
 await mouse('mouseMoved',target,{button:'left',buttons:1});await wait(220);
 result.preview=await ev(`({overlay:!!document.querySelector('.sortable-layout-preview'),neighborMoved:getComputedStyle(document.querySelector('.platform-layout-list > [data-sortable-id]:nth-child(2)')).transform!=='none',placeholder:document.querySelectorAll('.platform-layout-list > .is-sorting-source').length})`);
 check(result.preview.overlay&&result.preview.neighborMoved&&result.preview.placeholder===1,'No floating preview / animated avoidance: '+JSON.stringify(result.preview));
 check(JSON.stringify((await state()).order)===JSON.stringify(before.order)&&(await state()).writes===before.writes,'Drag persisted before drop');
 await end(target);const dropped=await state();
 check(dropped.order[0]===before.order[1]&&dropped.order[1]===before.order[0],'Pointer drop did not reorder');
 check(dropped.writes===before.writes+1,'Drop did not commit once');result.pointerDrop={savedOnce:true,orderChanged:true};
 const cancelBefore=await state();await start(roots+':nth-child(1) .platform-layout-drag-trigger');
 const cancelTarget=await rect(roots+':nth-child(3) .platform-layout-row');await mouse('mouseMoved',cancelTarget,{button:'left',buttons:1});await wait(100);
 await key('Escape','Escape',27);await end(cancelTarget);
 check(JSON.stringify((await state()).order)===JSON.stringify(cancelBefore.order)&&(await state()).writes===cancelBefore.writes&&(await state()).closed===0,'Escape changed order or closed modal');result.escape={cancelled:true,modalStayedOpen:true};
 const outsideBefore = await state();
 await start(roots+':nth-child(1) .platform-layout-drag-trigger');
 await mouse('mouseMoved',cancelTarget,{button:'left',buttons:1});
 await wait(100);
 const outside = {x: 1, y: cancelTarget.y};
 await mouse('mouseMoved',outside,{button:'left',buttons:1});
 await wait(100);
 await end(outside);
 const outsideAfter = await state();
 check(JSON.stringify(outsideAfter.order)===JSON.stringify(outsideBefore.order)&&outsideAfter.writes===outsideBefore.writes,'Outside drop saved a different order');
 result.outsideDropCancelled = true;
 await ev(`document.querySelector('.platform-layout-list > [data-sortable-id] .platform-layout-drag-trigger').focus()`);
 const keyboardBefore=await state();await key(' ','Space',32);await wait(100);await key('ArrowDown','ArrowDown',40);await wait(180);await key(' ','Space',32);await wait(250);
 check((await state()).order[1]===keyboardBefore.order[0],'Keyboard reorder failed');result.keyboard=true;
 const scrollBefore=await state();await start(roots+':nth-child(1) .platform-layout-drag-trigger');
 const edge=await ev(`(()=>{const r=document.querySelector('.platform-layout-modal-body').getBoundingClientRect();return{x:r.left+80,y:r.bottom-12}})()`);
 await mouse('mouseMoved',edge,{button:'left',buttons:1});await wait(700);
 result.autoScroll=await ev(`document.querySelector('.platform-layout-modal-body').scrollTop>0`);check(result.autoScroll,'Edge auto-scroll did not run');await key('Escape','Escape',27);await end(edge);
 check((await state()).writes===scrollBefore.writes,'Auto-scroll persisted a cancelled drag');
 await ev(`document.querySelector('[data-sortable-id="group:fixture-sort-group"] .platform-layout-expand-trigger').click()`);await wait(100);await ev(`document.querySelector('[data-sortable-id="group:fixture-sort-group"]').scrollIntoView({block:'center'})`);await wait(250);
 const group='[data-sortable-id="group:fixture-sort-group"] .platform-layout-child-sortables > [data-sortable-id]';
 const childBefore=await ev(`({ids:window.fixture.groupOrder,roots:window.fixture.order,writes:window.fixture.writes})`);
 await start(group+':nth-child(1) .platform-layout-child-drag-trigger');const childTarget=await rect(group+':nth-child(2)');await mouse('mouseMoved',childTarget,{button:'left',buttons:1});await wait(150);
 check(await ev(`window.fixture.writes`)===childBefore.writes,'Child drag persisted before drop');await end(childTarget);
 const childAfter=await ev(`({ids:window.fixture.groupOrder,roots:window.fixture.order,writes:window.fixture.writes})`);
 check(childAfter.ids[0]===childBefore.ids[1]&&childAfter.ids[1]===childBefore.ids[0]&&JSON.stringify(childAfter.roots)===JSON.stringify(childBefore.roots),'Nested reorder changed the wrong list: '+JSON.stringify({before:childBefore,after:childAfter}));result.nested={reordered:true,parentOrderPreserved:true,savedOnce:childAfter.writes===childBefore.writes+1};

 const expandedBefore=await state();
 const groupIndex=expandedBefore.order.indexOf('group:fixture-sort-group');
 await ev(`document.querySelector('[data-sortable-id="group:fixture-sort-group"]').scrollIntoView({block:'center'})`);await wait(100);
 await start('[data-sortable-id="group:fixture-sort-group"] > .platform-layout-row .platform-layout-drag-trigger');
 const above=await rect('.platform-layout-list > [data-sortable-id]:nth-child('+groupIndex+') > .platform-layout-row');
 await mouse('mouseMoved',above,{button:'left',buttons:1});await wait(120);await end(above);
 const expandedAfter=await state();check(expandedAfter.order[groupIndex-1]==='group:fixture-sort-group','Expanded group did not move as one block');result.expandedGroup=true;
 check(await ev(`document.querySelectorAll('.sortable-layout-preview').length===0`),'Preview not cleaned up');
 if(process.env.PLATFORM_SORT_REPORT)fs.writeFileSync(process.env.PLATFORM_SORT_REPORT,JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
    ws.send(JSON.stringify({ id: ++id, method: 'Browser.close' }));
})().catch(e => { console.error(e); process.exitCode = 1; }).finally(async () => {
    if(socket?.readyState===WebSocket.OPEN)socket.send(JSON.stringify({id:1000000,method:'Browser.close'}));
    socket?.close();
    await wait(300);
    browserProcess?.kill();
    // Only remove the unique scratch directory created by this test.
    if (path.dirname(path.resolve(base)) === path.resolve(os.tmpdir())
        && path.basename(base).startsWith('cockpit-sortable-')) {
        try {
            fs.rmSync(base, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
        }
        catch (error) {
            console.warn('Browser scratch cleanup:', error.message);
        }
    }
});
