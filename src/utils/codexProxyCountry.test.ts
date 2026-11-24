import assert from 'node:assert/strict';
import test from 'node:test';
import { codexCountryErrorKey, codexCountryErrorValues, codexCountryOptions } from './codexProxyCountry';

test('country options cover ISO countries without guessing the region from node labels', () => {
  const options = codexCountryOptions('zh-CN');
  assert.equal(options.length, 249);
  assert.equal(new Set(options.map(option => option.value)).size, 249);
  assert.match(options.find(option => option.value === 'JP')!.label, /JP/);
  assert.match(options.find(option => option.value === 'JP')!.searchText, /Japan/);
  assert.match(options.find(option => option.value === 'SG')!.label, /SG/);
});
test('country errors display only the verified expected and actual country codes', () => {
  assert.equal(codexCountryErrorKey('PROXY_COUNTRY_MISMATCH:JP:SG'), 'codex.proxy.country.mismatch');
  assert.deepEqual(codexCountryErrorValues(new Error('PROXY_COUNTRY_MISMATCH:JP:SG')), { expected: 'JP', actual: 'SG' });
  assert.equal(codexCountryErrorKey('PROXY_COUNTRY_UNAVAILABLE'), 'codex.proxy.country.unavailable');
  for (const input of ['PROXY_COUNTRY_MISMATCH:JP:https://user:secret@private.invalid', 'toString', 'PROXY_COUNTRY_UNAVAILABLE_OTHER']) {
    assert.equal(codexCountryErrorKey(input), null);
    assert.deepEqual(codexCountryErrorValues(input), {});
  }
});
