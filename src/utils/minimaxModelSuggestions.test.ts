import assert from 'node:assert/strict';
import test from 'node:test';
import { filterMiniMaxModelSuggestions } from './minimaxModelSuggestions.ts';

const suggestions = ['MiniMax-M2.7', 'MiniMax-M3', 'other-model', 'MiniMax-M3.1-Flash', 'speech-2.8-hd', 'MiniMax-Hailuo-2.3'].map(id => ({ id }));

test('direct discovery offers Flash first without mutating saved values or nontext models', () => {
  for (const base of ['https://api.minimax.io/v1', 'https://api.minimaxi.com/v1']) {
    assert.deepEqual(filterMiniMaxModelSuggestions(base, suggestions).map(model => model.id), ['MiniMax-M3.1-Flash', 'MiniMax-M3', 'other-model', 'speech-2.8-hd', 'MiniMax-Hailuo-2.3']);
  }
  assert.equal(suggestions[0].id, 'MiniMax-M2.7');
  assert.equal(filterMiniMaxModelSuggestions('https://custom.example/v1', suggestions), suggestions);
  assert.equal(filterMiniMaxModelSuggestions('invalid', suggestions), suggestions);
});

test('gateway discovery keeps only returned current routes without inventing models', () => {
  const gateway = ['minimax-m2.5', 'minimax-m3', 'minimax-m3.1-flash-free', 'kimi-k3'].map(id => ({ id }));
  assert.deepEqual(filterMiniMaxModelSuggestions('https://opencode.ai/zen/go/v1', gateway).map(model => model.id), ['minimax-m3.1-flash-free', 'minimax-m3', 'kimi-k3']);
  assert.deepEqual(filterMiniMaxModelSuggestions('https://opencode.ai/zen/go/v1', [{ id: 'minimax-m3' }]), [{ id: 'minimax-m3' }]);
  assert.deepEqual(filterMiniMaxModelSuggestions('https://api.siliconflow.cn', [{ id: 'Pro/MiniMaxAI/MiniMax-M2.7' }, { id: 'deepseek-ai/DeepSeek-V3' }]), [{ id: 'deepseek-ai/DeepSeek-V3' }]);
});
