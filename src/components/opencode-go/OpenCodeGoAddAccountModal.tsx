import {
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
} from 'react';
import { Eye, EyeOff, KeyRound, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscClose } from '../../hooks/useEscClose';
import {
  initialOpenCodeGoAddAccountForm,
  submitOpenCodeGoAddAccount,
  validateOpenCodeGoAddAccount,
  type OpenCodeGoAddAccountErrorKind,
  type OpenCodeGoAddAccountFieldError,
  type OpenCodeGoCreatedConnection,
} from './openCodeGoAddAccountForm';

export interface OpenCodeGoAddAccountModalProps {
  open: boolean;
  createConnection: (input: {
    name: string;
    apiKey: string;
    provider?: 'go' | 'zen';
  }) => Promise<OpenCodeGoCreatedConnection>;
  onClose: () => void;
  onCreated?: (connection: OpenCodeGoCreatedConnection) => void;
}

export function OpenCodeGoAddAccountModal({
  open,
  createConnection,
  onClose,
  onCreated,
}: OpenCodeGoAddAccountModalProps) {
  const { t } = useTranslation();
  const titleId = useId();
  const descriptionId = useId();
  const apiKeyErrorId = useId();
  const submitErrorId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const [form, setForm] = useState(initialOpenCodeGoAddAccountForm);
  const [fieldError, setFieldError] =
    useState<OpenCodeGoAddAccountFieldError | null>(null);
  const [submitError, setSubmitError] =
    useState<OpenCodeGoAddAccountErrorKind | null>(null);
  const [apiKeyVisible, setApiKeyVisible] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const close = () => {
    if (!submitting) onClose();
  };
  useEscClose(open && !submitting, close);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setForm(initialOpenCodeGoAddAccountForm());
    setFieldError(null);
    setSubmitError(null);
    setApiKeyVisible(false);
    const frame = window.requestAnimationFrame(() => nameInputRef.current?.focus());
    return () => {
      window.cancelAnimationFrame(frame);
      restoreFocusRef.current?.focus();
      restoreFocusRef.current = null;
    };
  }, [open]);

  if (!open) return null;

  const errorText = (error: OpenCodeGoAddAccountErrorKind) => {
    switch (error) {
      case 'duplicate':
        return t(
          'openCodeGo.add.errors.duplicate',
          'This API key is already configured.',
        );
      case 'limit':
        return t(
          'openCodeGo.add.errors.limit',
          'You have reached the OpenCode Go connection limit.',
        );
      case 'invalid':
        return t(
          'openCodeGo.add.errors.invalid',
          'Enter a valid API key without spaces.',
        );
      default:
        return t(
          'openCodeGo.add.errors.unavailable',
          'The connection could not be saved. Try again.',
        );
    }
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (submitting) return;
    const validation = validateOpenCodeGoAddAccount(form);
    const nextFieldError = validation.errors.apiKey ?? null;
    setFieldError(nextFieldError);
    setSubmitError(null);
    if (nextFieldError) return;

    setSubmitting(true);
    const result = await submitOpenCodeGoAddAccount(form, createConnection);
    setSubmitting(false);
    if (result.ok) {
      setForm(initialOpenCodeGoAddAccountForm());
      onCreated?.(result.connection);
      onClose();
      return;
    }
    if ('errors' in result) {
      setFieldError(result.errors.apiKey ?? null);
    } else if ('error' in result) {
      setSubmitError(result.error);
    }
  };

  const handleBackdropClick = (event: MouseEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget) close();
  };

  const handleDialogKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Tab') return;
    const focusable = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const apiKeyErrorText =
    fieldError === 'required'
      ? t('openCodeGo.add.errors.required', 'API key is required.')
      : fieldError === 'invalid'
        ? t(
            'openCodeGo.add.errors.invalid',
            'Enter a valid API key without spaces.',
          )
        : null;

  return (
    <div className="modal-overlay opencode-go-add-overlay" onMouseDown={handleBackdropClick}>
      <div
        ref={dialogRef}
        className="modal opencode-go-add-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        onKeyDown={handleDialogKeyDown}
      >
        <div className="modal-header">
          <div className="opencode-go-add-title">
            <span aria-hidden="true"><KeyRound size={18} /></span>
            <h2 id={titleId}>
              {t('openCodeGo.add.title', 'Add OpenCode Go connection')}
            </h2>
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={close}
            disabled={submitting}
            aria-label={t('common.close', 'Close')}
          >
            <X />
          </button>
        </div>

        <form onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="modal-body opencode-go-add-body">
            <p id={descriptionId} className="opencode-go-add-description">
              {t(
                'openCodeGo.add.description',
                'The key is encrypted in local app data and is masked after saving.',
              )}
            </p>

            {submitError && (
              <div id={submitErrorId} className="opencode-go-add-error" role="alert">
                {errorText(submitError)}
              </div>
            )}

            <div className="form-group">
              <label htmlFor={`${titleId}-name`}>
                {t('openCodeGo.add.name', 'Connection name')}
                <span className="opencode-go-add-optional">
                  {t('common.optional', 'Optional')}
                </span>
              </label>
              <input
                ref={nameInputRef}
                id={`${titleId}-name`}
                type="text"
                value={form.name}
                onChange={(event) =>
                  setForm((previous) => ({ ...previous, name: event.target.value }))
                }
                placeholder={t('openCodeGo.add.namePlaceholder', 'Primary')}
                autoComplete="off"
                disabled={submitting}
              />
            </div>

            <div className="form-group">
              <label htmlFor={`${titleId}-api-key`}>
                {t('openCodeGo.add.apiKey', 'API key')}
              </label>
              <div className="opencode-go-secret-input">
                <input
                  id={`${titleId}-api-key`}
                  type={apiKeyVisible ? 'text' : 'password'}
                  value={form.apiKey}
                  onChange={(event) => {
                    setForm((previous) => ({
                      ...previous,
                      apiKey: event.target.value,
                    }));
                    if (fieldError) setFieldError(null);
                  }}
                  autoComplete="new-password"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  disabled={submitting}
                  aria-invalid={Boolean(apiKeyErrorText)}
                  aria-describedby={
                    [
                      apiKeyErrorText ? apiKeyErrorId : '',
                      submitError ? submitErrorId : '',
                    ]
                      .filter(Boolean)
                      .join(' ') || undefined
                  }
                />
                <button
                  type="button"
                  className="opencode-go-secret-toggle"
                  onClick={() => setApiKeyVisible((visible) => !visible)}
                  disabled={submitting}
                  aria-pressed={apiKeyVisible}
                  aria-label={
                    apiKeyVisible
                      ? t('openCodeGo.add.hideKey', 'Hide API key')
                      : t('openCodeGo.add.showKey', 'Show API key')
                  }
                >
                  {apiKeyVisible ? <EyeOff size={17} /> : <Eye size={17} />}
                </button>
              </div>
              {apiKeyErrorText && (
                <span id={apiKeyErrorId} className="opencode-go-add-field-error" role="alert">
                  {apiKeyErrorText}
                </span>
              )}
            </div>
          </div>

          <div className="modal-footer">
            <button type="button" className="btn btn-secondary" onClick={close} disabled={submitting}>
              {t('common.cancel', 'Cancel')}
            </button>
            <button type="submit" className="btn btn-primary" disabled={submitting}>
              {submitting
                ? t('openCodeGo.add.saving', 'Saving...')
                : t('openCodeGo.add.submit', 'Add connection')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export {
  describeOpenCodeGoAddError,
  initialOpenCodeGoAddAccountForm,
  submitOpenCodeGoAddAccount,
  validateOpenCodeGoAddAccount,
} from './openCodeGoAddAccountForm';
