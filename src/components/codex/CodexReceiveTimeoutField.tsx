import { useEffect, useId, useRef } from "react";
import { ModalErrorMessage } from "../ModalErrorMessage";

interface Props {
  label: string;
  description: string;
  value: string;
  error: { message: string } | null;
  onChange: (value: string) => void;
}

export function CodexReceiveTimeoutField({ label, description, value, error, onChange }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const descriptionId = useId();
  useEffect(() => {
    if (error) {
      inputRef.current?.focus({ preventScroll: true });
    }
  }, [error]);

  return (
    <label>
      <span>{label}</span>
      <input
        type="number"
        min={1}
        max={600}
        step={0.001}
        ref={inputRef}
        aria-invalid={error !== null}
        aria-describedby={descriptionId}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <small id={descriptionId}>{description}</small>
      <ModalErrorMessage message={error?.message} />
    </label>
  );
}
