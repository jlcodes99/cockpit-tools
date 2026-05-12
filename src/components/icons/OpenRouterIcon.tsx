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
      {/* OpenRouter "O" stylized icon - rounded hexagon shape with inner O */}
      <path
        d="M12 2L20 6.5V17.5L12 22L4 17.5V6.5L12 2Z"
        stroke="currentColor"
        strokeWidth="1.5"
        fill="none"
      />
      <circle
        cx="12"
        cy="12"
        r="4"
        stroke="currentColor"
        strokeWidth="1.5"
        fill="none"
      />
    </svg>
  );
}
