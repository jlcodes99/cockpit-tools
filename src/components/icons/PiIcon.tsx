import { CSSProperties } from 'react';

type PiIconProps = {
  className?: string;
  style?: CSSProperties;
  size?: number;
};

/** Official pi coding agent mark (pi.dev). */
export function PiIcon({
  className = 'nav-item-icon',
  style,
  size = 20,
}: PiIconProps) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      fillRule="evenodd"
      height={size}
      style={style}
      viewBox="0 0 24 24"
      width={size}
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        clipRule="evenodd"
        d="M1 1h16.5v11H12v5.5H6.5V23H1V1zm5.5 5.5V12H12V6.5H6.5z"
      />
      <path d="M17.5 12H23v11h-5.5V12z" />
    </svg>
  );
}
