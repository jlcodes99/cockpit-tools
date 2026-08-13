'use strict';

(() => {
  const elements = {
    form: document.querySelector('#export-form'),
    profile: document.querySelector('#profile'),
    planFamily: document.querySelector('#plan-family'),
    dataDirectory: document.querySelector('#data-directory'),
    outputRoot: document.querySelector('#output-root'),
    staleAfterMinutes: document.querySelector('#stale-after-minutes'),
    skipInvalid: document.querySelector('#skip-invalid'),
    resetDataDirectory: document.querySelector('#reset-data-directory'),
    resetOutputRoot: document.querySelector('#reset-output-root'),
    validateButton: document.querySelector('#validate-button'),
    exportButton: document.querySelector('#export-button'),
    formMessage: document.querySelector('#form-message'),
    versionLabel: document.querySelector('#version-label'),
    resultTitle: document.querySelector('#result-title'),
    resultState: document.querySelector('#result-state'),
    emptyResult: document.querySelector('#empty-result'),
    resultContent: document.querySelector('#result-content'),
    metricGrid: document.querySelector('#metric-grid'),
    selectionChips: document.querySelector('#selection-chips'),
    outputDirectoryBlock: document.querySelector('#output-directory-block'),
    outputDirectory: document.querySelector('#output-directory'),
    copyOutputPath: document.querySelector('#copy-output-path'),
    filesBlock: document.querySelector('#files-block'),
    fileList: document.querySelector('#file-list'),
    summaryJson: document.querySelector('#summary-json'),
    shutdownButton: document.querySelector('#shutdown-button'),
  };

  const state = {
    defaults: null,
    busy: false,
    outputDirectory: null,
  };

  const datasetLabels = {
    accounts: '账号身份',
    quota: '额度窗口',
    gateway: '网关用量',
  };
  const planLabels = {
    team: 'Team 家族',
    pro: 'Pro',
    plus: 'Plus',
    free: 'Free',
    all: '全部套餐',
  };

  async function apiRequest(pathname, { method = 'GET', body } = {}) {
    const response = await fetch(pathname, {
      method,
      credentials: 'same-origin',
      headers: body ? { 'Content-Type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
      cache: 'no-store',
    });
    let payload;
    try {
      payload = await response.json();
    } catch {
      throw new Error(`本地服务返回了无法解析的响应（HTTP ${response.status}）。`);
    }
    if (!response.ok || !payload.ok) {
      throw new Error(payload.error || `本地服务请求失败（HTTP ${response.status}）。`);
    }
    return payload;
  }

  function checkedValues(name) {
    return [...document.querySelectorAll(`input[name="${name}"]:checked`)].map(
      (input) => input.value,
    );
  }

  function collectOptions() {
    const datasets = checkedValues('datasets');
    const formats = checkedValues('formats');
    if (datasets.length === 0) {
      throw new Error('请至少选择一类导出内容。');
    }
    if (formats.length === 0) {
      throw new Error('请至少选择一种输出格式。');
    }
    const staleAfterMinutes = Number(elements.staleAfterMinutes.value);
    if (!Number.isInteger(staleAfterMinutes) || staleAfterMinutes < 1 || staleAfterMinutes > 10080) {
      throw new Error('快照陈旧阈值必须是 1 到 10080 之间的整数分钟。');
    }
    if (!elements.dataDirectory.value.trim()) {
      throw new Error('数据目录不能为空。');
    }
    if (!elements.outputRoot.value.trim()) {
      throw new Error('输出根目录不能为空。');
    }
    return {
      profile: elements.profile.value,
      planFamily: elements.planFamily.value,
      dataDirectory: elements.dataDirectory.value.trim(),
      outputRoot: elements.outputRoot.value.trim(),
      datasets,
      formats,
      staleAfterMinutes,
      skipInvalid: elements.skipInvalid.checked,
    };
  }

  function showFormMessage(message) {
    elements.formMessage.textContent = message || '';
    elements.formMessage.classList.toggle('visible', Boolean(message));
  }

  function setBusy(busy, label) {
    state.busy = busy;
    elements.validateButton.disabled = busy;
    elements.exportButton.disabled = busy;
    elements.shutdownButton.disabled = busy;
    if (busy) {
      elements.resultState.className = 'result-state busy';
      elements.resultState.textContent = label || '处理中';
    }
  }

  function setResultState(kind, text) {
    elements.resultState.className = `result-state ${kind}`;
    elements.resultState.textContent = text;
  }

  function appendMetric(label, value) {
    const card = document.createElement('div');
    card.className = 'metric-card';
    const caption = document.createElement('span');
    caption.textContent = label;
    const number = document.createElement('strong');
    number.textContent = String(value ?? '—');
    card.append(caption, number);
    elements.metricGrid.append(card);
  }

  function appendChip(text) {
    const chip = document.createElement('span');
    chip.className = 'chip';
    chip.textContent = text;
    elements.selectionChips.append(chip);
  }

  function formatBytes(bytes) {
    const value = Number(bytes);
    if (!Number.isFinite(value) || value < 0) {
      return '—';
    }
    if (value < 1024) {
      return `${value} B`;
    }
    if (value < 1024 * 1024) {
      return `${(value / 1024).toFixed(1)} KB`;
    }
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }

  function renderFiles(files) {
    elements.fileList.replaceChildren();
    for (const file of files || []) {
      const item = document.createElement('li');
      item.className = 'file-item';
      const name = document.createElement('strong');
      name.textContent = file.name;
      const size = document.createElement('span');
      size.textContent = formatBytes(file.bytes);
      const hash = document.createElement('div');
      hash.className = 'file-hash';
      hash.title = file.sha256 || '';
      hash.textContent = file.sha256 ? `SHA-256  ${file.sha256}` : 'SHA-256 —';
      item.append(name, size, hash);
      elements.fileList.append(item);
    }
  }

  function renderResult(payload, options, mode) {
    const summary = payload.summary || {};
    elements.emptyResult.hidden = true;
    elements.resultContent.hidden = false;
    elements.metricGrid.replaceChildren();
    elements.selectionChips.replaceChildren();
    appendMetric('账号数量', summary.accountCount);
    appendMetric('API 池账号', summary.apiPoolAccountCount);
    appendMetric('额度耗尽', summary.exhaustedAccountCount);
    appendMetric('快照陈旧', summary.staleCount);
    appendMetric('需要重授权', summary.requiresReauthCount);
    appendMetric('跳过账号', summary.skippedAccountCount);

    appendChip(options.profile === 'production' ? '正式库' : '开发库');
    appendChip(planLabels[options.planFamily] || options.planFamily);
    for (const dataset of options.datasets) {
      appendChip(datasetLabels[dataset] || dataset);
    }
    appendChip(options.formats.map((format) => format.toUpperCase()).join(' + '));
    if (options.skipInvalid) {
      appendChip('允许部分结果');
    }

    state.outputDirectory = payload.outputDirectory || null;
    elements.outputDirectoryBlock.hidden = !state.outputDirectory;
    elements.outputDirectory.textContent = state.outputDirectory || '';
    const files = Array.isArray(summary.outputFiles) ? [...summary.outputFiles] : [];
    if (summary.summaryFile && !files.some((file) => file.name === summary.summaryFile)) {
      files.push({ name: summary.summaryFile, bytes: null, sha256: null });
    }
    elements.filesBlock.hidden = files.length === 0;
    renderFiles(files);
    elements.summaryJson.textContent = JSON.stringify(summary, null, 2);
    elements.resultTitle.textContent = mode === 'validate' ? '验证通过' : '导出完成';
    setResultState('success', mode === 'validate' ? '只读验证' : '已写入');
  }

  async function execute(mode) {
    if (state.busy) {
      return;
    }
    showFormMessage('');
    let options;
    try {
      options = collectOptions();
    } catch (error) {
      showFormMessage(error.message);
      return;
    }
    setBusy(true, mode === 'validate' ? '验证中' : '导出中');
    elements.resultTitle.textContent = mode === 'validate' ? '正在验证账号库' : '正在生成导出';
    try {
      const payload = await apiRequest(
        mode === 'validate' ? '/api/validate' : '/api/export',
        { method: 'POST', body: options },
      );
      renderResult(payload, options, mode);
    } catch (error) {
      showFormMessage(error.message);
      elements.resultTitle.textContent = mode === 'validate' ? '验证失败' : '导出失败';
      setResultState('error', '需要检查');
    } finally {
      setBusy(false);
    }
  }

  function applyDefaults(payload) {
    state.defaults = payload.defaults;
    elements.versionLabel.textContent = `v${payload.exporterVersion}`;
    elements.profile.value = payload.defaults.profile;
    elements.planFamily.value = payload.defaults.planFamily;
    elements.dataDirectory.value = payload.defaults.dataDirectories[payload.defaults.profile];
    elements.dataDirectory.dataset.managedDefault = 'true';
    elements.outputRoot.value = payload.defaults.outputRoot;
    elements.staleAfterMinutes.value = String(payload.defaults.staleAfterMinutes);
    elements.skipInvalid.checked = payload.defaults.skipInvalid;
    for (const input of document.querySelectorAll('input[name="datasets"]')) {
      input.checked = payload.defaults.datasets.includes(input.value);
    }
    for (const input of document.querySelectorAll('input[name="formats"]')) {
      input.checked = payload.defaults.formats.includes(input.value);
    }
  }

  async function initialize() {
    try {
      const payload = await apiRequest('/api/defaults');
      applyDefaults(payload);
    } catch (error) {
      showFormMessage(error.message);
      elements.resultTitle.textContent = '本地服务不可用';
      setResultState('error', '连接失败');
      elements.validateButton.disabled = true;
      elements.exportButton.disabled = true;
    }
  }

  elements.form.addEventListener('submit', (event) => {
    event.preventDefault();
    execute('export');
  });
  elements.validateButton.addEventListener('click', () => execute('validate'));
  elements.profile.addEventListener('change', () => {
    if (state.defaults && elements.dataDirectory.dataset.managedDefault === 'true') {
      elements.dataDirectory.value = state.defaults.dataDirectories[elements.profile.value];
    }
  });
  elements.dataDirectory.addEventListener('input', () => {
    elements.dataDirectory.dataset.managedDefault = 'false';
  });
  elements.resetDataDirectory.addEventListener('click', () => {
    if (!state.defaults) {
      return;
    }
    elements.dataDirectory.value = state.defaults.dataDirectories[elements.profile.value];
    elements.dataDirectory.dataset.managedDefault = 'true';
  });
  elements.resetOutputRoot.addEventListener('click', () => {
    if (state.defaults) {
      elements.outputRoot.value = state.defaults.outputRoot;
    }
  });
  elements.copyOutputPath.addEventListener('click', async () => {
    if (!state.outputDirectory) {
      return;
    }
    try {
      await navigator.clipboard.writeText(state.outputDirectory);
      elements.copyOutputPath.textContent = '已复制';
      window.setTimeout(() => {
        elements.copyOutputPath.textContent = '复制路径';
      }, 1600);
    } catch {
      showFormMessage('浏览器无法访问剪贴板，请手动复制输出路径。');
    }
  });
  elements.shutdownButton.addEventListener('click', async () => {
    if (state.busy) {
      return;
    }
    setBusy(true, '关闭中');
    try {
      await apiRequest('/api/shutdown', { method: 'POST', body: {} });
      elements.resultTitle.textContent = '本地服务已关闭';
      setResultState('idle', '已停止');
      elements.shutdownButton.textContent = '可以关闭页面';
    } catch (error) {
      showFormMessage(error.message);
      setResultState('error', '关闭失败');
    }
  });

  initialize();
})();
