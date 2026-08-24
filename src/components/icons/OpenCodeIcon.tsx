type OpenCodeIconProps = {
  className?: string;
  size?: number;
};

/** Theme-adaptive rendering of OpenCode's official square mark. */
export function OpenCodeIcon({ className = '', size = 24 }: OpenCodeIconProps) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 300 300"
      aria-hidden="true"
      fill="none"
    >
      <g transform="translate(30 0)">
        <path d="M180 240H60V120H180V240Z" fill="currentColor" opacity="0.28" />
        <path
          d="M180 60H60V240H180V60ZM240 300H0V0H240V300Z"
          fill="currentColor"
        />
      </g>
    </svg>
  );
}
