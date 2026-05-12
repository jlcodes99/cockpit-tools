import { CSSProperties } from 'react';

type OpenRouterIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function OpenRouterIcon({ className = 'nav-item-icon', style }: OpenRouterIconProps) {
  return (
    <svg
      className={className}
      style={style}
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <path
        d="M4 4h7l9 9v7h-7l-9-9V4z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
        fill="none"
      />
      <path
        d="M11 11l2 2"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <circle cx="7.5" cy="7.5" r="1.5" fill="currentColor" />
      <circle cx="16.5" cy="16.5" r="1.5" fill="currentColor" />
    </svg>
  );
}
