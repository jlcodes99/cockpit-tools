export interface OpenCodeGoAddAccountFormValues {
  name: string;
  email: string;
  apiKey: string;
  provider?: 'go' | 'zen';
}

export type OpenCodeGoAddAccountFieldError = 'required' | 'invalid' | 'invalidEmail';

export interface OpenCodeGoAddAccountValidation {
  values: OpenCodeGoAddAccountFormValues;
  errors: { apiKey?: OpenCodeGoAddAccountFieldError; email?: OpenCodeGoAddAccountFieldError };
}

export interface OpenCodeGoCreatedConnection {
  id: string;
  name: string;
  keyHint: string;
}

export type OpenCodeGoAddAccountErrorKind =
  | 'duplicate'
  | 'limit'
  | 'invalid'
  | 'unavailable';

export type OpenCodeGoAddAccountSubmitResult =
  | { ok: true; connection: OpenCodeGoCreatedConnection }
  | { ok: false; errors: { apiKey?: OpenCodeGoAddAccountFieldError } }
  | { ok: false; error: OpenCodeGoAddAccountErrorKind };

export function initialOpenCodeGoAddAccountForm(): OpenCodeGoAddAccountFormValues {
  return { name: '', email: '', apiKey: '' };
}

export function validateOpenCodeGoAddAccount(
  form: OpenCodeGoAddAccountFormValues,
): OpenCodeGoAddAccountValidation {
  const values = {
    name: form.name.trim(),
    email: form.email.trim(),
    apiKey: form.apiKey.trim(),
  };
  const errors: OpenCodeGoAddAccountValidation['errors'] = {};
  if (!values.apiKey) errors.apiKey = 'required';
  else if (/\s/.test(values.apiKey)) errors.apiKey = 'invalid';
  if (values.email && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(values.email)) {
    errors.email = 'invalidEmail';
  }
  return { values, errors };
}

export function describeOpenCodeGoAddError(
  error: unknown,
): OpenCodeGoAddAccountErrorKind {
  const code = String(error).toUpperCase();
  if (code.includes('API_KEY_EXISTS')) return 'duplicate';
  if (code.includes('CONNECTION_LIMIT')) return 'limit';
  if (code.includes('API_KEY_REQUIRED') || code.includes('API_KEY_INVALID')) {
    return 'invalid';
  }
  return 'unavailable';
}

export async function submitOpenCodeGoAddAccount(
  form: OpenCodeGoAddAccountFormValues,
  createConnection: (
    values: OpenCodeGoAddAccountFormValues,
  ) => Promise<OpenCodeGoCreatedConnection>,
): Promise<OpenCodeGoAddAccountSubmitResult> {
  const validation = validateOpenCodeGoAddAccount(form);
  if (Object.keys(validation.errors).length > 0) {
    return { ok: false, errors: validation.errors };
  }
  try {
    const connection = await createConnection(validation.values);
    return { ok: true, connection };
  } catch (error) {
    return { ok: false, error: describeOpenCodeGoAddError(error) };
  }
}
