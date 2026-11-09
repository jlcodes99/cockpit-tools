import assert from "node:assert/strict";
import test from "node:test";
import { deferred, loadHookModule, settlePromises } from "../../../tests/helpers/reactHookHarness";

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value && typeof value === "object" ? [value, ...nodes(value.props?.children)] : [];
}

test("a failed cleanup leaves random port recovery available in the same dialog", async () => {
  const cleanup = deferred<never>();
  const replacement = deferred<{ collection: { port: number } }>();
  const calls: string[] = [];
  const results: unknown[] = [];
  let dialogError: string | null = null;
  const harness = loadHookModule(new URL("./CodexLocalAccessPortRecovery.tsx", import.meta.url), {
    "react-dom": { createPortal: (tree: unknown) => tree },
    "react-i18next": { useTranslation: () => ({ t: (key: string) => key }) },
    "../../hooks/useEscClose": { useEscCloseTopmost() {} },
    "../../hooks/useModalFocusTrap": { useModalFocusTrap() {} },
    "../../services/codexLocalAccessService": {
      killCodexLocalAccessPort: () => { calls.push("cleanup"); return cleanup.promise; },
      changeCodexLocalAccessToRandomPort: () => { calls.push("random"); return replacement.promise; },
    },
    "../../utils/codexGatewayError": { formatCodexGatewayModeError: (_t: unknown, error: Error) => error.message },
    "../../utils/windowsOperationError": { parseWindowsOperationError: () => null },
    "../ModalErrorMessage": {
      ModalErrorMessage: "modal-error",
      useModalErrorState: () => ({
        message: dialogError,
        clear: () => { dialogError = null; },
        report: (message: string) => { dialogError = message; },
      }),
    },
  }, { document: { body: {} } });
  const render = () => harness.flush();
  const button = (key: string) => nodes(render().portRecoveryDialog)
    .find((node) => node.type === "button" && [node.props.children].flat().includes(key));

  harness.render(() => harness.exports.useCodexLocalAccessPortRecovery(57077,
    (state: unknown) => results.push(state)));
  render().requestPortRecovery();
  assert.deepEqual(calls, []);
  button("codex.localAccess.killPortAction").props.onClick();
  assert.equal(button("codex.localAccess.randomPortAction").props.disabled, true);
  cleanup.reject(new Error("The current port is still occupied"));
  await settlePromises();
  assert.equal(nodes(render().portRecoveryDialog)
    .find((node) => node.type === "modal-error").props.message, "The current port is still occupied");

  button("codex.localAccess.randomPortAction").props.onClick();
  assert.equal(dialogError, null);
  const state = { collection: { port: 51023 } };
  replacement.resolve(state);
  await settlePromises();
  assert.deepEqual(calls, ["cleanup", "random"]);
  assert.deepEqual(results, [state]);
  assert.equal(render().portRecoveryDialog, null);
});
