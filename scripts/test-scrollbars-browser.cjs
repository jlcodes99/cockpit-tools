const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), { spawn } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const base = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-scrollbars-'));
const port = 19336;
require('esbuild').buildSync({ entryPoints: [path.join(root, 'tests/scrollbars/fixture.ts')], bundle: true, outfile: path.join(base, 'scrollbar-fixture.js'), format: 'iife', loader: { '.css': 'css' } });
fs.copyFileSync(path.join(root, 'tests/scrollbars/fixture.html'), path.join(base, 'scrollbar-fixture.html'));
const edge = spawn(process.env.SCROLLBARS_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', ['--headless=new', '--window-size=1280,800', '--disable-gpu', '--no-first-run', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=' + port, '--user-data-dir=' + base + '/edge-scrollbar-fixture', 'file:///' + base.replaceAll('\\', '/') + '/scrollbar-fixture.html'], { windowsHide: true, stdio: 'ignore' });
let launchError;
edge.on('error', error => { launchError = error; });
const wait = ms => new Promise(r => setTimeout(r, ms));
let socket;
(async () => {
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
        throw Error('Browser did not expose a debug page. Set SCROLLBARS_BROWSER to a Chromium executable.');
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
    const result = {};
    await ev(`document.getElementById('plain').addEventListener('scroll',()=>{});`);
    result.initial = await ev(`(()=>{const ids=['plain','absolute','react-scroll','textarea'];return ids.map(id=>{const e=document.getElementById(id);return{id,position:getComputedStyle(e).position,overflow:getComputedStyle(e).overflow,hidden:e.getAttribute('data-overlayscrollbars-viewport'),children:e.children.length,rect:{width:e.getBoundingClientRect().width,height:e.getBoundingClientRect().height}}})})()`);
    check(result.initial.filter(x => x.id !== 'textarea').every(x => x.hidden?.includes('scrollbarHidden')), 'Missing enhancement: ' + JSON.stringify(result.initial));
    check(result.initial.find(x => x.id === 'textarea').hidden === null, 'Textarea should stay on the native scrollbar: ' + JSON.stringify(result.initial));
    check(result.initial.find(x => x.id === 'absolute').position === 'absolute', 'Position changed');
    check(result.initial.find(x => x.id === 'react-scroll').children === 20, 'React children changed');
    result.handleSize = await ev(`(()=>{const v=[...document.querySelectorAll('.os-scrollbar-vertical .os-scrollbar-handle')].map(h=>parseFloat(getComputedStyle(h).width));const h=[...document.querySelectorAll('.os-scrollbar-horizontal .os-scrollbar-handle')].map(e=>parseFloat(getComputedStyle(e).height));return {vertical:v,horizontal:h}})()`);
    check(result.handleSize.vertical.length && result.handleSize.vertical.every(w => w <= 5) && result.handleSize.horizontal.every(h => h <= 5), 'Scrollbar handle too thick: ' + JSON.stringify(result.handleSize));
    result.native = await ev(`(()=>{const e=document.getElementById('plain');let events=0;e.addEventListener('scroll',()=>events++);e.scrollTop=100;e.scrollLeft=80;return new Promise(r=>setTimeout(()=>r({top:e.scrollTop,left:e.scrollLeft,events}),100))})()`);
    check(result.native.top === 100 && result.native.left === 80 && result.native.events > 0, 'Native scroll broken');
    await ev(`window.renderRows(5);document.getElementById('textarea').value=Array.from({length:80},(_,i)=>'Line '+i).join('\\n');document.getElementById('textarea').dispatchEvent(new Event('input',{bubbles:true}))`);
    await wait(300);
    check(await ev(`document.getElementById('react-scroll').children.length===5`), 'React reconciliation changed');
    result.textarea = await ev(`(()=>{const e=document.getElementById('textarea');e.scrollTop=100;return {top:e.scrollTop,value:e.value.length,children:e.children.length}})()`);
    check(result.textarea.top === 100 && result.textarea.children === 0, 'Textarea behavior changed');
    const drag = async (id, axis = 'y') => {
        const point = await ev(`(()=>{const e=document.getElementById('${id}');e.scrollTop=0;e.scrollLeft=0;const r=e.getBoundingClientRect();e.dispatchEvent(new PointerEvent('pointerenter'));return {x:r.right-5,y:r.top+8}})()`);
        await wait(120);
        await send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
        await wait(120);
        await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
        await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: point.x, y: point.y + 65, button: 'left', buttons: 1 });
        await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: point.x, y: point.y + 65, button: 'left', clickCount: 1 });
        await wait(120);
        return ev(`document.getElementById('${id}').scrollTop`);
    };
    result.drag = { plain: await drag('plain') };
    check(result.drag.plain > 0, 'Dragging failed: ' + JSON.stringify(result.drag));
    await ev(`document.querySelector('.single-select-dropdown-trigger').click()`);
    await wait(300);
    await ev(`document.querySelector('.single-select-dropdown-menu').id='dropdown-scroll'`);
    result.dropdown = { dragged: await drag('dropdown-scroll'), remainedOpen: await ev(`!!document.querySelector('.single-select-dropdown-menu')`) };
    check(result.dropdown.dragged > 0 && result.dropdown.remainedOpen, 'Dropdown drag closed the menu');
    await ev(`document.body.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}))`);
    await wait(150);
    check(await ev(`!document.querySelector('.single-select-dropdown-menu')`), 'Outside click did not close dropdown');
    await ev(`document.getElementById('mount').innerHTML='<div class="modal fixture-motion"><div id="modal-scroll" class="box"><div class="content">Dynamic modal</div></div></div>'`);
    await wait(300);
    result.modal = await ev(`(()=>{const slots=[...document.querySelectorAll('.app-scrollbar-slot')];return {enhanced:document.getElementById('modal-scroll').hasAttribute('data-overlayscrollbars'),visible:slots.map(s=>({x:parseFloat(s.style.left),y:parseFloat(s.style.top),visible:s.querySelector('.os-scrollbar-vertical').style.visibility}))}})()`);
    check(result.modal.enhanced, 'Dynamic modal missed');
    result.modal.animationAligned = await ev(`(()=>{const e=document.getElementById('modal-scroll'),r=e.getBoundingClientRect();return [...document.querySelectorAll('.app-scrollbar-slot')].some(s=>Math.abs(parseFloat(s.style.left)-r.left-e.clientLeft)<1&&Math.abs(parseFloat(s.style.top)-r.top-e.clientTop)<1&&s.querySelector('.os-scrollbar-vertical').style.visibility!=='hidden')})()`);
    check(result.modal.animationAligned,'Scrollbar did not follow the completed modal animation');
    check(result.modal.visible.filter(s => s.visible !== 'hidden').length === 1, 'Background bars over modal: ' + JSON.stringify(result.modal));
    await ev(`document.getElementById('mount').replaceChildren()`);
    await wait(300);
    result.cleanup = await ev(`(()=>{window.disposeScrollbars();return {layers:document.querySelectorAll('.app-scrollbars').length,enhanced:document.querySelectorAll('[data-overlayscrollbars]').length,children:document.getElementById('react-scroll').children.length}})()`);
    check(result.cleanup.layers === 0 && result.cleanup.enhanced === 0 && result.cleanup.children === 5, 'Cleanup failed');
    if (process.env.SCROLLBARS_REPORT)
        fs.writeFileSync(process.env.SCROLLBARS_REPORT, JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result, null, 2));
    ws.send(JSON.stringify({ id: ++id, method: 'Browser.close' }));
})().catch(e => { console.error(e); process.exitCode = 1; }).finally(async () => {
    socket?.close();
    await wait(300);
    edge.kill();
    // Only remove the unique scratch directory created by this test.
    if (path.dirname(path.resolve(base)) === path.resolve(os.tmpdir())
        && path.basename(base).startsWith('cockpit-scrollbars-')) {
        try {
            fs.rmSync(base, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
        }
        catch (error) {
            console.warn('Browser scratch cleanup:', error.message);
        }
    }
});
