import copilotIcon from '../../assets/icons/github-copilot.svg';

export function CopilotIcon({ className, size = 20 }: { className?: string; size?: number }) {
  return (
    <span
      className={className}
      aria-hidden="true"
      style={{
        display: 'inline-block',
        flexShrink: 0,
        width: size,
        height: size,
        backgroundColor: 'currentColor',
        mask: `url("${copilotIcon}") center / contain no-repeat`,
      }}
    />
  );
}
