const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const { runInNewContext } = require('node:vm');

const script = readFileSync(path.join(__dirname, 'antigravity_desktop_localization.js'), 'utf8');
const dictionary = require('./antigravity_desktop_zh_cn.json');
const templates = require('./antigravity_desktop_zh_cn_templates.json');

function makePage() {
  const Node = { ELEMENT_NODE: 1, TEXT_NODE: 3 };
  const NodeFilter = { SHOW_ELEMENT: 1, SHOW_TEXT: 4, FILTER_ACCEPT: 1, FILTER_REJECT: 2 };
  const frames = new Map();
  let frameId = 0;
  let deliver;
  const window = {
    __APP_CONFIG__: {},
    requestAnimationFrame(callback) {
      frames.set(++frameId, callback);
      return frameId;
    },
    cancelAnimationFrame(id) { frames.delete(id); },
  };
  window.top = window;

  function element(tag, value, initialAttributes = {}, selector = null) {
    const attributes = new Map(Object.entries(initialAttributes));
    const node = {
      nodeType: Node.ELEMENT_NODE,
      parentElement: null,
      get isConnected() { return node === body || Boolean(node.parentElement?.isConnected); },
      children: [],
      getAttribute: (name) => attributes.get(name) ?? null,
      setAttribute: (name, next) => attributes.set(name, next),
      matches(selectors) { return selectors.split(',').some((entry) => entry === tag || entry === selector); },
      closest(selectors) { return node.matches(selectors) ? node : node.parentElement?.closest(selectors); },
      append(child) {
        node.children.push(child);
        child.parentElement = node;
      },
      remove(child) {
        node.children.splice(node.children.indexOf(child), 1);
        child.parentElement = null;
      },
    };
    if (value !== undefined) {
      node.text = {
        nodeType: Node.TEXT_NODE,
        nodeValue: value,
        get isConnected() { return Boolean(this.parentElement?.isConnected); },
      };
      node.append(node.text);
    }
    return node;
  }

  const body = element('body');
  const install = runInNewContext(script, {
    window, Node, NodeFilter,
    location: { protocol: 'https:', hostname: '127.0.0.1' },
    document: {
      body,
      querySelector: () => null,
      createTreeWalker(root, _mask, { acceptNode }) {
        const children = function* walkChildren(node) {
          for (const child of node.children ?? []) {
            if (acceptNode(child) === NodeFilter.FILTER_REJECT) continue;
            yield child;
            yield* walkChildren(child);
          }
        }(root);
        return { nextNode() { this.currentNode = children.next().value; return this.currentNode; } };
      },
    },
    MutationObserver: class {
      constructor(callback) { this.callback = callback; }
      observe() { deliver = this.callback; }
      disconnect() { deliver = undefined; }
    },
  });

  return {
    body, element, window,
    install: () => install(dictionary, templates),
    frame() {
      for (const [id, callback] of [...frames]) {
        frames.delete(id);
        callback();
      }
    },
    changed(...records) { deliver?.(records); },
  };
}

test('localizes UI text, attributes and controls added after mount', () => {
  const page = makePage();
  const title = page.element('h1', '  Settings\n');
  const button = page.element('button', 'New Conversation', { 'aria-label': 'New Conversation' });
  const search = page.element('input', undefined, { placeholder: 'Search conversations...' });
  for (const node of [title, button, search]) page.body.append(node);

  assert.equal(page.install(), true);
  page.frame();
  assert.equal(title.text.nodeValue, '  设置\n');
  assert.equal(button.text.nodeValue, '新建对话');
  assert.equal(button.getAttribute('aria-label'), '新建对话');
  assert.equal(search.getAttribute('placeholder'), '搜索对话…');

  const menu = page.element('div', 'Appearance', {}, '[role="menuitem"]');
  page.body.append(menu);
  page.changed({ type: 'childList', target: page.body, addedNodes: [menu], removedNodes: [] });
  page.frame();
  assert.equal(menu.text.nodeValue, '外观');

  button.text.nodeValue = 'Cancel';
  page.changed({ type: 'characterData', target: button.text });
  page.frame();
  assert.equal(button.text.nodeValue, '取消');

  button.text.nodeValue = 'Updated by the client';
  page.window.__cockpitAntigravityLocalization.setEnabled(false);
  assert.equal(button.text.nodeValue, 'Updated by the client');
  assert.equal(title.text.nodeValue, '  Settings\n');
  assert.equal(button.getAttribute('aria-label'), 'New Conversation');
  assert.equal(menu.text.nodeValue, 'Appearance');

  page.install();
  page.frame();
  assert.equal(menu.text.nodeValue, '外观');
});

test('leaves protected content alone and restores controls moved into it', () => {
  const page = makePage();
  const protectedAreas = [
    '.monaco-editor', '.xterm', '[data-testid*="assistant-message"]',
    '[data-testid*="project-name"]', '[data-cockpit-ag-details]',
  ].map((selector) => page.element('div', undefined, {}, selector));
  const protectedButtons = protectedAreas.map((area) => {
    page.body.append(area);
    const button = page.element('button', 'Settings', { title: 'Settings' });
    area.append(button);
    return button;
  });
  const movable = page.element('button', 'Open');
  page.body.append(movable);
  page.install();
  page.frame();
  for (const button of protectedButtons) {
    assert.equal(button.text.nodeValue, 'Settings');
    assert.equal(button.getAttribute('title'), 'Settings');
  }
  assert.equal(movable.text.nodeValue, '打开');

  page.body.remove(movable);
  protectedAreas[0].append(movable);
  page.changed({ type: 'childList', target: page.body, addedNodes: [], removedNodes: [movable] });
  page.frame();
  assert.equal(movable.text.nodeValue, 'Open');
});

test('leaves other loopback pages untouched', () => {
  const page = makePage();
  delete page.window.__APP_CONFIG__;
  assert.equal(page.install(), false);
  assert.equal(page.window.__cockpitAntigravityLocalization, undefined);
});

test('translates the Antigravity settings labels and plain descriptions without translating project names', () => {
  const page = makePage();
  const modal = page.element('div', undefined, {}, '.settings-modal-container');
  const sidebar = page.element('div', undefined, {}, '[data-testid="settings-sidebar"]');
  const pane = page.element('div', undefined, {}, '.settings-content-pane');
  page.body.append(modal);
  modal.append(sidebar);
  modal.append(pane);

  const settingsText = [
    ['Application', '应用', 'button', sidebar],
    ['Show All', '查看全部', 'span', sidebar],
    ['Not in Project', '无所属项目', 'span', sidebar],
    ['Give Feedback', '提交反馈', 'span', sidebar],
    ['Knowledge Work', '知识工作', 'div', pane],
    ['Software Engineering', '软件工程', 'div', pane],
    ['Configure agent execution, queued message delivery, and permissions.', '设置智能体的执行方式、消息队列和权限。', 'p', pane],
    ['Simplified interface for research, writing, and everyday tasks.', '为调研、写作及日常工作提供精简界面。', 'div', pane],
    ['Full interface for coding, developer tools, and managing agents.', '提供编程、开发工具及智能体管理所需的完整界面。', 'div', pane],
    ['This computer', '本机', 'div', pane],
    ['Remote', '远程', 'div', pane],
    ['Turbo', '全自动模式', 'div', pane],
    ['Controls permissions for remote conversations.', '管理远程对话中的权限。', 'div', pane],
    ['Configure what the agent can do in local and remote conversations.', '设置智能体在本地及远程对话中可以执行的操作。', 'p', pane],
    ['Learn more', '了解详情', 'a', pane],
    ['Input Box', '输入区', 'h3', pane],
    ['Queued messages', '消息队列', 'div', pane],
    ['Keyboard shortcuts', '键盘快捷键', 'span', pane],
    ['Queue', '加入队列', 'button', pane],
    ['Send Immediately', '立即发送', 'button', pane],
    ['Choose which key sends a message in the chat composer.', '指定在对话输入区用哪个按键发送消息。', 'div', pane],
    ['Prevent the computer from sleeping while the app is running.', '应用运行时阻止电脑休眠。', 'div', pane],
    ['Configure the agent\'s visual theme and display preferences.', '设置智能体界面的主题与显示偏好。', 'div', pane],
    ['Manage your notification preferences.', '管理通知偏好。', 'div', pane],
    ['Manage your model quota and credits.', '管理模型额度和点数。', 'div', pane],
    ['Choose where links open by default when clicked.', '选择点击链接时默认打开的位置。', 'div', pane],
  ].map(([source, translation, tag, parent]) => {
    const label = page.element(tag, source);
    parent.append(label);
    return { label, source, translation };
  });
  const project = page.element('button', 'Knowledge Work');
  sidebar.append(project);
  const userText = page.element('textarea', 'Remote');
  pane.append(userText);
  const outsideText = page.element('p', 'Knowledge Work');
  page.body.append(outsideText);

  assert.equal(page.install(), true);
  page.frame();
  assert.deepEqual(settingsText.map(({ label }) => label.text.nodeValue), settingsText.map(({ translation }) => translation));
  assert.equal(project.text.nodeValue, 'Knowledge Work');
  assert.equal(userText.text.nodeValue, 'Remote');
  assert.equal(outsideText.text.nodeValue, 'Knowledge Work');

  const nextSection = page.element('div', 'Voice mode');
  pane.append(nextSection);
  page.changed({ type: 'childList', target: pane, addedNodes: [nextSection], removedNodes: [] });
  page.frame();
  assert.equal(nextSection.text.nodeValue, '语音模式');

  page.window.__cockpitAntigravityLocalization.setEnabled(false);
  assert.deepEqual(settingsText.map(({ label }) => label.text.nodeValue), settingsText.map(({ source }) => source));
});

test('translates the composer hint and keeps equal project or editor content intact', () => {
  const page = makePage();
  const source = 'Ask anything, @ to mention, / for actions';
  const composerHint = page.element('p', source, {}, '[data-agent-composer-root] p.pointer-events-none.select-none');
  const editorText = page.element('p', source, {}, '[contenteditable]:not([contenteditable="false"])');
  const project = page.element('button', 'Home', {}, '[data-project-name]');
  for (const node of [composerHint, editorText, project]) page.body.append(node);

  page.install();
  page.frame();
  assert.equal(composerHint.text.nodeValue, '输入你的问题，输入 @ 引用内容，输入 / 查看操作');
  assert.equal(editorText.text.nodeValue, source);
  assert.equal(project.text.nodeValue, 'Home');
});

test('keeps Orchestrator and model names in English in the picker', () => {
  const page = makePage();
  const header = page.element('div', 'Model', {}, '[data-testid="model-selector-header"]');
  const row = page.element('button', undefined, { title: 'Orchestrator' });
  const prefix = page.element('span', 'Orchestrator');
  const model = page.element('span', 'Gemini 4 Neon High', {}, '[data-testid="model-selector-orchestrator-model"]');
  const customModel = page.element('span', 'Review', {}, '[data-testid="model-selector-custom-item"] span.truncate');
  const defaultPlanner = page.element('span', 'Default (Gemini Pro)', {}, '[data-testid="orchestrator-picker-default"] span.truncate');
  const trigger = page.element('button', 'Orchestrator Gemini 4 Neon High', {
    'data-model-label': 'Orchestrator Gemini 4 Neon High',
    'aria-label': 'Select model, current: Orchestrator Gemini 4 Neon High. Daily limit',
  }, '[data-testid="model-selector-trigger"]');
  row.append(prefix);
  row.append(model);
  for (const item of [header, row, trigger, customModel, defaultPlanner]) page.body.append(item);

  assert.equal(page.install(), true);
  page.frame();
  assert.equal(header.text.nodeValue, '模型');
  assert.equal(prefix.text.nodeValue, 'Orchestrator');
  assert.equal(row.getAttribute('title'), 'Orchestrator');
  assert.equal(model.text.nodeValue, 'Gemini 4 Neon High');
  assert.equal(customModel.text.nodeValue, 'Review');
  assert.equal(defaultPlanner.text.nodeValue, 'Default (Gemini Pro)');
  assert.equal(trigger.text.nodeValue, 'Orchestrator Gemini 4 Neon High');
  assert.equal(trigger.getAttribute('data-model-label'), 'Orchestrator Gemini 4 Neon High');
  assert.equal(trigger.getAttribute('aria-label'), '选择模型，当前为 Orchestrator Gemini 4 Neon High。Daily limit');

  trigger.setAttribute('aria-label', 'Select model, current: Gemini 4 Carbon');
  page.changed({ type: 'attributes', target: trigger, attributeName: 'aria-label' });
  assert.equal(trigger.getAttribute('aria-label'), '选择模型，当前为 Gemini 4 Carbon');
  page.window.__cockpitAntigravityLocalization.setEnabled(false);
  assert.equal(trigger.getAttribute('aria-label'), 'Select model, current: Gemini 4 Carbon');
});

test('localizes plain status text and tooltips while preserving queued prompts and project names', () => {
  const page = makePage();
  const status = page.element('span', 'Sends after agent finishes working', {
    title: 'Sends after agent finishes working',
  }, '[data-testid="queued-messages-status"]');
  const queuedEntry = page.element('div', undefined, {}, '[data-queued-msg-id]');
  const authoredPrompt = page.element('div', undefined, {}, '[data-quotable]');
  const prompt = page.element('a', 'Failed to start conversation', {}, 'a[href]');
  const sendNow = page.element('button', 'Send Now', { 'aria-label': 'Send now: Failed to start conversation' });
  authoredPrompt.append(prompt);
  queuedEntry.append(authoredPrompt);
  queuedEntry.append(sendNow);
  const toast = page.element('div', undefined, {}, '[data-testid="toast-notification"]');
  const error = page.element('span', 'Failed to start conversation');
  toast.append(error);
  const project = page.element('button', 'No conversation', {
    'aria-label': 'Select project, current: No conversation (Local)',
  }, '[data-testid="project-selector-trigger"]');
  const sidebar = page.element('span', 'No conversations yet', {}, '[data-sidebar-section-drop-id] > span.select-none.opacity-30');
  for (const item of [status, queuedEntry, toast, project, sidebar]) page.body.append(item);

  assert.equal(page.install(), true);
  page.frame();
  assert.equal(status.text.nodeValue, '智能体完成当前任务后发送');
  assert.equal(status.getAttribute('title'), '智能体完成当前任务后发送');
  assert.equal(prompt.text.nodeValue, 'Failed to start conversation');
  assert.equal(sendNow.getAttribute('aria-label'), '立即发送：Failed to start conversation');
  assert.equal(error.text.nodeValue, '无法开始对话');
  assert.equal(project.text.nodeValue, 'No conversation');
  assert.equal(project.getAttribute('aria-label'), '选择项目，当前为 No conversation (Local)');
  assert.equal(sidebar.text.nodeValue, '暂无对话');

  page.body.remove(status);
  authoredPrompt.append(status);
  page.changed({ type: 'childList', target: page.body, addedNodes: [], removedNodes: [status] });
  page.frame();
  assert.equal(status.text.nodeValue, 'Sends after agent finishes working');
  assert.equal(status.getAttribute('title'), 'Sends after agent finishes working');
});

test('translates workflow states and keeps variable content intact', () => {
  const page = makePage();
  const onboarding = page.element('div', undefined, {}, '[data-testid="post-onboarding-welcome-modal"]');
  const permissions = page.element('div', undefined, {}, '[data-testid="declared-permissions-modal"]');
  const browser = page.element('div', undefined, {}, '[data-testid="browser-error-overlay"]');
  const automation = page.element('div', undefined, {}, '[data-testid="automation-settings"]');
  for (const section of [onboarding, permissions, browser, automation]) page.body.append(section);

  const title = page.element('div', 'Welcome to Antigravity!');
  onboarding.append(title);
  const permission = page.element('div', 'Review permissions for Build  Plugin');
  permissions.append(permission);
  const description = page.element('button', undefined, { 'aria-description': 'Agent permissions preset selector, current: Custom' });
  permissions.append(description);
  const browserError = page.element('div', 'The preview did not respond on port 4567.');
  browser.append(browserError);
  const automationStatus = page.element('div', 'Created automation "Run  tests $&".');
  automation.append(automationStatus);
  const authoredName = page.element('button', 'Created automation "Run  tests $&".', {}, '[data-testid="automation-name"]');
  automation.append(authoredName);

  assert.equal(page.install(), true);
  page.frame();
  assert.equal(title.text.nodeValue, '欢迎使用 Antigravity！');
  assert.equal(permission.text.nodeValue, '检查 Build  Plugin 的权限');
  assert.equal(description.getAttribute('aria-description'), '智能体权限预设选择器，当前为 Custom');
  assert.equal(browserError.text.nodeValue, '端口 4567 上的预览服务未响应。');
  assert.equal(automationStatus.text.nodeValue, '已创建自动任务「Run  tests $&」。');
  assert.equal(authoredName.text.nodeValue, 'Created automation "Run  tests $&".');

  automationStatus.text.nodeValue = 'Failed to start "Run  tests $&".';
  page.changed({ type: 'characterData', target: automationStatus.text });
  page.frame();
  assert.equal(automationStatus.text.nodeValue, '无法启动「Run  tests $&」。');
  page.window.__cockpitAntigravityLocalization.setEnabled(false);
  assert.equal(automationStatus.text.nodeValue, 'Failed to start "Run  tests $&".');
  assert.equal(description.getAttribute('aria-description'), 'Agent permissions preset selector, current: Custom');
});

test('every localization template preserves the source arguments', () => {
  const slots = (value) => [...new Set([...value.matchAll(/\{(\d+)\}/g)].map((match) => match[1]))].sort();
  for (const [source, translation] of Object.entries(templates)) {
    assert.ok(slots(source).length > 0, source);
    assert.deepEqual(slots(translation), slots(source), source);
  }
});
