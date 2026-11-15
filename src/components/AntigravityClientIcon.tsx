import cliIcon from '../assets/icons/antigravity-cli.svg';
import { ANTIGRAVITY_CLIENT_LABELS, ANTIGRAVITY_DESKTOP_CLIENTS, type AntigravityLaunchClient, type AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';
import { AntigravityIcon } from './icons/AntigravityIcon';
import { AntigravityIdeIcon } from './icons/AntigravityIdeIcon';
import './AntigravityClientTabs.css';

export function AntigravityClientIcon({ client }: { client: AntigravityLaunchClient }) {
  if (client === 'cli') return <img src={cliIcon} className="antigravity-client-icon" alt="" aria-hidden="true" />;
  if (client === 'antigravity_ide') return <AntigravityIdeIcon className="antigravity-client-icon" />;
  return <AntigravityIcon className="antigravity-client-icon" />;
}

export function AntigravityClientTabs({
  selected,
  onSelect,
  disabled = false,
  label = ANTIGRAVITY_CLIENT_LABELS.antigravity,
}: {
  selected: AntigravityRuntimeTarget;
  onSelect: (client: AntigravityRuntimeTarget) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <div className="antigravity-client-tabs" role="group" aria-label={label}>
      {ANTIGRAVITY_DESKTOP_CLIENTS.map((client) => (
        <button type="button" key={client} className={`btn ${selected === client ? 'btn-primary' : 'btn-secondary'}`}
          aria-pressed={selected === client} disabled={disabled} onClick={() => onSelect(client)}>
          <AntigravityClientIcon client={client} />
          <span>{ANTIGRAVITY_CLIENT_LABELS[client]}</span>
        </button>
      ))}
    </div>
  );
}
