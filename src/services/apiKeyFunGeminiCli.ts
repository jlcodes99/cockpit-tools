import { invoke } from '@tauri-apps/api/core';

export function applyApiKeyFunGeminiCliEnv(input: {
  apiKey: string;
  model: string;
}): Promise<void> {
  return invoke('apply_apikey_fun_gemini_cli_env', {
    apiKey: input.apiKey,
    model: input.model,
  });
}
